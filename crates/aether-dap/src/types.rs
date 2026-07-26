//! DAP protocol data types.
//!
//! Covers the subset of the [DAP specification][spec] used by Bit Code:
//! capabilities negotiation, breakpoints, stack frames, scopes, variables,
//! threads, and the most common events. Unknown fields are silently ignored
//! via `#[serde(default)]` so the client stays forward-compatible.
//!
//! [spec]: https://microsoft.github.io/debug-adapter-protocol/specification

use serde::{Deserialize, Serialize};

// ── Initialization ─────────────────────────────────────────────────────────

/// Arguments for the `initialize` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeArgs {
    #[serde(rename = "clientID")]
    pub client_id: String,
    pub client_name: String,
    #[serde(rename = "adapterID")]
    pub adapter_id: String,
    pub locale: String,
    pub lines_start_at1: bool,
    pub columns_start_at1: bool,
    /// Either "path" or "uri".
    pub path_format: String,
    pub supports_variable_type: bool,
    pub supports_run_in_terminal_request: bool,
}

impl Default for InitializeArgs {
    fn default() -> Self {
        Self {
            client_id: "bitcode".to_string(),
            client_name: "Bit Code".to_string(),
            adapter_id: "bitcode".to_string(),
            locale: "en-US".to_string(),
            lines_start_at1: true,
            columns_start_at1: true,
            path_format: "path".to_string(),
            supports_variable_type: true,
            supports_run_in_terminal_request: false,
        }
    }
}

#[cfg(test)]
mod initialize_tests {
    use super::InitializeArgs;

    #[test]
    fn initialize_uses_dap_acronym_field_names() {
        let value = serde_json::to_value(InitializeArgs::default()).unwrap();
        assert_eq!(value["clientID"], "bitcode");
        assert_eq!(value["adapterID"], "bitcode");
        assert!(value.get("clientId").is_none());
        assert!(value.get("adapterId").is_none());
    }
}

/// Capabilities returned by the adapter in the `initialize` response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Capabilities {
    pub supports_configuration_done_request: Option<bool>,
    pub supports_function_breakpoints: Option<bool>,
    pub supports_conditional_breakpoints: Option<bool>,
    pub supports_evaluate_for_hovers: Option<bool>,
    pub supports_set_variable: Option<bool>,
    pub supports_restart_request: Option<bool>,
    pub supports_exception_info_request: Option<bool>,
    pub supports_value_formatting_options: Option<bool>,
    pub supports_terminate_request: Option<bool>,
}

// ── Source + Breakpoints ──────────────────────────────────────────────────

/// A reference to a source file (by path or reference number).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Source {
    pub name: Option<String>,
    pub path: Option<String>,
    pub source_reference: Option<u64>,
}

impl Source {
    pub fn from_path(path: impl Into<String>) -> Self {
        let p = path.into();
        let name = std::path::Path::new(&p)
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string);
        Source {
            name,
            path: Some(p),
            source_reference: None,
        }
    }
}

/// A breakpoint to set, described by its line (and optional condition).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceBreakpoint {
    pub line: u32,
    pub column: Option<u32>,
    pub condition: Option<String>,
    pub log_message: Option<String>,
}

impl SourceBreakpoint {
    pub fn at_line(line: u32) -> Self {
        Self {
            line,
            column: None,
            condition: None,
            log_message: None,
        }
    }
}

/// A confirmed breakpoint returned by the adapter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Breakpoint {
    pub id: Option<u64>,
    pub verified: bool,
    pub message: Option<String>,
    pub source: Option<Source>,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

// ── Execution state ───────────────────────────────────────────────────────

/// A single frame on the call stack.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackFrame {
    pub id: u64,
    pub name: String,
    pub source: Option<Source>,
    pub line: u32,
    pub column: u32,
}

/// A variable scope (locals, globals, registers, …).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    pub name: String,
    pub variables_reference: u64,
    /// Whether fetching variables for this scope is expensive (large objects).
    pub expensive: bool,
}

/// A single variable or object member.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Variable {
    pub name: String,
    pub value: String,
    #[serde(rename = "type")]
    pub type_: Option<String>,
    /// Non-zero if this variable has children (e.g. struct fields).
    pub variables_reference: u64,
    pub named_variables: Option<u64>,
    pub indexed_variables: Option<u64>,
}

/// A debuggee thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: u64,
    pub name: String,
}

// ── Events ────────────────────────────────────────────────────────────────

/// Body of the `stopped` event (execution paused).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoppedEvent {
    /// "breakpoint" | "step" | "exception" | "pause" | "entry" | "goto" | …
    pub reason: String,
    pub description: Option<String>,
    pub thread_id: Option<u64>,
    pub preserve_focus_hint: Option<bool>,
    pub all_threads_stopped: Option<bool>,
    pub hit_breakpoint_ids: Option<Vec<u64>>,
}

/// Body of the `exited` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExitedEvent {
    pub exit_code: i32,
}

/// Body of the `output` event (stdout/stderr from the debuggee).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OutputEvent {
    /// "console" | "stdout" | "stderr" | "telemetry"
    pub category: Option<String>,
    pub output: String,
    pub source: Option<Source>,
    pub line: Option<u32>,
}

// ── Raw message envelopes ─────────────────────────────────────────────────

/// Raw DAP request envelope (before command-specific parsing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DapRequest {
    pub seq: u64,
    #[serde(rename = "type")]
    pub msg_type: String, // always "request"
    pub command: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

/// Raw DAP response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DapResponse {
    pub seq: u64,
    #[serde(rename = "type")]
    pub msg_type: String, // always "response"
    pub request_seq: u64,
    pub success: bool,
    pub command: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub body: serde_json::Value,
}

/// Raw DAP event envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DapEvent {
    pub seq: u64,
    #[serde(rename = "type")]
    pub msg_type: String, // always "event"
    pub event: String,
    #[serde(default)]
    pub body: serde_json::Value,
}
