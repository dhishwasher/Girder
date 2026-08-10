//! `graph.*` checks — the reason this format exists. All operate on the
//! graph rebuilt after a step's edits (`callers_of`/`callees_of`/`tests_for`/
//! `node_exists`/`node_absent`/`unresolved`), or on the before/after pair
//! (`no_new_edges_into`/`edge_delta`). A node path that doesn't resolve is
//! always a check failure, never a silent pass — fail closed.

use super::CheckOutcome;
use crate::project::planfile::schema::{ExpectResult, Mode};
use aether_graph::{EdgeKind, NodeId, SemanticGraph};

fn resolve(graph: &SemanticGraph, path: &str) -> Option<NodeId> {
    graph.find_by_path(path).map(|node| node.id)
}

fn evaluate_set(
    kind: &str,
    mut actual: Vec<String>,
    expect: &[String],
    mode: Mode,
) -> CheckOutcome {
    actual.sort();
    let mut expected: Vec<String> = expect.to_vec();
    expected.sort();
    let passed = match mode {
        Mode::Exact => actual == expected,
        Mode::Superset => expected.iter().all(|item| actual.contains(item)),
        Mode::Subset => actual.iter().all(|item| expected.contains(item)),
        Mode::Absent => !expected.iter().any(|item| actual.contains(item)),
    };
    CheckOutcome {
        kind: kind.to_string(),
        passed,
        detail: if passed {
            format!("{kind} matched ({} actual, mode {mode:?})", actual.len())
        } else {
            format!("mode {mode:?}: expected {expected:?}, actual {actual:?}")
        },
    }
}

pub(crate) fn callers_of(
    graph: &SemanticGraph,
    node: &str,
    expect: &[String],
    mode: Mode,
) -> CheckOutcome {
    let Some(id) = resolve(graph, node) else {
        return not_found("graph.callers_of", node);
    };
    let actual: Vec<String> = graph
        .callers(id)
        .into_iter()
        .filter_map(|neighbor| graph.get(neighbor.id).map(|node| node.path.clone()))
        .collect();
    evaluate_set("graph.callers_of", actual, expect, mode)
}

pub(crate) fn callees_of(
    graph: &SemanticGraph,
    node: &str,
    expect: &[String],
    mode: Mode,
) -> CheckOutcome {
    let Some(id) = resolve(graph, node) else {
        return not_found("graph.callees_of", node);
    };
    let actual: Vec<String> = graph
        .neighbors(id, Some(EdgeKind::Calls))
        .into_iter()
        .filter_map(|neighbor| graph.get(neighbor.id).map(|node| node.path.clone()))
        .collect();
    evaluate_set("graph.callees_of", actual, expect, mode)
}

pub(crate) fn tests_for(
    graph: &SemanticGraph,
    node: &str,
    expect: &[String],
    mode: Mode,
) -> CheckOutcome {
    let Some(id) = resolve(graph, node) else {
        return not_found("graph.tests_for", node);
    };
    let actual: Vec<String> = graph
        .tests_for(id)
        .into_iter()
        .filter_map(|id| graph.get(id).map(|node| node.path.clone()))
        .collect();
    evaluate_set("graph.tests_for", actual, expect, mode)
}

pub(crate) fn node_exists(graph: &SemanticGraph, node: &str) -> CheckOutcome {
    let passed = graph.find_by_path(node).is_some();
    CheckOutcome {
        kind: "graph.node_exists".to_string(),
        passed,
        detail: if passed {
            format!("{node} exists")
        } else {
            format!("{node} not found")
        },
    }
}

pub(crate) fn node_absent(graph: &SemanticGraph, node: &str) -> CheckOutcome {
    let exists = graph.find_by_path(node).is_some();
    CheckOutcome {
        kind: "graph.node_absent".to_string(),
        passed: !exists,
        detail: if exists {
            format!("{node} still exists")
        } else {
            format!("{node} is absent, as expected")
        },
    }
}

