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

pub use agents::{Agent, QueryAgent};
pub use bus::{FnSpec, MsgKind, Role, SwarmMessage};
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn planner_emits_feature_spec_with_graph_context() {
        use aether_graph::Node;

        // Pre-populate the graph so the Planner can report what already exists.
        let graph = Arc::new(Mutex::new(SemanticGraph::new()));
        {
            let mut g = graph.lock().unwrap();
            g.upsert_node(Node::new(
                NodeKind::Function,
                "validate",
                "crate::auth::validate",
            ));
        }

        let ctx = Arc::new(SwarmContext::new(
            aether_ai::default_router(),
            graph.clone(),
            "crate::auth",
            "src/auth.rs",
        ));
        let orchestrator = Orchestrator::new(ctx).with_default_swarm();
        let msgs = orchestrator.plan_only("Add user authentication").await;

        // The Planner must produce a FeatureSpec.
        let spec = msgs.iter().find_map(|m| {
            if let MsgKind::FeatureSpec {
                graph_context,
                fn_specs,
                ..
            } = &m.kind
            {
                Some((graph_context.clone(), fn_specs.clone()))
            } else {
                None
            }
        });
        let (ctx_str, specs) = spec.expect("Planner should produce FeatureSpec for auth intent");

        // Graph context mentions the pre-existing function.
        assert!(
            ctx_str.contains("crate::auth::validate"),
            "context must mention existing fn"
        );

        // Auth intent produces multiple functions (validate_credentials, generate_token, authenticate).
        assert!(
            specs.len() >= 2,
            "auth feature should produce at least 2 fn specs"
        );
        assert!(
            specs
                .iter()
                .any(|s| s.name.contains("credential") || s.name.contains("validate")),
            "should include credential validation"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn forge_builds_multi_function_feature_with_call_edges() {
        // A multi-function feature (auth) should produce multiple nodes with
        // Calls edges wired between them by the Coder.
        let graph = Arc::new(Mutex::new(SemanticGraph::new()));
        let ctx = Arc::new(SwarmContext::new(
            aether_ai::default_router(),
            graph.clone(),
            "crate::auth",
            "src/auth.rs",
        ));
        let orchestrator = Orchestrator::new(ctx).with_default_swarm();
        let transcript = orchestrator
            .run("Add user authentication", Duration::from_secs(5))
            .await;

        // Multiple CodeReady messages (one per function in the spec).
        let code_ready_count = transcript
            .iter()
            .filter(|m| matches!(m.kind, MsgKind::CodeReady { .. }))
            .count();
        assert!(
            code_ready_count >= 2,
            "auth feature should build at least 2 functions"
        );

        // FeatureComplete should summarise what was built.
        let complete = transcript
            .iter()
            .find(|m| matches!(m.kind, MsgKind::FeatureComplete { .. }));
        assert!(complete.is_some(), "Coder should emit FeatureComplete");

        // The graph should contain the authenticate function.
        let g = graph.lock().unwrap();
        let auth_fn = g.find_by_path("crate::auth::authenticate");
        assert!(auth_fn.is_some(), "authenticate node should be in graph");

        // Call edges should wire authenticate → validate_credentials / generate_token.
        if let Some(node) = auth_fn {
            let callee_count = g.neighbors(node.id, Some(EdgeKind::Calls)).len();
            assert!(
                callee_count >= 1,
                "authenticate should have at least one Calls edge"
            );
        }
    }

    // ── Feature #6: knowledge graph queries ──────────────────────────────────

    #[tokio::test(flavor = "current_thread")]
    async fn query_agent_answers_impact_question() {
        use aether_graph::{Edge, EdgeKind, Node};

        // A simple call chain: authenticate → validate_credentials.
        let graph = Arc::new(Mutex::new(SemanticGraph::new()));
        {
            let mut g = graph.lock().unwrap();
            let vc = g.upsert_node(Node::new(
                NodeKind::Function,
                "validate_credentials",
                "crate::auth::validate_credentials",
            ));
            let auth = g.upsert_node(Node::new(
                NodeKind::Function,
                "authenticate",
                "crate::auth::authenticate",
            ));
            g.add_edge(auth, vc, Edge::new(EdgeKind::Calls)).unwrap();
        }

        let ctx = Arc::new(SwarmContext::new(
            aether_ai::default_router(),
            graph.clone(),
            "crate::auth",
            "src/auth.rs",
        ));
        let orch = Orchestrator::new(ctx).with_default_swarm();
        let msgs = orch
            .query_only("what would break if I change validate_credentials?")
            .await;

        // QueryAgent must emit a Note with the impact answer.
        let note = msgs.iter().find_map(|m| {
            if let MsgKind::Note { text } = &m.kind {
                Some(text.clone())
            } else {
                None
            }
        });
        let answer = note.expect("QueryAgent should emit a Note for impact question");
        assert!(
            answer.contains("authenticate"),
            "impact answer should mention authenticate; got: {answer}"
        );
        assert_eq!(
            msgs.iter().filter(|m| m.from == Role::QueryAgent).count(),
            1,
            "exactly one QueryAgent message expected"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn query_agent_stays_idle_for_forge_intent() {
        // A normal forge intent should NOT trigger the QueryAgent.
        let graph = Arc::new(Mutex::new(SemanticGraph::new()));
        let ctx = Arc::new(SwarmContext::new(
            aether_ai::default_router(),
            graph.clone(),
            "crate::math",
            "src/math.rs",
        ));
        let orch = Orchestrator::new(ctx).with_default_swarm();
        let msgs = orch.query_only("Add a multiply function").await;

        let query_msgs: Vec<_> = msgs.iter().filter(|m| m.from == Role::QueryAgent).collect();
        assert!(
            query_msgs.is_empty(),
            "QueryAgent should stay idle for non-question intent"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn query_agent_answers_callers_question() {
        use aether_graph::{Edge, EdgeKind, Node};

        let graph = Arc::new(Mutex::new(SemanticGraph::new()));
        {
            let mut g = graph.lock().unwrap();
            let helper = g.upsert_node(Node::new(
                NodeKind::Function,
                "hash_password",
                "crate::auth::hash_password",
            ));
            let caller1 = g.upsert_node(Node::new(
                NodeKind::Function,
                "register",
                "crate::auth::register",
            ));
            let caller2 = g.upsert_node(Node::new(
                NodeKind::Function,
                "change_password",
                "crate::auth::change_password",
            ));
            g.add_edge(caller1, helper, Edge::new(EdgeKind::Calls))
                .unwrap();
            g.add_edge(caller2, helper, Edge::new(EdgeKind::Calls))
                .unwrap();
        }

        let ctx = Arc::new(SwarmContext::new(
            aether_ai::default_router(),
            graph.clone(),
            "crate::auth",
            "src/auth.rs",
        ));
        let orch = Orchestrator::new(ctx).with_default_swarm();
        let msgs = orch.query_only("what calls hash_password?").await;

        let note = msgs
            .iter()
            .find_map(|m| {
                if let MsgKind::Note { text } = &m.kind {
                    Some(text.clone())
                } else {
                    None
                }
            })
            .expect("QueryAgent should answer callers question");

        assert!(
            note.contains("register") || note.contains("change_password"),
            "callers answer should list callers; got: {note}"
        );
    }

    #[test]
    fn parse_fn_specs_handles_em_dash_and_hyphen() {
        use crate::agents::planner::parse_fn_specs;
        let text = "fn validate_credentials — checks username and password\n\
             fn generate_token - creates a session token\n\
             not a fn line\n\
             fn empty_desc";
        let specs = parse_fn_specs(text);
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].name, "validate_credentials");
        assert!(specs[0].description.contains("username"));
        assert_eq!(specs[1].name, "generate_token");
        assert!(specs[1].description.contains("token"));
        assert_eq!(specs[2].name, "empty_desc");
        assert!(specs[2].description.is_empty());
    }
}
