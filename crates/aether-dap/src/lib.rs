//! # aether-dap
//!
//! **Debug Adapter Protocol** client, session management, and graph integration
//! for Girder.
//!
//! The [Debug Adapter Protocol][dap] (DAP) is the language-agnostic JSON-RPC
//! protocol used by VS Code to talk to external debuggers. Girder speaks
//! DAP as a *client* to any adapter the user installs:
//!
//! - **Python**: `python -m debugpy.adapter`
//! - **Rust**: `codelldb --port 0 --adapter`
//! - Any future language with a DAP adapter binary
//!
//! ## Architecture
//!
//! ```text
//! DebugManager        ← graph-aware façade, manages multiple sessions
//!   └─ DebugSession   ← state machine (Created→Initialized→Running→Stopped)
//!       └─ DapClient  ← async JSON-RPC over subprocess stdio
//!           └─ DapTransport  ← Content-Length framing
//! ```
//!
//! The graph integration in [`manager::DebugManager`] translates
//! `NodeId → (file, line)` using the node's `file` and `span` fields so
//! breakpoints can be set by semantic identity rather than hard-coded paths.
//!
//! [dap]: https://microsoft.github.io/debug-adapter-protocol/

pub mod client;
pub mod manager;
pub mod session;
pub mod transport;
pub mod types;

pub use client::DapClient;
pub use manager::{DebugManager, NodeBreakpoint, SessionId};
pub use session::{Capabilities, DebugSession, SessionState};
pub use types::{
    Breakpoint, Scope, Source, SourceBreakpoint, StackFrame, StoppedEvent, Thread, Variable,
};

/// Every error that can occur in the DAP layer.
#[derive(Debug, thiserror::Error)]
pub enum DapError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("protocol: {0}")]
    Protocol(String),

    #[error("request '{command}' failed: {message}")]
    RequestFailed { command: String, message: String },

    #[error("adapter process exited unexpectedly")]
    AdapterExited,

    #[error("timed out waiting for adapter response")]
    Timeout,

    #[error("session in wrong state for this operation: {0}")]
    WrongState(String),

    #[error("node {0:?} has no file/span information")]
    NodeNotLocatable(aether_graph::NodeId),

    #[error("session {0} not found")]
    SessionNotFound(SessionId),
}
