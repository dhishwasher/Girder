//! Planner: turns a natural-language intent into ordered steps.

use super::{Agent, AgentResult};
use crate::bus::{MsgKind, Role, SwarmMessage};
use crate::orchestrator::SwarmContext;
use aether_ai::{Prompt, TaskClass};
use async_trait::async_trait;

pub struct PlannerAgent;

#[async_trait]
impl Agent for PlannerAgent {
    fn role(&self) -> Role {
        Role::Planner
    }

    async fn handle(&self, msg: &SwarmMessage, ctx: &SwarmContext) -> AgentResult {
        let MsgKind::Intent(intent) = &msg.kind else {
            return AgentResult::Idle;
        };

        let prompt = Prompt::new(
            TaskClass::Planning,
            "You are the Planner. Decompose the user's intent into concrete steps.",
            intent,
        );
        let Ok(completion) = ctx.router.complete(prompt).await else {
            return AgentResult::Idle;
        };

        // Each non-empty line of the plan becomes a step. The intent is woven
        // into the mock's plan text, so it flows forward to the Coder.
        let steps: Vec<String> = completion
            .text
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();

        AgentResult::one(SwarmMessage::new(
            Role::Planner,
            MsgKind::PlanReady { steps },
        ))
    }
}
