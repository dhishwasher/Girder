//! `girder mcp [dir]` — serve the read-only graph commands to an AI coding
//! agent over the Model Context Protocol on stdin/stdout.
//!
//! This is the integration surface that matters for a tool whose value is
//! measured in context bytes: agents don't read READMEs, they call MCP
//! tools. `docs/context-vs-read-cost.md` measured `context --source-only`
//! at 97.85% fewer bytes than reading the whole file across ten nodes; that
//! saving only reaches an agent if the agent can call it, which is what this
//! server is for.
//!
//! # Transport
//!
//! Newline-delimited JSON-RPC 2.0 over stdin/stdout, per the MCP stdio
//! transport. **Nothing else may ever be written to stdout** — a stray
//! `println!` or log line corrupts the frame stream and the client drops the
//! connection. `main` routes tracing to stderr for this command specifically,
//! and every diagnostic here goes to stderr.
//!
//! # Protocol eras
//!
//! Both handshakes are answered, because deployed clients span them:
//!   * Legacy (`initialize` + `notifications/initialized`), which is what
//!     shipping agents use today. The negotiated version echoes the client's
//!     request when recognized.
//!   * Stateless (`server/discover`), which advertises
//!     [`SUPPORTED_PROTOCOL_VERSIONS`] and carries per-request `_meta`.
//!
//! Per-request `_meta` is accepted and ignored: every tool here is read-only
//! and stateless, so no client capability changes what a call does.
//!
//! # Why each tool call is a subprocess
//!
//! A tool call re-executes this same binary through
//! [`run_captured`](crate::project::process::run_captured) rather than
//! calling the command function in-process. Three reasons:
//!   * **No divergence.** The MCP tool returns exactly what the documented
//!     CLI contract returns, byte for byte. Two code paths for the same
//!     answer would drift.
//!   * **Fail-closed isolation.** A panic or a runaway graph build kills one
//!     child, bounded by time and output, and the agent gets an error it can
//!     act on. In-process, the same fault takes down the server mid-session.
//!   * **No behavioral risk to the CLI.** The six commands print through
//!     paths pinned by existing tests; wrapping them changes none of it.
//!
//! The cost is one process spawn against a graph build the CLI performs
//! anyway, so it is not the bottleneck.
//!
//! # Confinement
//!
//! The project root is fixed when the server starts. No tool argument can
//! redirect a call at another directory, so an agent cannot walk the
//! filesystem through this server.
//!
//! Tool arguments are also data, never options. The subcommands behind
//! these tools scan the whole of argv for their flags and honour no `--`
//! marker, so a value beginning with `-` would reach them as an option:
//! `nodes: ["--run"]` once ran the project's tests through `impacted_tests`
//! and `nodes: ["--out", path]` once wrote a file. Every argument is
//! refused if it starts with `-` — see [`reject_option_like`].

use crate::project::process::{run_captured, BoundedStatus};
use serde_json::{json, Map, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Versions this server will speak, newest first. The tool surface is
/// identical across them: these are all handshake and metadata differences.
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[
    "2026-07-28",
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];

/// Version reported when a legacy client asks for one this server does not
/// recognize. The spec's rule for that case is to answer with a version the
/// server does support and let the client decide.
const FALLBACK_PROTOCOL_VERSION: &str = "2025-06-18";

const SERVER_NAME: &str = "girder";

/// Per-tool-call wall-clock budget. A cold graph build on a mid-size
/// repository is seconds; `review` on this repo measured ~12s. 120s leaves
/// room for a large repository without letting a wedged child hang an agent
/// indefinitely. Override with `GIRDER_MCP_TIMEOUT_SECONDS`.
const DEFAULT_TOOL_TIMEOUT_SECONDS: u64 = 120;

/// Hard cap on one tool call's captured output. `run_captured` kills the
/// child and reports `OutputLimited` rather than returning truncated data,
/// so this fails loudly instead of handing an agent a half-parsed answer.
const MAX_TOOL_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

// JSON-RPC 2.0 error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

pub fn mcp(args: &[String]) -> std::io::Result<()> {
    let root = PathBuf::from(args.first().map(String::as_str).unwrap_or("."))
        .canonicalize()
        .map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!(
                    "cannot serve MCP for {:?}: {error}",
                    args.first().map(String::as_str).unwrap_or(".")
                ),
            )
        })?;

    let executable = std::env::current_exe()?;
    eprintln!(
        "girder MCP server on stdio: root {}, {} tools, read-only",
        root.display(),
        TOOLS.len()
    );

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = handle_line(&line, &root, &executable) {
            serde_json::to_writer(&mut stdout, &response)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            stdout.write_all(b"\n")?;
            // Flushed per frame: an agent is blocked on this answer, and a
            // buffered reply is indistinguishable from a hung server.
            stdout.flush()?;
        }
    }
    Ok(())
}

/// Parses and dispatches one frame. `None` means "write nothing", which is
/// required for notifications and for anything unparseable that carried no
/// id to answer.
fn handle_line(line: &str, root: &Path, executable: &Path) -> Option<Value> {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(error) => {
            return Some(error_response(
                Value::Null,
                PARSE_ERROR,
                &format!("invalid JSON: {error}"),
            ));
        }
    };

    let Some(object) = message.as_object() else {
        // Includes JSON-RPC batches, which MCP removed in 2025-06-18.
        return Some(error_response(
            Value::Null,
            INVALID_REQUEST,
            "expected a single JSON-RPC object",
        ));
    };

    let id = object.get("id").cloned();
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return id.map(|id| error_response(id, INVALID_REQUEST, "missing method"));
    };
    let params = object.get("params").cloned().unwrap_or(Value::Null);

    // No id means a notification: handle the effect, never answer.
    let id = id?;

    Some(match dispatch(method, &params, root, executable) {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(failure) => error_response(id, failure.code, &failure.message),
    })
}

