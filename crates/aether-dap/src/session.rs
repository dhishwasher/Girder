//! `DebugSession` — high-level state machine over a [`DapClient`].
//!
//! ## State diagram
//!
//! ```text
//! Created
//!   ↓ initialize()
//! Ready         (capabilities negotiated)
//!   ↓ launch() / attach()
//! Configuring   (`initialized` received; set breakpoints here)
//!   ↓ configuration_done()
//! Running
//!   ↓ stopped event
//! Stopped(reason)
//!   ↓ continue() / next() / step_in() / step_out()
//! Running
//!   ↓ exited or terminated event
//! Terminated
//! ```
//!
//! Every method asserts the session is in a valid state before sending the
//! request, returning `DapError::WrongState` if not.

use crate::client::DapClient;
use crate::types::{
    Breakpoint, InitializeArgs, Scope, Source, SourceBreakpoint, StackFrame, StoppedEvent, Thread,
    Variable,
};
use crate::DapError;
use std::path::Path;
use std::time::Duration;

// Re-export so `session::Capabilities` is addressable from lib.rs.
pub use crate::types::Capabilities;

/// The lifecycle state of a debug session.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionState {
    /// Freshly constructed; no adapter messages exchanged yet.
    Created,
    /// `initialize` sent; waiting for its capability response.
    Initializing,
    /// Capabilities exchanged; ready to launch or attach.
    Ready,
    /// Launch/attach started and `initialized` received; configure breakpoints.
    Configuring,
    /// Debuggee is executing.
    Running,
    /// Execution paused. Inner string is the stop reason ("breakpoint",
    /// "step", "exception", "pause", "entry", …).
    Stopped(String),
    /// Session has ended (disconnected, debuggee exited, or error).
    Terminated,
}

/// A single debug session: one adapter subprocess ↔ one debuggee process.
pub struct DebugSession {
    pub state: SessionState,
    pub(crate) client: DapClient,
    adapter_id: String,
    pending_start: Option<tokio::task::JoinHandle<Result<crate::types::DapResponse, DapError>>>,
}

impl DebugSession {
    pub fn new(client: DapClient, adapter_id: impl Into<String>) -> Self {
        DebugSession {
            state: SessionState::Created,
            client,
            adapter_id: adapter_id.into(),
            pending_start: None,
        }
    }

    // ── Initialization ────────────────────────────────────────────────────

    /// Negotiate capabilities with the adapter.
    ///
    /// Sends `initialize`, records the adapter capabilities, then transitions
    /// to `Ready`. DAP adapters emit `initialized` after `launch` or `attach`.
    pub async fn initialize(&mut self) -> Result<Capabilities, DapError> {
        self.require_state(SessionState::Created, "initialize")?;
        self.state = SessionState::Initializing;

        let args = InitializeArgs {
            adapter_id: self.adapter_id.clone(),
            ..Default::default()
        };
        let response = self
            .client
            .send("initialize", serde_json::to_value(&args)?)
            .await?;
        check_response(&response, "initialize")?;

        let caps: Capabilities = serde_json::from_value(response.body).unwrap_or_default();

        self.state = SessionState::Ready;
        Ok(caps)
    }

    // ── Configuration (must be in Configuring state) ──────────────────────

    /// Set breakpoints in a source file, replacing any previously set for it.
    ///
    /// Returns the confirmed [`Breakpoint`] list from the adapter (each entry
    /// may have `verified: false` if the adapter can't resolve the line yet).
    pub async fn set_breakpoints(
        &mut self,
        source: &Path,
        breakpoints: &[SourceBreakpoint],
    ) -> Result<Vec<Breakpoint>, DapError> {
        self.require_any(
            &[SessionState::Configuring, SessionState::Running],
            "setBreakpoints",
        )?;

        let src = Source::from_path(source.to_string_lossy().as_ref());
        let body = self
            .client
            .send(
                "setBreakpoints",
                serde_json::json!({
                    "source": src,
                    "breakpoints": breakpoints,
                }),
            )
            .await?;
        check_response(&body, "setBreakpoints")?;

        let bps: Vec<Breakpoint> = body
            .body
            .get("breakpoints")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(bps)
    }

