//! Keep the semantic graph in sync with source edits.
//!
//! The [`GraphBuilder`] owns one [`IncrementalParser`] per open file and the set
//! of node ids each file currently contributes. On every edit it incrementally
//! reparses, re-extracts, and *diffs* the result into the graph: new/changed
//! nodes are upserted, vanished nodes are removed, and edges are rebuilt for the
//! file. This is the machinery behind bidirectional editor⇄graph sync.

use crate::mapper::{
    extract, module_path_for, BuildOutput, CallRef, CallTargetRef, InheritRef, RouteEvidence,
    RustImportRef,
};
use crate::parser::{IncrementalParser, Lang};
use aether_graph::{Edge, EdgeKind, NodeId, NodeKind, SemanticGraph};
use std::collections::{HashMap, HashSet};
use tree_sitter::{InputEdit, Point};

/// Per-file parsing state.
struct FileState {
    parser: IncrementalParser,
    source: String,
    /// Node ids this file currently contributes to the graph.
    owned: HashSet<NodeId>,
    /// Semantic paths before graph upsert de-duplicates equal `NodeId`s.
    paths: Vec<String>,
    /// Unresolved call references found in this file, for the project resolver.
    calls: Vec<CallRef>,
    /// Unresolved inheritance references found in this file.
    inherits: Vec<InheritRef>,
    /// Rust imports and public re-exports used to preserve aliased identities.
    rust_imports: Vec<RustImportRef>,
    /// Type paths with `impl Drop for T` in this file.
    drop_impls: Vec<String>,
}

/// The module path that owns a node, derived from its full path:
/// `crate::math::add` -> `crate::math`.
fn module_of(path: &str) -> String {
    match path.rfind("::") {
        Some(i) => path[..i].to_string(),
        None => path.to_string(),
    }
}

fn source_module(file: Option<&str>, path: &str) -> String {
    file.map(module_path_for).unwrap_or_else(|| module_of(path))
}

const MAX_REEXPORT_DEPTH: usize = 16;

fn source_crate_root(file: &str) -> String {
    let normalized = file.replace('\\', "/");
    let segments = normalized
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect::<Vec<_>>();
    match segments.iter().rposition(|segment| *segment == "src") {
        Some(0) | None => "crate".to_string(),
        Some(index) => format!("crate::{}", segments[..=index].join("::")),
    }
}

fn rust_module_path_for(file: &str) -> String {
    let module = module_path_for(file);
    let normalized = file.replace('\\', "/");
    let mut segments = normalized
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect::<Vec<_>>();
    let file_name = segments.pop().unwrap_or_default();
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem);
    let crate_root_file = matches!(stem, "lib" | "main") && matches!(segments.last(), Some(&"src"));
    if stem == "mod" || crate_root_file {
        module_of(&module)
    } else {
        module
    }
}

fn normalize_rust_import_target(file: &str, module: &str, target: &str) -> Option<String> {
    let mut target = target
        .split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .peekable();
    let first = target.peek().copied()?;
    let crate_root = source_crate_root(file);
    let crate_root_parts = crate_root.split("::").collect::<Vec<_>>();
    let mut resolved = match first {
        "crate" => {
            target.next();
            crate_root_parts.clone()
        }
        "self" => {
            target.next();
            module.split("::").collect()
        }
        "super" => {
            let mut base = module.split("::").collect::<Vec<_>>();
            while matches!(target.peek(), Some(&"super")) {
                target.next();
                if base.len() <= crate_root_parts.len() {
                    return None;
                }
                base.pop();
            }
            base
        }
        _ => module.split("::").collect(),
    };
    resolved.extend(target);
    Some(resolved.join("::"))
}

fn reexport_index(files: &HashMap<String, FileState>) -> HashMap<String, Vec<String>> {
    let mut index: HashMap<String, Vec<String>> = HashMap::new();
    for (file, state) in files {
        let module = rust_module_path_for(file);
        for import in state
            .rust_imports
            .iter()
            .filter(|import| import.is_reexport)
        {
            let Some(target) = normalize_rust_import_target(file, &module, &import.target) else {
                continue;
            };
            index
                .entry(format!("{module}::{}", import.local))
                .or_default()
                .push(target);
        }
    }
    for targets in index.values_mut() {
        targets.sort();
        targets.dedup();
    }
    index
}

fn follow_reexports(start: String, reexports: &HashMap<String, Vec<String>>) -> Option<String> {
    let mut current = start;
    let mut visited = HashSet::new();
    for _ in 0..MAX_REEXPORT_DEPTH {
        if !visited.insert(current.clone()) {
            return None;
        }
        match reexports.get(&current) {
            None => return Some(current),
            Some(targets) if targets.len() == 1 => current.clone_from(&targets[0]),
            Some(_) => return None,
        }
    }
    None
}

enum AliasResolution {
    NotAliased,
    Resolved(String),
    Ambiguous,
}

