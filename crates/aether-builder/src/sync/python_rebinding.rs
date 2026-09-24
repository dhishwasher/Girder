//! Project-wide revert pass for Python same-file Must proofs whose target
//! name is rebound by a string literal living in a DIFFERENT file than the
//! target's own definition.
//!
//! `claims.rs`'s own per-file `annotate()` pass already excludes a
//! same-file string-literal rebinding (`setattr(obj, "target", ...)`,
//! `patch("pkg.mod.target")`) from being proven Must -- but it has no
//! visibility into any OTHER file. Checked directly against the real
//! audited packages, not just theoretically: a cross-file
//! `mocker.patch('pkg.module.target')` in a test file, naming a function
//! defined (and called from, elsewhere in the SAME file) in
//! `pkg/module.py`, would otherwise leave that same-file call site
//! incorrectly proven Must. This pass runs after all files are extracted,
//! builds a crate-wide index of every file's string literals, and reverts
//! any `proven-top-level-lexical-binding` Must claim whose target's own
//! name is rebound by a string literal ANYWHERE in the indexed project --
//! not just the target's own file.

use super::FileState;
use aether_graph::{CallClaim, CallClass, CallEvidence, SemanticGraph};
use std::collections::HashMap;

/// Whether `name` is named (bare, or as the last `.`-separated segment) by
/// any string literal in `all_string_literals`.
fn string_rebound(name: &str, all_string_literals: &std::collections::HashSet<String>) -> bool {
    all_string_literals
        .iter()
        .any(|s| s == name || s.ends_with(&format!(".{name}")))
}

pub(super) fn revert_string_rebound_python_claims(
    files: &HashMap<String, FileState>,
    graph: &mut SemanticGraph,
) {
    let mut all_string_literals = std::collections::HashSet::new();
    for state in files.values() {
        all_string_literals.extend(state.extraction.python_string_literals.iter().cloned());
    }
    if all_string_literals.is_empty() {
        return; // No Python files, or none contain any string literal at all.
    }

    for state in files.values() {
        for node in &state.extraction.nodes {
            let Ok(evidence) = graph.call_evidence(node.id) else {
                continue;
            };
            let mut changed = false;
            let mut calls: Vec<CallClaim> = Vec::with_capacity(evidence.calls.len());
            for claim in evidence.calls {
                let should_revert = claim.class == CallClass::Must
                    && claim.reason == "proven-top-level-lexical-binding"
                    && claim.targets.iter().any(|target_id| {
                        graph
                            .get(*target_id)
                            .is_some_and(|t| string_rebound(&t.name, &all_string_literals))
                    });
                if should_revert {
                    changed = true;
                    calls.push(CallClaim {
                        site: claim.site,
                        class: CallClass::Unknown,
                        targets: vec![],
                        reason: "python-target-string-rebound-elsewhere-in-crate".into(),
                        coverage_gap: true,
                    });
                } else {
                    calls.push(claim);
                }
            }
            if !changed {
                continue;
            }
            let Some(graph_node) = graph.get_mut(node.id) else {
                continue;
            };
            let _ = CallEvidence::new(graph_node, calls, evidence.assumptions).attach(graph_node);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::GraphBuilder;
    use aether_graph::{CallClass, SemanticGraph};

    #[test]
    fn a_same_file_must_claim_is_reverted_when_a_different_file_rebinds_the_target_by_string() {
        let files = [
            (
                "pkg/module.py",
                "def target():\n\
                 \x20\x20\x20\x20return 42\n\
                 def caller():\n\
                 \x20\x20\x20\x20return target()\n",
            ),
            (
                "tests/test_module.py",
                "def test_it(mocker):\n\
                 \x20\x20\x20\x20mocker.patch('pkg.module.target', side_effect=ImportError)\n",
            ),
        ];
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files);
        let caller = graph.nodes().find(|n| n.name == "caller").unwrap();
        let evidence = graph.call_evidence(caller.id).unwrap();
        assert!(
            evidence.calls.iter().all(|c| c.class != CallClass::Must),
            "{evidence:?}"
        );
        assert!(
            evidence
                .calls
                .iter()
                .any(|c| c.reason == "python-target-string-rebound-elsewhere-in-crate"),
            "{evidence:?}"
        );
    }

    #[test]
    fn an_unrelated_name_in_another_file_does_not_revert_this_target() {
        let files = [
            (
                "pkg/module.py",
                "def target():\n\
                 \x20\x20\x20\x20return 42\n\
                 def caller():\n\
                 \x20\x20\x20\x20return target()\n",
            ),
            (
                "tests/test_module.py",
                "def test_it(mocker):\n\
                 \x20\x20\x20\x20mocker.patch('pkg.module.something_else', side_effect=ImportError)\n",
            ),
        ];
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files);
        let caller = graph.nodes().find(|n| n.name == "caller").unwrap();
        let evidence = graph.call_evidence(caller.id).unwrap();
        assert!(
            evidence.calls.iter().any(|c| c.class == CallClass::Must),
            "{evidence:?}"
        );
    }
}
