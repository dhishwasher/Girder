//! The agent trait and the concrete swarm members.
//!
//! Three agents are fully live (Planner, Coder, Tester) and drive an end-to-end
//! intent→plan→code→test→graph-mutation pipeline against the offline mock
//! provider. Documenter is also functional. Refactorer/Optimizer/SecurityAuditor
//! are message-aware EXTENSION-POINT stubs: they react and annotate, but their
//! deep logic is left to flesh out.

mod coder;
mod documenter;
mod optimizer;
mod planner;
mod refactorer;
mod security;
mod tester;

pub use coder::CoderAgent;
pub use documenter::DocumenterAgent;
pub use optimizer::{HotTarget, OptimizerAgent};
pub use planner::PlannerAgent;
pub use refactorer::RefactorerAgent;
pub use security::SecurityAuditorAgent;
pub use tester::TesterAgent;

use crate::bus::{Role, SwarmMessage};
use crate::orchestrator::SwarmContext;
use async_trait::async_trait;

/// What an agent returns after handling a message.
pub enum AgentResult {
    /// Broadcast these follow-up messages.
    Emit(Vec<SwarmMessage>),
    /// Nothing to do for this message.
    Idle,
}

impl AgentResult {
    /// Convenience: emit a single message.
    pub fn one(msg: SwarmMessage) -> Self {
        AgentResult::Emit(vec![msg])
    }
}

/// A swarm agent. Implementors are `Send + Sync` and shared across tasks via
/// `Arc`, so `handle` takes `&self`; per-run state lives in the shared
/// [`SwarmContext`] (notably the graph) rather than on the agent.
#[async_trait]
pub trait Agent: Send + Sync {
    fn role(&self) -> Role;

    /// React to one message. Must NOT hold the graph mutex across an `.await`.
    async fn handle(&self, msg: &SwarmMessage, ctx: &SwarmContext) -> AgentResult;
}

/// Extract the first `fn NAME` identifier from a Rust snippet.
pub(crate) fn rust_fn_name(source: &str) -> Option<String> {
    let idx = source.find("fn ")?;
    let rest = &source[idx + 3..];
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}
