//! Query agent: answers natural-language questions about the codebase graph.
//!
//! The agent activates only for `Intent` messages that look like questions
//! (contain `?` or start with a question word), staying idle during normal
//! forge/plan workflows. It parses the question into a structured
//! [`KnowledgeQuery`], runs it against the live graph, and emits the answer
//! as a [`MsgKind::Note`].

use super::{Agent, AgentResult};
use crate::bus::{MsgKind, Role, SwarmMessage};
use crate::orchestrator::SwarmContext;
use aether_graph::parse_query;
use async_trait::async_trait;

pub struct QueryAgent;

#[async_trait]
impl Agent for QueryAgent {
    fn role(&self) -> Role {
        Role::QueryAgent
    }

    async fn handle(&self, msg: &SwarmMessage, ctx: &SwarmContext) -> AgentResult {
        let MsgKind::Intent(intent) = &msg.kind else {
            return AgentResult::Idle;
        };
        if !looks_like_question(intent) {
            return AgentResult::Idle;
        }

        let result = {
            let g = ctx.graph.lock().unwrap();
            g.answer_query(&parse_query(intent))
        };

        AgentResult::one(SwarmMessage::new(
            Role::QueryAgent,
            MsgKind::Note {
                text: result.display(),
            },
        ))
    }
}

fn looks_like_question(text: &str) -> bool {
    let t = text.trim_start().to_lowercase();
    t.contains('?')
        || t.starts_with("what ")
        || t.starts_with("who ")
        || t.starts_with("which ")
        || t.starts_with("how ")
        || t.starts_with("explain ")
        || t.starts_with("describe ")
        || t.starts_with("show subgraph")
        || t.starts_with("show neighborhood")
        || t.starts_with("find ")
}
