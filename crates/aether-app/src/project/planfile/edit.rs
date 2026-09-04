//! Plan edit lowering. Version 1 text edits lower directly to one
//! `ProjectWrite`; version 2 may resolve semantic nodes and sequentially
//! re-project multiple edits into deterministic per-file writes. Only the
//! disposable candidate is changed here. Real-tree writes still go through
//! the project journal in `commit_project_writes`.

use crate::project::config::ProjectConfig;
use crate::project::planfile::schema::Edit;
use crate::project::source::{
    build_from_dir_with_config, collect_project_files_with_config, is_configured_source_path,
    safe_project_input_path, safe_project_output_path, ProjectWrite,
};
use aether_builder::{
    callee_identifier_spans, has_leading_declaration_metadata, identifier_spans, GraphBuilder,
};
use aether_graph::{Node, NodeKind, SemanticGraph};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Count non-overlapping, byte-exact occurrences of `needle` in `haystack`.
/// Empty needles never occur (avoids an infinite/degenerate count).
pub(crate) fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut start = 0;
    while let Some(offset) = haystack[start..].find(needle) {
        count += 1;
        start += offset + needle.len();
    }
    count
}

/// Turn one edit into a `ProjectWrite`, re-checking its exact-occurrence
/// contract at apply time (not just precondition time) — fails closed, no
/// fuzzy retry, if the file changed between precondition checking and
/// application.
pub(crate) fn apply_edit(base_dir: &Path, edit: &Edit) -> std::io::Result<ProjectWrite> {
    match edit {
        Edit::Substitute {
            path,
            match_text,
            replace,
            occurrences,
        } => {
            let target = safe_project_input_path(base_dir, path)?;
            let contents = std::fs::read_to_string(&target).map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!("could not read {path} to apply edit: {error}"),
                )
            })?;
            let found = count_occurrences(&contents, match_text);
            let expected = *occurrences as usize;
            if found != expected {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "{path} expects {expected} occurrence(s) of the given match text, \
                         found {found}; refusing to guess"
                    ),
                ));
            }
            let new_contents = contents.replace(match_text.as_str(), replace);
            Ok(ProjectWrite::text(
                path.clone(),
                Some(contents.into_bytes()),
                new_contents,
            ))
        }
        Edit::Create { path, create } => {
            let target = safe_project_output_path(base_dir, path)?;
            if target.exists() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!("{path} already exists; create edits never overwrite"),
                ));
            }
            Ok(ProjectWrite::text(path.clone(), None, create.clone()))
        }
        Edit::Delete { path, delete } => {
            if !*delete {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("{path}: a delete edit must set \"delete\": true"),
                ));
            }
            let target = safe_project_input_path(base_dir, path)?;
            let contents = std::fs::read(&target).map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!("could not read {path} to delete it: {error}"),
                )
            })?;
            Ok(ProjectWrite::delete(path.clone(), contents))
        }
        Edit::ReplaceNode { .. }
        | Edit::RenameNode { .. }
        | Edit::DeleteNode { .. }
        | Edit::InsertIntoModule { .. } => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "graph-addressed edits require Plan Format v2 lowering",
        )),
    }
}

/// Physically write a `ProjectWrite` into `dir` (the disposable copy).
/// Real-tree writes always go through `commit_project_writes` instead —
/// this is only for making a copy's on-disk state match what the step
/// would commit, so checks can run against it.
pub(crate) fn write_into(dir: &Path, write: &ProjectWrite) -> std::io::Result<()> {
    let target = dir.join(write.relative());
    if let Some(contents) = write.contents() {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, contents)
    } else {
        std::fs::remove_file(&target)
    }
}

struct GraphWorkspace {
    graph: SemanticGraph,
    builder: GraphBuilder,
}

#[derive(Default)]
pub(crate) struct EditState {
    workspace: Option<GraphWorkspace>,
    renames: HashMap<String, (String, String)>,
}

impl EditState {
    pub(crate) fn ensure_graph(
        &mut self,
        root: &Path,
        config: &ProjectConfig,
    ) -> std::io::Result<()> {
        if self.workspace.is_none() {
            let (graph, builder, _) = build_from_dir_with_config(root, config)?;
            self.workspace = Some(GraphWorkspace { graph, builder });
        }
        Ok(())
    }

    pub(crate) fn graph(&self) -> Option<&SemanticGraph> {
        self.workspace.as_ref().map(|workspace| &workspace.graph)
    }

    fn refresh_file(
        &mut self,
        root: &Path,
        config: &ProjectConfig,
        file: &str,
    ) -> std::io::Result<()> {
        if !is_configured_source_path(config, file)? {
            return Ok(());
        }
        let source = match std::fs::read_to_string(root.join(file)) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if self.workspace.is_some() {
                    let (graph, builder, _) = build_from_dir_with_config(root, config)?;
                    self.workspace = Some(GraphWorkspace { graph, builder });
                }
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let Some(workspace) = self.workspace.as_mut() else {
            return Ok(());
        };
        workspace
            .builder
            .update_file(&mut workspace.graph, file, &source);
        Ok(())
    }
}

#[derive(Default)]
struct StepAccumulator {
    before: BTreeMap<PathBuf, Option<Vec<u8>>>,
}

impl StepAccumulator {
    fn capture(&mut self, root: &Path, relative: &Path) -> std::io::Result<()> {
        if self.before.contains_key(relative) {
            return Ok(());
        }
        let bytes = match std::fs::read(root.join(relative)) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        self.before.insert(relative.to_path_buf(), bytes);
        Ok(())
    }