fn resolve_local_call_alias(
    file: &str,
    state: &FileState,
    callee: &str,
    reexports: &HashMap<String, Vec<String>>,
) -> AliasResolution {
    let module = rust_module_path_for(file);
    let mut targets = state
        .rust_imports
        .iter()
        .filter(|import| import.local == callee)
        .filter_map(|import| {
            normalize_rust_import_target(file, &module, &import.target)
                .map(|target| (target, last_path_segment(&import.target) != callee))
        })
        .collect::<Vec<_>>();
    targets.sort();
    targets.dedup();
    let [(initial, renamed)] = targets.as_slice() else {
        return if targets.is_empty() {
            AliasResolution::NotAliased
        } else {
            AliasResolution::Ambiguous
        };
    };
    if !*renamed && !reexports.contains_key(initial) {
        return AliasResolution::NotAliased;
    }
    follow_reexports(initial.clone(), reexports)
        .map(AliasResolution::Resolved)
        .unwrap_or(AliasResolution::Ambiguous)
}

fn resolve_qualified_reexport(
    file: &str,
    qualifier: &str,
    callee: &str,
    reexports: &HashMap<String, Vec<String>>,
) -> AliasResolution {
    if qualifier.contains('.')
        || qualifier
            .chars()
            .any(|character| !(character == ':' || character == '_' || character.is_alphanumeric()))
    {
        return AliasResolution::NotAliased;
    }
    let module = rust_module_path_for(file);
    let Some(initial) =
        normalize_rust_import_target(file, &module, &format!("{qualifier}::{callee}"))
    else {
        return AliasResolution::Ambiguous;
    };
    if !reexports.contains_key(&initial) {
        return AliasResolution::NotAliased;
    }
    follow_reexports(initial, reexports)
        .map(AliasResolution::Resolved)
        .unwrap_or(AliasResolution::Ambiguous)
}

fn last_path_segment(path: &str) -> &str {
    path.rsplit("::")
        .find(|segment| !segment.is_empty())
        .unwrap_or(path)
        .trim()
}

fn normalized_symbol(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn qualifier_tail(qualifier: &str) -> &str {
    qualifier
        .rsplit(['.', ':'])
        .find(|part| !part.is_empty())
        .unwrap_or(qualifier)
        .trim()
}

/// An owner's bare type name for name-based matching: the last path segment
/// with any generic parameter list (`<'a>`, `<T>`, ...) dropped first.
/// Without stripping the generics before normalizing, `Interpreter<'a>`
/// would keep the lifetime letter and normalize to `interpretera`, which
/// matches neither `interpreter` nor its suffix — silently breaking
/// resolution for every method on a generic type (gap 22).
fn owner_tail(owner: &str) -> String {
    let name = owner.rsplit("::").next().unwrap_or(owner);
    let name = name.split('<').next().unwrap_or(name);
    normalized_symbol(name)
}

/// The normalized hint a qualifier/receiver-type string carries, or `None`
/// when it is too short or a self-reference to be useful evidence.
fn qualifier_hint(qualifier: &str) -> Option<String> {
    let hint = normalized_symbol(qualifier_tail(qualifier));
    (hint.len() >= 3 && !matches!(hint.as_str(), "self" | "cls")).then_some(hint)
}

/// Exact match only: the qualifier's hint equals the owner's bare type name.
/// An exact match is unambiguous evidence and must outrank any candidate
/// that only satisfies the looser suffix check below, so callers try this
/// first (gap 22: `Timeline` vs `PyTimeline` both satisfy the suffix check,
/// but only `Timeline` is an exact match).
fn qualifier_matches_owner_exactly(qualifier: &str, owner: &str) -> bool {
    qualifier_hint(qualifier).is_some_and(|hint| owner_tail(owner) == hint)
}

fn qualifier_matches_owner(qualifier: &str, owner: &str) -> bool {
    let Some(hint) = qualifier_hint(qualifier) else {
        return false;
    };
    let owner = owner_tail(owner);
    owner == hint || owner.ends_with(&hint)
}

struct FunctionCandidate {
    path: String,
    file: Option<String>,
    source_module: String,
    owner: String,
    id: NodeId,
    return_type: Option<String>,
    type_parameters: Option<String>,
    first_parameter_type: Option<String>,
}

/// Infer a Cargo target name from conventional binary locations. An empty
/// string means `src/main.rs`, whose target name comes from package metadata
/// that the source-only graph does not currently index.
fn conventional_rust_binary_target(file: Option<&str>) -> Option<String> {
    let file = file?;
    let normalized = file.replace('\\', "/");
    if normalized == "src/main.rs" || normalized.ends_with("/src/main.rs") {
        return Some(String::new());
    }
    let (_, bin_path) = normalized.rsplit_once("/src/bin/").or_else(|| {
        normalized
            .strip_prefix("src/bin/")
            .map(|bin_path| ("", bin_path))
    })?;
    if !bin_path.contains('/') {
        return bin_path.strip_suffix(".rs").map(str::to_string);
    }
    let (target, file_name) = bin_path.split_once('/')?;
    (file_name == "main.rs" && !target.is_empty()).then(|| target.to_string())
}

/// A file's Cargo target name, from convention or an exact manifest
/// `[[bin]] path` override. `bin_targets` maps a normalized project-relative
/// file path to its declared target name; an unparseable or absent manifest
/// simply yields an empty map, leaving convention as the only source.
fn rust_binary_target(file: Option<&str>, bin_targets: &HashMap<String, String>) -> Option<String> {
    let file = file?;
    let normalized = file.replace('\\', "/");
    bin_targets
        .get(&normalized)
        .cloned()
        .or_else(|| conventional_rust_binary_target(Some(&normalized)))
}

fn select_process_entrypoint<'a>(
    candidates: &'a [FunctionCandidate],
    target: &str,
    bin_targets: &HashMap<String, String>,
) -> Option<&'a FunctionCandidate> {
    let entrypoints = || {
        candidates.iter().filter(|candidate| {
            candidate.owner == candidate.source_module
                && candidate.path.ends_with("::main")
                && rust_binary_target(candidate.file.as_deref(), bin_targets).is_some()
        })
    };
    only_candidate(entrypoints().filter(|candidate| {
        rust_binary_target(candidate.file.as_deref(), bin_targets)
            .is_some_and(|candidate_target| candidate_target == target)
    }))
    .or_else(|| only_candidate(entrypoints()))
}

