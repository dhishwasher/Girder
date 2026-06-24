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
}
