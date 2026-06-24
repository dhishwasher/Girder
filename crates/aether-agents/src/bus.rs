//! The message bus the agent swarm collaborates over.
//!
//! Agents are decoupled: they broadcast typed [`SwarmMessage`]s and react only
//! to the kinds relevant to their [`Role`]. This is what lets the Planner,
//! Coder, Tester, Documenter, … run as independent concurrent tasks yet
//! cooperate on one intent.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// The specialized roles in the swarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    Planner,
    Coder,
    Tester,
    Refactorer,
    Optimizer,
    SecurityAuditor,
    Documenter,
    /// The human/orchestrator origin.
    Conductor,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Planner => "Planner",
            Role::Coder => "Coder",
            Role::Tester => "Tester",
            Role::Refactorer => "Refactorer",
            Role::Optimizer => "Optimizer",
            Role::SecurityAuditor => "SecurityAuditor",
            Role::Documenter => "Documenter",
            Role::Conductor => "Conductor",
        }
    }
}

/// The payload of a swarm message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MsgKind {
    /// A natural-language intent from the user, kicks off the pipeline.
    Intent(String),
    /// The Planner's decomposition into ordered steps.
    PlanReady { steps: Vec<String> },
    /// The Coder produced/updated a function and wrote it into the graph.
    CodeReady {
        module: String,
        name: String,
        source: String,
    },
    /// The Tester produced a test for a function.
    TestsReady { for_fn: String, source: String },
    /// A free-form annotation from any agent (stub agents emit these).
    Note { text: String },
}

/// An envelope: who sent it + the payload. Broadcast to every subscriber.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmMessage {
    pub from: Role,
    pub kind: MsgKind,
}

impl SwarmMessage {
    pub fn new(from: Role, kind: MsgKind) -> Self {
        SwarmMessage { from, kind }
    }
}

/// Sending half of the bus, cloned to every agent.
pub type Bus = broadcast::Sender<SwarmMessage>;
/// Receiving half, one per agent subscription.
pub type BusRx = broadcast::Receiver<SwarmMessage>;

/// Create a bus with the given backlog capacity.
pub fn channel(capacity: usize) -> Bus {
    broadcast::channel(capacity).0
}