pub(crate) fn no_new_edges_into(
    before: &SemanticGraph,
    after: &SemanticGraph,
    node: &str,
) -> CheckOutcome {
    let Some(id) = resolve(after, node).or_else(|| resolve(before, node)) else {
        return not_found("graph.no_new_edges_into", node);
    };
    let before_into: Vec<(NodeId, NodeId, EdgeKind)> = before
        .edges()
        .into_iter()
        .filter(|(_, dst, _)| *dst == id)
        .collect();
    let after_into: Vec<(NodeId, NodeId, EdgeKind)> = after
        .edges()
        .into_iter()
        .filter(|(_, dst, _)| *dst == id)
        .collect();
    let new_edges: Vec<&(NodeId, NodeId, EdgeKind)> = after_into
        .iter()
        .filter(|edge| !before_into.contains(edge))
        .collect();
    let passed = new_edges.is_empty();
    CheckOutcome {
        kind: "graph.no_new_edges_into".to_string(),
        passed,
        detail: if passed {
            format!("no new inbound edges into {node}")
        } else {
            let sources: Vec<String> = new_edges
                .iter()
                .filter_map(|(src, _, kind)| {
                    after.get(*src).map(|n| format!("{} ({kind:?})", n.path))
                })
                .collect();
            format!(
                "{} new inbound edge(s) into {node}: {}",
                sources.len(),
                sources.join(", ")
            )
        },
    }
}

pub(crate) fn edge_delta(
    before: &SemanticGraph,
    after: &SemanticGraph,
    max_added: usize,
    max_removed: usize,
) -> CheckOutcome {
    let before_edges = before.edges();
    let after_edges = after.edges();
    let added = after_edges
        .iter()
        .filter(|edge| !before_edges.contains(edge))
        .count();
    let removed = before_edges
        .iter()
        .filter(|edge| !after_edges.contains(edge))
        .count();
    let passed = added <= max_added && removed <= max_removed;
    CheckOutcome {
        kind: "graph.edge_delta".to_string(),
        passed,
        detail: format!("added {added} (max {max_added}), removed {removed} (max {max_removed})"),
    }
}

/// Edge-absence semantics for v1 (a real "resolver saw this and declined
/// it" concept does not exist in the graph yet — true unresolved-call
/// tracking is a named v2 fast-follow). Both `from` and `node` must resolve
/// to real nodes first; an unresolvable path fails closed with "node not
/// found" rather than vacuously passing.
pub(crate) fn unresolved(
    graph: &SemanticGraph,
    node: &str,
    from: &str,
    expect_result: ExpectResult,
) -> CheckOutcome {
    let Some(from_id) = resolve(graph, from) else {
        return not_found("graph.unresolved", from);
    };
    let Some(node_id) = resolve(graph, node) else {
        return not_found("graph.unresolved", node);
    };
    let resolves = graph
        .neighbors(from_id, Some(EdgeKind::Calls))
        .into_iter()
        .any(|neighbor| neighbor.id == node_id);
    let is_unresolved = !resolves;
    let passed = match expect_result {
        ExpectResult::Pass => is_unresolved,
        ExpectResult::Fail => !is_unresolved,
    };
    CheckOutcome {
        kind: "graph.unresolved".to_string(),
        passed,
        detail: format!(
            "{from} {} {node} (expect_result={expect_result:?})",
            if resolves {
                "resolves to"
            } else {
                "does not resolve to"
            }
        ),
    }
}

