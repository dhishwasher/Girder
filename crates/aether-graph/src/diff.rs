//! Semantic graph diffing — "what changed?" as graph mutations, not text lines.
//!
//! A [`GraphDiff`] describes how the semantic graph changed between two states:
//! which nodes were added/removed/modified, and which edges changed. This is the
//! foundation of graph-semantic code review: instead of character-by-character
//! text comparison, a change is expressed as typed mutations to the program model.

use crate::{EdgeKind, NodeId, NodeKind, SemanticGraph};
use std::collections::HashSet;

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
}
