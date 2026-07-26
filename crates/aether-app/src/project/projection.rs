use crate::project::config::{ProjectConfig, CONFIG_FILE};
use crate::project::source::{
    commit_project_writes, graph_project_write, read_project_bytes, ProjectWrite,
};
use aether_builder::GraphBuilder;
use aether_graph::{Node, NodeKind, RenameOutcome, SemanticGraph};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct ProjectionBaseline {
    files: BTreeMap<String, Option<Vec<u8>>>,
    graph: Option<Vec<u8>>,
    config: Option<Vec<u8>>,
}

pub(crate) struct ProjectionPlan {
    writes: Vec<ProjectWrite>,
    projected: Vec<PathBuf>,
}

impl ProjectionPlan {
    pub(crate) fn writes(&self) -> &[ProjectWrite] {
        &self.writes
    }

    pub(crate) fn commit(self, root: &Path) -> std::io::Result<Vec<PathBuf>> {
        commit_project_writes(root, self.writes)?;
        Ok(self.projected)
    }
}

#[derive(Clone)]
struct FunctionProjection {
    path: String,
    source: String,
}

#[derive(Clone)]
struct Replacement {
    start: usize,
    end: usize,
    source: String,
}

pub(crate) fn capture_agent_baseline(
    root: &Path,
    config: &ProjectConfig,
) -> std::io::Result<ProjectionBaseline> {
    let mut files = BTreeMap::new();
    files.insert(
        config.agents.output_file.clone(),
        read_project_bytes(root, &config.agents.output_file)?,
    );
    Ok(ProjectionBaseline {
        files,
        graph: read_project_bytes(root, &config.graph.path)?,
        config: read_project_bytes(root, CONFIG_FILE)?,
    })
}

/// Plan Coder-authored source functions and the durable graph together.
///
/// Existing functions are replaced by their parsed spans. New functions are
/// appended to the target file. Every candidate file is reparsed and checked for
/// the expected function paths before a caller validates or commits the plan.
pub(crate) fn plan_authored_functions(
    root: &Path,
    config: &ProjectConfig,
    graph: &SemanticGraph,
    module: &str,
    baseline: Option<&ProjectionBaseline>,
) -> std::io::Result<ProjectionPlan> {
    if let Some(baseline) = baseline {
        let current_config = read_project_bytes(root, CONFIG_FILE)?;
        if current_config != baseline.config {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{CONFIG_FILE} changed after the agent run started"),
            ));
        }
    }

    let mut by_file: BTreeMap<String, Vec<FunctionProjection>> = BTreeMap::new();
    let prefix = format!("{module}::");

    for node in graph.nodes() {
        if node.kind != NodeKind::Function
            || !node.path.starts_with(&prefix)
            || node.attr("authored_by") != Some("Coder")
        {
            continue;
        }
        let Some(file) = &node.file else {
            continue;
        };
        by_file
            .entry(file.clone())
            .or_default()
            .push(FunctionProjection {
                path: node.path.clone(),
                source: node.source.clone(),
            });
    }

    let mut writes = Vec::new();
    let mut projected = Vec::new();
    for (rel, mut functions) in by_file {
        functions.sort_by(|a, b| a.path.cmp(&b.path));
        let current = read_project_bytes(root, &rel)?;
        let expected = projection_expected(baseline, &rel, &current);
        let original = decode_source(&rel, expected.as_deref().unwrap_or_default())?;

        let mut existing = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut existing, &rel, &original);

        let mut candidate = original;
        let mut replacements = Vec::new();
        let mut appends = Vec::new();

        for function in &functions {
            if let Some(existing_node) = existing.find_by_path(&function.path) {
                replacements.push(Replacement {
                    start: existing_node.span.start_byte,
                    end: existing_node.span.end_byte,
                    source: render_function(&function.source),
                });
            } else {
                appends.push(render_function(&function.source));
            }
        }

        apply_replacements(&mut candidate, &replacements)?;
        for source in &appends {
            append_function(&mut candidate, source);
        }

        let expected_paths: Vec<&str> = functions.iter().map(|f| f.path.as_str()).collect();
        validate_projection(&rel, &candidate, &expected_paths)?;
        writes.push(ProjectWrite::text(&rel, expected, candidate));
        projected.push(PathBuf::from(rel));
    }

    let graph_expected = match baseline {
        Some(baseline) => baseline.graph.clone(),
        None => read_project_bytes(root, &config.graph.path)?,
    };
    writes.push(graph_project_write(root, config, graph, graph_expected)?);
    Ok(ProjectionPlan { writes, projected })
}