    fn finish(self, root: &Path) -> std::io::Result<Vec<ProjectWrite>> {
        let mut writes = Vec::new();
        for (relative, before) in self.before {
            let after = match std::fs::read(root.join(&relative)) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            if before == after {
                continue;
            }
            match after {
                Some(contents) => writes.push(ProjectWrite::bytes(relative, before, contents)),
                None => {
                    let expected = before.ok_or_else(|| {
                        std::io::Error::other("a missing file cannot be deleted twice")
                    })?;
                    writes.push(ProjectWrite::delete(relative, expected));
                }
            }
        }
        Ok(writes)
    }
}

#[derive(Clone)]
struct Splice {
    node: String,
    file: String,
    start: usize,
    end: usize,
    expected: String,
    replacement: String,
}

/// Apply one v2 step sequentially to its disposable candidate and coalesce all
/// intermediate mutations into one deterministic `ProjectWrite` per file.
pub(crate) fn apply_step_edits_v2(
    root: &Path,
    config: &ProjectConfig,
    step_id: &str,
    edits: &[Edit],
    state: &mut EditState,
) -> std::io::Result<Vec<ProjectWrite>> {
    let mut accumulator = StepAccumulator::default();
    for (index, edit) in edits.iter().enumerate() {
        let result: std::io::Result<()> = (|| {
            if edit.is_graph_addressed() {
                state.ensure_graph(root, config)?;
                apply_graph_edit(root, config, step_id, edit, state, &mut accumulator)
            } else {
                let path = edit.path().expect("text edits have paths");
                let normalized = Path::new(path)
                    .components()
                    .map(|component| match component {
                        std::path::Component::Normal(value) => {
                            Some(value.to_string_lossy().into_owned())
                        }
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| {
                        invalid(format!(
                            "v2 text edit path {path:?} must be a normalized project-relative path"
                        ))
                    })?
                    .join("/");
                if normalized != path || path.contains('\\') {
                    return Err(invalid(format!(
                        "v2 text edit path {path:?} must be a normalized project-relative path"
                    )));
                }
                let write = apply_edit(root, edit)?;
                accumulator.capture(root, write.relative())?;
                write_into(root, &write)?;
                state.refresh_file(root, config, path)?;
                Ok(())
            }
        })();
        if let Err(error) = result {
            let target = edit
                .node()
                .or_else(|| edit.path())
                .unwrap_or("<missing target>");
            return Err(std::io::Error::new(
                error.kind(),
                format!("step {step_id}: edits[{index}] target {target:?}: {error}"),
            ));
        }
    }
    accumulator.finish(root)
}

fn apply_graph_edit(
    root: &Path,
    config: &ProjectConfig,
    step_id: &str,
    edit: &Edit,
    state: &mut EditState,
    accumulator: &mut StepAccumulator,
) -> std::io::Result<()> {
    let node_path = edit
        .node()
        .ok_or_else(|| std::io::Error::other("missing node path"))?;
    match edit {
        Edit::ReplaceNode { replacement, .. } => {
            let node = resolve_node(root, config, state, node_path)?;
            if !matches!(node.kind, NodeKind::Function | NodeKind::Type) {
                return Err(invalid(format!(
                    "node {node_path:?} is {:?}; replace_node requires a function or type with a complete source projection",
                    node.kind
                )));
            }
            let splice = splice_for_node(root, config, &node, replacement.clone(), false)?;
            apply_splices(root, config, state, accumulator, vec![splice])?;
            require_occurrences(state, node_path, 1)?;
        }
        Edit::DeleteNode { delete, .. } => {
            if !*delete {
                return Err(invalid(format!(
                    "node {node_path:?}: delete_node must be true"
                )));
            }
            let node = resolve_node(root, config, state, node_path)?;
            if !matches!(node.kind, NodeKind::Function | NodeKind::Type) {
                return Err(invalid(format!(
                    "node {node_path:?} is {:?}; delete_node requires a function or type with a complete source projection",
                    node.kind
                )));
            }
            let file = node.file.as_deref().ok_or_else(|| {
                invalid(format!("node {node_path:?} has no source projection file"))
            })?;
            let source = std::fs::read_to_string(root.join(file))?;
            if has_leading_declaration_metadata(&source, file, node.span.start_byte) {
                return Err(invalid(format!(
                    "node {node_path:?} has leading attributes, documentation, or decorators outside its projection; refusing delete_node because that metadata would be orphaned"
                )));
            }
            let splice = splice_for_node(root, config, &node, String::new(), false)?;
            apply_splices(root, config, state, accumulator, vec![splice])?;
            require_occurrences(state, node_path, 0)?;
        }
        Edit::InsertIntoModule { insertion, .. } => {
            let node = resolve_node(root, config, state, node_path)?;
            if node.kind != NodeKind::Module {
                return Err(invalid(format!(
                    "node {node_path:?} is {:?}; insert_into_module requires a module",
                    node.kind
                )));
            }
            let splice = splice_for_node(root, config, &node, insertion.clone(), true)?;
            apply_splices(root, config, state, accumulator, vec![splice])?;
            require_occurrences(state, node_path, 1)?;
        }
        Edit::RenameNode { new_name, .. } => {
            validate_new_name(node_path, new_name)?;
            let node = resolve_node(root, config, state, node_path)?;
            if node.kind != NodeKind::Function {
                return Err(invalid(format!(
                    "node {node_path:?} is {:?}; rename_node currently requires a function",
                    node.kind
                )));
            }
            let old_name = node.name.clone();
            let same_named = state
                .workspace
                .as_ref()
                .expect("graph ensured above")
                .graph
                .query_by_kind(NodeKind::Function)
                .into_iter()
                .filter(|candidate| candidate.name == old_name)
                .count();
            if same_named != 1 {
                return Err(invalid(format!(
                    "node {node_path:?} cannot be renamed safely: {same_named} functions share the identifier {old_name:?}"
                )));
            }
            let original = state
                .workspace
                .as_ref()
                .expect("graph ensured above")
                .graph
                .clone();
            let outcome = state
                .workspace
                .as_mut()
                .expect("graph ensured above")
                .graph
                .rename_node(node.id, new_name)
                .map_err(|error| {
                    invalid(format!("could not rename node {node_path:?}: {error}"))
                })?;
            let caller_nodes: Vec<Node> = outcome
                .updated_callers
                .iter()
                .map(|caller_path| {
                    original.find_by_path(caller_path).cloned().ok_or_else(|| {
                        invalid(format!(
                            "rename caller {caller_path:?} disappeared from the old graph"
                        ))
                    })
                })
                .collect::<std::io::Result<_>>()?;
            let splices =
                safe_rename_splices(root, config, &original, &node, &caller_nodes, new_name)?;
            apply_splices(root, config, state, accumulator, splices)?;
            require_occurrences(state, node_path, 0)?;
            require_occurrences(state, &outcome.new_path, 1)?;
            state.renames.insert(
                node_path.to_string(),
                (outcome.new_path.clone(), step_id.to_string()),
            );
        }
        Edit::Substitute { .. } | Edit::Create { .. } | Edit::Delete { .. } => {
            unreachable!("text edits are handled by apply_step_edits_v2")
        }
    }
    Ok(())
}

fn resolve_node(
    root: &Path,
    config: &ProjectConfig,
    state: &EditState,
    path: &str,
) -> std::io::Result<Node> {
    if let Some((new_path, step)) = state.renames.get(path) {
        return Err(invalid(format!(
            "node path {path:?} was renamed in step {step:?} to {new_path:?}; use the new path"
        )));
    }
    let workspace = state.workspace.as_ref().expect("graph must be initialized");
    let occurrences = workspace.builder.path_occurrences(path);
    if occurrences > 1 {
        return Err(invalid(format!(
            "ambiguous node path {path:?}: {occurrences} parsed declarations claim it"
        )));
    }
    let exact: Vec<&Node> = workspace
        .graph
        .nodes()
        .filter(|node| node.path == path)
        .collect();
    if occurrences == 0 || exact.is_empty() {
        if let Some(file) = unsupported_projection_for(root, config, path)? {
            return Err(invalid(format!(
                "node path {path:?} addresses unsupported-language file {file:?}; graph edits support only Rust and Python"
            )));
        }
        return Err(invalid(format!(
            "unknown node path {path:?}; graph edits address only parsed Rust/Python projections"
        )));
    }
    if exact.len() != 1 {
        return Err(invalid(format!(
            "ambiguous node path {path:?}: {} exact graph nodes match",
            exact.len()
        )));
    }
    let node = exact[0];
    if node.id != aether_graph::NodeId::from_path(path) {
        return Err(invalid(format!(
            "node path {path:?} has an inconsistent path-derived identity"
        )));
    }
    Ok(node.clone())
}

fn unsupported_projection_for(
    root: &Path,
    config: &ProjectConfig,
    node_path: &str,
) -> std::io::Result<Option<String>> {
    for (_, relative) in collect_project_files_with_config(root, config)? {
        if crate::project::source::is_supported_source_path(&relative) {
            continue;
        }
        let module = aether_builder::module_path_for(&relative);
        if node_path == module || node_path.starts_with(&format!("{module}::")) {
            return Ok(Some(relative));
        }
    }
    Ok(None)
}

fn require_occurrences(state: &EditState, path: &str, expected: usize) -> std::io::Result<()> {
    let workspace = state.workspace.as_ref().expect("graph must be initialized");
    let actual = workspace.builder.path_occurrences(path);
    if actual != expected {
        return Err(invalid(format!(
            "node path {path:?} resolved {actual} time(s) after lowering; expected {expected}"
        )));
    }
    Ok(())
}

fn splice_for_node(
    root: &Path,
    config: &ProjectConfig,
    node: &Node,
    replacement: String,
    insertion: bool,
) -> std::io::Result<Splice> {
    validate_projection(root, config, node)?;
    let file = node.file.clone().expect("validated above");
    let (start, end, expected) = if insertion {
        (node.span.end_byte, node.span.end_byte, String::new())
    } else {
        (
            node.span.start_byte,
            node.span.end_byte,
            node.source.clone(),
        )
    };
    Ok(Splice {
        node: node.path.clone(),
        file,
        start,
        end,
        expected,
        replacement,
    })
}

fn safe_rename_splices(
    root: &Path,
    config: &ProjectConfig,
    graph: &SemanticGraph,
    definition: &Node,
    callers: &[Node],
    new_name: &str,
) -> std::io::Result<Vec<Splice>> {
    validate_projection(root, config, definition)?;
    let old_name = &definition.name;
    let mut all_identifiers: BTreeMap<(String, usize, usize), ()> = BTreeMap::new();
    let mut call_identifiers: BTreeMap<(String, usize, usize), ()> = BTreeMap::new();
    let files: std::collections::BTreeSet<String> =
        graph.nodes().filter_map(|node| node.file.clone()).collect();
    for file in files {
        let source = std::fs::read_to_string(root.join(&file))?;
        for (start, end) in identifier_spans(&source, &file, old_name) {
            all_identifiers.insert((file.clone(), start, end), ());
        }
        let callee_spans = callee_identifier_spans(&source, &file, old_name);
        for caller in callers
            .iter()
            .filter(|caller| caller.file.as_deref() == Some(&file))
        {
            let within = callee_spans
                .iter()
                .filter(|(start, end)| {
                    *start >= caller.span.start_byte && *end <= caller.span.end_byte
                })
                .count();
            if within != 1 {
                return Err(invalid(format!(
                    "node {:?} cannot be renamed safely: graph-proven caller {:?} contains {within} matching callee sites; call-site provenance is ambiguous",
                    definition.path, caller.path
                )));
            }
        }
        if definition.file.as_deref() == Some(&file) {
            let recursive_sites = callee_spans
                .iter()
                .filter(|(start, end)| {
                    *start >= definition.span.start_byte && *end <= definition.span.end_byte
                })
                .count();
            if recursive_sites > 0 {
                return Err(invalid(format!(
                    "node {:?} cannot be renamed safely: its definition contains {recursive_sites} same-named callee sites; recursive call-site provenance is not represented",
                    definition.path
                )));
            }
        }
        for (start, end) in callee_spans {
            let proven = callers.iter().any(|caller| {
                caller.file.as_deref() == Some(file.as_str())
                    && start >= caller.span.start_byte
                    && end <= caller.span.end_byte
            });
            if proven {
                call_identifiers.insert((file.clone(), start, end), ());
            }
        }
    }
    let definition_file = definition.file.clone().expect("validated above");
    let definition_name = all_identifiers
        .keys()
        .find(|(file, start, end)| {
            file == &definition_file
                && *start >= definition.span.start_byte
                && *end <= definition.span.end_byte
                && !call_identifiers.contains_key(&(file.clone(), *start, *end))
        })
        .cloned()
        .ok_or_else(|| {
            invalid(format!(
                "node {:?} has no syntax-verified definition identifier",
                definition.path
            ))
        })?;
    let mut rewrite = call_identifiers;
    rewrite.insert(definition_name, ());
    if rewrite.keys().ne(all_identifiers.keys()) {
        let unaccounted: Vec<String> = all_identifiers
            .keys()
            .filter(|span| !rewrite.contains_key(*span))
            .map(|(file, start, end)| format!("{file}:{start}..{end}"))
            .collect();
        return Err(invalid(format!(
            "node {:?} cannot be renamed safely: identifier occurrences outside its definition and graph-proven call sites at {}",
            definition.path,
            unaccounted.join(", ")
        )));
    }
    Ok(rewrite
        .into_keys()
        .map(|(file, start, end)| Splice {
            node: definition.path.clone(),
            file,
            start,
            end,
            expected: old_name.clone(),
            replacement: new_name.to_string(),
        })
        .collect())
}

fn validate_projection(root: &Path, config: &ProjectConfig, node: &Node) -> std::io::Result<()> {
    if !matches!(node.language.as_str(), "rust" | "python") {
        return Err(invalid(format!(
            "node {:?} uses unsupported language {:?}; graph edits support only Rust and Python",
            node.path, node.language
        )));
    }
    let file = node.file.as_deref().ok_or_else(|| {
        invalid(format!(
            "node {:?} has no source projection file",
            node.path
        ))
    })?;
    if !is_configured_source_path(config, file)? {
        return Err(invalid(format!(
            "node {:?} projects to unsupported or unconfigured source file {file:?}; graph edits support only configured Rust and Python files",
            node.path
        )));
    }
    let target = safe_project_input_path(root, file)?;
    let source = std::fs::read_to_string(&target).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!(
                "could not read projection {file:?} for node {:?}: {error}",
                node.path
            ),
        )
    })?;
    let span = node.span;
    if span.start_byte >= span.end_byte {
        return Err(invalid(format!(
            "node {:?} has an empty or reversed span {}..{}",
            node.path, span.start_byte, span.end_byte
        )));
    }
    if span.end_byte > source.len()
        || !source.is_char_boundary(span.start_byte)
        || !source.is_char_boundary(span.end_byte)
    {
        return Err(invalid(format!(
            "node {:?} has an out-of-bounds or non-UTF-8 span {}..{} for {file:?}",
            node.path, span.start_byte, span.end_byte
        )));
    }
    if source[span.start_byte..span.end_byte] != node.source {
        return Err(invalid(format!(
            "node {:?} has a stale projection span in {file:?}; source bytes do not match the graph",
            node.path
        )));
    }
    Ok(())
}

