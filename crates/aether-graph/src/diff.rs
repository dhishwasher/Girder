//! Semantic graph diffing — "what changed?" as graph mutations, not text lines.
//!
//! A [`GraphDiff`] describes how the semantic graph changed between two states:
//! which nodes were added/removed/modified, and which edges changed. This is the
//! foundation of graph-semantic code review: instead of character-by-character
//! text comparison, a change is expressed as typed mutations to the program model.

use crate::{EdgeKind, NodeId, NodeKind, SemanticGraph};
use std::collections::{HashMap, HashSet};

/// A node that participated in a graph change.
#[derive(Debug, Clone)]
pub struct NodeChange {
    pub id: NodeId,
    pub path: String,
    pub kind: NodeKind,
    pub language: String,
}

/// The semantic diff between two graph states.
///
/// Computed by [`SemanticGraph::diff_from`] by comparing node/edge sets.
/// Only meaningful semantic edges (`Calls`, `Inherits`, `DataFlow`) appear in
/// the edge diff — structural (`Contains`) and derived (`SemanticSimilar`,
/// `Impacts`) edges are omitted to keep the review signal-to-noise high.
#[derive(Debug, Default)]
pub struct GraphDiff {
    pub added: Vec<NodeChange>,
    pub removed: Vec<NodeChange>,
    pub modified: Vec<NodeChange>,
    pub added_edges: Vec<(NodeId, NodeId, EdgeKind)>,
    pub removed_edges: Vec<(NodeId, NodeId, EdgeKind)>,
}

impl GraphDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.modified.is_empty()
            && self.added_edges.is_empty()
            && self.removed_edges.is_empty()
    }

    /// All node IDs that changed (added + removed + modified).
    pub fn changed_node_ids(&self) -> Vec<NodeId> {
        self.added
            .iter()
            .chain(&self.removed)
            .chain(&self.modified)
            .map(|c| c.id)
            .collect()
    }

    /// Total number of changed nodes.
    pub fn node_change_count(&self) -> usize {
        self.added.len() + self.removed.len() + self.modified.len()
    }
}

/// Edge kinds worth showing in a semantic diff (structural/derived noise excluded).
fn is_review_edge(kind: EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::Calls | EdgeKind::Inherits | EdgeKind::DataFlow
    )
}

impl SemanticGraph {
    /// Compute the semantic diff between `self` (the current state) and
    /// `baseline` (the previous state, e.g. at git HEAD).
    ///
    /// - Added: nodes present in `self` but not in `baseline`.
    /// - Removed: nodes present in `baseline` but not in `self`.
    /// - Modified: nodes present in both but whose `source` text changed.
    /// - Edge diff: semantic edges (`Calls`/`Inherits`/`DataFlow`) that appeared
    ///   or disappeared between the two states.
    pub fn diff_from(&self, baseline: &SemanticGraph) -> GraphDiff {
        let mut diff = GraphDiff::default();

        let current_ids: HashSet<NodeId> = self.nodes().map(|n| n.id).collect();
        let baseline_ids: HashSet<NodeId> = baseline.nodes().map(|n| n.id).collect();

        for &id in current_ids.difference(&baseline_ids) {
            if let Some(n) = self.get(id) {
                diff.added.push(NodeChange {
                    id: n.id,
                    path: n.path.clone(),
                    kind: n.kind,
                    language: n.language.clone(),
                });
            }
        }

        for &id in baseline_ids.difference(&current_ids) {
            if let Some(n) = baseline.get(id) {
                diff.removed.push(NodeChange {
                    id: n.id,
                    path: n.path.clone(),
                    kind: n.kind,
                    language: n.language.clone(),
                });
            }
        }

        for &id in current_ids.intersection(&baseline_ids) {
            if let (Some(cur), Some(base)) = (self.get(id), baseline.get(id)) {
                if cur.source != base.source {
                    diff.modified.push(NodeChange {
                        id: cur.id,
                        path: cur.path.clone(),
                        kind: cur.kind,
                        language: cur.language.clone(),
                    });
                }
            }
        }

        let current_edges: HashSet<(NodeId, NodeId, EdgeKind)> = self
            .edges()
            .into_iter()
            .filter(|(_, _, k)| is_review_edge(*k))
            .collect();
        let baseline_edges: HashSet<(NodeId, NodeId, EdgeKind)> = baseline
            .edges()
            .into_iter()
            .filter(|(_, _, k)| is_review_edge(*k))
            .collect();

        diff.added_edges = current_edges.difference(&baseline_edges).copied().collect();
        diff.removed_edges = baseline_edges.difference(&current_edges).copied().collect();

        diff.added.sort_by(|a, b| a.path.cmp(&b.path));
        diff.removed.sort_by(|a, b| a.path.cmp(&b.path));
        diff.modified.sort_by(|a, b| a.path.cmp(&b.path));
        diff.added_edges.sort();
        diff.removed_edges.sort();

        diff
    }
}

