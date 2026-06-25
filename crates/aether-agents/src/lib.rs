//! # aether-agents
//!
//! The **Parallel Agent Swarm**: specialized agents (Planner, Coder, Tester,
//! Documenter, Refactorer, Optimizer, SecurityAuditor) that run concurrently,
//! collaborate over a broadcast [`bus`], and mutate the shared semantic graph in
//! real time. The orchestrator injects an intent and the swarm self-organizes a
//! plan→code→test→annotate pipeline.

pub mod agents;
pub mod bus;
pub mod orchestrator;

pub use agents::Agent;
pub use bus::{MsgKind, Role, SwarmMessage};
pub use orchestrator::{Orchestrator, SwarmContext};

#[cfg(test)]
mod tests {
    use super::*;
    use aether_graph::{EdgeKind, NodeId, NodeKind, SemanticGraph};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn swarm_turns_intent_into_a_graph_node_with_tests() {
        // Start from an empty project. The swarm should create the math module
        // and a `multiply` function, wired and annotated, purely from intent.
        let graph = Arc::new(Mutex::new(SemanticGraph::new()));
        let router = aether_ai::default_router();
        let ctx = Arc::new(SwarmContext::new(
            router,
            graph.clone(),
            "crate::math",
            "src/math.rs",
        ));

        let orchestrator = Orchestrator::new(ctx).with_default_swarm();
        let transcript = orchestrator
            .run(
                "Add a multiply function to the math module",
                Duration::from_secs(5),
            )
            .await;

        // The pipeline ran to its terminal state.
        assert!(
            transcript
                .iter()
                .any(|m| matches!(m.kind, MsgKind::TestsReady { .. })),
            "expected the Tester to finish the pipeline"
        );

        // The graph (source of truth) now contains the new function.
        let g = graph.lock().unwrap();
        let mult = g
            .find_by_path("crate::math::multiply")
            .expect("multiply node should exist");
        assert_eq!(mult.kind, NodeKind::Function);
        assert!(mult.source.contains("a * b"), "Coder wrote the body");

        // Agents annotated it: tests, summary, risk all present on the node.
        assert!(mult.attr("test").is_some(), "Tester pinned a test");
        assert!(mult.attr("summary").is_some(), "Documenter wrote a summary");
        assert_eq!(mult.attr("risk"), Some("low"));

        // The module contains the function (Contains edge from Coder).
        let module_id = NodeId::from_path("crate::math");
        let contained: Vec<_> = g
            .neighbors(module_id, Some(EdgeKind::Contains))
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert!(contained.contains(&NodeId::from_path("crate::math::multiply")));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn refactorer_flags_a_semantic_duplicate() {
        use crate::agents::{Agent, AgentResult, RefactorerAgent};
        use aether_graph::Node;

        // Two near-identical functions already in the graph.
        let graph = Arc::new(Mutex::new(SemanticGraph::new()));
        {
            let mut g = graph.lock().unwrap();
            g.upsert_node(
                Node::new(
                    NodeKind::Function,
                    "hash_password",
                    "crate::auth::hash_password",
                )
                .with_source(
                    "fn hash_password(password: String) -> String { compute_hash(password) }",
                ),
            );
            g.upsert_node(
                Node::new(
                    NodeKind::Function,
                    "hash_password_v2",
                    "crate::auth::hash_password_v2",
                )
                .with_source(
                    "fn hash_password_v2(password: String) -> String { compute_hash(password) }",
                ),
            );
        }

        let ctx = Arc::new(SwarmContext::new(
            aether_ai::default_router(),
            graph.clone(),
            "crate::auth",
            "src/auth.rs",
        ));

        // Tell the Refactorer the v2 function just landed.
        let msg = SwarmMessage::new(
            Role::Coder,
            MsgKind::CodeReady {
                module: "crate::auth".to_string(),
                name: "hash_password_v2".to_string(),
                source:
                    "fn hash_password_v2(password: String) -> String { compute_hash(password) }"
                        .to_string(),
            },
        );
        let result = RefactorerAgent.handle(&msg, &ctx).await;
        assert!(matches!(result, AgentResult::Emit(_)));

        let g = graph.lock().unwrap();
        let v2 = g.find_by_path("crate::auth::hash_password_v2").unwrap();
        assert_eq!(v2.attr("duplicate_candidates"), Some("1"));
        assert_eq!(v2.attr("duplicate_of"), Some("crate::auth::hash_password"));
    }

    #[test]
    fn optimizer_consumes_a_hot_path_profile() {
        use crate::agents::{HotTarget, OptimizerAgent};
        use aether_graph::{Node, NodeKind};

        // A profile as the debugger's Timeline::hot_functions would produce it.
        let profile = vec![("inner".to_string(), 5), ("outer".to_string(), 1)];

        // Ranking: hottest first, with a stronger recommendation.
        let ranked = OptimizerAgent::rank_hot_paths(&profile);
        assert_eq!(ranked[0].function, "inner");
        assert_eq!(ranked[0].calls, 5);
        assert!(ranked[0].recommendation.contains("hottest"));
        assert_eq!(
            ranked[1],
            HotTarget {
                function: "outer".to_string(),
                calls: 1,
                recommendation: "called 1×".to_string(),
            }
        );

        // Annotation lands on matching graph function nodes.
        let mut g = SemanticGraph::new();
        g.upsert_node(Node::new(NodeKind::Function, "inner", "crate::m::inner"));
        g.upsert_node(Node::new(NodeKind::Function, "outer", "crate::m::outer"));
        let annotated = OptimizerAgent::annotate_graph(&mut g, &profile);
        assert_eq!(annotated.len(), 2);

        let inner = g.find_by_path("crate::m::inner").unwrap();
        assert_eq!(inner.attr("hot_calls"), Some("5"));
        assert_eq!(inner.attr("optimize_priority"), Some("hot"));
        let outer = g.find_by_path("crate::m::outer").unwrap();
        assert_eq!(outer.attr("hot_calls"), Some("1"));
        assert_eq!(outer.attr("optimize_priority"), None);
    }
}
