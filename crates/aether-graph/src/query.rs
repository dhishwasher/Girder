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
        use petgraph::visit::{EdgeRef, IntoEdgeReferences};
        self.raw()
            .edge_references()
            .map(|e| {
                (
                    self.id_at(e.source()),
                    self.id_at(e.target()),
                    e.weight().kind,
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
