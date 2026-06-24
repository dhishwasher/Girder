//! Optimizer — EXTENSION POINT.
//!
//! Would profile hot paths (via the debugger's traces) and propose performance
//! rewrites, annotating nodes with measured/estimated complexity. The prototype
//! records a placeholder complexity estimate.

use super::{Agent, AgentResult};
use crate::bus::{MsgKind, Role, SwarmMessage};
use crate::orchestrator::SwarmContext;
use aether_graph::NodeId;
use async_trait::async_trait;

pub struct OptimizerAgent;

#[async_trait]
impl Agent for OptimizerAgent {
    fn role(&self) -> Role {
        Role::Optimizer
    }

    async fn handle(&self, msg: &SwarmMessage, ctx: &SwarmContext) -> AgentResult {
        let MsgKind::CodeReady { module, name, .. } = &msg.kind else {
            return AgentResult::Idle;
        };
        let path = format!("{module}::{name}");
        {
            let mut graph = ctx.graph.lock().unwrap();
            if let Some(node) = graph.get_mut(NodeId::from_path(&path)) {
                // EXTENSION POINT: derive complexity from the trace / AST.
                node.set_attr("complexity_estimate", "O(1)");
            }
        }
        AgentResult::one(SwarmMessage::new(
            Role::Optimizer,
            MsgKind::Note {
                text: format!("analyzed {name}: complexity=O(1)"),
            },
        ))
    }
}