struct Failure {
    code: i64,
    message: String,
}

fn failure(code: i64, message: impl Into<String>) -> Failure {
    Failure {
        code,
        message: message.into(),
    }
}

fn dispatch(
    method: &str,
    params: &Value,
    root: &Path,
    executable: &Path,
) -> Result<Value, Failure> {
    match method {
        "initialize" => Ok(initialize_result(params)),
        "server/discover" => Ok(discover_result()),
        "tools/list" => Ok(json!({"tools": TOOLS.iter().map(describe_tool).collect::<Vec<_>>()})),
        "tools/call" => call_tool(params, root, executable),
        "ping" => Ok(json!({})),
        other => Err(failure(
            METHOD_NOT_FOUND,
            format!("unsupported method: {other}"),
        )),
    }
}

/// Legacy handshake. The client's requested version is echoed when this
/// server knows it, since the tool surface is the same across all of them.
fn initialize_result(params: &Value) -> Value {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(FALLBACK_PROTOCOL_VERSION);
    let negotiated = if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested) {
        requested
    } else {
        FALLBACK_PROTOCOL_VERSION
    };
    json!({
        "protocolVersion": negotiated,
        "capabilities": {"tools": {}},
        "serverInfo": server_info(),
        "instructions": INSTRUCTIONS,
    })
}

/// Stateless-era discovery. `listChanged` is absent because the tool set is
/// a compile-time constant: it cannot change while the server runs.
fn discover_result() -> Value {
    json!({
        "resultType": "complete",
        "supportedVersions": SUPPORTED_PROTOCOL_VERSIONS,
        "capabilities": {"tools": {}},
        "_meta": {"io.modelcontextprotocol/serverInfo": server_info()},
    })
}

fn server_info() -> Value {
    json!({"name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION")})
}

/// Shown to the model once at connection time. Its whole job is to stop an
/// agent reaching for Read and grep when a cheaper answer exists, which is
/// the entire premise of these tools.
const INSTRUCTIONS: &str = "\
Girder answers questions about this repository from a semantic graph of it, \
rather than by reading files. Prefer these tools over opening files or \
grepping: `get_source` returns one function's source without the file around \
it (measured 97.85% fewer bytes than reading the whole file across ten \
nodes), `find_definition` beats grep for locating a declaration (measured \
97.98% fewer bytes across ten identifiers), and `impacted_tests` names only \
the tests that can reach what changed. Every tool is read-only and confined \
to the project root the server started in. Byte measurements, not token \
measurements: see docs/context-vs-read-cost.md and docs/names-cost.md.";

/// Translates a tool call's validated arguments, plus the pinned project
/// root, into the argv that answers it. `Err` becomes a client-visible
/// invalid-params message.
type ArgvBuilder = fn(&Map<String, Value>, &str) -> Result<Vec<String>, String>;

/// One MCP tool, bound to the CLI invocation that answers it.
struct Tool {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    /// JSON Schema for `arguments`, built on demand to keep the table static.
    schema: fn() -> Value,
    argv: ArgvBuilder,
}

