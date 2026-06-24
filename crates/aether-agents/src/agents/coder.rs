//! Coder: generates code from the plan and writes it directly into the graph.
//!
//! This is the clearest demonstration of the core thesis: the agent does not
//! edit a text file and hope a parser catches up — it mutates the semantic
//! graph (the source of truth), and *that* causes every projection (editor,
//! graph view, impact panel) to update.

use super::{rust_fn_name, Agent, AgentResult};
use crate::bus::{MsgKind, Role, SwarmMessage};
use crate::orchestrator::SwarmContext;
use aether_ai::{Prompt, TaskClass};
use aether_graph::{Edge, EdgeKind, Node, NodeId, NodeKind};
use async_trait::async_trait;

pub struct CoderAgent;

#[async_trait]
impl Agent for CoderAgent {
    fn role(&self) -> Role {
        Role::Coder
    }

    async fn handle(&self, msg: &SwarmMessage, ctx: &SwarmContext) -> AgentResult {
        let MsgKind::PlanReady { steps } = &msg.kind else {
            return AgentResult::Idle;
        };

        // The plan text carries the intent; feed it to the codegen provider.
        let brief = steps.join(" ");
        let prompt = Prompt::new(
            TaskClass::Codegen,
            "You are the Coder. Emit a single Rust function implementing the plan.",
            brief,
        );
        let Ok(completion) = ctx.router.complete(prompt).await else {
            return AgentResult::Idle;
        };
        let source = completion.text;
        let Some(name) = rust_fn_name(&source) else {
            return AgentResult::Idle;
        };

        let module = ctx.target_module.clone();
        let path = format!("{module}::{name}");

        // --- graph mutation (brief critical section, no await held) ---
        {
            let mut graph = ctx.graph.lock().unwrap();

            // Ensure the module node exists.
            let module_id = NodeId::from_path(&module);
            if !graph.contains(module_id) {
                let seg = module.rsplit("::").next().unwrap_or(&module).to_string();
                graph.upsert_node(Node::new(NodeKind::Module, seg, &module).with_language("rust"));
            }

            // Upsert the new function node and link module -> fn (Contains).
            let mut fn_node = Node::new(NodeKind::Function, &name, &path)
                .with_language("rust")
                .with_source(&source);
            fn_node.file = Some(ctx.target_file.clone());
            fn_node.set_attr("authored_by", "Coder");
            let fn_id = graph.upsert_node(fn_node);
            let _ = graph.add_edge(module_id, fn_id, Edge::new(EdgeKind::Contains));

            // Refresh predicted-impact edges so the impact panel is live.
            graph.materialize_impact_edges(fn_id);
        }

        AgentResult::one(SwarmMessage::new(
            Role::Coder,
            MsgKind::CodeReady {
                module,
                name,
                source,
            },
        ))
    }
}
