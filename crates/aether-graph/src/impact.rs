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
        // Route context lives exactly one hop: traversing a dispatch
        // function's guarded call sets the context, and the dispatch
        // function's incoming entry edges consume it — a launcher whose
        // recorded route differs provably never executes this origin. A node
        // expanded under a context may have pruned entries a later
        // context-free arrival must traverse, so contextual expansion never
        // blocks a full one. Absent route metadata behaves exactly like the
        // plain BFS (fail-open).
        let mut expanded_fully = std::collections::HashSet::new();
        let mut expanded_contextually = std::collections::HashSet::new();
        let mut queue = VecDeque::new();
        expanded_fully.insert(start);
        queue.push_back((start, 0u32, None::<String>));

        while let Some((idx, dist, context)) = queue.pop_front() {
            let current_path = self.get(self.id_at(idx)).map(|node| node.path.clone());
            for edge in raw.edges_directed(idx, Direction::Incoming) {
                use petgraph::visit::EdgeRef;
                if !edge.weight().kind.propagates_impact() {
                    continue;
                }
                let caller = edge.source();
                let caller_id = self.id_at(caller);
                let caller_node = self.get(caller_id);
                if let (Some(route), Some(path), Some(node)) =
                    (context.as_deref(), current_path.as_deref(), caller_node)
                {
                    let entry = node.attr(&crate::entry_route_key(path));
                    if entry.is_some_and(|recorded| recorded != route) {
                        continue;
                    }
                    if node.attr(&crate::entry_route_params_key(path)).is_some() {
                        continue;
                    }
                }
                // First time we reach a node is its shortest distance (BFS).
                if caller_id != origin && !report.affected.contains_key(&caller_id) {
                    report.affected.insert(caller_id, dist + 1);
                }
                let next_context = current_path.as_deref().and_then(|path| {
                    caller_node
                        .and_then(|node| node.attr(&crate::route_guard_key(path)))
                        .map(str::to_string)
                });
                if caller_id == origin {
                    continue;
                }
                match next_context {
                    None => {
                        if expanded_fully.insert(caller) {
                            queue.push_back((caller, dist + 1, None));
                        }
                    }
                    Some(route) => {
                        if !expanded_fully.contains(&caller) && expanded_contextually.insert(caller)
                        {
                            queue.push_back((caller, dist + 1, Some(route)));
                        }
                    }
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

    fn route_fixture() -> SemanticGraph {
        let mut g = SemanticGraph::new();
        let main = g.upsert_node(Node::new(NodeKind::Function, "main", "crate::app::main"));
        let route_a = g.upsert_node(Node::new(
            NodeKind::Function,
            "route_a",
            "crate::app::route_a",
        ));
        let route_b = g.upsert_node(Node::new(
            NodeKind::Function,
            "route_b",
            "crate::app::route_b",
        ));
        let helper = g.upsert_node(Node::new(
            NodeKind::Function,
            "run_cli",
            "crate::t::run_cli",
        ));
        let test_a = g.upsert_node(Node::new(NodeKind::Function, "test_a", "crate::t::test_a"));
        let test_b = g.upsert_node(Node::new(NodeKind::Function, "test_b", "crate::t::test_b"));
        g.add_edge(main, route_a, Edge::new(EdgeKind::Calls))
            .unwrap();
        g.add_edge(main, route_b, Edge::new(EdgeKind::Calls))
            .unwrap();
        g.add_edge(test_a, helper, Edge::new(EdgeKind::Calls))
            .unwrap();
        g.add_edge(test_b, helper, Edge::new(EdgeKind::Calls))
            .unwrap();
        g.add_edge(helper, main, Edge::new(EdgeKind::Calls))
            .unwrap();
        g.add_edge(test_a, main, Edge::new(EdgeKind::Calls))
            .unwrap();
        g.add_edge(test_b, main, Edge::new(EdgeKind::Calls))
            .unwrap();
        g
    }

    #[test]
    fn route_context_prunes_provably_unreachable_launchers() {
        let mut g = route_fixture();
        let main = crate::NodeId::from_path("crate::app::main");
        let route_a = crate::NodeId::from_path("crate::app::route_a");
        let helper = crate::NodeId::from_path("crate::t::run_cli");
        let test_a = crate::NodeId::from_path("crate::t::test_a");
        let test_b = crate::NodeId::from_path("crate::t::test_b");
        g.get_mut(main)
            .unwrap()
            .set_attr(crate::route_guard_key("crate::app::route_a"), "a");
        g.get_mut(main)
            .unwrap()
            .set_attr(crate::route_guard_key("crate::app::route_b"), "b");
        g.get_mut(test_a)
            .unwrap()
            .set_attr(crate::entry_route_key("crate::app::main"), "a");
        g.get_mut(test_b)
            .unwrap()
            .set_attr(crate::entry_route_key("crate::app::main"), "b");
        g.get_mut(helper).unwrap().set_attr(
            crate::entry_route_params_key("crate::app::main"),
            "via-params",
        );

        let report = g.impact_of(route_a);
        assert!(report.affected.contains_key(&main), "main still executes");
        assert!(
            report.affected.contains_key(&test_a),
            "the launcher whose route reaches the origin is affected"
        );
        assert!(
            !report.affected.contains_key(&test_b),
            "a launcher with a provably different route is pruned"
        );
        assert!(
            !report.affected.contains_key(&helper),
            "the parameterized helper is pruned; per-caller routes carry the flow"
        );
    }

    #[test]
    fn route_context_is_fail_open_without_metadata() {
        let g = route_fixture();
        let route_a = crate::NodeId::from_path("crate::app::route_a");
        let report = g.impact_of(route_a);
        // No route attributes: identical to the plain reverse BFS.
        for path in [
            "crate::app::main",
            "crate::t::run_cli",
            "crate::t::test_a",
            "crate::t::test_b",
        ] {
            assert!(
                report
                    .affected
                    .contains_key(&crate::NodeId::from_path(path)),
                "{path} must stay affected without route evidence"
            );
        }
    }

    #[test]
    fn context_free_arrival_reexpands_a_contextually_expanded_node() {
        let mut g = route_fixture();
        let main = crate::NodeId::from_path("crate::app::main");
        let route_a = crate::NodeId::from_path("crate::app::route_a");
        let test_a = crate::NodeId::from_path("crate::t::test_a");
        let test_b = crate::NodeId::from_path("crate::t::test_b");
        let wrapper = g.upsert_node(Node::new(
            NodeKind::Function,
            "wrapper",
            "crate::app::wrapper",
        ));
        // main also reaches route_a through an unguarded wrapper, so every
        // launcher can execute it and no entry may be pruned.
        g.add_edge(wrapper, route_a, Edge::new(EdgeKind::Calls))
            .unwrap();
        g.add_edge(main, wrapper, Edge::new(EdgeKind::Calls))
            .unwrap();
        g.get_mut(main)
            .unwrap()
            .set_attr(crate::route_guard_key("crate::app::route_a"), "a");
        g.get_mut(test_a)
            .unwrap()
            .set_attr(crate::entry_route_key("crate::app::main"), "a");
        g.get_mut(test_b)
            .unwrap()
            .set_attr(crate::entry_route_key("crate::app::main"), "b");

        let report = g.impact_of(route_a);
        assert!(report.affected.contains_key(&test_a));
        assert!(
            report.affected.contains_key(&test_b),
            "an unguarded path into the origin must re-open every entry"
        );
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