/// Drops a Module-kind id from `origin_ids` when its own content, with
/// every sibling Function-kind origin's span (from the same file) masked
/// out, is byte-identical between `baseline` and `current` -- i.e. every
/// part of the module that changed is already accounted for by a Function
/// origin already in the list. Shared by every caller that turns a
/// before/after `SemanticGraph` pair into `classified_impact`'s origins
/// (`crates/aether-app`'s `test-impact` CLI path and the `tests.impacted`
/// plan check), so both apply the identical, precise rule instead of each
/// re-deriving it (the earlier, independent "any same-file Function origin
/// exists" approximation both used gave the wrong, silent-miss answer
/// whenever an edit also touched content OUTSIDE every origin function's
/// own span -- confirmed on a real Go graph, see
/// docs/observations/stage3-typescript-audit/before-observation-
/// addendum-10.md). Fails closed (keeps the Module) on any uncertainty: a
/// missing node in either snapshot, or a function whose span can't be
/// read in both.
pub fn origins_excluding_explained_modules(
    current: &SemanticGraph,
    baseline: &SemanticGraph,
    mut origin_ids: Vec<NodeId>,
) -> Vec<NodeId> {
    let module_ids: Vec<NodeId> = origin_ids
        .iter()
        .copied()
        .filter(|id| current.get(*id).is_some_and(|n| n.kind == NodeKind::Module))
        .collect();
    if module_ids.is_empty() {
        return origin_ids;
    }
    let mut function_origins_by_file: HashMap<&str, Vec<NodeId>> = HashMap::new();
    for node in origin_ids.iter().filter_map(|id| current.get(*id)) {
        if node.kind != NodeKind::Function {
            continue;
        }
        if let Some(file) = node.file.as_deref() {
            function_origins_by_file
                .entry(file)
                .or_default()
                .push(node.id);
        }
    }
    let explained: HashSet<NodeId> = module_ids
        .into_iter()
        .filter(|&module_id| {
            let Some(file) = current.get(module_id).and_then(|n| n.file.as_deref()) else {
                return false;
            };
            let Some(function_ids) = function_origins_by_file.get(file) else {
                return false;
            };
            module_change_fully_explained_by_function_origins(
                current,
                baseline,
                module_id,
                function_ids,
            )
        })
        .collect();
    origin_ids.retain(|id| !explained.contains(id));
    origin_ids
}

fn module_change_fully_explained_by_function_origins(
    current: &SemanticGraph,
    baseline: &SemanticGraph,
    module_id: NodeId,
    function_ids: &[NodeId],
) -> bool {
    let (Some(current_module), Some(baseline_module)) =
        (current.get(module_id), baseline.get(module_id))
    else {
        return false;
    };
    let mut current_spans = Vec::with_capacity(function_ids.len());
    let mut baseline_spans = Vec::with_capacity(function_ids.len());
    for &id in function_ids {
        let (Some(current_fn), Some(baseline_fn)) = (current.get(id), baseline.get(id)) else {
            // A function that only exists in one snapshot (newly added or
            // removed) can't be masked out of the other -- fail closed
            // rather than guess at its span there.
            return false;
        };
        current_spans.push((current_fn.span.start_byte, current_fn.span.end_byte));
        baseline_spans.push((baseline_fn.span.start_byte, baseline_fn.span.end_byte));
    }
    mask_spans(&current_module.source, current_spans)
        == mask_spans(&baseline_module.source, baseline_spans)
}