fn apply_splices(
    root: &Path,
    config: &ProjectConfig,
    state: &mut EditState,
    accumulator: &mut StepAccumulator,
    splices: Vec<Splice>,
) -> std::io::Result<()> {
    let mut by_file: BTreeMap<String, Vec<Splice>> = BTreeMap::new();
    for splice in splices {
        by_file.entry(splice.file.clone()).or_default().push(splice);
    }
    let files: Vec<String> = by_file.keys().cloned().collect();
    for (file, mut file_splices) in by_file {
        file_splices.sort_by_key(|splice| (splice.start, splice.end));
        for pair in file_splices.windows(2) {
            if pair[0].end > pair[1].start {
                return Err(invalid(format!(
                    "overlapping graph edits in {file:?}: node {:?} span {}..{} overlaps node {:?} span {}..{}",
                    pair[0].node,
                    pair[0].start,
                    pair[0].end,
                    pair[1].node,
                    pair[1].start,
                    pair[1].end
                )));
            }
        }
        let relative = PathBuf::from(&file);
        accumulator.capture(root, &relative)?;
        let mut source = std::fs::read_to_string(root.join(&relative))?;
        for splice in file_splices.into_iter().rev() {
            if splice.end > source.len()
                || !source.is_char_boundary(splice.start)
                || !source.is_char_boundary(splice.end)
                || source[splice.start..splice.end] != splice.expected
            {
                return Err(invalid(format!(
                    "node {:?} projection changed while lowering; refusing stale span {}..{} in {file:?}",
                    splice.node, splice.start, splice.end
                )));
            }
            source.replace_range(splice.start..splice.end, &splice.replacement);
        }
        std::fs::write(root.join(&relative), source)?;
    }
    for file in files {
        state.refresh_file(root, config, &file)?;
    }
    Ok(())
}

