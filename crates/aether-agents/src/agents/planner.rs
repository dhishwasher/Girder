//! Planner: graph-aware feature decomposition.
//!
//! Before asking the AI anything, the Planner reads the live semantic graph
//! to understand what already exists — modules, functions, call relationships.
//! That context is injected into the planning prompt so the AI can design code
//! that *fits* the codebase (reuses helpers, avoids duplicates, targets the
//! right module). The response is parsed into a [`FeatureSpec`] — an ordered
//! list of [`FnSpec`]s from leaf helpers up to the entry-point — and broadcast
//! for the Coder to build incrementally.

use super::{Agent, AgentResult};
use crate::bus::{FnSpec, MsgKind, Role, SwarmMessage};
use crate::orchestrator::SwarmContext;
use aether_ai::{Prompt, TaskClass};
use aether_graph::NodeKind;
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

        // ── 1. Read the graph for planning context (brief critical section) ────
        let graph_context = {
            let g = ctx.graph.lock().unwrap();

            let modules: Vec<String> = g
                .nodes()
                .filter(|n| n.kind == NodeKind::Module)
                .map(|n| n.path.clone())
                .collect();

            let functions: Vec<String> = g
                .nodes()
                .filter(|n| n.kind == NodeKind::Function)
                .map(|n| n.path.clone())
                .collect();

            let types: Vec<String> = g
                .nodes()
                .filter(|n| n.kind == NodeKind::Type)
                .map(|n| n.path.clone())
                .collect();

            let mut parts = Vec::new();
            if !modules.is_empty() {
                parts.push(format!("modules: [{}]", modules.join(", ")));
            }
            if !functions.is_empty() {
                parts.push(format!("functions: [{}]", functions.join(", ")));
            }
            if !types.is_empty() {
                parts.push(format!("types: [{}]", types.join(", ")));
            }
            if parts.is_empty() {
                "graph is empty".to_string()
            } else {
                parts.join("\n  ")
            }
        };

        // ── 2. Ask the AI to design a multi-function feature spec ─────────────
        let enriched = format!(
            "Intent: {intent}\n\nCurrent codebase graph:\n  {graph_context}\n\n\
             Design the functions needed to implement this feature.\n\
             For each function write exactly one line: fn <name> — <what it does>\n\
             Order from leaf helpers first, entry-point last."
        );

        let prompt = Prompt::new(
            TaskClass::Planning,
            "You are the Planner. Analyse the codebase graph and design a \
             minimal, well-structured set of functions for the intent.",
            &enriched,
        );

        let Ok(completion) = ctx.router.complete(prompt).await else {
            return AgentResult::Idle;
        };

        // ── 3. Parse `fn <name> — <description>` lines ───────────────────────
        let fn_specs = parse_fn_specs(&completion.text);

        // Fall back: if no structured specs parsed, treat every non-empty line
        // as a plan step (legacy PlanReady path) so the Coder can still proceed.
        if fn_specs.is_empty() {
            let steps: Vec<String> = completion
                .text
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            return AgentResult::one(SwarmMessage::new(
                Role::Planner,
                MsgKind::PlanReady { steps },
            ));
        }

        // ── 4. Emit the rich FeatureSpec ──────────────────────────────────────
        AgentResult::one(SwarmMessage::new(
            Role::Planner,
            MsgKind::FeatureSpec {
                intent: intent.clone(),
                graph_context,
                fn_specs,
            },
        ))
    }
}

/// Parse `fn <name> — <description>` lines from a model response.
pub(crate) fn parse_fn_specs(text: &str) -> Vec<FnSpec> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let rest = trimmed.strip_prefix("fn ")?;
            // Split on em-dash (5 bytes: space + U+2014 + space) or ASCII hyphen-dash (3 bytes).
            let (name_part, desc_part) = if let Some(i) = rest.find(" — ") {
                (&rest[..i], &rest[i + " — ".len()..])
            } else if let Some(i) = rest.find(" - ") {
                (&rest[..i], &rest[i + 3..])
            } else {
                (rest, "")
            };
            let name = name_part
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect::<String>();
            if name.is_empty() {
                return None;
            }
            Some(FnSpec {
                name,
                description: desc_part.trim().to_string(),
            })
        })
        .collect()
}
