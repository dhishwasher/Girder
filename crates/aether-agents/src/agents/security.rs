//! SecurityAuditor — EXTENSION POINT.
//!
//! Would taint-track untrusted inputs along `DataFlow` edges and flag risky
//! sinks (SQL, shell, deserialization). The prototype tags a baseline risk
//! level; the dataflow taint engine is left to implement.

use super::{Agent, AgentResult};
use crate::bus::{MsgKind, Role, SwarmMessage};
use crate::orchestrator::SwarmContext;
use aether_graph::NodeId;
use async_trait::async_trait;

pub struct SecurityAuditorAgent;

#[async_trait]
impl Agent for SecurityAuditorAgent {
    fn role(&self) -> Role {
        Role::SecurityAuditor
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
        // EXTENSION POINT: real taint analysis. Cheap heuristic for the demo.
        let risk = if source.contains("unsafe") || source.contains("eval") {
            "high"
        } else {
            "low"
        };
        let path = format!("{module}::{name}");
        {
            let mut graph = ctx.graph.lock().unwrap();
            if let Some(node) = graph.get_mut(NodeId::from_path(&path)) {
                node.set_attr("risk", risk);
            }
        }
        // Announce completion so the orchestrator's quiescence detector waits
        // for this annotation before reading the graph.
        AgentResult::one(SwarmMessage::new(
            Role::SecurityAuditor,
            MsgKind::Note {
                text: format!("audited {name}: risk={risk}"),
            },
        ))
    }
}
