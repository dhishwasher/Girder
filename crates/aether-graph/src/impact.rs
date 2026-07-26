//! Impact analysis: "if I change node X, what else is affected?"
//!
//! This powers Bit Code's real-time predictive impact panel. Because the
//! graph is the source of truth, impact is a graph reachability query rather
//! than a fragile text/heuristic search.

use crate::{EdgeKind, NodeId, SemanticGraph};
use petgraph::Direction;
use std::collections::{HashMap, VecDeque};

/// The result of an impact query: which nodes are reachable from a change, and
/// how many hops away they are (proximity ≈ risk).
#[derive(Debug, Clone, Default)]
pub struct ImpactReport {
    pub origin: Option<NodeId>,
    /// Affected node -> shortest hop distance from the origin.
    pub affected: HashMap<NodeId, u32>,
}

impl ImpactReport {
    /// Affected nodes ordered nearest-first (most likely to break first).
    pub fn ranked(&self) -> Vec<(NodeId, u32)> {
        let mut v: Vec<_> = self.affected.iter().map(|(&id, &d)| (id, d)).collect();
        v.sort_by_key(|(id, d)| (*d, *id));
        v
    }

    pub fn is_empty(&self) -> bool {
        self.affected.is_empty()
    }
}

impl SemanticGraph {
    /// Compute the impact set of changing `origin`.
    ///
    /// Impact flows *backwards* along call/dataflow edges: if `add` changes,
    /// everything that *calls* `add` (and transitively their callers) is at
    /// risk. We therefore walk `Incoming` edges, but only those whose kind
    /// [`EdgeKind::propagates_impact`].
    pub fn impact_of(&self, origin: NodeId) -> ImpactReport {
        let mut report = ImpactReport {
            origin: Some(origin),
            affected: HashMap::new(),
        };
        let Some(start) = self.index_of(origin) else {
            return report;
        };

        let raw = self.raw();
        let mut queue = VecDeque::new();
        queue.push_back((start, 0u32));

        while let Some((idx, dist)) = queue.pop_front() {
            for edge in raw.edges_directed(idx, Direction::Incoming) {
                use petgraph::visit::EdgeRef;
                if !edge.weight().kind.propagates_impact() {
                    continue;
                }
                let caller = edge.source();
                let caller_id = self.id_at(caller);
                // First time we reach a node is its shortest distance (BFS).
                if caller_id != origin && !report.affected.contains_key(&caller_id) {
                    report.affected.insert(caller_id, dist + 1);
                    queue.push_back((caller, dist + 1));
                }
            }
        }
        report
    }

    /// Materialize predicted-impact edges from a fresh analysis. The Coder /
    /// Optimizer / Refactorer agents call this so the impact relationships become
    /// first-class, queryable graph edges (with decaying confidence by distance).
    ///
    /// Idempotent: an `Impacts` edge is only added where one doesn't already
    /// exist from `origin`, so calling it repeatedly (e.g. after each edit) won't
    /// accumulate duplicates. Returns the number of new edges added.
    pub fn materialize_impact_edges(&mut self, origin: NodeId) -> usize {
        use std::collections::HashSet;
        let existing: HashSet<NodeId> = self
            .neighbors(origin, Some(EdgeKind::Impacts))
            .into_iter()
            .map(|n| n.id)
            .collect();

        let report = self.impact_of(origin);
        let mut added = 0;
        for (target, dist) in report.affected {
            if existing.contains(&target) {
                continue;
            }
            let weight = (1.0 / (dist as f32 + 1.0)).max(0.05);
            if self
                .add_edge(
                    origin,
                    target,
                    crate::Edge::with_weight(EdgeKind::Impacts, weight),
                )
                .is_ok()
            {
                added += 1;
            }
        }
        added
    }
}

#[cfg(test)]
mod tests {
    use crate::{Edge, EdgeKind, Node, NodeKind, SemanticGraph};

    #[test]
    fn materialize_impact_edges_is_queryable_and_idempotent() {
        // sum_list calls add; changing add impacts sum_list.
        let mut g = SemanticGraph::new();
        let add = g.upsert_node(Node::new(NodeKind::Function, "add", "crate::m::add"));
        let sum = g.upsert_node(Node::new(NodeKind::Function, "sum", "crate::m::sum"));
        g.add_edge(sum, add, Edge::new(EdgeKind::Calls)).unwrap();

        // Prediction becomes a first-class, re-queryable Impacts edge.
        let added = g.materialize_impact_edges(add);
        assert_eq!(added, 1);
        let impacted: Vec<_> = g
            .neighbors(add, Some(EdgeKind::Impacts))
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert_eq!(impacted, vec![sum]);

        // Calling again adds nothing (idempotent — no duplicate edges).
        assert_eq!(g.materialize_impact_edges(add), 0);
        assert_eq!(g.neighbors(add, Some(EdgeKind::Impacts)).len(), 1);
    }

    #[test]
    fn ranked_orders_nearest_first() {
        // a <- b <- c (calls), so changing a impacts b (1) then c (2).
        let mut g = SemanticGraph::new();
        let a = g.upsert_node(Node::new(NodeKind::Function, "a", "crate::m::a"));
        let b = g.upsert_node(Node::new(NodeKind::Function, "b", "crate::m::b"));
        let c = g.upsert_node(Node::new(NodeKind::Function, "c", "crate::m::c"));
        g.add_edge(b, a, Edge::new(EdgeKind::Calls)).unwrap();
        g.add_edge(c, b, Edge::new(EdgeKind::Calls)).unwrap();

        let ranked = g.impact_of(a).ranked();
        assert_eq!(ranked, vec![(b, 1), (c, 2)]);
    }
}
