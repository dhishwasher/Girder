//! Tester: synthesizes a test for freshly written code and pins it to the node.

use super::{Agent, AgentResult};
use crate::bus::{MsgKind, Role, SwarmMessage};
use crate::orchestrator::SwarmContext;
use aether_ai::{Prompt, TaskClass};
use aether_graph::NodeId;
use async_trait::async_trait;

pub struct TesterAgent;

#[async_trait]
impl Agent for TesterAgent {
    fn role(&self) -> Role {
        Role::Tester
    }

    async fn handle(&self, msg: &SwarmMessage, ctx: &SwarmContext) -> AgentResult {
        let MsgKind::CodeReady {
            module,
            name,
            source,
        } = &msg.kind
        else {
            return AgentResult::Idle;
        };

        let prompt = Prompt::new(
            TaskClass::Testing,
            "You are the Tester. Write a Rust unit test for the given function.",
            source,
        );
        let Ok(completion) = ctx.router.complete(prompt).await else {
            return AgentResult::Idle;
        };
        let test_src = completion.text;

        // Pin the generated test onto the function node as an attribute, so it
        // travels with the node in the graph (and into the `.aether` file).
        let path = format!("{module}::{name}");
        {
            let mut graph = ctx.graph.lock().unwrap();
            if let Some(node) = graph.get_mut(NodeId::from_path(&path)) {
                node.set_attr("test", &test_src);
            }
        }

        AgentResult::one(SwarmMessage::new(
            Role::Tester,
            MsgKind::TestsReady {
                for_fn: path,
                source: test_src,
            },
        ))
    }
}