fn only_candidate<'a>(
    mut candidates: impl Iterator<Item = &'a FunctionCandidate>,
) -> Option<&'a FunctionCandidate> {
    let first = candidates.next()?;
    candidates.next().is_none().then_some(first)
}

fn select_exact_path<'a>(
    target: &str,
    by_name: &'a HashMap<String, Vec<FunctionCandidate>>,
) -> Option<&'a FunctionCandidate> {
    let candidates = by_name.get(last_path_segment(target))?;
    only_candidate(
        candidates
            .iter()
            .filter(|candidate| candidate.path == target),
    )
}

fn select_candidate<'a>(
    candidates: &'a [FunctionCandidate],
    caller_source_module: &str,
    caller_owner: &str,
    qualifier: Option<&str>,
    receiver_type: Option<&str>,
    qualifier_owner_fallback: bool,
    shadowed_by_local: bool,
) -> Option<&'a FunctionCandidate> {
    // A bare call whose name is also a local parameter can never mean some
    // unrelated same-named global function — Rust scoping always resolves
    // it to the parameter instead, however tempting a same-spelled, even
    // globally-unique, candidate looks below. Guessing here is exactly
    // gap 22's misattribution: a wrong caller is worse than none, so fail
    // closed rather than risk it.
    if qualifier.is_none() && shadowed_by_local {
        return None;
    }
    if let Some(qualifier) = qualifier {
        if matches!(qualifier_tail(qualifier), "self" | "Self" | "cls") {
            only_candidate(
                candidates
                    .iter()
                    .filter(|candidate| candidate.owner == caller_owner),
            )
        } else {
            if receiver_type.is_none() && !qualifier_owner_fallback {
                return None;
            }
            let receiver_hint = receiver_type.unwrap_or(qualifier);
            // Try an exact owner-name match first: it is unambiguous even
            // when another candidate's owner merely ends with the same
            // hint (gap 22's `PyTimeline` colliding with `Timeline`). Only
            // widen to the suffix-inclusive match when no candidate is an
            // exact match.
            only_candidate(candidates.iter().filter(|candidate| {
                qualifier_matches_owner_exactly(receiver_hint, &candidate.owner)
            }))
            .or_else(|| {
                only_candidate(
                    candidates.iter().filter(|candidate| {
                        qualifier_matches_owner(receiver_hint, &candidate.owner)
                    }),
                )
            })
        }
    } else {
        only_candidate(candidates.iter().filter(|candidate| {
            candidate.source_module == caller_source_module
                && candidate.owner == candidate.source_module
        }))
        .or_else(|| {
            only_candidate(
                candidates
                    .iter()
                    .filter(|candidate| candidate.source_module == caller_source_module),
            )
        })
        .or_else(|| (candidates.len() == 1).then(|| &candidates[0]))
    }
}