fn validate_new_name(path: &str, new_name: &str) -> std::io::Result<()> {
    let mut chars = new_name.chars();
    let valid = chars
        .next()
        .is_some_and(|first| first == '_' || first.is_alphabetic())
        && chars.all(|character| character == '_' || character.is_alphanumeric());
    if !valid {
        return Err(invalid(format!(
            "node {path:?} rename target {new_name:?} is not one identifier"
        )));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_non_overlapping_matches() {
        assert_eq!(count_occurrences("aaaa", "aa"), 2);
        assert_eq!(count_occurrences("abcabcabc", "abc"), 3);
    }

    #[test]
    fn zero_when_absent() {
        assert_eq!(count_occurrences("hello world", "goodbye"), 0);
    }

    #[test]
    fn empty_needle_counts_as_zero_not_infinite() {
        assert_eq!(count_occurrences("hello", ""), 0);
    }

    #[test]
    fn respects_utf8_boundaries() {
        // "café" repeated; needle "é" must match exactly twice, not corrupt
        // byte offsets across the multi-byte character.
        assert_eq!(count_occurrences("café café", "é"), 2);
        assert_eq!(count_occurrences("café café", "caf"), 2);
    }

    #[test]
    fn counts_across_multiline_text() {
        let haystack = "line one\nline two\nline one\n";
        assert_eq!(count_occurrences(haystack, "line one\n"), 2);
    }

    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "girder-planfile-edit-{name}-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn substitute_edit_produces_the_expected_project_write() {
        let dir = TempDir::new("substitute");
        std::fs::write(dir.0.join("a.rs"), "fn old() {}\n").unwrap();
        let edit = Edit::Substitute {
            path: "a.rs".into(),
            match_text: "old".into(),
            replace: "new".into(),
            occurrences: 1,
        };
        let write = apply_edit(&dir.0, &edit).unwrap();
        assert_eq!(write.expected(), Some(b"fn old() {}\n".as_slice()));
        assert_eq!(write.contents(), Some(b"fn new() {}\n".as_slice()));
    }

    #[test]
    fn substitute_edit_fails_closed_on_wrong_occurrence_count_and_does_not_retry() {
        let dir = TempDir::new("substitute-mismatch");
        std::fs::write(dir.0.join("a.rs"), "fn a() {} fn a() {}\n").unwrap();
        let edit = Edit::Substitute {
            path: "a.rs".into(),
            match_text: "fn a()".into(),
            replace: "fn b()".into(),
            occurrences: 1,
        };
        let error = apply_edit(&dir.0, &edit).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("expects 1"), "{error}");
        assert!(error.to_string().contains("found 2"), "{error}");
        // The source file itself is never touched by a failed apply_edit call.
        assert_eq!(
            std::fs::read_to_string(dir.0.join("a.rs")).unwrap(),
            "fn a() {} fn a() {}\n"
        );
    }

    #[test]
    fn create_edit_fails_if_the_path_already_exists() {
        let dir = TempDir::new("create-exists");
        std::fs::write(dir.0.join("a.rs"), "present\n").unwrap();
        let edit = Edit::Create {
            path: "a.rs".into(),
            create: "fn new() {}\n".into(),
        };
        let error = apply_edit(&dir.0, &edit).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn create_edit_produces_a_write_with_no_expected_bytes() {
        let dir = TempDir::new("create-new");
        let edit = Edit::Create {
            path: "new.rs".into(),
            create: "fn new() {}\n".into(),
        };
        let write = apply_edit(&dir.0, &edit).unwrap();
        assert_eq!(write.expected(), None);
        assert_eq!(write.contents(), Some(b"fn new() {}\n".as_slice()));
    }

    #[test]
    fn delete_edit_captures_the_old_bytes() {
        let dir = TempDir::new("delete");
        std::fs::write(dir.0.join("gone.rs"), "bye\n").unwrap();
        let edit = Edit::Delete {
            path: "gone.rs".into(),
            delete: true,
        };
        let write = apply_edit(&dir.0, &edit).unwrap();
        assert_eq!(write.expected(), Some(b"bye\n".as_slice()));
        assert_eq!(write.contents(), None);
    }

    #[test]
    fn write_into_applies_all_three_kinds_to_disk() {
        let dir = TempDir::new("write-into");
        std::fs::write(dir.0.join("keep.rs"), "old\n").unwrap();
        std::fs::write(dir.0.join("gone.rs"), "bye\n").unwrap();

        let substitute = apply_edit(
            &dir.0,
            &Edit::Substitute {
                path: "keep.rs".into(),
                match_text: "old".into(),
                replace: "new".into(),
                occurrences: 1,
            },
        )
        .unwrap();
        let create = apply_edit(
            &dir.0,
            &Edit::Create {
                path: "created.rs".into(),
                create: "fresh\n".into(),
            },
        )
        .unwrap();
        let delete = apply_edit(
            &dir.0,
            &Edit::Delete {
                path: "gone.rs".into(),
                delete: true,
            },
        )
        .unwrap();

        write_into(&dir.0, &substitute).unwrap();
        write_into(&dir.0, &create).unwrap();
        write_into(&dir.0, &delete).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.0.join("keep.rs")).unwrap(),
            "new\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.0.join("created.rs")).unwrap(),
            "fresh\n"
        );
        assert!(!dir.0.join("gone.rs").exists());
    }

    fn v2_fixture(name: &str, file: &str, source: &str) -> (TempDir, ProjectConfig) {
        let dir = TempDir::new(name);
        let target = dir.0.join(file);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, source).unwrap();
        (dir, ProjectConfig::default())
    }

    #[test]
    fn graph_edits_in_one_file_re_resolve_shifted_spans() {
        let (dir, config) = v2_fixture(
            "graph-shift",
            "src/lib.rs",
            "pub fn first() -> i32 { 1 }\npub fn second() -> i32 { 2 }\n",
        );
        let edits = vec![
            Edit::ReplaceNode {
                node: "crate::lib::first".into(),
                replacement: "pub fn first() -> i32 { 111111 }".into(),
            },
            Edit::ReplaceNode {
                node: "crate::lib::second".into(),
                replacement: "pub fn second() -> i32 { 222222 }".into(),
            },
        ];
        let mut state = EditState::default();
        let writes = apply_step_edits_v2(&dir.0, &config, "shift", &edits, &mut state).unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(
            std::fs::read_to_string(dir.0.join("src/lib.rs")).unwrap(),
            "pub fn first() -> i32 { 111111 }\npub fn second() -> i32 { 222222 }\n"
        );
    }

    #[test]
    fn deleting_a_source_file_removes_its_module_from_the_live_graph() {
        let (dir, config) = v2_fixture("graph-delete-file", "src/old.rs", "pub fn old() {}\n");
        std::fs::write(dir.0.join("src/keep.rs"), "pub fn keep() {}\n").unwrap();
        let mut state = EditState::default();
        state.ensure_graph(&dir.0, &config).unwrap();
        apply_step_edits_v2(
            &dir.0,
            &config,
            "delete-file",
            &[Edit::Delete {
                path: "src/old.rs".into(),
                delete: true,
            }],
            &mut state,
        )
        .unwrap();
        assert!(state.graph().unwrap().find_by_path("crate::old").is_none());
        assert!(state.graph().unwrap().find_by_path("crate::keep").is_some());
    }

    #[test]
    fn nested_graph_projections_are_re_resolved_after_parent_replacement() {
        let (dir, config) = v2_fixture(
            "graph-nested-projections",
            "src/lib.rs",
            "pub trait Boxed {\n    fn nested() -> i32 { 1 }\n}\n",
        );
        let edits = vec![
            Edit::ReplaceNode {
                node: "crate::lib::Boxed".into(),
                replacement: "pub trait Boxed {\n    fn nested() -> i32 { 444444 }\n}".into(),
            },
            Edit::ReplaceNode {
                node: "crate::lib::Boxed::nested".into(),
                replacement: "fn nested() -> i32 { 555555 }".into(),
            },
        ];
        let mut state = EditState::default();
        apply_step_edits_v2(&dir.0, &config, "nested", &edits, &mut state).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.0.join("src/lib.rs")).unwrap(),
            "pub trait Boxed {\n    fn nested() -> i32 { 555555 }\n}\n"
        );
    }

    #[test]
    fn rename_updates_definition_and_graph_proven_callers() {
        let (dir, config) = v2_fixture(
            "graph-rename",
            "src/lib.rs",
            "pub fn target() -> i32 { 1 }\npub fn caller() -> i32 { target() }\n",
        );
        let mut state = EditState::default();
        apply_step_edits_v2(
            &dir.0,
            &config,
            "rename",
            &[Edit::RenameNode {
                node: "crate::lib::target".into(),
                new_name: "renamed".into(),
            }],
            &mut state,
        )
        .unwrap();
        let source = std::fs::read_to_string(dir.0.join("src/lib.rs")).unwrap();
        assert!(source.contains("fn renamed()"), "{source}");
        assert!(source.contains("renamed() }"), "{source}");

        let error = apply_step_edits_v2(
            &dir.0,
            &config,
            "later",
            &[Edit::DeleteNode {
                node: "crate::lib::target".into(),
                delete: true,
            }],
            &mut state,
        )
        .unwrap_err();
        assert!(error.to_string().contains("renamed in step"), "{error}");
        assert!(error.to_string().contains("crate::lib::renamed"), "{error}");
    }

    #[test]
    fn duplicate_and_empty_span_targets_fail_closed_with_context() {
        let (duplicates, config) = v2_fixture(
            "graph-duplicate",
            "src/lib.py",
            "def target():\n    return 1\n\ndef target():\n    return 2\n",
        );
        let mut state = EditState::default();
        let error = apply_step_edits_v2(
            &duplicates.0,
            &config,
            "ambiguous-step",
            &[Edit::DeleteNode {
                node: "crate::lib::target".into(),
                delete: true,
            }],
            &mut state,
        )
        .unwrap_err();
        let diagnostic = error.to_string();
        assert!(diagnostic.contains("ambiguous-step"), "{diagnostic}");
        assert!(diagnostic.contains("crate::lib::target"), "{diagnostic}");
        assert!(diagnostic.contains("ambiguous"), "{diagnostic}");

        let (empty, config) = v2_fixture("graph-empty", "src/lib.rs", "");
        let mut state = EditState::default();
        let error = apply_step_edits_v2(
            &empty.0,
            &config,
            "spanless-step",
            &[Edit::InsertIntoModule {
                node: "crate::lib".into(),
                insertion: "fn added() {}\n".into(),
            }],
            &mut state,
        )
        .unwrap_err();
        let diagnostic = error.to_string();
        assert!(diagnostic.contains("spanless-step"), "{diagnostic}");
        assert!(diagnostic.contains("crate::lib"), "{diagnostic}");
        assert!(diagnostic.contains("empty"), "{diagnostic}");
    }

    #[test]
    fn unknown_and_fileless_targets_fail_closed_with_step_and_node() {
        let (dir, config) = v2_fixture(
            "graph-resolution-failures",
            "src/lib.rs",
            "fn target() {}\n",
        );
        let mut state = EditState::default();
        let error = apply_step_edits_v2(
            &dir.0,
            &config,
            "unknown-step",
            &[Edit::DeleteNode {
                node: "crate::lib::missing".into(),
                delete: true,
            }],
            &mut state,
        )
        .unwrap_err();
        let diagnostic = error.to_string();
        assert!(diagnostic.contains("unknown-step"), "{diagnostic}");
        assert!(diagnostic.contains("crate::lib::missing"), "{diagnostic}");
        assert!(diagnostic.contains("unknown node path"), "{diagnostic}");

        state.ensure_graph(&dir.0, &config).unwrap();
        state.workspace.as_mut().unwrap().graph.upsert_node(
            Node::new(NodeKind::Function, "target", "crate::lib::target")
                .with_language("rust")
                .with_source("fn target() {}"),
        );
        let error = apply_step_edits_v2(
            &dir.0,
            &config,
            "fileless-step",
            &[Edit::ReplaceNode {
                node: "crate::lib::target".into(),
                replacement: "fn target() { panic!() }".into(),
            }],
            &mut state,
        )
        .unwrap_err();
        let diagnostic = error.to_string();
        assert!(diagnostic.contains("fileless-step"), "{diagnostic}");
        assert!(diagnostic.contains("crate::lib::target"), "{diagnostic}");
        assert!(
            diagnostic.contains("no source projection file"),
            "{diagnostic}"
        );
    }

    #[test]
    fn unsupported_language_targets_fail_closed_with_step_and_node() {
        let (dir, config) = v2_fixture(
            "graph-unsupported-language",
            "src/legacy.js",
            "function target() {}\n",
        );
        let mut state = EditState::default();
        let error = apply_step_edits_v2(
            &dir.0,
            &config,
            "unsupported-step",
            &[Edit::DeleteNode {
                node: "crate::legacy::target".into(),
                delete: true,
            }],
            &mut state,
        )
        .unwrap_err();
        let diagnostic = error.to_string();
        assert!(diagnostic.contains("unsupported-step"), "{diagnostic}");
        assert!(diagnostic.contains("crate::legacy::target"), "{diagnostic}");
        assert!(diagnostic.contains("unsupported-language"), "{diagnostic}");
    }

    #[test]
    fn delete_node_rejects_leading_attributes_and_decorators() {
        for (name, file, source) in [
            (
                "rust-attribute",
                "src/lib.rs",
                "#[test]\nfn target() {}\nfn next() {}\n",
            ),
            (
                "python-decorator",
                "src/lib.py",
                "@decorator\ndef target():\n    pass\n\ndef next():\n    pass\n",
            ),
        ] {
            let (dir, config) = v2_fixture(name, file, source);
            let mut state = EditState::default();
            let error = apply_step_edits_v2(
                &dir.0,
                &config,
                "decorated-delete",
                &[Edit::DeleteNode {
                    node: "crate::lib::target".into(),
                    delete: true,
                }],
                &mut state,
            )
            .unwrap_err();
            assert!(
                error.to_string().contains("metadata would be orphaned"),
                "{error}"
            );
            assert_eq!(std::fs::read_to_string(dir.0.join(file)).unwrap(), source);
        }
    }

    #[test]
    fn v2_text_paths_cannot_escape_or_alias_the_candidate() {
        let (dir, config) = v2_fixture("graph-safe-path", "src/lib.rs", "fn old() {}\n");
        for path in [
            "../outside.rs",
            "./src/lib.rs",
            "src/./lib.rs",
            "src//lib.rs",
            "src\\lib.rs",
        ] {
            let mut state = EditState::default();
            let error = apply_step_edits_v2(
                &dir.0,
                &config,
                "safe-path",
                &[Edit::Substitute {
                    path: path.into(),
                    match_text: "old".into(),
                    replace: "new".into(),
                    occurrences: 1,
                }],
                &mut state,
            )
            .unwrap_err();
            assert!(error.to_string().contains("safe-path"), "{error}");
        }
        assert_eq!(
            std::fs::read_to_string(dir.0.join("src/lib.rs")).unwrap(),
            "fn old() {}\n"
        );
    }

    #[test]
    fn rename_rejects_ambiguous_same_named_call_sites() {
        let (dir, config) = v2_fixture(
            "graph-rename-ambiguous-call",
            "src/lib.rs",
            "pub fn target() -> i32 { 1 }\npub fn caller() -> i32 { target() + object.target() }\n",
        );
        let mut state = EditState::default();
        let error = apply_step_edits_v2(
            &dir.0,
            &config,
            "ambiguous-call",
            &[Edit::RenameNode {
                node: "crate::lib::target".into(),
                new_name: "renamed".into(),
            }],
            &mut state,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("call-site provenance is ambiguous"),
            "{error}"
        );
    }

    #[test]
    fn rename_rejects_ambiguous_same_named_recursive_call_sites() {
        let (dir, config) = v2_fixture(
            "graph-rename-ambiguous-recursive",
            "src/lib.rs",
            "pub fn target() -> i32 { target() + object.target() }\n",
        );
        let mut state = EditState::default();
        let error = apply_step_edits_v2(
            &dir.0,
            &config,
            "ambiguous-recursive-call",
            &[Edit::RenameNode {
                node: "crate::lib::target".into(),
                new_name: "renamed".into(),
            }],
            &mut state,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("recursive call-site provenance is not represented"),
            "{error}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.0.join("src/lib.rs")).unwrap(),
            "pub fn target() -> i32 { target() + object.target() }\n"
        );
    }

    #[test]
    fn rename_rejects_shadowable_bare_calls_inside_the_definition() {
        let (dir, config) = v2_fixture(
            "graph-rename-shadowed-recursive",
            "src/lib.py",
            "def target():\n    def target():\n        return 1\n    return target()\n",
        );
        let mut state = EditState::default();
        let error = apply_step_edits_v2(
            &dir.0,
            &config,
            "shadowed-recursive-call",
            &[Edit::RenameNode {
                node: "crate::lib::target".into(),
                new_name: "renamed".into(),
            }],
            &mut state,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("cannot be renamed safely"),
            "{error}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.0.join("src/lib.py")).unwrap(),
            "def target():\n    def target():\n        return 1\n    return target()\n"
        );
    }

    // Gap 24 design note (docs/core-gap-analysis.md item 24, and the design
    // discussion that preceded it): the span-safety rule re-resolves nodes
    // after every edit, and `EditState::refresh_file` runs after a `create`
    // edit exactly like it runs after any other text edit — so a node a
    // `create` edit just introduced is already visible to `resolve_node` by
    // the time a later graph-addressed edit looks for it, whether that edit
    // is in the same step or a later one sharing the same `EditState`. These
    // two tests pin that as proven behavior, not just established by
    // inspection, ahead of anything (`girder new`) that would rely on it.
    #[test]
    fn a_node_created_earlier_in_the_same_step_can_be_graph_edited_in_that_step() {
        let (dir, config) = v2_fixture("graph-create-then-edit-same-step", "src/lib.rs", "");
        let edits = vec![
            Edit::Create {
                path: "src/new.rs".into(),
                create: "pub fn brand_new() -> i32 { 1 }\n".into(),
            },
            Edit::ReplaceNode {
                node: "crate::new::brand_new".into(),
                replacement: "pub fn brand_new() -> i32 { 111111 }".into(),
            },
        ];
        let mut state = EditState::default();
        // Primed before the create edit runs, exactly as `executor::run_plan_v2`
        // primes it whenever a step mixes any graph-addressed edit in with a
        // create edit — so the create's `refresh_file` call updates this
        // *live* graph incrementally, instead of the create simply landing on
        // disk for a later fresh rebuild to discover (the scenario the
        // "later step" test below exercises instead).
        state.ensure_graph(&dir.0, &config).unwrap();
        apply_step_edits_v2(&dir.0, &config, "create-then-edit", &edits, &mut state).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.0.join("src/new.rs")).unwrap(),
            "pub fn brand_new() -> i32 { 111111 }\n"
        );
        assert!(state
            .graph()
            .unwrap()
            .find_by_path("crate::new::brand_new")
            .is_some());
    }

    #[test]
    fn a_node_created_in_an_earlier_step_can_be_graph_edited_in_a_later_step() {
        let (dir, config) = v2_fixture("graph-create-then-edit-later-step", "src/lib.rs", "");
        let mut state = EditState::default();

        apply_step_edits_v2(
            &dir.0,
            &config,
            "create-step",
            &[Edit::Create {
                path: "src/new.rs".into(),
                create: "pub fn brand_new() -> i32 { 1 }\n".into(),
            }],
            &mut state,
        )
        .unwrap();

        apply_step_edits_v2(
            &dir.0,
            &config,
            "edit-step",
            &[Edit::ReplaceNode {
                node: "crate::new::brand_new".into(),
                replacement: "pub fn brand_new() -> i32 { 222222 }".into(),
            }],
            &mut state,
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.0.join("src/new.rs")).unwrap(),
            "pub fn brand_new() -> i32 { 222222 }\n"
        );
    }

    #[test]
    fn graph_re_resolution_handles_shared_utf8_prefix_bytes() {
        let (dir, config) = v2_fixture(
            "graph-utf8-prefix",
            "src/lib.py",
            "def greeting():\n    return \"é\"\n\ndef second():\n    return 2\n",
        );
        let mut state = EditState::default();
        let writes = apply_step_edits_v2(
            &dir.0,
            &config,
            "utf8-prefix",
            &[Edit::ReplaceNode {
                node: "crate::lib::greeting".into(),
                replacement: "def greeting():\n    return \"ê\"".into(),
            }],
            &mut state,
        )
        .unwrap();
        assert_eq!(writes.len(), 1);
        assert!(state
            .graph()
            .unwrap()
            .find_by_path("crate::lib::greeting")
            .unwrap()
            .source
            .contains("ê"));
    }
}