static TOOLS: &[Tool] = &[
    Tool {
        name: "get_source",
        title: "Get a function's source",
        description: "\
Return the source of specific functions without the surrounding file. Pin \
exact node paths with `nodes`, or describe what you want in `intent` to let \
concept search choose. Measured 97.85% fewer bytes than reading the whole \
file across ten nodes, and cheaper on all ten \
(docs/context-vs-read-cost.md). Prefer this over opening a file to read one \
function. Use `find_definition` or `search_code` first if you do not know \
the node path.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "nodes": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Exact node paths, e.g. crate::crates::aether-graph::src::lib::SemanticGraph::tests_for. Use find_definition or search_code to discover them."
                    },
                    "intent": {
                        "type": "string",
                        "description": "Natural-language description of the code you want, used only when `nodes` is omitted."
                    },
                    "include_tests": {
                        "type": "boolean",
                        "description": "Also return the source of tests that cover each node. Off by default: it costs substantially more bytes."
                    }
                },
                "additionalProperties": false
            })
        },
        argv: |arguments, root| {
            let mut argv = vec![
                "context".to_string(),
                root.to_string(),
                "--json".to_string(),
                "--source-only".to_string(),
            ];
            let nodes = optional_string_array(arguments, "nodes")?;
            let intent = optional_string(arguments, "intent")?;
            match (nodes.as_deref(), intent) {
                (Some(nodes), _) if !nodes.is_empty() => {
                    argv.push("--nodes".to_string());
                    argv.push(nodes.join(","));
                }
                (_, Some(intent)) if !intent.trim().is_empty() => {
                    argv.push(intent.to_string());
                }
                _ => {
                    return Err(
                        "provide `nodes` (exact node paths) or `intent` (a description)"
                            .to_string(),
                    )
                }
            }
            if optional_bool(arguments, "include_tests")?.unwrap_or(false) {
                argv.push("--with-tests".to_string());
            }
            Ok(argv)
        },
    },
    Tool {
        name: "find_definition",
        title: "Find where a name is declared",
        description: "\
Locate the declarations of an exact identifier: every function or type with \
that name, and nothing else. This is not a substring or full-text search — \
it will not match call sites, comments, or strings, which is why it is \
drastically cheaper than grep for the question \"where is X defined?\" \
(measured 97.98% fewer bytes across ten identifiers, docs/names-cost.md). \
Use `search_code` instead when you only know roughly what you are after.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The exact identifier to look up."
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["function", "type", "all"],
                        "description": "Restrict results to functions or types. Defaults to all."
                    }
                },
                "required": ["name"],
                "additionalProperties": false
            })
        },
        argv: |arguments, root| {
            let name = required_string(arguments, "name")?;
            let mut argv = vec![
                "names".to_string(),
                root.to_string(),
                name.to_string(),
                "--json".to_string(),
            ];
            if let Some(kind) = optional_string(arguments, "kind")? {
                argv.push("--kind".to_string());
                argv.push(kind.to_string());
            }
            Ok(argv)
        },
    },
    Tool {
        name: "search_code",
        title: "Search for code by concept",
        description: "\
Rank functions and types by relevance to a natural-language description, \
returning the top matches as scores with node paths. Use this when you do \
not know the name of what you want; use `find_definition` when you do. \
Feed the node paths it returns to `get_source`.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "What the code you are looking for does, in plain language."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            })
        },
        argv: |arguments, root| {
            let query = required_string(arguments, "query")?;
            Ok(vec![
                "search".to_string(),
                root.to_string(),
                query.to_string(),
            ])
        },
    },
    Tool {
        name: "ask_codebase",
        title: "Ask about call relationships",
        description: "\
Answer a question about this codebase by traversing the semantic graph: what \
calls a function, what it calls, what a change to it would affect, and what \
a node is. No code is generated and no model is consulted — this is graph \
traversal, so it finds callers that grep cannot (and does not invent ones \
that do not exist). Prefer it over grepping for call sites.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "e.g. \"what calls resolve_calls?\", \"what would change if I edit tests_for?\""
                    }
                },
                "required": ["question"],
                "additionalProperties": false
            })
        },
        argv: |arguments, root| {
            let question = required_string(arguments, "question")?;
            if question.trim().is_empty() {
                // A blank question makes the CLI open an interactive REPL on
                // stdin. The child's stdin is closed so it could only ever
                // hit EOF, but rejecting it here gives a usable error.
                return Err("`question` must not be empty".to_string());
            }
            Ok(vec![
                "query".to_string(),
                root.to_string(),
                question.to_string(),
            ])
        },
    },
    Tool {
        name: "impacted_tests",
        title: "List tests affected by a change",
        description: "\
Paid license required. \
Name only the tests that can reach the functions that changed, detected from \
the git diff or from explicit node paths. One test name per line, ready to \
pass to a test runner. An empty result means nothing needs testing — it does \
NOT mean run everything. Treat this as advisory: it is known to over-select \
unrelated tests and to miss tests reached only through dynamic dispatch, so \
a full run remains the authority before you claim a change is safe.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "nodes": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Exact node paths to analyze. Omit to use the current git diff."
                    }
                },
                "additionalProperties": false
            })
        },
        argv: |arguments, root| {
            let mut argv = vec![
                "test-impact".to_string(),
                root.to_string(),
                "--quiet".to_string(),
            ];
            if let Some(nodes) = optional_string_array(arguments, "nodes")? {
                argv.extend(nodes);
            }
            Ok(argv)
        },
    },
    Tool {
        name: "review_changes",
        title: "Review changes semantically",
        description: "\
Report what changed in this working tree as semantics rather than text: which \
functions and types were added, modified, or removed, and (in full mode) what \
they affect and which changes lack test coverage. Complements a text diff \
instead of replacing it — use it to see the blast radius of a change you or \
someone else just made. Coverage gaps it reports are advisory.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "since": {
                        "type": "string",
                        "description": "Git ref to compare against. Defaults to HEAD."
                    },
                    "full": {
                        "type": "boolean",
                        "description": "Include impact radius and coverage gaps. Off by default, which lists only changed node paths."
                    }
                },
                "additionalProperties": false
            })
        },
        argv: |arguments, root| {
            let mut argv = vec!["review".to_string(), root.to_string()];
            if let Some(since) = optional_string(arguments, "since")? {
                argv.push("--since".to_string());
                argv.push(since.to_string());
            }
            if !optional_bool(arguments, "full")?.unwrap_or(false) {
                argv.push("--quiet".to_string());
            }
            Ok(argv)
        },
    },
    Tool {
        name: "orient",
        title: "Orient at a node in one call",
        description: "\
Paid license required. \
Answer \"what am I about to touch and what does it reach\" for one starting \
point in a single round trip: its source, direct callers and callees (to \
`depth`, default 1, capped at 2), the tests that cover it, and its impact \
set — everything `get_source` + `ask_codebase` (callers, callees, and \
impact questions) + `impacted_tests` would answer across several separate \
calls, bundled into one. Prefer this over chaining those tools when \
orienting in unfamiliar code. Pin `nodes` when you know the exact path; use \
`intent` only when you do not, and check the returned `confidence` — a \
`\"low\"` value means the match is a guess (natural-language search on this \
codebase resolves correctly only part of the time, see \
docs/description-search-accuracy.md), not a verified answer, and \
`find_definition` or `search_code` first is the safer path. Large \
caller/callee/test/impact sections report a true count and are capped \
rather than silently dropped.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "nodes": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Exact node paths to orient at. Use find_definition or search_code to discover them."
                    },
                    "intent": {
                        "type": "string",
                        "description": "Natural-language description of the code to orient at, used only when `nodes` is omitted. A low-confidence resolution is flagged in the response, not hidden."
                    },
                    "depth": {
                        "type": "integer",
                        "description": "Hops of callers/callees to report beyond the immediate ones. Defaults to 1. Capped at 2."
                    }
                },
                "additionalProperties": false
            })
        },
        argv: |arguments, root| {
            let mut argv = vec!["orient".to_string(), root.to_string(), "--json".to_string()];
            let nodes = optional_string_array(arguments, "nodes")?;
            let intent = optional_string(arguments, "intent")?;
            match (nodes.as_deref(), intent) {
                (Some(nodes), _) if !nodes.is_empty() => {
                    argv.push("--nodes".to_string());
                    argv.push(nodes.join(","));
                }
                (_, Some(intent)) if !intent.trim().is_empty() => {
                    argv.push(intent.to_string());
                }
                _ => {
                    return Err(
                        "provide `nodes` (exact node paths) or `intent` (a description)"
                            .to_string(),
                    )
                }
            }
            if let Some(depth) = optional_u32(arguments, "depth")? {
                argv.push("--depth".to_string());
                argv.push(depth.to_string());
            }
            Ok(argv)
        },
    },
];

