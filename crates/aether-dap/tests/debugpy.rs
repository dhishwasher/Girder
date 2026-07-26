use aether_dap::{DapClient, DebugSession, SourceBreakpoint};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let unique = format!(
            "aether-dap-debugpy-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn join(&self, rel: &str) -> PathBuf {
        self.path.join(rel)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn debugpy_available() -> bool {
    Command::new("python3")
        .args(["-c", "import debugpy"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Opt-in adapter smoke test.
///
/// Run with:
/// `cargo test -p aether-dap --test debugpy -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires python3 with debugpy installed"]
async fn debugpy_launch_breakpoint_stacktrace_disconnect() {
    if !debugpy_available() {
        eprintln!("skipping: python3 debugpy module is not installed");
        return;
    }

    let temp = TempDir::new();
    let script = temp.join("target.py");
    std::fs::write(
        &script,
        "def main():\n    value = 41\n    value += 1\n    print(value)\n\nif __name__ == '__main__':\n    main()\n",
    )
    .unwrap();

    let client = DapClient::spawn("python3", &["-m", "debugpy.adapter"])
        .await
        .unwrap();
    let mut session = DebugSession::new(client, "python");

    session.initialize().await.unwrap();
    session
        .launch(serde_json::json!({
            "program": script.to_string_lossy(),
            "stopOnEntry": false,
            "console": "internalConsole",
        }))
        .await
        .unwrap();
    let breakpoints = session
        .set_breakpoints(Path::new(&script), &[SourceBreakpoint::at_line(3)])
        .await
        .unwrap();
    assert_eq!(breakpoints.len(), 1);

    session.configuration_done().await.unwrap();

    let stopped = session.wait_for_stopped().await.unwrap();
    assert_eq!(stopped.reason, "breakpoint");

    let thread_id = stopped.thread_id.unwrap_or(1);
    let frames = session.stack_trace(thread_id, 10).await.unwrap();
    assert!(
        frames.iter().any(|frame| {
            frame
                .source
                .as_ref()
                .and_then(|source| source.path.as_deref())
                .map(|path| path.ends_with("target.py"))
                .unwrap_or(false)
        }),
        "expected stack trace to include target.py, got {frames:?}"
    );

    session.disconnect().await.unwrap();
}