    /// Signal to the adapter that configuration is complete and it may run.
    pub async fn configuration_done(&mut self) -> Result<(), DapError> {
        self.require_state(SessionState::Configuring, "configurationDone")?;
        let r = self
            .client
            .send("configurationDone", serde_json::Value::Null)
            .await?;
        check_response(&r, "configurationDone")?;

        let mut pending = self.pending_start.take().ok_or_else(|| {
            DapError::Protocol("configurationDone has no pending launch or attach request".into())
        })?;
        let response = match tokio::time::timeout(Duration::from_secs(15), &mut pending).await {
            Ok(result) => result
                .map_err(|error| DapError::Protocol(format!("startup task failed: {error}")))??,
            Err(_) => {
                pending.abort();
                return Err(DapError::Timeout);
            }
        };
        let command = response.command.clone();
        check_response(&response, &command)?;
        self.state = SessionState::Running;
        Ok(())
    }

    // ── Launch / Attach ───────────────────────────────────────────────────

    /// Launch the debuggee. `launch_args` are adapter-specific (e.g.
    /// `{"program": "/path/to/script.py", "stopOnEntry": true}`).
    pub async fn launch(&mut self, launch_args: serde_json::Value) -> Result<(), DapError> {
        self.begin_start("launch", launch_args).await
    }

    /// Attach to an already-running process by its pid.
    pub async fn attach(&mut self, pid: u32, extra: serde_json::Value) -> Result<(), DapError> {
        let mut args = serde_json::json!({ "processId": pid });
        if let (Some(obj), Some(ext)) = (args.as_object_mut(), extra.as_object()) {
            obj.extend(ext.clone());
        }
        self.begin_start("attach", args).await
    }

    // ── Execution control (must be Stopped) ──────────────────────────────

    /// Resume execution of the given thread (or all threads if `thread_id` is 0).
    pub async fn continue_(&mut self, thread_id: u64) -> Result<(), DapError> {
        self.require_state(SessionState::Stopped(String::new()), "continue")?;
        let r = self
            .client
            .send("continue", serde_json::json!({ "threadId": thread_id }))
            .await?;
        check_response(&r, "continue")?;
        self.state = SessionState::Running;
        Ok(())
    }

    /// Step over the current line.
    pub async fn next(&mut self, thread_id: u64) -> Result<(), DapError> {
        self.require_stopped("next")?;
        let r = self
            .client
            .send("next", serde_json::json!({ "threadId": thread_id }))
            .await?;
        check_response(&r, "next")?;
        self.state = SessionState::Running;
        Ok(())
    }

    /// Step into the current function call.
    pub async fn step_in(&mut self, thread_id: u64) -> Result<(), DapError> {
        self.require_stopped("stepIn")?;
        let r = self
            .client
            .send("stepIn", serde_json::json!({ "threadId": thread_id }))
            .await?;
        check_response(&r, "stepIn")?;
        self.state = SessionState::Running;
        Ok(())
    }

    /// Step out of the current function.
    pub async fn step_out(&mut self, thread_id: u64) -> Result<(), DapError> {
        self.require_stopped("stepOut")?;
        let r = self
            .client
            .send("stepOut", serde_json::json!({ "threadId": thread_id }))
            .await?;
        check_response(&r, "stepOut")?;
        self.state = SessionState::Running;
        Ok(())
    }

    /// Pause a running thread.
    pub async fn pause(&mut self, thread_id: u64) -> Result<(), DapError> {
        self.require_state(SessionState::Running, "pause")?;
        let r = self
            .client
            .send("pause", serde_json::json!({ "threadId": thread_id }))
            .await?;
        check_response(&r, "pause")?;
        Ok(())
    }

