//! Verified edits (Stage 5): certified plan steps. Implements docs/verified-edits-policy.md.
//!
//! A step that carries a `verify` block names, for every node it replaces, a baseline
//! fingerprint, and declares the complete node and edge delta the edit will cause. The
//! verifier refuses (a step failure, before any commit) when the promise and the facts differ.
//! Checks run in the policy's order and the first failing one names the refusal category.

use super::schema::{DeclaredDelta, Edit, Step, Verify};
use aether_graph::{Node, NodeId, NodeKind, SemanticGraph};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const PRELUDE: &[u8] = b"girder-node-fingerprint-v1\0";
const LIST_LIMIT: usize = 20;

/// `sha256(PRELUDE + path + 0 + source)`, lowercase hex. Binding the path means two same-named
/// overloads with identical bodies still differ. The single definition used by the authoring
/// output and by this verifier.
pub(crate) fn fingerprint(path: &str, source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(PRELUDE);
    hasher.update(path.as_bytes());
    hasher.update([0u8]);
    hasher.update(source.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Refusal {
    pub(crate) category: &'static str,
    pub(crate) detail: String,
}

impl Refusal {
    fn new(category: &'static str, detail: impl Into<String>) -> Self {
        Self {
            category,
            detail: detail.into(),
        }
    }

    pub(crate) fn message(&self) -> String {
        format!("verification refused [{}]: {}", self.category, self.detail)
    }
}

type Triple = (String, String, String);

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub(crate) struct ActualDelta {
    pub(crate) nodes_changed: Vec<String>,
    pub(crate) nodes_added: Vec<String>,
    pub(crate) nodes_removed: Vec<String>,
    pub(crate) edges_added: Vec<Triple>,
    pub(crate) edges_removed: Vec<Triple>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct ClassCounts {
    pub(crate) must: usize,
    pub(crate) may: usize,
    pub(crate) unknown: usize,
}

/// Predicted reachability from the changed nodes. Never execution evidence.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PredictedImpact {
    pub(crate) label: &'static str,
    pub(crate) origins: usize,
    pub(crate) impacted_nodes: ClassCounts,
    pub(crate) boundaries: usize,
    pub(crate) tests_before: ClassCounts,
    pub(crate) tests_after: ClassCounts,
    pub(crate) tests_after_listed: Vec<String>,
    pub(crate) newly_reachable: Vec<String>,
    pub(crate) no_longer_reachable: Vec<String>,
    pub(crate) truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Certification {
    pub(crate) certified: bool,
    pub(crate) refusal: Option<Refusal>,
    pub(crate) delta: Option<ActualDelta>,
    pub(crate) impact: Option<PredictedImpact>,
}

impl Certification {
    pub(crate) fn uncertified() -> Self {
        Self {
            certified: false,
            refusal: None,
            delta: None,
            impact: None,
        }
    }

    pub(crate) fn refused(refusal: Refusal) -> Self {
        Self {
            certified: false,
            refusal: Some(refusal),
            delta: None,
            impact: None,
        }
    }
}

#[derive(Debug)]
struct PreparedEdit {
    path: String,
    file: String,
    start: usize,
    end: usize,
    replacement: String,
}

#[derive(Debug, Default)]
struct Declared {
    changed: BTreeSet<String>,
    added: BTreeSet<String>,
    removed: BTreeSet<String>,
    edges_added: BTreeSet<Triple>,
    edges_removed: BTreeSet<Triple>,
}

/// State captured before the edit is applied, needed to judge it afterwards.
#[derive(Debug)]
pub(crate) struct Prepared {
    edits: Vec<PreparedEdit>,
    declared: Declared,
    originals: BTreeMap<String, Vec<u8>>,
}

fn to_set(list: &Option<Vec<String>>) -> BTreeSet<String> {
    list.iter().flatten().cloned().collect()
}

fn to_triples(list: &Option<Vec<[String; 3]>>) -> BTreeSet<Triple> {
    list.iter()
        .flatten()
        .map(|t| (t[0].clone(), t[1].clone(), t[2].clone()))
        .collect()
}

fn declared_delta(delta: &DeclaredDelta) -> Result<Declared, Refusal> {
    let missing = |key: &str| {
        Refusal::new(
            "insufficient_evidence",
            format!("incomplete verify block: delta.{key} is missing (a complete delta states every key explicitly)"),
        )
    };
    let nodes = delta.nodes.as_ref().ok_or_else(|| missing("nodes"))?;
    let edges = delta.edges.as_ref().ok_or_else(|| missing("edges"))?;
    for (key, present) in [
        ("nodes.changed", nodes.changed.is_some()),
        ("nodes.added", nodes.added.is_some()),
        ("nodes.removed", nodes.removed.is_some()),
        ("edges.added", edges.added.is_some()),
        ("edges.removed", edges.removed.is_some()),
    ] {
        if !present {
            return Err(missing(key));
        }
    }
    Ok(Declared {
        changed: to_set(&nodes.changed),
        added: to_set(&nodes.added),
        removed: to_set(&nodes.removed),
        edges_added: to_triples(&edges.added),
        edges_removed: to_triples(&edges.removed),
    })
}

/// Policy checks 1 to 4: evidence, resolution, and baseline, all before anything is applied.
pub(crate) fn pre_check(
    step: &Step,
    verify: &Verify,
    before: &SemanticGraph,
    workspace: &Path,
) -> Result<Prepared, Refusal> {
    // 1. insufficient_evidence: incomplete block, text edits, unsupported kinds.
    let baseline = verify.baseline.as_ref().ok_or_else(|| {
        Refusal::new(
            "insufficient_evidence",
            "incomplete verify block: baseline is missing",
        )
    })?;
    let delta = verify.delta.as_ref().ok_or_else(|| {
        Refusal::new(
            "insufficient_evidence",
            "incomplete verify block: delta is missing",
        )
    })?;
    let declared = declared_delta(delta)?;
    if step.edits.is_empty() {
        return Err(Refusal::new(
            "insufficient_evidence",
            "a certified step needs at least one edit",
        ));
    }
    let mut replacements = Vec::new();
    for edit in &step.edits {
        match edit {
            Edit::ReplaceNode { node, replacement } => replacements.push((node, replacement)),
            Edit::Substitute { .. } | Edit::Create { .. } | Edit::Delete { .. } => {
                return Err(Refusal::new(
                    "insufficient_evidence",
                    "a certified step cannot carry text edits; only replace_node is certified",
                ))
            }
            other => {
                return Err(Refusal::new(
                    "insufficient_evidence",
                    format!(
                        "edit kind on {:?} is outside the certified scope (only replace_node is certified)",
                        other.node().unwrap_or("<path>")
                    ),
                ))
            }
        }
    }
    // 2. ambiguity: every addressed path must resolve to exactly one node.
    let mut edits = Vec::new();
    let mut originals = BTreeMap::new();
    for (path, replacement) in &replacements {
        let matches: Vec<&Node> = before.nodes().filter(|n| &n.path == *path).collect();
        let node = match matches.as_slice() {
            [one] => *one,
            [] => {
                return Err(Refusal::new(
                    "ambiguity",
                    format!("node path {path:?} resolves to no node"),
                ))
            }
            many => {
                return Err(Refusal::new(
                    "ambiguity",
                    format!("node path {path:?} resolves to {} nodes", many.len()),
                ))
            }
        };
        if !matches!(node.language.as_str(), "rust" | "python") {
            return Err(Refusal::new(
                "insufficient_evidence",
                format!(
                    "node {path:?} uses unsupported language {:?}",
                    node.language
                ),
            ));
        }
        let file = node.file.clone().ok_or_else(|| {
            Refusal::new(
                "insufficient_evidence",
                format!("node {path:?} has no file projection"),
            )
        })?;
        if !originals.contains_key(&file) {
            let bytes = std::fs::read(workspace.join(&file)).map_err(|error| {
                Refusal::new(
                    "insufficient_evidence",
                    format!("could not read {file}: {error}"),
                )
            })?;
            originals.insert(file.clone(), bytes);
        }
        edits.push(PreparedEdit {
            path: (*path).clone(),
            file,
            start: node.span.start_byte,
            end: node.span.end_byte,
            replacement: (*replacement).clone(),
        });
    }
    let edited: BTreeSet<&String> = edits.iter().map(|e| &e.path).collect();
    for edit in &edits {
        if !baseline.contains_key(&edit.path) {
            return Err(Refusal::new(
                "insufficient_evidence",
                format!("no baseline fingerprint for edited node {:?}", edit.path),
            ));
        }
    }
    if let Some(extra) = baseline.keys().find(|key| !edited.contains(key)) {
        return Err(Refusal::new(
            "insufficient_evidence",
            format!("baseline names {extra:?}, a node this step does not edit"),
        ));
    }
    // 3. wrong_overload (pre-apply) and 4. stale_input.
    for edit in &edits {
        let node = before
            .nodes()
            .find(|n| n.path == edit.path)
            .expect("resolved above");
        let supplied = &baseline[&edit.path];
        if *supplied == fingerprint(&node.path, &node.source) {
            continue;
        }
        let sibling = before.nodes().find(|other| {
            other.path != node.path
                && other.name == node.name
                && other.kind == node.kind
                && fingerprint(&other.path, &other.source) == *supplied
        });
        return Err(match sibling {
            Some(other) => Refusal::new(
                "wrong_overload",
                format!(
                    "the baseline for {:?} is the fingerprint of {:?}, a different node named {:?}",
                    node.path, other.path, node.name
                ),
            ),
            None => Refusal::new(
                "stale_input",
                format!(
                    "the baseline for {:?} does not match its current source (the node changed since it was read)",
                    node.path
                ),
            ),
        });
    }
    Ok(Prepared {
        edits,
        declared,
        originals,
    })
}

fn canonical_nodes(graph: &SemanticGraph) -> BTreeMap<String, String> {
    graph
        .nodes()
        .filter(|n| n.kind != NodeKind::Module)
        .map(|n| (n.path.clone(), n.source.clone()))
        .collect()
}

fn canonical_edges(graph: &SemanticGraph) -> BTreeSet<Triple> {
    graph
        .edges()
        .into_iter()
        .filter_map(|(from, to, kind)| {
            Some((
                graph.get(from)?.path.clone(),
                graph.get(to)?.path.clone(),
                format!("{kind:?}"),
            ))
        })
        .collect()
}

fn actual_delta(before: &SemanticGraph, after: &SemanticGraph) -> ActualDelta {
    let (b, a) = (canonical_nodes(before), canonical_nodes(after));
    let (be, ae) = (canonical_edges(before), canonical_edges(after));
    ActualDelta {
        nodes_changed: b
            .iter()
            .filter(|(path, source)| a.get(*path).is_some_and(|now| now != *source))
            .map(|(path, _)| path.clone())
            .collect(),
        nodes_added: a.keys().filter(|p| !b.contains_key(*p)).cloned().collect(),
        nodes_removed: b.keys().filter(|p| !a.contains_key(*p)).cloned().collect(),
        edges_added: ae.difference(&be).cloned().collect(),
        edges_removed: be.difference(&ae).cloned().collect(),
    }
}

fn bare_name<'a>(graph: &'a SemanticGraph, path: &str) -> Option<&'a str> {
    graph
        .nodes()
        .find(|n| n.path == path)
        .map(|n| n.name.as_str())
}

/// Policy checks after the edit: parse gaps, projection exactness, incremental versus cold,
/// post-apply wrong_overload, then delta_mismatch and unexpected_edge.
pub(crate) fn post_check(
    prepared: &Prepared,
    before_cold: &SemanticGraph,
    after_cold: &SemanticGraph,
    incremental_after: Option<&SemanticGraph>,
    workspace: &Path,
) -> Result<ActualDelta, Refusal> {
    let touched: BTreeSet<&String> = prepared.edits.iter().map(|e| &e.file).collect();
    for file in &touched {
        for node in after_cold
            .nodes()
            .filter(|n| n.file.as_deref() == Some(file.as_str()))
        {
            if let Ok(evidence) = after_cold.call_evidence(node.id) {
                if let Some(gap) = evidence.calls.iter().find(|c| {
                    c.coverage_gap
                        && matches!(c.reason.as_str(), "parse-error" | "duplicate-semantic-path")
                }) {
                    return Err(Refusal::new(
                        "insufficient_evidence",
                        format!("{file} has a {} gap after the edit", gap.reason),
                    ));
                }
            }
        }
    }
    // Projection exactness: the file is the original with exactly the addressed spans replaced.
    for file in &touched {
        let mut splices: Vec<&PreparedEdit> =
            prepared.edits.iter().filter(|e| &&e.file == file).collect();
        splices.sort_by_key(|e| std::cmp::Reverse(e.start));
        let mut expected = prepared.originals[file.as_str()].clone();
        let mut limit = usize::MAX;
        for splice in &splices {
            if splice.end > limit || splice.end > expected.len() || splice.start > splice.end {
                return Err(Refusal::new(
                    "insufficient_evidence",
                    format!("edits to {file} overlap or lie outside the file"),
                ));
            }
            expected.splice(splice.start..splice.end, splice.replacement.bytes());
            limit = splice.start;
        }
        let actual = std::fs::read(workspace.join(file.as_str())).map_err(|error| {
            Refusal::new(
                "insufficient_evidence",
                format!("could not re-read {file}: {error}"),
            )
        })?;
        if actual != expected {
            return Err(Refusal::new(
                "insufficient_evidence",
                format!("projection mismatch: {file} is not the original with exactly the addressed spans replaced"),
            ));
        }
    }
    for edit in &prepared.edits {
        if let Some(node) = after_cold.nodes().find(|n| n.path == edit.path) {
            if node.source != edit.replacement {
                return Err(Refusal::new(
                    "insufficient_evidence",
                    format!(
                        "projection mismatch: {:?} does not hold exactly the replacement text",
                        edit.path
                    ),
                ));
            }
        }
    }
    if let Some(incremental) = incremental_after {
        if canonical_nodes(incremental) != canonical_nodes(after_cold)
            || canonical_edges(incremental) != canonical_edges(after_cold)
        {
            return Err(Refusal::new(
                "insufficient_evidence",
                "the incremental graph differs from a cold rebuild of the edited project",
            ));
        }
    }
    let actual = actual_delta(before_cold, after_cold);
    let declared = &prepared.declared;
    let actual_changed: BTreeSet<String> = actual.nodes_changed.iter().cloned().collect();
    if actual_changed != declared.changed {
        let extra: Vec<&String> = actual_changed.difference(&declared.changed).collect();
        let missing: Vec<&String> = declared.changed.difference(&actual_changed).collect();
        for x in &extra {
            for y in &missing {
                let (nx, ny) = (bare_name(after_cold, x), bare_name(before_cold, y));
                if nx.is_some() && nx == ny {
                    return Err(Refusal::new(
                        "wrong_overload",
                        format!("the edit changed {x:?} but the plan declared {y:?}, a different node named {:?}", nx.unwrap_or("")),
                    ));
                }
            }
        }
    }
    let nodes_match = actual_changed == declared.changed
        && actual.nodes_added.iter().cloned().collect::<BTreeSet<_>>() == declared.added
        && actual
            .nodes_removed
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            == declared.removed;
    if !nodes_match {
        return Err(Refusal::new(
            "delta_mismatch",
            format!(
                "declared node delta (changed {:?}, added {:?}, removed {:?}) differs from the actual (changed {:?}, added {:?}, removed {:?})",
                declared.changed, declared.added, declared.removed, actual.nodes_changed, actual.nodes_added, actual.nodes_removed
            ),
        ));
    }
    let edges_added: BTreeSet<Triple> = actual.edges_added.iter().cloned().collect();
    let edges_removed: BTreeSet<Triple> = actual.edges_removed.iter().cloned().collect();
    if edges_added != declared.edges_added || edges_removed != declared.edges_removed {
        return Err(Refusal::new(
            "unexpected_edge",
            format!(
                "declared edge delta (added {:?}, removed {:?}) differs from the actual (added {:?}, removed {:?})",
                declared.edges_added, declared.edges_removed, edges_added, edges_removed
            ),
        ));
    }
    Ok(actual)
}

fn test_paths(
    graph: &SemanticGraph,
    origins: &[NodeId],
) -> (ClassCounts, usize, Vec<(String, &'static str)>) {
    let impact = graph.classified_impact(origins);
    let tests = impact.tests(graph);
    let name = |id: &NodeId| graph.get(*id).map(|n| n.path.clone());
    let mut listed = Vec::new();
    for (ids, class) in [
        (&tests.must, "must"),
        (&tests.may, "may"),
        (&tests.unknown, "unknown"),
    ] {
        listed.extend(ids.iter().filter_map(name).map(|p| (p, class)));
    }
    (
        ClassCounts {
            must: tests.must.len(),
            may: tests.may.len(),
            unknown: tests.unknown.len(),
        },
        impact.boundaries.len(),
        listed,
    )
}

/// Predicted impact of a certified, successful edit. Reachability only: never execution evidence.
pub(crate) fn predicted_impact(
    before: &SemanticGraph,
    after: &SemanticGraph,
    delta: &ActualDelta,
) -> PredictedImpact {
    let ids = |graph: &SemanticGraph, paths: &[&Vec<String>]| -> Vec<NodeId> {
        paths
            .iter()
            .flat_map(|list| list.iter())
            .filter_map(|p| graph.nodes().find(|n| &n.path == p).map(|n| n.id))
            .collect()
    };
    let after_origins = ids(after, &[&delta.nodes_changed, &delta.nodes_added]);
    let before_origins = ids(before, &[&delta.nodes_changed, &delta.nodes_removed]);
    let impacted = after.classified_impact(&after_origins);
    let (tests_after, boundaries, listed_after) = test_paths(after, &after_origins);
    let (tests_before, _, listed_before) = test_paths(before, &before_origins);
    let after_set: BTreeSet<String> = listed_after.iter().map(|(p, _)| p.clone()).collect();
    let before_set: BTreeSet<String> = listed_before.iter().map(|(p, _)| p.clone()).collect();
    let newly: Vec<String> = after_set.difference(&before_set).cloned().collect();
    let gone: Vec<String> = before_set.difference(&after_set).cloned().collect();
    let truncated =
        after_set.len() > LIST_LIMIT || newly.len() > LIST_LIMIT || gone.len() > LIST_LIMIT;
    let bounded = |mut list: Vec<String>| {
        list.truncate(LIST_LIMIT);
        list
    };
    PredictedImpact {
        label: "predicted, not execution evidence",
        origins: after_origins.len(),
        impacted_nodes: ClassCounts {
            must: impacted.must.len(),
            may: impacted.may.len(),
            unknown: impacted.unknown.len(),
        },
        boundaries,
        tests_before,
        tests_after,
        tests_after_listed: bounded(after_set.into_iter().collect()),
        newly_reachable: bounded(newly),
        no_longer_reachable: bounded(gone),
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_binds_the_path_so_identical_bodies_differ() {
        let body = "fn render(&self) -> String { String::new() }";
        assert_ne!(
            fingerprint("crate::a::render", body),
            fingerprint("crate::a::render@Render", body)
        );
        assert_eq!(
            fingerprint("crate::a::render", body),
            fingerprint("crate::a::render", body)
        );
        assert_eq!(fingerprint("p", "s").len(), 64);
    }

    use crate::project::config::ProjectConfig;
    use crate::project::planfile::schema::{DeclaredEdges, DeclaredNodes};
    use crate::project::source::build_from_dir_with_config;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    const ALPHA: &str = "pub fn alpha() -> i32 {\n    1\n}";
    const SOURCE: &str = "pub fn alpha() -> i32 {\n    1\n}\n\npub fn beta() -> i32 {\n    2\n}\n";

    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "girder-verify-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(path.join("src")).unwrap();
            std::fs::write(path.join("src/lib.rs"), SOURCE).unwrap();
            Self(path)
        }

        fn graph(&self) -> SemanticGraph {
            build_from_dir_with_config(&self.0, &ProjectConfig::load(&self.0).unwrap())
                .unwrap()
                .0
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn node_source(graph: &SemanticGraph, path: &str) -> String {
        graph
            .nodes()
            .find(|n| n.path == path)
            .unwrap()
            .source
            .clone()
    }

    fn step(replacement: &str, baseline: &[(&str, String)], changed: &[&str]) -> Step {
        let list = |items: &[&str]| Some(items.iter().map(|i| i.to_string()).collect::<Vec<_>>());
        Step {
            id: "s1".into(),
            description: String::new(),
            edits: vec![Edit::ReplaceNode {
                node: "crate::lib::alpha".into(),
                replacement: replacement.into(),
            }],
            checks: Vec::new(),
            verify: Some(Verify {
                baseline: Some(
                    baseline
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.clone()))
                        .collect(),
                ),
                delta: Some(DeclaredDelta {
                    nodes: Some(DeclaredNodes {
                        changed: list(changed),
                        added: list(&[]),
                        removed: list(&[]),
                    }),
                    edges: Some(DeclaredEdges {
                        added: Some(Vec::new()),
                        removed: Some(Vec::new()),
                    }),
                }),
            }),
        }
    }

    fn prepared(dir: &Dir, replacement: &str) -> (Prepared, SemanticGraph) {
        let before = dir.graph();
        let baseline = [(
            "crate::lib::alpha",
            fingerprint(
                "crate::lib::alpha",
                &node_source(&before, "crate::lib::alpha"),
            ),
        )];
        let step = step(replacement, &baseline, &["crate::lib::alpha"]);
        let ready = pre_check(&step, step.verify.as_ref().unwrap(), &before, &dir.0).unwrap();
        (ready, before)
    }

    #[test]
    fn a_correct_edit_passes_every_post_check() {
        let dir = Dir::new();
        let replacement = "pub fn alpha() -> i32 {\n    10\n}";
        let (ready, before) = prepared(&dir, replacement);
        std::fs::write(dir.0.join("src/lib.rs"), SOURCE.replace(ALPHA, replacement)).unwrap();
        let after = dir.graph();
        let delta = post_check(&ready, &before, &after, Some(&after), &dir.0).unwrap();
        assert_eq!(delta.nodes_changed, vec!["crate::lib::alpha".to_string()]);
        assert!(delta.edges_added.is_empty() && delta.edges_removed.is_empty());
    }

    #[test]
    fn projection_exactness_refuses_a_file_changed_outside_the_span() {
        let dir = Dir::new();
        let replacement = "pub fn alpha() -> i32 {\n    10\n}";
        let (ready, before) = prepared(&dir, replacement);
        // The span is replaced correctly, but a comment is also added elsewhere in the file.
        let doctored = format!("// stray\n{}", SOURCE.replace(ALPHA, replacement));
        std::fs::write(dir.0.join("src/lib.rs"), doctored).unwrap();
        let after = dir.graph();
        let refusal = post_check(&ready, &before, &after, None, &dir.0).unwrap_err();
        assert_eq!(refusal.category, "insufficient_evidence");
        assert!(
            refusal.detail.contains("projection mismatch"),
            "{}",
            refusal.detail
        );
    }

    #[test]
    fn an_incremental_graph_that_differs_from_cold_is_refused() {
        let dir = Dir::new();
        let replacement = "pub fn alpha() -> i32 {\n    10\n}";
        let (ready, before) = prepared(&dir, replacement);
        std::fs::write(dir.0.join("src/lib.rs"), SOURCE.replace(ALPHA, replacement)).unwrap();
        let after = dir.graph();
        // The "incremental" graph still holds the pre-edit state.
        let refusal = post_check(&ready, &before, &after, Some(&before), &dir.0).unwrap_err();
        assert_eq!(refusal.category, "insufficient_evidence");
        assert!(
            refusal.detail.contains("incremental graph differs"),
            "{}",
            refusal.detail
        );
    }

    #[test]
    fn a_parse_error_introduced_by_the_edit_is_refused() {
        let dir = Dir::new();
        let broken = "pub fn alpha() -> i32 {\n    1 +\n";
        let (ready, before) = prepared(&dir, broken);
        std::fs::write(dir.0.join("src/lib.rs"), SOURCE.replace(ALPHA, broken)).unwrap();
        let after = dir.graph();
        let refusal = post_check(&ready, &before, &after, None, &dir.0).unwrap_err();
        assert_eq!(refusal.category, "insufficient_evidence");
        assert!(refusal.detail.contains("parse-error"), "{}", refusal.detail);
    }

    #[test]
    fn a_baseline_for_a_node_the_step_does_not_edit_is_refused() {
        let dir = Dir::new();
        let before = dir.graph();
        let baseline = [
            (
                "crate::lib::alpha",
                fingerprint(
                    "crate::lib::alpha",
                    &node_source(&before, "crate::lib::alpha"),
                ),
            ),
            (
                "crate::lib::beta",
                fingerprint(
                    "crate::lib::beta",
                    &node_source(&before, "crate::lib::beta"),
                ),
            ),
        ];
        let step = step(
            "pub fn alpha() -> i32 {\n    10\n}",
            &baseline,
            &["crate::lib::alpha"],
        );
        let refusal = pre_check(&step, step.verify.as_ref().unwrap(), &before, &dir.0).unwrap_err();
        assert_eq!(refusal.category, "insufficient_evidence");
        assert!(
            refusal.detail.contains("does not edit"),
            "{}",
            refusal.detail
        );
    }

    #[test]
    fn an_unknown_node_path_is_an_ambiguity_refusal() {
        let dir = Dir::new();
        let before = dir.graph();
        let mut step = step(
            "pub fn alpha() -> i32 {\n    10\n}",
            &[("crate::lib::nope", "x".into())],
            &[],
        );
        step.edits = vec![Edit::ReplaceNode {
            node: "crate::lib::nope".into(),
            replacement: "pub fn nope() {}".into(),
        }];
        let refusal = pre_check(&step, step.verify.as_ref().unwrap(), &before, &dir.0).unwrap_err();
        assert_eq!(refusal.category, "ambiguity");
    }

    #[test]
    fn refusal_message_names_the_category() {
        let refusal = Refusal::new("stale_input", "x");
        assert_eq!(refusal.message(), "verification refused [stale_input]: x");
    }
}