/// Apply a graph-semantic rename outcome back into source file projections.
///
/// The old graph supplies stable spans from the current file text; the renamed
/// graph supplies the replacement source for the target function and graph-known
/// callers whose projected source was rewritten by `SemanticGraph::rename_node`.
pub(crate) fn project_rename(
    root: &Path,
    config: &ProjectConfig,
    before: &SemanticGraph,
    after: &SemanticGraph,
    outcome: &RenameOutcome,
) -> std::io::Result<Vec<PathBuf>> {
    let mut by_file: BTreeMap<String, Vec<Replacement>> = BTreeMap::new();
    let mut expected_by_file: BTreeMap<String, Vec<String>> = BTreeMap::new();

    if let (Some(old), Some(new)) = (
        before.find_by_path(&outcome.old_path),
        after.get(outcome.new_id),
    ) {
        add_node_replacement(&mut by_file, &mut expected_by_file, old, new);
    }

    for caller_path in &outcome.updated_callers {
        if let (Some(old), Some(new)) = (
            before.find_by_path(caller_path),
            after.find_by_path(caller_path),
        ) {
            add_node_replacement(&mut by_file, &mut expected_by_file, old, new);
        }
    }

    let mut writes = Vec::new();
    let mut projected = Vec::new();
    for (rel, replacements) in by_file {
        let original = read_project_bytes(root, &rel)?;
        let Some(original_bytes) = original.as_deref() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("source projection does not exist: {rel}"),
            ));
        };
        let mut candidate = decode_source(&rel, original_bytes)?;
        apply_replacements(&mut candidate, &replacements)?;

        let expected = expected_by_file
            .get(&rel)
            .map(|paths| paths.iter().map(String::as_str).collect::<Vec<_>>())
            .unwrap_or_default();
        validate_projection(&rel, &candidate, &expected)?;
        writes.push(ProjectWrite::text(&rel, original, candidate));
        projected.push(PathBuf::from(rel));
    }

    let graph_expected = read_project_bytes(root, &config.graph.path)?;
    writes.push(graph_project_write(root, config, after, graph_expected)?);
    commit_project_writes(root, writes)?;
    Ok(projected)
}

fn add_node_replacement(
    by_file: &mut BTreeMap<String, Vec<Replacement>>,
    expected_by_file: &mut BTreeMap<String, Vec<String>>,
    old: &Node,
    new: &Node,
) {
    let Some(file) = &old.file else {
        return;
    };
    by_file.entry(file.clone()).or_default().push(Replacement {
        start: old.span.start_byte,
        end: old.span.end_byte,
        source: render_function(&new.source),
    });
    expected_by_file
        .entry(file.clone())
        .or_default()
        .push(new.path.clone());
}

fn projection_expected(
    baseline: Option<&ProjectionBaseline>,
    relative: &str,
    current: &Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    match baseline.and_then(|baseline| baseline.files.get(relative)) {
        Some(expected) => expected.clone(),
        None => current.clone(),
    }
}

fn decode_source(relative: &str, bytes: &[u8]) -> std::io::Result<String> {
    String::from_utf8(bytes.to_vec()).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{relative} is not valid UTF-8: {error}"),
        )
    })
}

fn render_function(source: &str) -> String {
    let mut source = source.trim().to_string();
    source.push('\n');
    source
}

fn append_function(text: &mut String, source: &str) {
    if !text.trim().is_empty() {
        while text.ends_with('\n') {
            text.pop();
        }
        text.push_str("\n\n");
    }
    text.push_str(source);
}

fn apply_replacements(text: &mut String, replacements: &[Replacement]) -> std::io::Result<()> {
    let mut replacements = replacements.to_vec();
    replacements.sort_by_key(|replacement| std::cmp::Reverse(replacement.start));

    for replacement in replacements {
        if replacement.start > replacement.end
            || replacement.end > text.len()
            || !text.is_char_boundary(replacement.start)
            || !text.is_char_boundary(replacement.end)
        {
            return Err(std::io::Error::other(format!(
                "invalid source span {}..{} for projection",
                replacement.start, replacement.end
            )));
        }
        text.replace_range(replacement.start..replacement.end, &replacement.source);
    }

    Ok(())
}

fn validate_projection(rel: &str, text: &str, expected_paths: &[&str]) -> std::io::Result<()> {
    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();
    builder.load_file(&mut graph, rel, text);

    for path in expected_paths {
        if graph.find_by_path(path).is_none() {
            return Err(std::io::Error::other(format!(
                "projected source for {rel} did not parse expected node {path}"
            )));
        }
    }

    Ok(())
}