fn not_found(kind: &str, node: &str) -> CheckOutcome {
    CheckOutcome {
        kind: kind.to_string(),
        passed: false,
        detail: format!("node not found: {node}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_builder::GraphBuilder;

    fn graph_from(files: &[(&str, &str)]) -> SemanticGraph {
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files.iter().copied());
        graph
    }

    #[test]
    fn callers_of_matches_exact_mode() {
        // `module_path_for` (aether-builder) has no crate-root special case
        // for lib.rs/main.rs — every file, "src/code.rs" included, becomes
        // `crate::code::...`.
        let graph = graph_from(&[(
            "src/code.rs",
            "pub fn callee() -> i64 { 1 }\npub fn caller() -> i64 { callee() }\n",
        )]);
        let outcome = callers_of(
            &graph,
            "crate::code::callee",
            &["crate::code::caller".to_string()],
            Mode::Exact,
        );
        assert!(outcome.passed, "{}", outcome.detail);
    }

    #[test]
    fn callers_of_fails_closed_on_an_unresolvable_node() {
        let graph = graph_from(&[("src/code.rs", "pub fn a() {}\n")]);
        let outcome = callers_of(&graph, "crate::code::does_not_exist", &[], Mode::Exact);
        assert!(!outcome.passed);
        assert!(outcome.detail.contains("not found"), "{}", outcome.detail);
    }

    #[test]
    fn node_exists_and_node_absent_are_exact_complements() {
        let graph = graph_from(&[("src/code.rs", "pub fn present() {}\n")]);
        assert!(node_exists(&graph, "crate::code::present").passed);
        assert!(!node_absent(&graph, "crate::code::present").passed);
        assert!(!node_exists(&graph, "crate::code::missing").passed);
        assert!(node_absent(&graph, "crate::code::missing").passed);
    }

    #[test]
    fn unresolved_fails_closed_when_either_node_path_is_unknown() {
        let graph = graph_from(&[("src/code.rs", "pub fn a() {}\n")]);
        let outcome = unresolved(
            &graph,
            "crate::code::b",
            "crate::code::a",
            ExpectResult::Pass,
        );
        assert!(!outcome.passed);
        assert!(outcome.detail.contains("not found"), "{}", outcome.detail);
    }

    #[test]
    fn unresolved_prove_then_fix_pattern() {
        // Before a fix: the call resolves (a bug). expect_result "fail"
        // inverts the assertion to prove the bug is present.
        let buggy = graph_from(&[(
            "src/code.rs",
            "pub fn callee() -> i64 { 1 }\npub fn caller() -> i64 { callee() }\n",
        )]);
        let prove_gap = unresolved(
            &buggy,
            "crate::code::callee",
            "crate::code::caller",
            ExpectResult::Fail,
        );
        assert!(prove_gap.passed, "{}", prove_gap.detail);

        // After a fix: the call no longer resolves. Default expect_result
        // (Pass) asserts it stays unresolved.
        let fixed = graph_from(&[(
            "src/code.rs",
            "pub fn callee() -> i64 { 1 }\npub fn caller() -> i64 { 0 }\n",
        )]);
        let closed = unresolved(
            &fixed,
            "crate::code::callee",
            "crate::code::caller",
            ExpectResult::Pass,
        );
        assert!(closed.passed, "{}", closed.detail);
    }

    #[test]
    fn edge_delta_counts_added_and_removed_edges() {
        let before = graph_from(&[("src/code.rs", "pub fn a() {}\npub fn b() { a(); }\n")]);
        let after = graph_from(&[("src/code.rs", "pub fn a() {}\npub fn b() {}\n")]);
        let outcome = edge_delta(&before, &after, 0, 1);
        assert!(outcome.passed, "{}", outcome.detail);
        let too_strict = edge_delta(&before, &after, 0, 0);
        assert!(!too_strict.passed);
    }

    #[test]
    fn no_new_edges_into_detects_a_freshly_introduced_caller() {
        let before = graph_from(&[("src/code.rs", "pub fn target() {}\npub fn other() {}\n")]);
        let after = graph_from(&[(
            "src/code.rs",
            "pub fn target() {}\npub fn other() { target(); }\n",
        )]);
        let outcome = no_new_edges_into(&before, &after, "crate::code::target");
        assert!(!outcome.passed);
        assert!(outcome.detail.contains("other"), "{}", outcome.detail);
    }
}