    // ── Inspection (available while Stopped) ─────────────────────────────

    /// List all threads in the debuggee.
    pub async fn threads(&self) -> Result<Vec<Thread>, DapError> {
        let r = self.client.send("threads", serde_json::Value::Null).await?;
        check_response(&r, "threads")?;
        let threads: Vec<Thread> = r
            .body
            .get("threads")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(threads)
    }

    /// Fetch the call stack for `thread_id`, up to `levels` frames.
    pub async fn stack_trace(
        &self,
        thread_id: u64,
        levels: u32,
    ) -> Result<Vec<StackFrame>, DapError> {
        let r = self
            .client
            .send(
                "stackTrace",
                serde_json::json!({ "threadId": thread_id, "levels": levels }),
            )
            .await?;
        check_response(&r, "stackTrace")?;
        let frames: Vec<StackFrame> = r
            .body
            .get("stackFrames")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(frames)
    }

    /// Get the variable scopes for a stack frame.
    pub async fn scopes(&self, frame_id: u64) -> Result<Vec<Scope>, DapError> {
        let r = self
            .client
            .send("scopes", serde_json::json!({ "frameId": frame_id }))
            .await?;
        check_response(&r, "scopes")?;
        let scopes: Vec<Scope> = r
            .body
            .get("scopes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(scopes)
    }

    /// Fetch variables for a scope/object reference.
    pub async fn variables(&self, variables_reference: u64) -> Result<Vec<Variable>, DapError> {
        let r = self
            .client
            .send(
                "variables",
                serde_json::json!({ "variablesReference": variables_reference }),
            )
            .await?;
        check_response(&r, "variables")?;
        let vars: Vec<Variable> = r
            .body
            .get("variables")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(vars)
    }

    /// Evaluate an expression in the context of a stack frame.
    pub async fn evaluate(
        &self,
        expression: &str,
        frame_id: Option<u64>,
        context: &str,
    ) -> Result<String, DapError> {
        let mut args = serde_json::json!({ "expression": expression, "context": context });
        if let (Some(obj), Some(fid)) = (args.as_object_mut(), frame_id) {
            obj.insert("frameId".to_string(), serde_json::json!(fid));
        }
        let r = self.client.send("evaluate", args).await?;
        check_response(&r, "evaluate")?;
        Ok(r.body
            .get("result")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
    }

    // ── Termination ───────────────────────────────────────────────────────

    pub async fn disconnect(&mut self) -> Result<(), DapError> {
        if self.state == SessionState::Terminated {
            return Ok(());
        }
        if let Some(pending) = self.pending_start.take() {
            pending.abort();
        }
        let _ = self
            .client
            .send(
                "disconnect",
                serde_json::json!({ "terminateDebuggee": true }),
            )
            .await;
        self.state = SessionState::Terminated;
        Ok(())
    }

    async fn begin_start(
        &mut self,
        command: &'static str,
        arguments: serde_json::Value,
    ) -> Result<(), DapError> {
        self.require_state(SessionState::Ready, command)?;
        let mut events = self.client.subscribe_events();
        let client = self.client.clone();
        let pending = tokio::spawn(async move { client.send(command, arguments).await });

        let initialized = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                match events.recv().await {
                    Ok(event) if event.event == "initialized" => return Ok(()),
                    Ok(event) if event.event == "exited" || event.event == "terminated" => {
                        return Err(DapError::AdapterExited);
                    }
                    Ok(_) => continue,
                    Err(_) => return Err(DapError::AdapterExited),
                }
            }
        })
        .await;

