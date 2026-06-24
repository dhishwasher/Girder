//! Refactorer — EXTENSION POINT.
//!
//! Reacts to new code and is the natural home for graph-powered, semantics-aware
//! refactors (rename across call edges, extract function, inline, dedup via
//! `SemanticSimilar` edges). The prototype only tags the node with a refactor
//! opportunity score; the real transform engine is left to implement.

use super::{Agent, AgentResult};
use crate::bus::{MsgKind, Role, SwarmMessage};
use crate::orchestrator::SwarmContext;
use aether_graph::NodeId;
use async_trait::async_trait;

pub struct RefactorerAgent;

#[async_trait]
impl Agent for RefactorerAgent {
    fn role(&self) -> Role {
        Role::Refactorer
    }

    async fn handle(&self, msg: &SwarmMessage, ctx: &SwarmContext) -> AgentResult {
        let MsgKind::CodeReady { module, name, .. } = &msg.kind else {
            return AgentResult::Idle;
        };
        let path = format!("{module}::{name}");
        {
            let mut graph = ctx.graph.lock().unwrap();
            if let Some(node) = graph.get_mut(NodeId::from_path(&path)) {
                // EXTENSION POINT: compute real refactor opportunities here.
                node.set_attr("refactor_opportunities", "0");
            }
        }
        AgentResult::one(SwarmMessage::new(
            Role::Refactorer,
            MsgKind::Note {
                text: format!("reviewed {name}: 0 refactor opportunities"),
            },
        ))
    }
}
