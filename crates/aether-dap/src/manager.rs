//! `DebugManager` — graph-aware façade over multiple debug sessions.
//!
//! The manager is the bridge between Bit Code's semantic graph and the DAP
//! layer. Its key value-adds over raw [`DebugSession`]:
//!
//! 1. **Node breakpoints**: translate a [`NodeId`] → `(file, line)` using the
//!    node's `file` and `span` fields, then delegate to the session.
//! 2. **Hit annotation**: when the debuggee stops at a breakpoint, find the
//!    matching graph node and increment its `debug_hits` attribute so the graph
//!    panel can visualise hot paths.
//! 3. **Multi-session tracking**: manage a pool of named sessions and route
//!    events to the right one.

use crate::client::DapClient;
use crate::session::DebugSession;
use crate::types::{Breakpoint, SourceBreakpoint, StoppedEvent};
use crate::DapError;
use aether_graph::{NodeId, SemanticGraph};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Opaque identifier for a debug session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(pub u64);

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "session#{}", self.0)
    }
}

/// A breakpoint set by graph node identity (not file:line).
///
/// When the graph node has a valid `file` and `span`, the manager translates
/// it to a `SourceBreakpoint` and delegates to the session. The `breakpoint_id`
/// is filled in once the adapter confirms the breakpoint.
#[derive(Debug, Clone)]
pub struct NodeBreakpoint {
    pub node_id: NodeId,
    pub session_id: SessionId,
    /// Resolved file path (from `node.file`).
    pub file: String,
    /// 1-indexed source line (from `node.span.start_row + 1`).
    pub line: u32,
    /// Adapter-assigned ID; `None` until confirmed.
    pub breakpoint_id: Option<u64>,
    /// How many times this breakpoint has been hit in this session.
    pub hit_count: u32,
}

/// Manages a pool of debug sessions with graph integration.
pub struct DebugManager {
    sessions: HashMap<SessionId, DebugSession>,
    node_breakpoints: Vec<NodeBreakpoint>,
    graph: Arc<Mutex<SemanticGraph>>,
    next_id: u64,
}

impl DebugManager {
    pub fn new(graph: Arc<Mutex<SemanticGraph>>) -> Self {
        DebugManager {
            sessions: HashMap::new(),
            node_breakpoints: Vec::new(),
            graph,
            next_id: 1,
        }
    }

    // ── Session lifecycle ─────────────────────────────────────────────────

    /// Spawn an adapter subprocess and open a new session, returning its id.
    ///
    /// `adapter_id` is the string passed to the `initialize` request (e.g.
    /// `"python"`, `"cppdbg"`, `"codelldb"`).
    pub async fn launch_adapter(
        &mut self,
        adapter_cmd: &str,
        adapter_args: &[&str],
        adapter_id: &str,
    ) -> Result<SessionId, DapError> {
        let client = DapClient::spawn(adapter_cmd, adapter_args).await?;
        let mut session = DebugSession::new(client, adapter_id);
        session.initialize().await?;
        let id = SessionId(self.next_id);
        self.next_id += 1;
        self.sessions.insert(id, session);
        Ok(id)
    }

    pub fn session(&self, id: SessionId) -> Option<&DebugSession> {
        self.sessions.get(&id)
    }

    pub fn session_mut(&mut self, id: SessionId) -> Option<&mut DebugSession> {
        self.sessions.get_mut(&id)
    }

    // ── Node breakpoints ──────────────────────────────────────────────────

    /// Set a breakpoint at the start of the graph node identified by `node_id`.
    ///
    /// Looks up the node in the graph to find its source file and start line,
    /// then forwards a `setBreakpoints` request to the session. Returns the
    /// resolved `NodeBreakpoint` with the adapter's confirmed `breakpoint_id`.
    pub async fn set_node_breakpoint(
        &mut self,
        session_id: SessionId,
        node_id: NodeId,
    ) -> Result<NodeBreakpoint, DapError> {
        let (file, line) = {
            let g = self.graph.lock().unwrap();
            let node = g.get(node_id).ok_or(DapError::NodeNotLocatable(node_id))?;
            let file = node
                .file
                .clone()
                .ok_or(DapError::NodeNotLocatable(node_id))?;
            // DAP lines are 1-indexed; span rows are 0-indexed.
            let line = node.span.start_row as u32 + 1;
            (file, line)
        };

        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(DapError::SessionNotFound(session_id))?;

        let bps: Vec<Breakpoint> = session
            .set_breakpoints(Path::new(&file), &[SourceBreakpoint::at_line(line)])
            .await?;

        let breakpoint_id = bps.first().and_then(|b| b.id);

        let nb = NodeBreakpoint {
            node_id,
            session_id,
            file,
            line,
            breakpoint_id,
            hit_count: 0,
        };
        self.node_breakpoints.push(nb.clone());
        Ok(nb)
    }

