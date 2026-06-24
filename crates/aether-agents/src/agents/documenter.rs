//! Documenter: writes a live summary attribute for new code (self-documentation).

use super::{Agent, AgentResult};
use crate::bus::{MsgKind, Role, SwarmMessage};
use crate::orchestrator::SwarmContext;
use aether_ai::{Prompt, TaskClass};
use aether_graph::NodeId;
use async_trait::async_trait;

pub struct DocumenterAgent;

#[async_trait]
impl Agent for DocumenterAgent {
    fn role(&self) -> Role {
        Role::Documenter
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
            TaskClass::Summarize,
            "You are the Documenter. Summarize what this function does in one line.",
            source,
        );
        let summary = match ctx.router.complete(prompt).await {
            Ok(c) => c.text,
            Err(_) => return AgentResult::Idle,
        };

        let path = format!("{module}::{name}");
        {
            let mut graph = ctx.graph.lock().unwrap();
            if let Some(node) = graph.get_mut(NodeId::from_path(&path)) {
                node.set_attr("summary", &summary);
            }
        }

        AgentResult::one(SwarmMessage::new(
            Role::Documenter,
            MsgKind::Note {
                text: format!("documented {name}: {summary}"),
            },
        ))
    }
}