fn describe_tool(tool: &Tool) -> Value {
    json!({
        "name": tool.name,
        "title": tool.title,
        "description": tool.description,
        "inputSchema": (tool.schema)(),
        "annotations": {
            "title": tool.title,
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false,
        },
    })
}

fn call_tool(params: &Value, root: &Path, executable: &Path) -> Result<Value, Failure> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| failure(INVALID_PARAMS, "tools/call requires a tool name"))?;
    let tool = TOOLS
        .iter()
        .find(|tool| tool.name == name)
        .ok_or_else(|| failure(INVALID_PARAMS, format!("unknown tool: {name}")))?;

    // Absent `arguments` is legal for a tool whose fields are all optional.
    let empty = Map::new();
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => &empty,
        Some(Value::Object(object)) => object,
        Some(_) => return Err(failure(INVALID_PARAMS, "`arguments` must be an object")),
    };

    let root = root.to_str().ok_or_else(|| {
        failure(
            INVALID_PARAMS,
            "project root is not valid UTF-8 and cannot be passed as an argument",
        )
    })?;
    let argv = (tool.argv)(arguments, root)
        .map_err(|message| failure(INVALID_PARAMS, format!("{name}: {message}")))?;

    Ok(run_tool(executable, &argv))
}

/// Runs one tool's CLI invocation and shapes the outcome as an MCP tool
/// result. A command that fails is reported as `isError: true` content
/// rather than a JSON-RPC error, per the spec: the model should see the
/// diagnostic and be able to correct its own next call.
fn run_tool(executable: &Path, argv: &[String]) -> Value {
    let mut command = Command::new(executable);
    command.args(argv);
    // The MCP client owns this process's stdin. A child must never be able
    // to read from it, or an interactive subcommand would consume the
    // protocol stream itself.
    command.stdin(Stdio::null());

    let captured = match run_captured(command, tool_timeout(), MAX_TOOL_OUTPUT_BYTES) {
        Ok(captured) => captured,
        Err(error) => {
            return tool_error(format!(
                "could not run `girder {}`: {error}",
                argv.join(" ")
            ))
        }
    };

    let stdout = String::from_utf8_lossy(&captured.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&captured.stderr).into_owned();

    match captured.status {
        BoundedStatus::Completed(status) if status.success() => {
            if stdout.trim().is_empty() {
                // Real and meaningful for the --quiet commands: no changed
                // nodes, or no impacted tests. Say so, because a model
                // handed "" tends to assume the tool broke.
                tool_text("(no results)")
            } else {
                tool_text(&stdout)
            }
        }
        BoundedStatus::Completed(_) => {
            let detail = if stderr.trim().is_empty() {
                stdout
            } else {
                stderr
            };
            tool_error(format!("`girder {}` failed: {}", argv.join(" "), detail.trim()))
        }
        BoundedStatus::TimedOut => tool_error(format!(
            "`girder {}` exceeded its {}s budget and was terminated. Large repository, or narrow the request.",
            argv.join(" "),
            tool_timeout().as_secs()
        )),
        BoundedStatus::OutputLimited => tool_error(format!(
            "`girder {}` produced more than {} bytes and was terminated rather than truncated. Narrow the request.",
            argv.join(" "),
            MAX_TOOL_OUTPUT_BYTES
        )),
        BoundedStatus::Cancelled => {
            tool_error(format!("`girder {}` was cancelled", argv.join(" ")))
        }
    }
}

/// Per-call timeout, overridable for very large repositories. An unparseable
/// or zero value falls back to the default rather than disabling the bound.
fn tool_timeout() -> Duration {
    let seconds = std::env::var("GIRDER_MCP_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .unwrap_or(DEFAULT_TOOL_TIMEOUT_SECONDS);
    Duration::from_secs(seconds)
}

fn tool_text(text: &str) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": false})
}

