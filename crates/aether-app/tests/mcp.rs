//! End-to-end `girder mcp` sessions over the real stdio transport.
//!
//! The unit tests in `mcp.rs` cover framing and argv construction in
//! isolation. These drive the actual binary the way an MCP client does —
//! spawn it, write newline-delimited JSON-RPC to its stdin, read frames back
//! from its stdout — because the failure modes that matter most here are
//! exactly the ones a unit test cannot see: a log line landing on stdout, an
//! unflushed reply, or a tool call that hangs.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// A live MCP server subprocess plus its stdio pipes.
struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: i64,
}

impl Session {
    fn start(root: &Path) -> Self {
        Self::start_with_license(root, true)
    }

    fn start_unlicensed(root: &Path) -> Self {
        Self::start_with_license(root, false)
    }

    fn start_with_license(root: &Path, licensed: bool) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_girder"));
        command
            .arg("mcp")
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if licensed {
            command.env("GIRDER_LICENSE_KEY", TEST_LICENSE_KEY);
        } else {
            command
                .env_remove("GIRDER_LICENSE_KEY")
                .env("XDG_CONFIG_HOME", root)
                .env("APPDATA", root)
                .env("HOME", root);
        }
        let mut child = command.spawn().expect("spawn girder mcp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next_id: 0,
        }
    }

    /// Sends a request and reads exactly one response frame.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();

        let mut line = String::new();
        let read = self.stdout.read_line(&mut line).unwrap();
        assert!(read > 0, "server closed stdout while answering {method}");
        let response: Value = serde_json::from_str(&line).unwrap_or_else(|error| {
            panic!("{method} sent a frame that is not JSON ({error}): {line:?}")
        });
        assert_eq!(
            response["jsonrpc"], "2.0",
            "{method} response is not JSON-RPC 2.0: {line}"
        );
        assert_eq!(
            response["id"], id,
            "{method} response id does not match the request: {line}"
        );
        response
    }

    fn notify(&mut self, method: &str) {
        let message = json!({"jsonrpc": "2.0", "method": method});
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn initialize(&mut self) -> Value {
        let response = self.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "girder-integration-test", "version": "1.0"},
            }),
        );
        self.notify("notifications/initialized");
        response
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
    }

    /// Closes stdin and returns the exit status plus everything on stderr.
    fn finish(self) -> (bool, String) {
        drop(self.stdin);
        let output = self.child.wait_with_output().unwrap();
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}

#[test]
fn unlicensed_paid_tool_is_a_readable_normal_tool_result() {
    let root = fixture("license-gate");
    let mut session = Session::start_unlicensed(&root);
    session.initialize();

    let response = session.call_tool("orient", json!({"nodes": ["crate::sample::answer"]}));
    assert_eq!(response["result"]["isError"], true, "{response}");
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("license failure must be readable text");
    assert!(
        text.contains("`orient` tool needs a paid Girder license"),
        "{text}"
    );
    assert!(
        text.contains("`get_source` and `find_definition`"),
        "{text}"
    );
    assert_eq!(session.request("ping", json!({}))["result"], json!({}));

    let (clean_exit, stderr) = session.finish();
    assert!(clean_exit, "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

/// A git repository with two functions and a test, so the graph has real
/// nodes, a real call edge, and a real covering test to find.
fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "girder-mcp-{name}-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("calc.rs"),
        "pub fn double(value: i32) -> i32 {\n    value * 2\n}\n\n\
         pub fn quadruple(value: i32) -> i32 {\n    double(double(value))\n}\n\n\
         #[test]\nfn test_double() {\n    assert_eq!(double(2), 4);\n}\n",
    )
    .unwrap();

    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    };
    git(&["init", "--quiet"]);
    git(&["config", "user.email", "girder@example.invalid"]);
    git(&["config", "user.name", "Girder Test"]);
    git(&["add", "."]);
    git(&["commit", "--quiet", "-m", "base"]);
    root
}

fn tool_text(response: &Value) -> String {
    assert_eq!(
        response["result"]["isError"], false,
        "tool reported an error: {response}"
    );
    response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result must carry text content")
        .to_string()
}