/// `text` with every byte range in `spans` removed, concatenating the
/// gaps between (and around) them. `spans` need not be sorted or
/// non-overlapping on input; out-of-range bytes are clamped rather than
/// panicking, since a span computed against a differently-edited snapshot
/// of the same file could legitimately run past this one's own length.
fn mask_spans(text: &str, mut spans: Vec<(usize, usize)>) -> String {
    spans.sort_unstable();
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut pos = 0usize;
    for (start, end) in spans {
        let start = start.min(bytes.len());
        let end = end.min(bytes.len());
        if start > pos {
            out.extend_from_slice(&bytes[pos..start]);
        }
        pos = pos.max(end);
    }
    if pos < bytes.len() {
        out.extend_from_slice(&bytes[pos..]);
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, Node};

    fn make_fn(path: &str, source: &str) -> Node {
        Node::new(
            NodeKind::Function,
            path.rsplit("::").next().unwrap_or(path),
            path,
        )
        .with_source(source)
        .with_language("rust")
    }

    #[test]
    fn detects_added_and_removed_nodes() {
        let mut baseline = SemanticGraph::new();
        baseline.upsert_node(make_fn("crate::m::add", "fn add(a:i64,b:i64)->i64{a+b}"));

        let mut current = SemanticGraph::new();
        current.upsert_node(make_fn("crate::m::add", "fn add(a:i64,b:i64)->i64{a+b}"));
        current.upsert_node(make_fn("crate::m::sub", "fn sub(a:i64,b:i64)->i64{a-b}"));

        let diff = current.diff_from(&baseline);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].path, "crate::m::sub");
        assert!(diff.removed.is_empty());
        assert!(diff.modified.is_empty());
    }

    #[test]
    fn detects_modified_source() {
        let mut baseline = SemanticGraph::new();
        baseline.upsert_node(make_fn("crate::m::add", "fn add(a:i64,b:i64)->i64{a+b}"));

        let mut current = SemanticGraph::new();
        current.upsert_node(make_fn("crate::m::add", "fn add(a:i64,b:i64)->i64{a*b}"));

        let diff = current.diff_from(&baseline);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert_eq!(diff.modified.len(), 1);
        assert_eq!(diff.modified[0].path, "crate::m::add");
    }

    #[test]
    fn detects_new_call_edge() {
        let add_id = NodeId::from_path("crate::m::add");
        let sum_id = NodeId::from_path("crate::m::sum");

        let mut baseline = SemanticGraph::new();
        baseline.upsert_node(make_fn("crate::m::add", "fn add(a:i64,b:i64)->i64{a+b}"));
        baseline.upsert_node(make_fn("crate::m::sum", "fn sum()->i64{0}"));

        let mut current = baseline.clone();
        current
            .add_edge(sum_id, add_id, Edge::new(EdgeKind::Calls))
            .unwrap();

        let diff = current.diff_from(&baseline);
        assert_eq!(diff.added_edges.len(), 1);
        assert_eq!(diff.added_edges[0], (sum_id, add_id, EdgeKind::Calls));
        assert!(diff.removed_edges.is_empty());
    }

    #[test]
    fn unchanged_graph_produces_empty_diff() {
        let mut g = SemanticGraph::new();
        g.upsert_node(make_fn("crate::m::add", "fn add(a:i64,b:i64)->i64{a+b}"));
        let diff = g.diff_from(&g.clone());
        assert!(diff.is_empty());
    }

    fn make_module(path: &str, source: &str, file: &str) -> Node {
        let mut n = Node::new(NodeKind::Module, path, path).with_source(source);
        n.file = Some(file.to_string());
        n
    }

    fn with_span(mut n: Node, start_byte: usize, end_byte: usize) -> Node {
        n.span = crate::Span {
            start_byte,
            end_byte,
            start_row: 0,
            start_col: 0,
        };
        n
    }

    fn with_file(mut n: Node, file: &str) -> Node {
        n.file = Some(file.to_string());
        n
    }

    #[test]
    fn module_origin_explained_by_its_sole_function_origin_is_excluded() {
        // "fn target(){1}\nfn other(){2}\n" -> "fn target(){9}\nfn other(){2}\n":
        // only `target`'s own span differs; everything outside it (the
        // `\nfn other(){2}\n` suffix) is byte-identical.
        let file = "sample.rs";
        let baseline_source = "fn target(){1}\nfn other(){2}\n";
        let current_source = "fn target(){9}\nfn other(){2}\n";

        let mut baseline = SemanticGraph::new();
        let module_id = baseline.upsert_node(make_module("crate::sample", baseline_source, file));
        let target_id = baseline.upsert_node(with_file(
            with_span(make_fn("crate::sample::target", "fn target(){1}"), 0, 15),
            file,
        ));

        let mut current = SemanticGraph::new();
        current.upsert_node(make_module("crate::sample", current_source, file));
        current.upsert_node(with_file(
            with_span(make_fn("crate::sample::target", "fn target(){9}"), 0, 15),
            file,
        ));

        let result =
            origins_excluding_explained_modules(&current, &baseline, vec![module_id, target_id]);
        assert_eq!(
            result,
            vec![target_id],
            "a Module origin fully explained by its sole Function origin \
             must be excluded"
        );
    }

    #[test]
    fn module_origin_with_unexplained_content_alongside_an_edited_function_is_kept() {
        // The confirmed combined-origin residual (docs/observations/
        // stage3-typescript-audit/before-observation-addendum-10.md): a
        // module-level `const` ALSO changes, outside `other`'s own span,
        // so masking out only `other` still leaves a real difference.
        let file = "sample.rs";
        let baseline_source = "const X: i64 = 1;\nfn target(){1}\nfn other(){2}\n";
        let current_source = "const X: i64 = 2;\nfn target(){1}\nfn other(){9}\n";
        let other_start = "const X: i64 = 1;\nfn target(){1}\n".len();
        let other_end = other_start + "fn other(){2}".len();

        let mut baseline = SemanticGraph::new();
        let module_id = baseline.upsert_node(make_module("crate::sample", baseline_source, file));
        let other_id = baseline.upsert_node(with_file(
            with_span(
                make_fn("crate::sample::other", "fn other(){2}"),
                other_start,
                other_end,
            ),
            file,
        ));

        let mut current = SemanticGraph::new();
        current.upsert_node(make_module("crate::sample", current_source, file));
        current.upsert_node(with_file(
            with_span(
                make_fn("crate::sample::other", "fn other(){9}"),
                other_start,
                other_end,
            ),
            file,
        ));

        let result =
            origins_excluding_explained_modules(&current, &baseline, vec![module_id, other_id]);
        assert_eq!(
            result,
            vec![module_id, other_id],
            "a Module origin with content changed OUTSIDE every origin \
             function's own span must stay, even alongside an edited \
             sibling function"
        );
    }
}