fn tool_error(message: impl Into<String>) -> Value {
    json!({
        "content": [{"type": "text", "text": message.into()}],
        "isError": true,
    })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message},
    })
}

/// Refuses a model-supplied value that a child command would read as an
/// option instead of as data.
///
/// Every tool appends its arguments to a fixed subcommand, and those
/// subcommands parse by scanning the whole of argv for known flags rather
/// than by stopping at a `--` end-of-options marker. A value beginning with
/// `-` is therefore not inert. Two reached past the read-only boundary this
/// server advertises with `readOnlyHint`:
///   * `impacted_tests` with `nodes: ["--run"]` became
///     `test-impact --run`, which executes the project's configured test
///     commands — arbitrary code execution.
///   * `nodes: ["--out", "/path"]` became `test-impact --out /path`, which
///     writes its listing there, truncating any existing file at any
///     absolute path. `review_changes` with `since: "--out"` did the same
///     with the following argument.
///
/// There is no marker to pass these scanners, so refusing the value is the
/// fail-closed equivalent. Nothing legitimate is lost: node paths,
/// identifiers, and git refs cannot begin with `-`. A natural-language
/// field could, and gets a message that names the fix.
fn reject_option_like(key: &str, value: &str) -> Result<(), String> {
    if value.starts_with('-') {
        return Err(format!(
            "`{key}` must not start with '-' (got {value:?}). It is passed to a command that \
             would read it as an option rather than as data; rephrase it without the leading dash."
        ));
    }
    Ok(())
}

fn required_string<'a>(arguments: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    match arguments.get(key) {
        Some(Value::String(value)) => {
            reject_option_like(key, value)?;
            Ok(value)
        }
        Some(_) => Err(format!("`{key}` must be a string")),
        None => Err(format!("`{key}` is required")),
    }
}

fn optional_string<'a>(
    arguments: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a str>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            reject_option_like(key, value)?;
            Ok(Some(value))
        }
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

fn optional_bool(arguments: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!("`{key}` must be a boolean")),
    }
}

/// A non-negative integer argument. Unlike the string accessors above, this
/// never needs [`reject_option_like`]: a JSON number can't carry a leading
/// `-` in its argv rendering the way a string can, since it is only ever
/// formatted from a validated `u32`.
fn optional_u32(arguments: &Map<String, Value>, key: &str) -> Result<Option<u32>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| format!("`{key}` must be a non-negative integer")),
        Some(_) => Err(format!("`{key}` must be a non-negative integer")),
    }
}