#[test]
fn a_full_session_handshakes_lists_tools_and_answers_calls_in_order() {
    let root = fixture("full-session");
    let mut session = Session::start(&root);

    let initialized = session.initialize();
    assert_eq!(initialized["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(initialized["result"]["serverInfo"]["name"], "girder");
    assert!(initialized["result"]["capabilities"]["tools"].is_object());

    let listed = session.request("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    let mut names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "ask_codebase",
            "find_definition",
            "get_source",
            "impacted_tests",
            "orient",
            "review_changes",
            "search_code"
        ]
    );

    // Several calls on one connection: each reply must be flushed and
    // correlated, not buffered until exit or answered out of order.
    let found = tool_text(&session.call_tool("find_definition", json!({"name": "double"})));
    let declarations: Value = serde_json::from_str(&found).unwrap();
    let node = declarations[0]["path"].as_str().unwrap().to_string();
    assert!(node.ends_with("double"), "{found}");

    let source = tool_text(&session.call_tool("get_source", json!({"nodes": [node]})));
    let payload: Value = serde_json::from_str(&source).unwrap();
    assert!(
        payload["nodes"][0]["source"]
            .as_str()
            .unwrap()
            .contains("fn double"),
        "{source}"
    );

    let searched = tool_text(&session.call_tool("search_code", json!({"query": "double a value"})));
    assert!(searched.contains("double"), "{searched}");

    let (clean_exit, stderr) = session.finish();
    assert!(clean_exit, "server exited non-zero; stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

/// The regression this pins: `get_source` must emit the lean shape. If it
/// ever grows the plan-authoring envelope back, the byte saving in
/// `docs/context-vs-read-cost.md` silently disappears for every agent.
#[test]
fn get_source_returns_no_authoring_envelope_and_beats_reading_the_file() {
    let root = fixture("lean-shape");
    let mut session = Session::start(&root);
    session.initialize();

    let found = tool_text(&session.call_tool("find_definition", json!({"name": "double"})));
    let declarations: Value = serde_json::from_str(&found).unwrap();
    let node = declarations[0]["path"].as_str().unwrap().to_string();

    let source = tool_text(&session.call_tool("get_source", json!({"nodes": [node]})));
    let payload: Value = serde_json::from_str(&source).unwrap();
    let mut keys: Vec<&str> = payload
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["intent", "nodes"],
        "get_source must not carry a schema or plan skeleton: {source}"
    );

    let file_bytes = std::fs::metadata(root.join("calc.rs")).unwrap().len() as usize;
    assert!(
        source.len() < file_bytes * 4,
        "one 34-byte function cost {} bytes against a {file_bytes}-byte file, which suggests the \
         envelope is back",
        source.len()
    );

    let (clean_exit, stderr) = session.finish();
    assert!(clean_exit, "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn covering_tests_are_included_only_when_requested() {
    let root = fixture("with-tests");
    let mut session = Session::start(&root);
    session.initialize();

    let without = tool_text(&session.call_tool("get_source", json!({"intent": "double a value"})));
    assert!(
        !without.contains("test_double"),
        "covering tests must not appear by default: {without}"
    );

    let with = tool_text(&session.call_tool(
        "get_source",
        json!({"intent": "double a value", "include_tests": true}),
    ));
    assert!(
        with.contains("test_double"),
        "include_tests must attach the covering test: {with}"
    );

    let (clean_exit, stderr) = session.finish();
    assert!(clean_exit, "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

/// Nothing but JSON-RPC frames may reach stdout. The startup banner is the
/// obvious offender, so this asserts it lands on stderr instead.
#[test]
fn diagnostics_go_to_stderr_and_never_into_the_frame_stream() {
    let root = fixture("stream-hygiene");
    let mut session = Session::start(&root);
    session.initialize();
    session.request("tools/list", json!({}));
    let (clean_exit, stderr) = session.finish();

    assert!(clean_exit);
    assert!(
        stderr.contains("MCP server on stdio"),
        "the startup banner belongs on stderr: {stderr:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A tool that fails must come back as `isError` content the model can read
/// and correct, while a protocol-level mistake must come back as a JSON-RPC
/// error. Conflating the two leaves an agent unable to tell "your argument
/// was wrong" from "the repository has no such node".
#[test]
fn protocol_errors_and_tool_failures_are_reported_differently() {
    let root = fixture("errors");
    let mut session = Session::start(&root);
    session.initialize();

    let unknown_tool = session.call_tool("definitely_not_a_tool", json!({}));
    assert_eq!(unknown_tool["error"]["code"], -32602, "{unknown_tool}");

    let no_selection = session.call_tool("get_source", json!({}));
    assert_eq!(no_selection["error"]["code"], -32602, "{no_selection}");

    let unknown_method = session.request("prompts/list", json!({}));
    assert_eq!(unknown_method["error"]["code"], -32601, "{unknown_method}");

    // A node that does not exist is a real command failure, not bad params.
    let missing_node = session.call_tool("get_source", json!({"nodes": ["crate::nope::gone"]}));
    assert_eq!(missing_node["result"]["isError"], true, "{missing_node}");
    assert!(
        missing_node["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("crate::nope::gone"),
        "the error should name the path the model got wrong: {missing_node}"
    );

    // The session must still be usable after all of that.
    assert_eq!(session.request("ping", json!({}))["result"], json!({}));

    let (clean_exit, stderr) = session.finish();
    assert!(clean_exit, "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

/// `impacted_tests` and `review_changes` legitimately return nothing on a
/// clean tree. A model handed an empty string tends to conclude the tool is
/// broken and fall back to reading files, so the emptiness must be explicit.
#[test]
fn commands_with_no_results_say_so_explicitly() {
    let root = fixture("no-results");
    let mut session = Session::start(&root);
    session.initialize();

    let reviewed = tool_text(&session.call_tool("review_changes", json!({})));
    assert_eq!(reviewed, "(no results)", "clean tree should review empty");

    let (clean_exit, stderr) = session.finish();
    assert!(clean_exit, "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

/// The stateless-era probe: a modern client sends `server/discover` before
/// anything else and must get a `DiscoverResult` rather than an error that
/// would send it down the legacy fallback path.
#[test]
fn a_modern_client_can_discover_without_the_legacy_handshake() {
    let root = fixture("discover-first");
    let mut session = Session::start(&root);

    let discovered = session.request(
        "server/discover",
        json!({"_meta": {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {},
        }}),
    );
    assert_eq!(discovered["result"]["resultType"], "complete");
    let versions = discovered["result"]["supportedVersions"]
        .as_array()
        .unwrap();
    assert!(
        versions.iter().any(|version| version == "2026-07-28"),
        "{discovered}"
    );
    assert_eq!(
        discovered["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "girder"
    );

    // And tools must be callable with no `initialize` at all.
    let listed = session.request("tools/list", json!({}));
    assert!(!listed["result"]["tools"].as_array().unwrap().is_empty());

    let (clean_exit, stderr) = session.finish();
    assert!(clean_exit, "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

/// The read-only guarantee, end to end. `impacted_tests` appends its node
/// list to `test-impact`'s argv, and `test-impact` honours `--out` and
/// `--run` wherever they appear there — so before those were refused, a
/// tool annotated `readOnlyHint` would truncate any absolute path handed to
/// it and execute the project's configured test commands. Asserted against
/// the real binary because the damage is a real filesystem write.
#[test]
fn a_tool_argument_cannot_smuggle_an_option_that_writes_or_runs() {
    let root = fixture("no-option-smuggling");
    let victim = std::env::temp_dir().join(format!(
        "girder-mcp-must-not-write-{}-{}.txt",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&victim, "PRE-EXISTING CONTENT").unwrap();

    let mut session = Session::start(&root);
    session.initialize();

    let wrote = session.call_tool(
        "impacted_tests",
        json!({"nodes": ["--out", victim.to_str().unwrap()]}),
    );
    assert_eq!(
        wrote["error"]["code"], -32602,
        "an option-like node must be refused as bad params: {wrote}"
    );
    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        "PRE-EXISTING CONTENT",
        "a read-only server overwrote {}",
        victim.display()
    );

    let ran = session.call_tool("impacted_tests", json!({"nodes": ["--run"]}));
    assert_eq!(ran["error"]["code"], -32602, "{ran}");

    // A legitimate call on the same session must still work, so the guard
    // is a rejection of these arguments and not of the tool.
    let listed = session.call_tool("impacted_tests", json!({}));
    assert_eq!(listed["result"]["isError"], false, "{listed}");

    let (clean_exit, stderr) = session.finish();
    assert!(clean_exit, "stderr: {stderr}");
    let _ = std::fs::remove_file(&victim);
    let _ = std::fs::remove_dir_all(&root);
}

/// A server that cannot resolve its root must fail at startup rather than
/// accept a session and error on every call.
#[test]
fn a_nonexistent_root_fails_at_startup() {
    let output = Command::new(env!("CARGO_BIN_EXE_girder"))
        .arg("mcp")
        .arg("/nonexistent/definitely/not/here")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot serve MCP"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
const TEST_LICENSE_KEY: &str = "girder-v1.2026-09-06.paid.c3a189213567f3aced881143c0d600df36c162252ff026ee6a6377a85959215b90ca7ea51e2eb474d3e8ca4e60b09648994a1e6513772393dcde1b4e7752bd02";