    /// Clear all breakpoints for a session and remove them from the index.
    pub async fn clear_node_breakpoints(&mut self, session_id: SessionId) -> Result<(), DapError> {
        // Collect files that had node breakpoints in this session.
        let files: Vec<String> = self
            .node_breakpoints
            .iter()
            .filter(|nb| nb.session_id == session_id)
            .map(|nb| nb.file.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(DapError::SessionNotFound(session_id))?;

        for file in &files {
            // Send an empty breakpoints list to clear all bps in this file.
            session.set_breakpoints(Path::new(file), &[]).await?;
        }

        self.node_breakpoints
            .retain(|nb| nb.session_id != session_id);
        Ok(())
    }

    // ── Stop event handling ───────────────────────────────────────────────

    /// Called when the adapter fires a `stopped` event.
    ///
    /// Fetches the current stack trace, tries to match the top frame to a
    /// graph node, and increments `debug_hits` on that node so the graph panel
    /// can show execution hot spots.
    pub async fn on_stopped(
        &mut self,
        session_id: SessionId,
        event: &StoppedEvent,
    ) -> Result<Option<NodeId>, DapError> {
        let thread_id = event.thread_id.unwrap_or(1);

        let frames = {
            let session = self
                .sessions
                .get(&session_id)
                .ok_or(DapError::SessionNotFound(session_id))?;
            session.stack_trace(thread_id, 10).await?
        };

        let top = match frames.first() {
            Some(f) => f,
            None => return Ok(None),
        };

        let top_file = top
            .source
            .as_ref()
            .and_then(|s| s.path.as_deref())
            .unwrap_or("");
        let top_line = top.line;

        // Find the graph node whose file+span best matches the top frame.
        let hit_node: Option<NodeId> = {
            let g = self.graph.lock().unwrap();
            let found = g
                .nodes()
                .find(|n| {
                    let start_line = n.span.start_row as u32 + 1;
                    n.file
                        .as_deref()
                        .map(|f| f == top_file || top_file.ends_with(f))
                        .unwrap_or(false)
                        && start_line <= top_line
                        && top_line <= start_line + 50
                })
                .map(|n| n.id);
            found
        };

        // Annotate the hit node.
        if let Some(node_id) = hit_node {
            let mut g = self.graph.lock().unwrap();
            if let Some(node) = g.get_mut(node_id) {
                let hits: u32 = node
                    .attr("debug_hits")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0)
                    + 1;
                node.set_attr("debug_hits", hits.to_string());

                // Also track which breakpoint was hit (if it came from a node bp).
                if let Some(ids) = &event.hit_breakpoint_ids {
                    for &bp_id in ids {
                        if let Some(nb) = self
                            .node_breakpoints
                            .iter_mut()
                            .find(|nb| nb.breakpoint_id == Some(bp_id))
                        {
                            nb.hit_count += 1;
                        }
                    }
                }
            }
        }

        Ok(hit_node)
    }

    // ── Convenience launchers ─────────────────────────────────────────────

    /// Launch a Python program under `debugpy`.
    ///
    /// Requires `debugpy` to be installed (`pip install debugpy`).
    /// `program` is the path to the `.py` file; `extra_args` are passed after
    /// the program path (e.g. `["--", "--verbose"]`).
    pub async fn launch_python(
        &mut self,
        program: &Path,
        extra_args: &[&str],
    ) -> Result<SessionId, DapError> {
        let sid = self
            .launch_adapter("python", &["-m", "debugpy.adapter"], "python")
            .await?;

        let session = self.sessions.get_mut(&sid).unwrap();
        session
            .launch(serde_json::json!({
                "program": program.to_string_lossy(),
                "args": extra_args,
                "stopOnEntry": false,
            }))
            .await?;
        session.configuration_done().await?;

        Ok(sid)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use aether_graph::{Node, NodeKind, Span};

    fn graph_with_node() -> (Arc<Mutex<SemanticGraph>>, NodeId) {
        let mut g = SemanticGraph::new();
        let mut node = Node::new(NodeKind::Function, "main", "crate::app::main");
        node.file = Some("src/app.rs".to_string());
        node.span = Span {
            start_row: 4,
            start_col: 0,
            start_byte: 0,
            end_byte: 0,
        };
        let id = g.upsert_node(node);
        (Arc::new(Mutex::new(g)), id)
    }

    #[test]
    fn node_translates_to_file_and_line() {
        let (graph, node_id) = graph_with_node();
        let g = graph.lock().unwrap();
        let node = g.get(node_id).unwrap();
        // Line is 1-indexed: start_row 4 → line 5.
        assert_eq!(node.span.start_row as u32 + 1, 5);
        assert_eq!(node.file.as_deref(), Some("src/app.rs"));
    }

    #[test]
    fn manager_constructs_without_error() {
        let (graph, _) = graph_with_node();
        let _mgr = DebugManager::new(graph);
    }

    #[test]
    fn session_id_display() {
        assert_eq!(SessionId(3).to_string(), "session#3");
    }
}
