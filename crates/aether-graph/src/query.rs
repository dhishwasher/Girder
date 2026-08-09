//! Read-only queries over the semantic graph.

use crate::{Edge, EdgeKind, Node, NodeId, NodeKind, SemanticGraph};
use petgraph::Direction;

/// A neighbor reached by traversing one edge.
#[derive(Debug, Clone)]
pub struct Neighbor {
    pub id: NodeId,
    pub kind: EdgeKind,
    pub weight: f32,
}

impl SemanticGraph {
    /// All nodes of a given kind.
    pub fn query_by_kind(&self, kind: NodeKind) -> Vec<&Node> {
        self.nodes().filter(|n| n.kind == kind).collect()
    }

    /// Every edge as `(source_id, target_id, kind)` — used by the graph viewer.
    pub fn edges(&self) -> Vec<(NodeId, NodeId, EdgeKind)> {
        self.edge_records()
            .into_iter()
            .map(|(from, to, edge)| (from, to, edge.kind))
            .collect()
    }

    /// Every edge with its complete payload.
    pub fn edge_records(&self) -> Vec<(NodeId, NodeId, Edge)> {
        use petgraph::visit::{EdgeRef, IntoEdgeReferences};
        self.raw()
            .edge_references()
            .map(|e| {
                (
                    self.id_at(e.source()),
                    self.id_at(e.target()),
                    e.weight().clone(),
                )
            })
            .collect()
    }

    /// Look a node up by its fully-qualified path.
    pub fn find_by_path(&self, path: &str) -> Option<&Node> {
        self.get(NodeId::from_path(path))
    }

    /// Substring/case-insensitive search over names and paths — the backing
    /// store for the command palette and agent "find the X function" queries.
    pub fn search(&self, needle: &str) -> Vec<&Node> {
        let needle = needle.to_lowercase();
        self.nodes()
            .filter(|n| {
                n.name.to_lowercase().contains(&needle) || n.path.to_lowercase().contains(&needle)
            })
            .collect()
    }

    /// Outgoing neighbors, optionally filtered to a single edge kind.
    pub fn neighbors(&self, id: NodeId, kind: Option<EdgeKind>) -> Vec<Neighbor> {
        self.directional_neighbors(id, Direction::Outgoing, kind)
    }

    /// Incoming neighbors (who points *at* this node).
    pub fn callers(&self, id: NodeId) -> Vec<Neighbor> {
        self.directional_neighbors(id, Direction::Incoming, Some(EdgeKind::Calls))
    }

    /// All test-marked nodes in the impact set of `origin` — i.e. every test
    /// that can be reached by a change to `origin` via the call graph. This is
    /// the minimal set of tests that must re-run when that function changes.
    /// Ordered by node path so output and generated commands are
    /// deterministic across runs.
    pub fn tests_for(&self, origin: NodeId) -> Vec<NodeId> {
        let impact = self.impact_of(origin);
        let mut tests: Vec<NodeId> = impact
            .affected
            .keys()
            .copied()
            .chain(std::iter::once(origin))
            .filter(|&id| {
                self.get(id)
                    .map(|n| n.attr("is_test").is_some())
                    .unwrap_or(false)
            })
            .collect();
        self.sort_by_path(&mut tests);
        tests
    }

    /// Union of test nodes reachable from any of the given origin nodes.
    /// Deduplicates so each test appears at most once; path-ordered.
    pub fn tests_for_nodes(&self, origins: &[NodeId]) -> Vec<NodeId> {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        let mut tests: Vec<NodeId> = origins
            .iter()
            .flat_map(|&id| self.tests_for(id))
            .filter(|id| seen.insert(*id))
            .collect();
        self.sort_by_path(&mut tests);
        tests
    }

    fn sort_by_path(&self, ids: &mut [NodeId]) {
        ids.sort_by(|a, b| {
            let left = self.get(*a).map(|n| n.path.as_str()).unwrap_or_default();
            let right = self.get(*b).map(|n| n.path.as_str()).unwrap_or_default();
            left.cmp(right).then_with(|| a.cmp(b))
        });
    }

    fn directional_neighbors(
        &self,
        id: NodeId,
        dir: Direction,
        kind: Option<EdgeKind>,
    ) -> Vec<Neighbor> {
        let Some(idx) = self.index_of(id) else {
            return Vec::new();
        };
        let raw = self.raw();
        let mut out = Vec::new();
        for edge in raw.edges_directed(idx, dir) {
            let e: &Edge = edge.weight();
            if kind.is_none_or(|k| k == e.kind) {
                use petgraph::visit::EdgeRef;
                let other = match dir {
                    Direction::Outgoing => edge.target(),
                    Direction::Incoming => edge.source(),
                };
                out.push(Neighbor {
                    id: self.id_at(other),
                    kind: e.kind,
                    weight: e.weight,
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_for_finds_test_in_impact_set() {
        let mut g = SemanticGraph::new();
        let add = g.upsert_node(Node::new(NodeKind::Function, "add", "crate::m::add"));
        let mut test_add = Node::new(NodeKind::Function, "test_add", "crate::m::test_add");
        test_add.set_attr("is_test", "true");
        let test_add_id = g.upsert_node(test_add);
        // test_add calls add, so changing add puts test_add in impact set
        g.add_edge(test_add_id, add, Edge::new(EdgeKind::Calls))
            .unwrap();

        let tests = g.tests_for(add);
        assert_eq!(tests, vec![test_add_id]);
    }

    #[test]
    fn tests_for_nodes_deduplicates_shared_tests() {
        // test_both calls add and sub; changing either should return test_both once.
        let mut g = SemanticGraph::new();
        let add = g.upsert_node(Node::new(NodeKind::Function, "add", "crate::m::add"));
        let sub = g.upsert_node(Node::new(NodeKind::Function, "sub", "crate::m::sub"));
        let mut t = Node::new(NodeKind::Function, "test_both", "crate::m::test_both");
        t.set_attr("is_test", "true");
        let t_id = g.upsert_node(t);
        g.add_edge(t_id, add, Edge::new(EdgeKind::Calls)).unwrap();
        g.add_edge(t_id, sub, Edge::new(EdgeKind::Calls)).unwrap();

        let tests = g.tests_for_nodes(&[add, sub]);
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0], t_id);
    }

    #[test]
    fn non_test_callers_excluded() {
        let mut g = SemanticGraph::new();
        let add = g.upsert_node(Node::new(NodeKind::Function, "add", "crate::m::add"));
        let sum = g.upsert_node(Node::new(NodeKind::Function, "sum", "crate::m::sum"));
        g.add_edge(sum, add, Edge::new(EdgeKind::Calls)).unwrap();

        // sum calls add but is not a test — should not appear
        assert!(g.tests_for(add).is_empty());
    }

    #[test]
    fn duplicate_edges_are_updated_not_accumulated() {
        let mut g = SemanticGraph::new();
        let add = g.upsert_node(Node::new(NodeKind::Function, "add", "crate::m::add"));
        let sum = g.upsert_node(Node::new(NodeKind::Function, "sum", "crate::m::sum"));

        g.add_edge(sum, add, Edge::new(EdgeKind::Calls)).unwrap();
        g.add_edge(sum, add, Edge::with_weight(EdgeKind::Calls, 0.5))
            .unwrap();

        let callers = g.callers(add);
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].id, sum);
        assert_eq!(callers[0].weight, 0.5);
    }
}
