//! Refactorer — semantics-aware duplicate detection.
//!
//! When new code lands, the Refactorer recomputes the graph's `SemanticSimilar`
//! edges and reports genuine duplicate candidates for the new function (by token
//! overlap, the same signal that powers concept search). It annotates the node
//! with the count and the nearest match. Deeper transforms (rename across call
//! edges, extract-function, inline) remain EXTENSION POINTs.

use super::{Agent, AgentResult};
use crate::bus::{MsgKind, Role, SwarmMessage};
use crate::orchestrator::SwarmContext;
use aether_graph::{EdgeKind, NodeId};
use async_trait::async_trait;

/// Token-overlap threshold above which two functions are flagged as duplicates.
const DUPLICATE_THRESHOLD: f32 = 0.6;

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
        let id = NodeId::from_path(&path);

        let (count, nearest) = {
            let mut graph = ctx.graph.lock().unwrap();

            // Recompute similarity over the whole graph, then read this node's
            // duplicate candidates straight off the SemanticSimilar edges.
            graph.compute_similarity_edges(DUPLICATE_THRESHOLD);
            let similar = graph.neighbors(id, Some(EdgeKind::SemanticSimilar));
            let nearest = similar
                .iter()
                .max_by(|a, b| a.weight.total_cmp(&b.weight))
                .and_then(|n| graph.get(n.id).map(|node| node.path.clone()));
            let count = similar.len();

            if let Some(node) = graph.get_mut(id) {
                node.set_attr("duplicate_candidates", count.to_string());
                if let Some(dup) = &nearest {
                    node.set_attr("duplicate_of", dup);
                }
            }
            (count, nearest)
        };

        let text = match nearest {
            Some(dup) => format!("{name}: {count} duplicate candidate(s); closest is {dup}"),
            None => format!("reviewed {name}: no duplicates found"),
        };
        AgentResult::one(SwarmMessage::new(Role::Refactorer, MsgKind::Note { text }))
    }
}
