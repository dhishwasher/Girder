//! Fail-open structured read advisory for agent PreToolUse hooks.
//!
//! This helper only checks whether a saved graph file is already present. It
//! never loads or builds that graph, because a hook must not delay a read.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

const MAX_INPUT_BYTES: usize = 64 * 1024;
const METADATA_TIMEOUT: Duration = Duration::from_millis(20);
const ADVISORY: &str = "Girder: consider `girder context . --nodes <node::path> --json --source-only` before this whole-file read.";

static PANIC_HOOK_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Eq, PartialEq)]
struct ReadRequest {
    cwd: PathBuf,
    file_path: String,
}

/// Handles one structured hook payload. Every failure is intentionally
/// ignored: callers must be able to continue their read when this helper is
/// unavailable or receives an unfamiliar payload.
pub fn hook(_args: &[String]) -> std::io::Result<()> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let _ = std::thread::Builder::new().spawn(move || {
        let _ = sender.send(catch_silently(advisory_response).flatten());
    });
    if let Ok(Some(response)) = receiver.recv_timeout(METADATA_TIMEOUT) {
        let mut stdout = std::io::stdout().lock();
        let _ = serde_json::to_writer(&mut stdout, &response);
        let _ = stdout.write_all(b"\n");
        let _ = stdout.flush();
    }
    Ok(())
}

/// Rust invokes the process-wide panic hook before unwinding. This command is
/// deliberately silent on malformed hook data, so temporarily replace that
/// hook while running the isolated worker and restore it before returning.
fn catch_silently(work: impl FnOnce() -> Option<Value>) -> Option<Option<Value>> {
    let Ok(_lock) = PANIC_HOOK_LOCK.lock() else {
        return None;
    };
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).ok();
    std::panic::set_hook(previous);
    result
}

fn advisory_response() -> Option<Value> {
    let payload = read_payload()?;
    advisory_response_for_payload(&payload)
}

fn advisory_response_for_payload(payload: &Value) -> Option<Value> {
    let request = eligible_read_request(payload)?;
    if ready_whole_file_read(&request.cwd, &request.file_path) {
        Some(advisory_output())
    } else {
        None
    }
}

fn eligible_read_request(payload: &Value) -> Option<ReadRequest> {
    let object = payload.as_object()?;
    let tool_name = object.get("tool_name")?.as_str()?;
    if !is_whole_file_read(tool_name) {
        return None;
    }

    let tool_input = object.get("tool_input")?.as_object()?;
    if ["offset", "limit", "start_line", "end_line"]
        .iter()
        .any(|key| tool_input.contains_key(*key))
    {
        return None;
    }

    let file_path = tool_input
        .get("file_path")
        .and_then(Value::as_str)
        .or_else(|| tool_input.get("path").and_then(Value::as_str))?;
    Some(ReadRequest {
        cwd: hook_cwd(object)?,
        file_path: file_path.to_owned(),
    })
}

fn advisory_output() -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "additionalContext": ADVISORY,
        }
    })
}

fn read_payload() -> Option<Value> {
    let mut bytes = Vec::with_capacity(MAX_INPUT_BYTES + 1);
    let mut stdin = std::io::stdin().lock().take((MAX_INPUT_BYTES + 1) as u64);
    stdin.read_to_end(&mut bytes).ok()?;
    if bytes.len() > MAX_INPUT_BYTES {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

fn is_whole_file_read(tool_name: &str) -> bool {
    tool_name.eq_ignore_ascii_case("read")
        || tool_name == "read_file"
        || tool_name == "__read_file"
        || (tool_name.starts_with("mcp__") && tool_name.ends_with("__read_file"))
}

fn hook_cwd(object: &serde_json::Map<String, Value>) -> Option<PathBuf> {
    object
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| {
            object
                .get("workspace_roots")
                .and_then(Value::as_array)
                .and_then(|roots| roots.iter().find_map(Value::as_str))
                .map(PathBuf::from)
        })
        .or_else(|| std::env::var_os("CLAUDE_PROJECT_DIR").map(PathBuf::from))
        .or_else(|| std::env::current_dir().ok())
}

fn ready_whole_file_read(cwd: &Path, file_path: &str) -> bool {
    let Ok(root) = cwd.canonicalize() else {
        return false;
    };
    let requested = Path::new(file_path);
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let Ok(candidate) = candidate.canonicalize() else {
        return false;
    };
    if !candidate.starts_with(&root)
        || !matches!(
            candidate
                .extension()
                .and_then(|extension| extension.to_str()),
            Some("rs" | "py" | "ts" | "tsx" | "go")
        )
    {
        return false;
    }
    root.join("project.aether").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "girder-hook-unit-{name}-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir_all(path.join("src")).unwrap();
            std::fs::write(path.join("src/lib.rs"), "pub fn answer() {}\n").unwrap();
            Self(path)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn payload(tool_name: &str) -> Value {
        json!({
            "tool_name": tool_name,
            "tool_input": {"path": "src/lib.rs"},
            "cwd": "/project",
        })
    }

    #[test]
    fn eligible_read_aliases_have_a_deterministic_advisory_response() {
        for tool_name in ["Read", "read_file", "mcp__filesystem__read_file"] {
            assert_eq!(
                eligible_read_request(&payload(tool_name)),
                Some(ReadRequest {
                    cwd: PathBuf::from("/project"),
                    file_path: "src/lib.rs".into(),
                }),
                "{tool_name}"
            );
        }
        assert_eq!(
            advisory_output(),
            json!({"hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "additionalContext": ADVISORY,
            }}),
        );
    }

    #[test]
    fn bounded_and_unstructured_payloads_are_ineligible() {
        let mut bounded = payload("Read");
        bounded["tool_input"]["limit"] = json!(1);
        assert!(eligible_read_request(&bounded).is_none());
        assert!(eligible_read_request(&json!({"tool_name": "Read"})).is_none());
        assert!(eligible_read_request(&payload("Bash")).is_none());
    }

    #[test]
    fn ready_graph_payloads_build_advice_end_to_end_without_a_deadline() {
        let root = TempRoot::new("ready");
        let outside = TempRoot::new("outside");
        std::fs::write(root.0.join("project.aether"), "saved graph marker\n").unwrap();
        for tool_name in ["Read", "read_file", "mcp__filesystem__read_file"] {
            let payload = json!({
                "tool_name": tool_name,
                "tool_input": {"path": root.0.join("src/lib.rs")},
                "cwd": root.0.clone(),
            });
            assert_eq!(
                advisory_response_for_payload(&payload),
                Some(advisory_output())
            );
        }

        let bounded = json!({
            "tool_name": "Read",
            "tool_input": {"path": root.0.join("src/lib.rs"), "limit": 1},
            "cwd": root.0.clone(),
        });
        assert!(advisory_response_for_payload(&bounded).is_none());
        let outside = json!({
            "tool_name": "Read",
            "tool_input": {"path": outside.0.join("src/lib.rs")},
            "cwd": root.0.clone(),
        });
        assert!(advisory_response_for_payload(&outside).is_none());
    }
}