fn optional_string_array(
    arguments: &Map<String, Value>,
    key: &str,
) -> Result<Option<Vec<String>>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::String(value) => {
                    reject_option_like(key, value)?;
                    Ok(value.clone())
                }
                _ => Err(format!("`{key}` must contain only strings")),
            })
            .collect::<Result<Vec<String>, String>>()
            .map(Some),
        Some(_) => Err(format!("`{key}` must be an array of strings")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/tmp/project")
    }

    fn executable() -> PathBuf {
        PathBuf::from("/usr/bin/girder")
    }

    fn request(method: &str, params: Value) -> String {
        json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string()
    }

    fn handle(line: &str) -> Option<Value> {
        handle_line(line, &root(), &executable())
    }

    fn argv_for(name: &str, arguments: Value) -> Result<Vec<String>, String> {
        let tool = TOOLS.iter().find(|tool| tool.name == name).unwrap();
        (tool.argv)(arguments.as_object().unwrap(), "/tmp/project")
    }

    #[test]
    fn initialize_echoes_a_recognized_protocol_version() {
        for version in SUPPORTED_PROTOCOL_VERSIONS {
            let line = request("initialize", json!({"protocolVersion": version}));
            let response = handle(&line).unwrap();
            assert_eq!(
                response["result"]["protocolVersion"], *version,
                "should have echoed {version}"
            );
        }
    }

    #[test]
    fn initialize_falls_back_for_an_unknown_protocol_version() {
        let line = request("initialize", json!({"protocolVersion": "1999-01-01"}));
        let response = handle(&line).unwrap();
        assert_eq!(
            response["result"]["protocolVersion"],
            FALLBACK_PROTOCOL_VERSION
        );
    }

    #[test]
    fn initialize_advertises_tools_and_identifies_the_server() {
        let response = handle(&request("initialize", json!({}))).unwrap();
        assert!(response["result"]["capabilities"]["tools"].is_object());
        assert_eq!(response["result"]["serverInfo"]["name"], SERVER_NAME);
        assert_eq!(
            response["result"]["serverInfo"]["version"],
            env!("CARGO_PKG_VERSION")
        );
    }

    #[test]
    fn discover_advertises_every_supported_version() {
        let response = handle(&request("server/discover", json!({}))).unwrap();
        let result = &response["result"];
        assert_eq!(result["resultType"], "complete");
        let advertised: Vec<&str> = result["supportedVersions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect();
        assert_eq!(advertised, SUPPORTED_PROTOCOL_VERSIONS);
        assert_eq!(
            result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            SERVER_NAME
        );
    }

    /// Per-request `_meta` is the stateless era's carrier for protocol
    /// version and client capabilities. This server ignores it, so a request
    /// carrying it must behave exactly like one that does not.
    #[test]
    fn per_request_meta_is_accepted_and_ignored() {
        let with_meta = request(
            "tools/list",
            json!({"_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": {"elicitation": {}},
            }}),
        );
        let bare = request("tools/list", json!({}));
        assert_eq!(handle(&with_meta).unwrap(), handle(&bare).unwrap());
    }

    #[test]
    fn tools_list_describes_every_tool_as_read_only_with_a_schema() {
        let response = handle(&request("tools/list", json!({}))).unwrap();
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), TOOLS.len());
        for tool in tools {
            let name = tool["name"].as_str().unwrap();
            let description = tool["description"].as_str().unwrap();
            assert!(!description.is_empty(), "{name} needs a description");
            assert_eq!(
                description.starts_with("Paid license required."),
                matches!(name, "orient" | "impacted_tests"),
                "only paid tools should advertise the license requirement: {name}"
            );
            assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
            assert_eq!(
                tool["annotations"]["readOnlyHint"], true,
                "{name} must be advertised read-only"
            );
            assert_eq!(tool["annotations"]["destructiveHint"], false, "{name}");
        }
    }

    /// Every tool schema forbids extra properties, so a model that invents
    /// an argument gets told instead of silently having it dropped.
    #[test]
    fn every_tool_schema_rejects_unknown_arguments() {
        for tool in TOOLS {
            let schema = (tool.schema)();
            assert_eq!(
                schema["additionalProperties"], false,
                "{} must reject unknown arguments",
                tool.name
            );
        }
    }

    #[test]
    fn ping_answers_empty() {
        let response = handle(&request("ping", json!({}))).unwrap();
        assert_eq!(response["result"], json!({}));
    }

    #[test]
    fn a_notification_is_never_answered() {
        let line = json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string();
        assert!(handle(&line).is_none());
    }

    #[test]
    fn an_unknown_method_is_a_method_not_found_error() {
        let response = handle(&request("resources/list", json!({}))).unwrap();
        assert_eq!(response["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn malformed_json_is_a_parse_error_with_a_null_id() {
        let response = handle("{not json").unwrap();
        assert_eq!(response["error"]["code"], PARSE_ERROR);
        assert_eq!(response["id"], Value::Null);
    }

    /// MCP removed JSON-RPC batching in 2025-06-18; a batch must be refused
    /// rather than half-processed.
    #[test]
    fn a_json_rpc_batch_is_refused() {
        let response = handle("[{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}]").unwrap();
        assert_eq!(response["error"]["code"], INVALID_REQUEST);
    }

    #[test]
    fn an_unknown_tool_name_is_an_invalid_params_error() {
        let response = handle(&request("tools/call", json!({"name": "rm_rf"}))).unwrap();
        assert_eq!(response["error"]["code"], INVALID_PARAMS);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("rm_rf"));
    }

    #[test]
    fn get_source_pins_exact_nodes_as_a_comma_separated_list() {
        let argv = argv_for(
            "get_source",
            json!({"nodes": ["crate::a::b", "crate::c::d"]}),
        )
        .unwrap();
        assert_eq!(
            argv,
            vec![
                "context",
                "/tmp/project",
                "--json",
                "--source-only",
                "--nodes",
                "crate::a::b,crate::c::d"
            ]
        );
    }

    /// The whole point of this tool: it must never emit the plan-authoring
    /// envelope that measured more expensive than reading the file.
    #[test]
    fn get_source_always_requests_the_lean_shape() {
        for arguments in [
            json!({"nodes": ["crate::a::b"]}),
            json!({"intent": "parse a config file"}),
            json!({"nodes": ["crate::a::b"], "include_tests": true}),
        ] {
            let argv = argv_for("get_source", arguments.clone()).unwrap();
            assert!(
                argv.contains(&"--source-only".to_string()),
                "{arguments} produced {argv:?}"
            );
        }
    }

    #[test]
    fn get_source_falls_back_to_intent_when_no_nodes_are_pinned() {
        let argv = argv_for("get_source", json!({"intent": "parse a config file"})).unwrap();
        assert_eq!(argv.last().unwrap(), "parse a config file");
        assert!(!argv.contains(&"--nodes".to_string()));
    }

    #[test]
    fn get_source_treats_an_empty_node_list_as_no_selection() {
        let error = argv_for("get_source", json!({"nodes": []})).unwrap_err();
        assert!(error.contains("nodes"), "{error}");
    }

    #[test]
    fn get_source_requires_some_selection() {
        let error = argv_for("get_source", json!({})).unwrap_err();
        assert!(error.contains("intent"), "{error}");
    }

    #[test]
    fn get_source_adds_covering_tests_only_when_asked() {
        let without = argv_for("get_source", json!({"nodes": ["crate::a::b"]})).unwrap();
        assert!(!without.contains(&"--with-tests".to_string()));
        let with = argv_for(
            "get_source",
            json!({"nodes": ["crate::a::b"], "include_tests": true}),
        )
        .unwrap();
        assert!(with.contains(&"--with-tests".to_string()));
    }

    #[test]
    fn find_definition_requires_a_name_and_passes_the_kind_filter() {
        let error = argv_for("find_definition", json!({})).unwrap_err();
        assert!(error.contains("name"), "{error}");

        let argv = argv_for("find_definition", json!({"name": "NodeId", "kind": "type"})).unwrap();
        assert_eq!(
            argv,
            vec![
                "names",
                "/tmp/project",
                "NodeId",
                "--json",
                "--kind",
                "type"
            ]
        );
    }

    #[test]
    fn find_definition_always_requests_json() {
        let argv = argv_for("find_definition", json!({"name": "NodeId"})).unwrap();
        assert!(argv.contains(&"--json".to_string()));
    }

    #[test]
    fn a_wrongly_typed_argument_is_rejected_rather_than_coerced() {
        let error = argv_for("find_definition", json!({"name": 42})).unwrap_err();
        assert!(error.contains("must be a string"), "{error}");
        let error = argv_for("get_source", json!({"nodes": [42]})).unwrap_err();
        assert!(error.contains("only strings"), "{error}");
        let error = argv_for(
            "get_source",
            json!({"nodes": ["crate::a::b"], "include_tests": "yes"}),
        )
        .unwrap_err();
        assert!(error.contains("must be a boolean"), "{error}");
    }

    #[test]
    fn ask_codebase_refuses_a_blank_question_that_would_open_a_repl() {
        for question in ["", "   "] {
            let error = argv_for("ask_codebase", json!({"question": question})).unwrap_err();
            assert!(error.contains("must not be empty"), "{error}");
        }
    }

    #[test]
    fn ask_codebase_passes_the_question_as_one_argument() {
        let argv = argv_for("ask_codebase", json!({"question": "what calls tests_for?"})).unwrap();
        assert_eq!(argv, vec!["query", "/tmp/project", "what calls tests_for?"]);
    }

    #[test]
    fn impacted_tests_defaults_to_the_git_diff_and_stays_quiet() {
        let argv = argv_for("impacted_tests", json!({})).unwrap();
        assert_eq!(argv, vec!["test-impact", "/tmp/project", "--quiet"]);
    }

    #[test]
    fn impacted_tests_appends_explicit_nodes() {
        let argv = argv_for("impacted_tests", json!({"nodes": ["crate::a::b"]})).unwrap();
        assert_eq!(argv.last().unwrap(), "crate::a::b");
    }

    /// `--run` is deliberately unreachable: this server is read-only, and
    /// running a project's test suite executes its code.
    #[test]
    fn impacted_tests_never_executes_the_tests() {
        let argv = argv_for("impacted_tests", json!({"nodes": ["crate::a::b"]})).unwrap();
        assert!(!argv.contains(&"--run".to_string()));
    }

    /// The node list is appended to `test-impact`'s argv, and `test-impact`
    /// honours `--run` and `--out` wherever they appear in it. Before this
    /// was refused, `nodes: ["--run"]` executed the project's configured
    /// test commands and `nodes: ["--out", path]` wrote that path — through
    /// a tool advertising `readOnlyHint`.
    #[test]
    fn impacted_tests_refuses_a_node_list_that_smuggles_an_option() {
        for nodes in [
            json!(["--run"]),
            json!(["crate::a::b", "--run"]),
            json!(["--out", "/tmp/written-by-a-read-only-server"]),
        ] {
            let error = argv_for("impacted_tests", json!({"nodes": nodes})).unwrap_err();
            assert!(
                error.contains("must not start with '-'"),
                "{nodes} was accepted: {error}"
            );
        }
    }

    /// `review` finds `--out` by scanning pairs, so a `since` of `--out`
    /// made the *next* argument the path it wrote to.
    #[test]
    fn review_changes_refuses_a_since_ref_that_smuggles_an_option() {
        let error = argv_for("review_changes", json!({"since": "--out"})).unwrap_err();
        assert!(error.contains("must not start with '-'"), "{error}");
    }

    /// The invariant, applied to every argument of every tool rather than
    /// only the two that were exploitable: an argument is data, never an
    /// option. `search_code` and `ask_codebase` pass their text to
    /// commands that happen to parse no flags today, so they are covered
    /// here to keep a flag added to either later from reopening this.
    #[test]
    fn orient_pins_exact_nodes_as_a_comma_separated_list() {
        let argv = argv_for("orient", json!({"nodes": ["crate::a::b", "crate::c::d"]})).unwrap();
        assert_eq!(
            argv,
            vec![
                "orient",
                "/tmp/project",
                "--json",
                "--nodes",
                "crate::a::b,crate::c::d"
            ]
        );
    }

    #[test]
    fn orient_falls_back_to_intent_when_no_nodes_are_pinned() {
        let argv = argv_for("orient", json!({"intent": "parse a config file"})).unwrap();
        assert_eq!(
            argv,
            vec!["orient", "/tmp/project", "--json", "parse a config file"]
        );
    }

    #[test]
    fn orient_requires_some_selection() {
        let error = argv_for("orient", json!({})).unwrap_err();
        assert!(
            error.contains("nodes") && error.contains("intent"),
            "{error}"
        );
    }

    #[test]
    fn orient_passes_depth_when_provided() {
        let argv = argv_for("orient", json!({"nodes": ["crate::a::b"], "depth": 2})).unwrap();
        assert_eq!(
            argv,
            vec![
                "orient",
                "/tmp/project",
                "--json",
                "--nodes",
                "crate::a::b",
                "--depth",
                "2"
            ]
        );
    }

    #[test]
    fn orient_rejects_a_negative_depth() {
        let error = argv_for("orient", json!({"nodes": ["crate::a::b"], "depth": -1})).unwrap_err();
        assert!(error.contains("non-negative"), "{error}");
    }

    #[test]
    fn every_tool_refuses_option_like_arguments() {
        let cases = [
            ("get_source", json!({"nodes": ["--out"]})),
            ("get_source", json!({"intent": "--with-tests"})),
            ("find_definition", json!({"name": "--json"})),
            (
                "find_definition",
                json!({"name": "NodeId", "kind": "--out"}),
            ),
            ("search_code", json!({"query": "--json"})),
            ("ask_codebase", json!({"question": "--json"})),
            ("impacted_tests", json!({"nodes": ["--run"]})),
            ("review_changes", json!({"since": "--out"})),
            ("orient", json!({"nodes": ["--out"]})),
        ];
        let covered: std::collections::BTreeSet<&str> =
            cases.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            covered.len(),
            TOOLS.len(),
            "every tool needs a case here; missing: {:?}",
            TOOLS
                .iter()
                .map(|tool| tool.name)
                .filter(|name| !covered.contains(name))
                .collect::<Vec<_>>()
        );

        for (name, arguments) in cases {
            let error = argv_for(name, arguments.clone()).unwrap_err();
            assert!(
                error.contains("must not start with '-'"),
                "{name} accepted {arguments}: {error}"
            );
        }
    }

    /// The guard must not reject the values these tools exist to carry: a
    /// node path with `::`, a hyphenated crate segment, a git ref, and
    /// prose containing a dash all have to keep working.
    #[test]
    fn ordinary_arguments_with_internal_dashes_still_pass() {
        argv_for(
            "get_source",
            json!({"nodes": ["crate::crates::aether-graph::src::lib::tests_for"]}),
        )
        .unwrap();
        argv_for("review_changes", json!({"since": "HEAD~1"})).unwrap();
        argv_for("review_changes", json!({"since": "main-harden"})).unwrap();
        argv_for(
            "ask_codebase",
            json!({"question": "what calls resolve_calls -- the project-wide one?"}),
        )
        .unwrap();
    }

    #[test]
    fn review_changes_is_quiet_by_default_and_full_on_request() {
        let quiet = argv_for("review_changes", json!({})).unwrap();
        assert_eq!(quiet, vec!["review", "/tmp/project", "--quiet"]);

        let full = argv_for("review_changes", json!({"full": true})).unwrap();
        assert_eq!(full, vec!["review", "/tmp/project"]);
    }

    /// `review --out` cannot be combined with `--quiet`, so full mode must
    /// not carry `--quiet` when `--since` is also present.
    #[test]
    fn review_changes_passes_since_in_both_modes() {
        let quiet = argv_for("review_changes", json!({"since": "main"})).unwrap();
        assert_eq!(
            quiet,
            vec!["review", "/tmp/project", "--since", "main", "--quiet"]
        );
        let full = argv_for("review_changes", json!({"since": "main", "full": true})).unwrap();
        assert_eq!(full, vec!["review", "/tmp/project", "--since", "main"]);
    }

    /// Every tool must target the root the server was started with, so no
    /// argument can redirect a call at another directory.
    #[test]
    fn every_tool_targets_the_pinned_root() {
        let arguments = [
            ("get_source", json!({"nodes": ["crate::a::b"]})),
            ("find_definition", json!({"name": "NodeId"})),
            ("search_code", json!({"query": "parse config"})),
            ("ask_codebase", json!({"question": "what calls x?"})),
            ("impacted_tests", json!({})),
            ("review_changes", json!({})),
            ("orient", json!({"nodes": ["crate::a::b"]})),
        ];
        assert_eq!(arguments.len(), TOOLS.len(), "every tool needs a case here");
        for (name, argument) in arguments {
            let argv = argv_for(name, argument).unwrap();
            assert_eq!(argv[1], "/tmp/project", "{name} lost the pinned root");
        }
    }

    #[test]
    fn a_failed_command_is_reported_as_tool_content_not_a_protocol_error() {
        // `false` exits non-zero with no output, standing in for a command
        // that fails: the model must get isError content it can react to.
        let result = run_tool(Path::new("/usr/bin/false"), &[]);
        assert_eq!(result["isError"], true);
        assert!(!result["content"][0]["text"].as_str().unwrap().is_empty());
    }

    #[test]
    fn a_missing_executable_is_reported_as_tool_content() {
        let result = run_tool(Path::new("/nonexistent/girder"), &[]);
        assert_eq!(result["isError"], true);
        assert!(result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("could not run"));
    }

    #[test]
    fn empty_output_is_reported_as_no_results_rather_than_an_empty_string() {
        let result = run_tool(Path::new("/usr/bin/true"), &[]);
        assert_eq!(result["isError"], false);
        assert_eq!(result["content"][0]["text"], "(no results)");
    }

    #[test]
    fn the_tool_timeout_is_overridable_but_never_unbounded() {
        let restore = std::env::var("GIRDER_MCP_TIMEOUT_SECONDS").ok();
        // SAFETY: single-threaded test process for this variable; restored below.
        std::env::set_var("GIRDER_MCP_TIMEOUT_SECONDS", "7");
        assert_eq!(tool_timeout(), Duration::from_secs(7));
        for bogus in ["0", "-1", "not a number", ""] {
            std::env::set_var("GIRDER_MCP_TIMEOUT_SECONDS", bogus);
            assert_eq!(
                tool_timeout(),
                Duration::from_secs(DEFAULT_TOOL_TIMEOUT_SECONDS),
                "{bogus:?} must fall back to the default, not disable the bound"
            );
        }
        match restore {
            Some(value) => std::env::set_var("GIRDER_MCP_TIMEOUT_SECONDS", value),
            None => std::env::remove_var("GIRDER_MCP_TIMEOUT_SECONDS"),
        }
    }
}