fn factory_candidate<'a>(
    factory: &CallTargetRef,
    by_name: &'a HashMap<String, Vec<FunctionCandidate>>,
    caller_source_module: &str,
    caller_owner: &str,
) -> Option<&'a FunctionCandidate> {
    let candidates = by_name.get(&factory.callee)?;
    if let Some(receiver_factory) = factory.receiver_factory.as_deref() {
        resolve_factory_receiver(
            receiver_factory,
            candidates,
            by_name,
            caller_source_module,
            caller_owner,
        )
    } else {
        select_candidate(
            candidates,
            caller_source_module,
            caller_owner,
            factory.qualifier.as_deref(),
            factory.receiver_type.as_deref(),
            true,
            // `CallTargetRef` describes a producer call already grounded in
            // a real AST call node (via a `let` binding's hints-map entry
            // or the chained-call walker), not a bare identifier that could
            // itself be a shadowing local parameter, so this never applies.
            false,
        )
    }
}

fn return_type_matches_owner(return_type: &str, owner: &str) -> bool {
    let owner = owner_tail(owner);
    return_type
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .map(normalized_symbol)
        .filter(|token| token.len() >= 3)
        .any(|token| owner == token)
}

fn has_type_token(value: &str, expected: &str) -> bool {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .any(|token| token == expected)
}