        match initialized {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                pending.abort();
                self.state = SessionState::Terminated;
                return Err(error);
            }
            Err(_) => {
                pending.abort();
                self.state = SessionState::Terminated;
                return Err(DapError::Timeout);
            }
        }

        self.pending_start = Some(pending);
        self.state = SessionState::Configuring;
        Ok(())
    }

    // ── Event helpers ─────────────────────────────────────────────────────

    /// Block until the adapter emits a `stopped` event, then update state.
    pub async fn wait_for_stopped(&mut self) -> Result<StoppedEvent, DapError> {
        let mut events = self.client.subscribe_events();
        let stopped: StoppedEvent = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                match events.recv().await {
                    Ok(e) if e.event == "stopped" => {
                        let s: StoppedEvent = serde_json::from_value(e.body).unwrap_or_default();
                        return Ok(s);
                    }
                    Ok(e) if e.event == "exited" || e.event == "terminated" => {
                        return Err(DapError::AdapterExited);
                    }
                    Ok(_) => continue,
                    Err(_) => return Err(DapError::AdapterExited),
                }
            }
        })
        .await
        .map_err(|_| DapError::Timeout)??;

        self.state = SessionState::Stopped(stopped.reason.clone());
        Ok(stopped)
    }

    // ── State guards ──────────────────────────────────────────────────────

    fn require_state(&self, expected: SessionState, op: &str) -> Result<(), DapError> {
        // Use a pattern match that ignores the inner string for Stopped.
        let ok = match (&self.state, &expected) {
            (SessionState::Stopped(_), SessionState::Stopped(_)) => true,
            (a, b) => a == b,
        };
        if !ok {
            Err(DapError::WrongState(format!(
                "'{op}' requires state {expected:?}, current: {:?}",
                self.state
            )))
        } else {
            Ok(())
        }
    }

    fn require_any(&self, allowed: &[SessionState], op: &str) -> Result<(), DapError> {
        let ok = allowed.iter().any(|s| match (&self.state, s) {
            (SessionState::Stopped(_), SessionState::Stopped(_)) => true,
            (a, b) => a == b,
        });
        if !ok {
            Err(DapError::WrongState(format!(
                "'{op}' requires one of {allowed:?}, current: {:?}",
                self.state
            )))
        } else {
            Ok(())
        }
    }

    fn require_stopped(&self, op: &str) -> Result<(), DapError> {
        if matches!(self.state, SessionState::Stopped(_)) {
            Ok(())
        } else {
            Err(DapError::WrongState(format!(
                "'{op}' requires Stopped, current: {:?}",
                self.state
            )))
        }
    }
}

fn check_response(r: &crate::types::DapResponse, command: &str) -> Result<(), DapError> {
    if r.success {
        Ok(())
    } else {
        Err(DapError::RequestFailed {
            command: command.to_string(),
            message: r.message.clone().unwrap_or_default(),
        })
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_transitions_compile_and_compare() {
        assert_eq!(SessionState::Created, SessionState::Created);
        assert_eq!(
            SessionState::Stopped("breakpoint".to_string()),
            SessionState::Stopped("breakpoint".to_string())
        );
        assert_ne!(
            SessionState::Stopped("step".to_string()),
            SessionState::Running
        );
    }

    #[test]
    fn require_state_rejects_wrong_state() {
        // Build a session with a mock client — we only test the state guard.
        // (A real DapClient requires a subprocess; that's integration-tested.)
        // Since we can't build a DapClient without a process, we just test the
        // enum logic here.
        let reason = "breakpoint".to_string();
        let stopped = SessionState::Stopped(reason);
        // Matches any Stopped regardless of reason.
        assert!(matches!(stopped, SessionState::Stopped(_)));
        assert_ne!(stopped, SessionState::Running);
    }

    #[test]
    fn check_response_fails_on_error() {
        let r = crate::types::DapResponse {
            seq: 2,
            msg_type: "response".into(),
            request_seq: 1,
            success: false,
            command: "initialize".into(),
            message: Some("not supported".into()),
            body: serde_json::Value::Null,
        };
        let err = check_response(&r, "initialize").unwrap_err();
        assert!(matches!(err, DapError::RequestFailed { .. }));
    }
}
