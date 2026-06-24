//! Impact analysis: "if I change node X, what else is affected?"
//!
//! This powers AetherForge's real-time predictive impact panel. Because the
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
        v.sort_by_key(|(_, d)| *d);
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

    /// Materialize predicted-impact edges from a fresh analysis. The Optimizer
    /// and Refactorer agents call this so the impact relationships become
    /// first-class, queryable graph edges (with decaying confidence by distance).
    pub fn materialize_impact_edges(&mut self, origin: NodeId) {
        let report = self.impact_of(origin);
        for (target, dist) in report.affected {
            let weight = (1.0 / (dist as f32 + 1.0)).max(0.05);
            let _ = self.add_edge(
                origin,
                target,
                crate::Edge::with_weight(EdgeKind::Impacts, weight),
            );
        }
    }
}
