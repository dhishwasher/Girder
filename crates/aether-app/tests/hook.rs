//! Process-level coverage for the fail-open structured read advisory.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "girder-hook-{name}-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir_all(path.join("src")).unwrap();
        std::fs::write(path.join("src/lib.rs"), "pub fn answer() -> i32 { 42 }\n").unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn source(&self) -> PathBuf {
        self.0.join("src/lib.rs")
    }

    fn warm(&self) {
        std::fs::write(self.0.join("project.aether"), "saved graph marker\n").unwrap();
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_with_input(mut command: Command, input: &[u8]) -> Output {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn girder");
    child
        .stdin
        .take()
        .expect("girder stdin")
        .write_all(input)
        .expect("write girder stdin");
    child.wait_with_output().expect("wait for girder")
}

fn run_hook(root: &Path, payload: &[u8]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_girder"));
    command.arg("hook").current_dir(root);
    run_with_input(command, payload)
}

fn read_payload(root: &Path) -> Value {
    json!({
        "tool_name": "Read",
        "tool_input": {"file_path": root.join("src/lib.rs")},
        "cwd": root,
    })
}

fn expected_hook_output() -> Value {
    json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "additionalContext": "Girder: consider `girder context . --nodes <node::path> --json --source-only` before this whole-file read.",
    }})
}

#[test]
fn malformed_oversized_missing_cold_bounded_and_outside_reads_fail_open_silently() {
    let root = TempRoot::new("fail-open");
    let outside = TempRoot::new("outside");
    root.warm();
    let mut oversized = vec![b'x'; 64 * 1024 + 1];
    oversized[0] = b'{';
    let cases = vec![
        b"not json".to_vec(),
        oversized,
        serde_json::to_vec(&json!({"tool_name": "Read"})).unwrap(),
        serde_json::to_vec(&json!({
            "tool_name": "Read",
            "tool_input": {"file_path": root.source(), "offset": 1},
            "cwd": root.path(),
        }))
        .unwrap(),
        serde_json::to_vec(&json!({
            "tool_name": "Read",
            "tool_input": {"file_path": outside.source()},
            "cwd": root.path(),
        }))
        .unwrap(),
    ];

    for payload in cases {
        let output = run_hook(root.path(), &payload);
        assert!(output.status.success());
        assert!(output.stdout.is_empty(), "stdout: {:?}", output.stdout);
        assert!(output.stderr.is_empty(), "stderr: {:?}", output.stderr);
    }

    std::fs::remove_file(root.path().join("project.aether")).unwrap();
    let cold = run_hook(
        root.path(),
        &serde_json::to_vec(&read_payload(root.path())).unwrap(),
    );
    assert!(cold.status.success());
    assert!(cold.stdout.is_empty());
    assert!(cold.stderr.is_empty());
}

#[test]
fn standalone_hook_advice_never_changes_nonempty_mcp_stdout() {
    let disabled = TempRoot::new("mcp-disabled");
    let enabled = TempRoot::new("mcp-enabled");
    enabled.warm();
    let disabled_hook = run_hook(
        disabled.path(),
        &serde_json::to_vec(&read_payload(disabled.path())).unwrap(),
    );
    let enabled_hook = run_hook(
        enabled.path(),
        &serde_json::to_vec(&read_payload(enabled.path())).unwrap(),
    );
    // Either invocation may time out before finishing its metadata work. The
    // hook's contract is fail-open, so this test asserts only the process
    // boundary before proving the real MCP response remains byte-identical.
    assert!(disabled_hook.status.success());
    assert!(enabled_hook.status.success());
    assert!(disabled_hook.stdout.is_empty());
    assert!(disabled_hook.stderr.is_empty());
    assert!(enabled_hook.stderr.is_empty());
    if !enabled_hook.stdout.is_empty() {
        assert_eq!(
            serde_json::from_slice::<Value>(&enabled_hook.stdout).unwrap(),
            expected_hook_output()
        );
    }

    let request = b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\",\"params\":{}}\n";
    let mut disabled_command = Command::new(env!("CARGO_BIN_EXE_girder"));
    disabled_command.arg("mcp").arg(disabled.path());
    let mut enabled_command = Command::new(env!("CARGO_BIN_EXE_girder"));
    enabled_command.arg("mcp").arg(enabled.path());
    let disabled_mcp = run_with_input(disabled_command, request);
    let enabled_mcp = run_with_input(enabled_command, request);

    assert!(
        disabled_mcp.status.success(),
        "stderr: {:?}",
        disabled_mcp.stderr
    );
    assert!(
        enabled_mcp.status.success(),
        "stderr: {:?}",
        enabled_mcp.stderr
    );
    assert!(
        !disabled_mcp.stdout.is_empty(),
        "the comparison needs a real MCP frame"
    );
    assert_eq!(disabled_mcp.stdout, enabled_mcp.stdout);
    assert_eq!(
        serde_json::from_slice::<Value>(&disabled_mcp.stdout).unwrap(),
        json!({"jsonrpc": "2.0", "id": 7, "result": {}}),
    );
}

#[test]
fn stalled_stdin_fails_open_silently_within_one_second() {
    let root = TempRoot::new("stalled-stdin");
    let mut command = Command::new(env!("CARGO_BIN_EXE_girder"));
    command
        .arg("hook")
        .current_dir(root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn girder hook");
    let stdin = child.stdin.take().expect("hook stdin");
    let mut stdout = child.stdout.take().expect("hook stdout");
    let mut stderr = child.stderr.take().expect("hook stderr");

    let started = Instant::now();
    let status = child.wait().expect("wait for stalled hook");
    assert!(status.success());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "stalled stdin kept the hook alive for {:?}",
        started.elapsed()
    );
    drop(stdin);

    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    stdout.read_to_end(&mut stdout_bytes).unwrap();
    stderr.read_to_end(&mut stderr_bytes).unwrap();
    assert!(stdout_bytes.is_empty(), "stdout: {stdout_bytes:?}");
    assert!(stderr_bytes.is_empty(), "stderr: {stderr_bytes:?}");
}