fn top_level_parts(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    for (index, ch) in value.char_indices() {
        match ch {
            '<' | '(' | '[' | '{' => depth += 1,
            '>' | ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(value[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(value[start..].trim());
    parts
}

fn generic_type_parameters(value: &str) -> Vec<&str> {
    let inner = value
        .trim()
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
        .unwrap_or(value);
    top_level_parts(inner)
        .into_iter()
        .filter_map(|parameter| {
            let parameter = parameter.trim();
            if parameter.starts_with('\'') || parameter.starts_with("const ") {
                return None;
            }
            let end = parameter
                .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                .unwrap_or(parameter.len());
            (end > 0).then(|| &parameter[..end])
        })
        .collect()
}

fn outer_type_name(value: &str) -> Option<&str> {
    let head = value.split('<').next()?.trim();
    head.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .rfind(|token| !token.is_empty())
}

fn has_direct_generic_argument(value: &str, generic: &str) -> bool {
    let value = value.trim();
    if value == generic {
        return true;
    }
    let Some(start) = value.find('<') else {
        return false;
    };
    let Some(end) = value.rfind('>') else {
        return false;
    };
    start < end
        && top_level_parts(&value[start + 1..end])
            .into_iter()
            .any(|argument| argument == generic)
}

fn passes_generic_type(candidate: &FunctionCandidate) -> bool {
    let (Some(type_parameters), Some(parameter), Some(return_type)) = (
        candidate.type_parameters.as_deref(),
        candidate.first_parameter_type.as_deref(),
        candidate.return_type.as_deref(),
    ) else {
        return false;
    };
    if outer_type_name(parameter) != outer_type_name(return_type) {
        return false;
    }
    generic_type_parameters(type_parameters)
        .into_iter()
        .any(|generic| {
            has_direct_generic_argument(parameter, generic)
                && has_direct_generic_argument(return_type, generic)
        })
}

fn resolve_factory_receiver<'a>(
    factory: &CallTargetRef,
    candidates: &'a [FunctionCandidate],
    by_name: &'a HashMap<String, Vec<FunctionCandidate>>,
    caller_source_module: &str,
    caller_owner: &str,
) -> Option<&'a FunctionCandidate> {
    let producer = factory_candidate(factory, by_name, caller_source_module, caller_owner)?;
    let return_type = producer.return_type.as_deref()?;
    let direct = if has_type_token(return_type, "Self") {
        only_candidate(
            candidates
                .iter()
                .filter(|candidate| candidate.owner == producer.owner),
        )
    } else {
        only_candidate(
            candidates
                .iter()
                .filter(|candidate| return_type_matches_owner(return_type, &candidate.owner)),
        )
    };
    if direct.is_some() || !passes_generic_type(producer) {
        return direct;
    }
    if let Some(fallback_type) = factory.fallback_type.as_deref() {
        if let Some(candidate) = only_candidate(
            candidates
                .iter()
                .filter(|candidate| return_type_matches_owner(fallback_type, &candidate.owner)),
        ) {
            return Some(candidate);
        }
    }
    factory.fallback_factory.as_deref().and_then(|fallback| {
        resolve_factory_receiver(
            fallback,
            candidates,
            by_name,
            caller_source_module,
            caller_owner,
        )
    })
}

/// Incrementally maps source files into a [`SemanticGraph`].
#[derive(Default)]
pub struct GraphBuilder {
    files: HashMap<String, FileState>,
    /// Cargo `[[bin]]` target overrides: normalized project-relative file
    /// path -> declared target name. Consulted before convention when
    /// resolving `CARGO_BIN_EXE_<target>` subprocess entrypoints, so a
    /// custom `path` still links its exact `main` rather than staying
    /// unresolved.
    bin_targets: HashMap<String, String>,
}

impl GraphBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare exact Cargo binary target locations from manifest metadata
    /// the source-only graph cannot otherwise see. Replaces any previously
    /// set targets; call again after a manifest change.
    pub fn set_bin_targets(&mut self, bin_targets: HashMap<String, String>) {
        self.bin_targets = bin_targets;
    }

    /// Initial load of a file. Full parse + extract + insert.
    pub fn load_file(&mut self, graph: &mut SemanticGraph, file: &str, source: &str) {
        self.load_file_unresolved(graph, file, source);
        self.resolve_calls(graph);
    }

    /// Initial load of a complete project snapshot, resolving project-wide
    /// references once after every source projection has been inserted.
    pub fn load_files<'file, 'source, I>(&mut self, graph: &mut SemanticGraph, files: I)
    where
        I: IntoIterator<Item = (&'file str, &'source str)>,
    {
        for (file, source) in files {
            self.load_file_unresolved(graph, file, source);
        }
        self.resolve_calls(graph);
    }

    fn load_file_unresolved(&mut self, graph: &mut SemanticGraph, file: &str, source: &str) {
        let Some(lang) = Lang::from_path(file) else {
            return;
        };
        let mut parser = IncrementalParser::new(lang);
        let tree = parser.parse(source);
        let out = extract(&tree, source, file, lang);
        let owned = self.apply(graph, file, &out, &HashSet::new());
        self.files.insert(
            file.to_string(),
            FileState {
                parser,
                source: source.to_string(),
                owned,
                paths: out.nodes.iter().map(|node| node.path.clone()).collect(),
                calls: out.calls.clone(),
                inherits: out.inherits.clone(),
                rust_imports: out.rust_imports.clone(),
                drop_impls: out.drop_impls.clone(),
            },
        );
    }

    /// Re-sync a file after its full text changed (e.g. the editor buffer).
    /// Uses tree-sitter incremental reparse seeded with a coarse whole-buffer
    /// edit, then diffs the freshly-extracted nodes against what the file owned.
    pub fn update_file(&mut self, graph: &mut SemanticGraph, file: &str, new_source: &str) {
        let Some(lang) = Lang::from_path(file) else {
            return;
        };
        let prev_owned = self
            .files
            .get(file)
            .map(|s| s.owned.clone())
            .unwrap_or_default();

        let entry = self
            .files
            .entry(file.to_string())
            .or_insert_with(|| FileState {
                parser: IncrementalParser::new(lang),
                source: String::new(),
                owned: HashSet::new(),
                paths: Vec::new(),
                calls: Vec::new(),
                inherits: Vec::new(),
                rust_imports: Vec::new(),
                drop_impls: Vec::new(),
            });

        // Inform tree-sitter where the edit happened so it reparses incrementally.
        let edit = whole_buffer_edit(&entry.source, new_source);
        entry.parser.apply_edit(&edit);
        let tree = entry.parser.reparse(new_source);
        entry.source = new_source.to_string();

        let out = extract(&tree, new_source, file, lang);
        let new_owned = self.apply(graph, file, &out, &prev_owned);
        if let Some(state) = self.files.get_mut(file) {
            state.owned = new_owned;
            state.paths = out.nodes.iter().map(|node| node.path.clone()).collect();
            state.calls = out.calls.clone();
            state.inherits = out.inherits.clone();
            state.rust_imports = out.rust_imports.clone();
            state.drop_impls = out.drop_impls.clone();
        }
        self.resolve_calls(graph);
    }

    /// Project-wide call resolution. Rebuilds **all** `Calls` edges from the
    /// accumulated unresolved references against a whole-graph symbol index, so a
    /// call links to its callee even when the callee lives in another file. When
    /// a name is ambiguous, a same-module definition wins; otherwise a unique
    /// global match is used. Renamed Rust imports and public re-export chains
    /// select exact paths; ambiguous or cyclic aliases are left unlinked.
    pub fn resolve_calls(&self, graph: &mut SemanticGraph) {
        let source_owned: HashSet<NodeId> = self
            .files
            .values()
            .flat_map(|state| state.owned.iter().copied())
            .collect();
        let graph_owned_calls: Vec<_> = graph
            .edge_records()
            .into_iter()
            .filter(|(from, to, edge)| {
                edge.kind == EdgeKind::Calls
                    && (!source_owned.contains(from) || !source_owned.contains(to))
            })
            .collect();
        graph.clear_edges_of_kind(EdgeKind::Calls);

        // name -> [(source module, lexical owner, function id)]
        let mut by_name: HashMap<String, Vec<FunctionCandidate>> = HashMap::new();
        for n in graph.query_by_kind(NodeKind::Function) {
            by_name
                .entry(n.name.clone())
                .or_default()
                .push(FunctionCandidate {
                    path: n.path.clone(),
                    file: n.file.clone(),
                    source_module: source_module(n.file.as_deref(), &n.path),
                    owner: module_of(&n.path),
                    id: n.id,
                    return_type: n.attr("return_type").map(str::to_string),
                    type_parameters: n.attr("type_parameters").map(str::to_string),
                    first_parameter_type: n.attr("first_parameter_type").map(str::to_string),
                });
        }

        // Route evidence is re-derived on every resolve; stale keys from a
        // previous parse must not survive a removed launch or dispatch arm.
        let route_owned: Vec<NodeId> = graph
            .nodes()
            .filter(|node| {
                node.attributes
                    .iter()
                    .any(|(key, _)| aether_graph::is_route_attribute(key))
            })
            .map(|node| node.id)
            .collect();
        for id in route_owned {
            if let Some(node) = graph.get_mut(id) {
                node.attributes
                    .retain(|(key, _)| !aether_graph::is_route_attribute(key));
            }
        }

        let reexports = reexport_index(&self.files);
        // Project-wide RAII model: a resolved call to a `Self`-returning
        // associated function of one of these types also, over-approximately,
        // calls that type's `drop`. Recall-safe (may over-select), never
        // recall-losing.
        let drop_types: HashSet<&str> = self
            .files
            .values()
            .flat_map(|state| state.drop_impls.iter().map(String::as_str))
            .collect();
        let mut added: HashSet<(NodeId, NodeId)> = HashSet::new();
        // callee id -> every resolved (caller, provable first string literal).
        let mut literal_callers: HashMap<NodeId, Vec<(NodeId, Option<String>)>> = HashMap::new();
        // (dispatch fn, guarded callee path) -> literal; conflicting
        // literals for one callee make the guard unprovable.
        let mut dispatch_guards: HashMap<(NodeId, String), Option<String>> = HashMap::new();
        // Launcher entry routes: (launcher, entry path, literal).
        let mut entry_routes: Vec<(NodeId, String, String)> = Vec::new();
        // Launch helpers whose route is their first parameter:
        // (helper, entry id, entry path).
        let mut parameter_entries: Vec<(NodeId, NodeId, String)> = Vec::new();
        for (file, state) in &self.files {
            for call in &state.calls {
                let (caller_source_module, caller_owner) = match graph.get(call.caller) {
                    Some(node) => (
                        source_module(node.file.as_deref(), &node.path),
                        module_of(&node.path),
                    ),
                    None => continue,
                };
                if let Some(target) = call.process_entrypoint.as_deref() {
                    let chosen = by_name.get(&call.callee).and_then(|candidates| {
                        select_process_entrypoint(candidates, target, &self.bin_targets)
                    });
                    if let Some(candidate) = chosen {
                        if candidate.id != call.caller && added.insert((call.caller, candidate.id))
                        {
                            let _ = graph.add_edge(
                                call.caller,
                                candidate.id,
                                Edge::new(EdgeKind::Calls),
                            );
                        }
                        match &call.route {
                            Some(RouteEvidence::Literal(literal)) => entry_routes.push((
                                call.caller,
                                candidate.path.clone(),
                                literal.clone(),
                            )),
                            // Only a first-parameter route can be matched to
                            // callers' first-argument literals.
                            Some(RouteEvidence::Param(0)) => parameter_entries.push((
                                call.caller,
                                candidate.id,
                                candidate.path.clone(),
                            )),
                            Some(RouteEvidence::Param(_)) | None => {}
                        }
                    }
                    continue;
                }
                let alias = match call.qualifier.as_deref() {
                    None => resolve_local_call_alias(file, state, &call.callee, &reexports),
                    Some(qualifier) => {
                        resolve_qualified_reexport(file, qualifier, &call.callee, &reexports)
                    }
                };
                let chosen = match alias {
                    AliasResolution::Resolved(target) => select_exact_path(&target, &by_name),
                    AliasResolution::Ambiguous => None,
                    AliasResolution::NotAliased => {
                        let Some(candidates) = by_name.get(&call.callee) else {
                            continue;
                        };
                        if let Some(factory) = call.receiver_factory.as_ref() {
                            resolve_factory_receiver(
                                factory,
                                candidates,
                                &by_name,
                                &caller_source_module,
                                &caller_owner,
                            )
                        } else {
                            select_candidate(
                                candidates,
                                &caller_source_module,
                                &caller_owner,
                                call.qualifier.as_deref(),
                                call.receiver_type.as_deref(),
                                call.qualifier_owner_fallback,
                                call.shadowed_by_local,
                            )
                        }
                    }
                };
                if let Some(candidate) = chosen {
                    if candidate.id != call.caller && added.insert((call.caller, candidate.id)) {
                        let _ =
                            graph.add_edge(call.caller, candidate.id, Edge::new(EdgeKind::Calls));
                    }
                    if candidate.return_type.as_deref() == Some("Self")
                        && drop_types.contains(candidate.owner.as_str())
                    {
                        let drop_id = NodeId::from_path(&format!("{}::drop", candidate.owner));
                        if drop_id != call.caller
                            && graph.contains(drop_id)
                            && added.insert((call.caller, drop_id))
                        {
                            let _ =
                                graph.add_edge(call.caller, drop_id, Edge::new(EdgeKind::Calls));
                        }
                    }
                    literal_callers
                        .entry(candidate.id)
                        .or_default()
                        .push((call.caller, call.first_string_argument.clone()));
                    if let Some(guard) = call.route_guard.as_deref() {
                        dispatch_guards
                            .entry((call.caller, candidate.path.clone()))
                            .and_modify(|existing| {
                                if existing.as_deref() != Some(guard) {
                                    *existing = None;
                                }
                            })
                            .or_insert_with(|| Some(guard.to_string()));
                    }
                }
            }
        }

        // One provable literal→parameter substitution: when every resolved
        // caller of a launch helper passes a first-argument literal, each
        // caller gets its own entry edge and recorded route, and the
        // helper's entry is marked parameter-carried. Any non-literal caller
        // makes the whole substitution unprovable and nothing is recorded.
        for (helper, entry_id, entry_path) in parameter_entries {
            let Some(callers) = literal_callers.get(&helper) else {
                continue;
            };
            let provable: Option<Vec<(NodeId, String)>> = callers
                .iter()
                .map(|(caller, literal)| literal.as_ref().map(|literal| (*caller, literal.clone())))
                .collect();
            let Some(provable) = provable else {
                continue;
            };
            if provable.is_empty() {
                continue;
            }
            for (caller, literal) in provable {
                if caller != entry_id && added.insert((caller, entry_id)) {
                    let _ = graph.add_edge(caller, entry_id, Edge::new(EdgeKind::Calls));
                }
                entry_routes.push((caller, entry_path.clone(), literal));
            }
            if let Some(node) = graph.get_mut(helper) {
                node.set_attr(
                    aether_graph::entry_route_params_key(&entry_path),
                    "via-params",
                );
            }
        }
        for (launcher, entry_path, literal) in entry_routes {
            if let Some(node) = graph.get_mut(launcher) {
                node.set_attr(aether_graph::entry_route_key(&entry_path), literal);
            }
        }
        for ((dispatcher, callee_path), literal) in dispatch_guards {
            let Some(literal) = literal else {
                continue;
            };
            if let Some(node) = graph.get_mut(dispatcher) {
                node.set_attr(aether_graph::route_guard_key(&callee_path), literal);
            }
        }

        for (from, to, edge) in graph_owned_calls {
            if graph.contains(from) && graph.contains(to) {
                let _ = graph.add_edge(from, to, edge);
            }
        }

        self.resolve_inherits(graph);
    }

    /// Project-wide inheritance resolution. Rebuilds **all** `Inherits` edges
    /// from accumulated references against a whole-graph *type* index, so a
    /// Python subclass or Rust trait impl links to its base even across files.
    /// Same-module definitions win ties; otherwise a unique global match is used.
    fn resolve_inherits(&self, graph: &mut SemanticGraph) {
        let source_owned: HashSet<NodeId> = self
            .files
            .values()
            .flat_map(|state| state.owned.iter().copied())
            .collect();
        let graph_owned_inherits: Vec<_> = graph
            .edge_records()
            .into_iter()
            .filter(|(from, to, edge)| {
                edge.kind == EdgeKind::Inherits
                    && (!source_owned.contains(from) || !source_owned.contains(to))
            })
            .collect();
        graph.clear_edges_of_kind(EdgeKind::Inherits);

        // name -> [(owning module, type id)]
        let mut by_type: HashMap<String, Vec<(String, NodeId)>> = HashMap::new();
        for n in graph.query_by_kind(NodeKind::Type) {
            by_type
                .entry(n.name.clone())
                .or_default()
                .push((module_of(&n.path), n.id));
        }

        let mut added: HashSet<(NodeId, NodeId)> = HashSet::new();
        for state in self.files.values() {
            for inh in &state.inherits {
                let sub_module = match graph.get(inh.sub) {
                    Some(node) => module_of(&node.path),
                    None => continue,
                };
                let Some(candidates) = by_type.get(&inh.base) else {
                    continue;
                };
                let chosen = candidates.iter().find(|(m, _)| *m == sub_module).or(
                    if candidates.len() == 1 {
                        candidates.first()
                    } else {
                        None
                    },
                );
                if let Some((_, base_id)) = chosen {
                    if *base_id != inh.sub && added.insert((inh.sub, *base_id)) {
                        let _ = graph.add_edge(inh.sub, *base_id, Edge::new(EdgeKind::Inherits));
                    }
                }
            }
        }
        for (from, to, edge) in graph_owned_inherits {
            if graph.contains(from) && graph.contains(to) {
                let _ = graph.add_edge(from, to, edge);
            }
        }
    }

    /// Upsert all nodes/edges from `out`, then remove any previously-owned node
    /// that is no longer present. Returns the new owned-id set.
    fn apply(
        &self,
        graph: &mut SemanticGraph,
        _file: &str,
        out: &BuildOutput,
        prev_owned: &HashSet<NodeId>,
    ) -> HashSet<NodeId> {
        let new_owned: HashSet<NodeId> = out.node_ids().into_iter().collect();

        let stale_projection_edges: Vec<_> = graph
            .edges()
            .into_iter()
            .filter(|(from, _, kind)| {
                prev_owned.contains(from) && matches!(kind, EdgeKind::Contains | EdgeKind::DataFlow)
            })
            .collect();
        for (from, to, kind) in stale_projection_edges {
            graph.remove_edge(from, to, kind);
        }

        for node in &out.nodes {
            graph.upsert_projection_node(node.clone());
        }
        for (from, to, edge) in &out.edges {
            // These are Contains edges (module->fn/type, type->field); both
            // endpoints were just upserted. Calls are resolved project-wide later.
            let _ = graph.add_edge(*from, *to, edge.clone());
        }

        // Remove nodes that this file used to own but no longer does (deletions).
        for stale in prev_owned.difference(&new_owned) {
            graph.remove_node(*stale);
        }
        new_owned
    }

    /// The current text projection of a file, if loaded.
    pub fn source_of(&self, file: &str) -> Option<&str> {
        self.files.get(file).map(|s| s.source.as_str())
    }

    /// Number of parsed declarations currently claiming an exact semantic path.
    /// This is intentionally counted before graph upsert, whose path-derived id
    /// would otherwise collapse duplicate definitions and hide ambiguity.
    pub fn path_occurrences(&self, path: &str) -> usize {
        self.files
            .values()
            .flat_map(|state| state.paths.iter())
            .filter(|candidate| candidate.as_str() == path)
            .count()
    }
}

/// Build a conservative [`InputEdit`] describing "the whole buffer changed".
///
/// A production editor would derive a minimal edit from the keystroke; for the
/// prototype we hand tree-sitter the changed byte range from the start of the
/// first difference, which still lets it reuse the unchanged prefix's subtree.
fn whole_buffer_edit(old: &str, new: &str) -> InputEdit {
    let mut common = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while common > 0 && (!old.is_char_boundary(common) || !new.is_char_boundary(common)) {
        common -= 1;
    }
    let start_point = byte_to_point(old, common);
    InputEdit {
        start_byte: common,
        old_end_byte: old.len(),
        new_end_byte: new.len(),
        start_position: start_point,
        old_end_position: byte_to_point(old, old.len()),
        new_end_position: byte_to_point(new, new.len()),
    }
}

fn byte_to_point(text: &str, byte: usize) -> Point {
    let mut row = 0;
    let mut col = 0;
    for (i, c) in text.char_indices() {
        if i >= byte {
            break;
        }
        if c == '\n' {
            row += 1;
            col = 0;
        } else {
            col += c.len_utf8();
        }
    }
    Point::new(row, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reexport_resolution_is_bounded_and_rejects_ambiguity() {
        let mut reexports = HashMap::new();
        reexports.insert("crate::a".into(), vec!["crate::b".into()]);
        reexports.insert("crate::b".into(), vec!["crate::a".into()]);
        assert!(follow_reexports("crate::a".into(), &reexports).is_none());

        reexports.clear();
        reexports.insert(
            "crate::ambiguous".into(),
            vec!["crate::left".into(), "crate::right".into()],
        );
        assert!(follow_reexports("crate::ambiguous".into(), &reexports).is_none());

        reexports.clear();
        for index in 0..MAX_REEXPORT_DEPTH {
            reexports.insert(
                format!("crate::alias_{index}"),
                vec![format!("crate::alias_{}", index + 1)],
            );
        }
        assert!(follow_reexports("crate::alias_0".into(), &reexports).is_none());
    }

    #[test]
    fn crate_relative_imports_follow_each_source_crate_root() {
        let module = "crate::crates::aether-app::src::app";
        assert_eq!(
            normalize_rust_import_target(
                "crates/aether-app/src/app.rs",
                module,
                "crate::project::join_collaboration",
            )
            .as_deref(),
            Some("crate::crates::aether-app::src::project::join_collaboration")
        );
        assert_eq!(
            normalize_rust_import_target(
                "crates/aether-app/src/project.rs",
                "crate::crates::aether-app::src::project",
                "collaboration_transport::join",
            )
            .as_deref(),
            Some("crate::crates::aether-app::src::project::collaboration_transport::join")
        );
        assert_eq!(rust_module_path_for("src/main.rs"), "crate");
        assert_eq!(
            rust_module_path_for("crates/aether-app/src/project/commands/mod.rs"),
            "crate::crates::aether-app::src::project::commands"
        );
        assert_eq!(
            rust_module_path_for("crates/aether-app/src/project.rs"),
            "crate::crates::aether-app::src::project"
        );
    }
}
