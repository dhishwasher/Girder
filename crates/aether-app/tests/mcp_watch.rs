//! Real stdio, native events, OS ownership, and journal restart gates.
#[path = "support/license.rs"]
mod license;
use license::TEST_LICENSE_KEY;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::{Duration, Instant};
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "girder-mcp-watch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("lib.rs"), "mod math; pub use math::add;").unwrap();
        std::fs::write(path.join("math.rs"),"pub fn add(a:i32,b:i32)->i32 { a+b }\n#[test] fn test_add(){ assert_eq!(add(1,2),3); }").unwrap();
        std::fs::write(path.join("app.rs"), "pub fn run()->i32 { crate::add(1,2) }").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.name", "Girder fixture"],
            vec!["config", "user.email", "fixture@example.invalid"],
            vec!["add", "lib.rs", "math.rs", "app.rs"],
            vec!["commit", "-qm", "baseline"],
        ] {
            assert!(Command::new("git")
                .current_dir(&path)
                .args(args)
                .output()
                .unwrap()
                .status
                .success());
        }
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Session {
    child: Child,
    input: Option<ChildStdin>,
    frames: mpsc::Receiver<Value>,
    events: Arc<Mutex<Vec<Value>>>,
    next: u64,
    _blocked_stderr: Option<std::process::ChildStderr>,
}
impl Session {
    fn start(root: &Path, watch: bool, licensed: bool, extra: &[(&str, &str)]) -> Self {
        Self::start_with_stderr(root, watch, licensed, extra, true)
    }

    fn start_with_stderr(
        root: &Path,
        watch: bool,
        licensed: bool,
        extra: &[(&str, &str)],
        drain: bool,
    ) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_girder"));
        command
            .arg("mcp")
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if watch {
            command.arg("--watch");
        }
        command
            .env_remove("GIRDER_FAULT_EXIT")
            .env_remove("GIRDER_WATCH_TEST_QUERY_DELAY_MS")
            .env_remove("GIRDER_MCP_TIMEOUT_SECONDS");
        if licensed {
            command.env("GIRDER_LICENSE_KEY", TEST_LICENSE_KEY);
        } else {
            command
                .env_remove("GIRDER_LICENSE_KEY")
                .env("XDG_CONFIG_HOME", root)
                .env("APPDATA", root)
                .env("HOME", root);
        }
        for (key, value) in extra {
            command.env(key, value);
        }
        let mut child = command.spawn().unwrap();
        let input = child.stdin.take();
        let output = child.stdout.take().unwrap();
        let errors = child.stderr.take().unwrap();
        let (send, frames) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let line = line.unwrap();
                let frame: Value =
                    serde_json::from_str(&line).expect("only JSON-RPC frames on stdout");
                if send.send(frame).is_err() {
                    break;
                }
            }
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let records = events.clone();
        let blocked_stderr = if drain {
            std::thread::spawn(move || {
                for line in BufReader::new(errors).lines().map_while(Result::ok) {
                    if let Some(record) = line.strip_prefix("girder watch: ") {
                        if let Ok(value) = serde_json::from_str(record) {
                            records.lock().unwrap().push(value);
                        }
                    }
                }
            });
            None
        } else {
            Some(errors)
        };
        Self {
            child,
            input,
            frames,
            events,
            next: 0,
            _blocked_stderr: blocked_stderr,
        }
    }
    fn send(&mut self, method: &str, params: Value) -> u64 {
        self.next += 1;
        writeln!(
            self.input.as_mut().unwrap(),
            "{}",
            json!({"jsonrpc":"2.0","id":self.next,"method":method,"params":params})
        )
        .unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
        self.next
    }
    fn receive(&self) -> Value {
        self.frames
            .recv_timeout(Duration::from_secs(40))
            .expect("MCP response within integration deadline")
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.send(method, params);
        let frame = self.receive();
        assert_eq!(frame["id"], id);
        frame
    }
    fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name":name,"arguments":arguments}))
    }
    fn published(&self) {
        let deadline = Instant::now() + Duration::from_secs(40);
        while !self
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|v| v["event"] == "published")
        {
            assert!(
                Instant::now() < deadline,
                "watcher did not publish initial generation"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.input.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn text(frame: &Value) -> &str {
    frame["result"]["content"][0]["text"].as_str().unwrap()
}

#[test]
fn watch_all_seven_tools_and_errors_preserve_default_output() {
    let root = Fixture::new();
    std::fs::write(
        root.0.join("app.rs"),
        "pub fn run()->i32 { crate::add(3,4) }",
    )
    .unwrap();
    let mut watch = Session::start(&root.0, true, true, &[]);
    let mut cold = Session::start(&root.0, false, true, &[]);
    let a = watch.request("tools/list", json!({}));
    let b = cold.request("tools/list", json!({}));
    assert_eq!(a, b);
    let calls = [
        ("get_source", json!({"nodes":["crate::math::add"]})),
        ("find_definition", json!({"name":"add"})),
        ("search_code", json!({"query":"add"})),
        ("ask_codebase", json!({"question":"what calls add?"})),
        ("impacted_tests", json!({"nodes":["crate::math::add"]})),
        ("review_changes", json!({})),
        ("orient", json!({"nodes":["crate::math::add"],"depth":2})),
        ("review_changes", json!({"full":true})),
        (
            "get_source",
            json!({"nodes":["crate::math::add"],"include_tests":true}),
        ),
        ("get_source", json!({"nodes":["crate::missing"]})),
        ("find_definition", json!({"name":"add","kind":"invalid"})),
        ("orient", json!({"nodes":[]})),
    ];
    for (name, args) in calls {
        let a = watch.call(name, args.clone());
        let b = cold.call(name, args);
        assert_eq!(a, b, "tool parity: {name}");
    }
}

#[test]
fn watch_unlicensed_pair_and_second_owner_fail_clearly() {
    let root = Fixture::new();
    let mut watch = Session::start(&root.0, true, false, &[]);
    watch.published();
    for name in ["orient", "impacted_tests"] {
        let response = watch.call(name, json!({"nodes":["crate::math::add"]}));
        assert_eq!(response["result"]["isError"], true);
        assert!(text(&response).contains("paid Girder license"));
    }
    let alternate_temp = Fixture::new();
    let alternate_temp_text = alternate_temp.0.to_str().unwrap();
    let mut second = Session::start(
        &root.0,
        true,
        false,
        &[
            ("TMPDIR", alternate_temp_text),
            ("TEMP", alternate_temp_text),
            ("TMP", alternate_temp_text),
        ],
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = second.child.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "second watcher should be rejected at startup"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        watch.call("find_definition", json!({"name":"add"}))["result"]["isError"],
        false
    );
}

#[test]
#[cfg(debug_assertions)]
fn watch_invalidation_before_paid_response_retries_against_new_generation() {
    let root = Fixture::new();
    let mut watch = Session::start(
        &root.0,
        true,
        true,
        &[("GIRDER_WATCH_TEST_QUERY_DELAY_MS", "500")],
    );
    watch.published();
    watch.send(
        "tools/call",
        json!({"name":"orient","arguments":{"nodes":["crate::math::add"]}}),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !watch
        .events
        .lock()
        .unwrap()
        .iter()
        .any(|v| v["event"] == "query_computed")
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    std::fs::write(
        root.0.join("math.rs"),
        "pub fn add(a:i32,b:i32)->i32 { a+b+700 }",
    )
    .unwrap();
    let response = watch.receive();
    assert_eq!(response["result"]["isError"], false);
    assert!(
        text(&response).contains("a+b+700"),
        "stale paid source must never be emitted"
    );
}

#[test]
#[cfg(debug_assertions)]
fn watch_query_timeout_discards_late_output_and_preserves_protocol() {
    let root = Fixture::new();
    let mut watch = Session::start(
        &root.0,
        true,
        true,
        &[
            ("GIRDER_WATCH_TEST_QUERY_DELAY_MS", "2000"),
            ("GIRDER_MCP_TIMEOUT_SECONDS", "1"),
        ],
    );
    watch.published();
    let start = Instant::now();
    let response = watch.call("get_source", json!({"nodes":["crate::math::add"]}));
    assert_eq!(response["result"]["isError"], true);
    assert!(text(&response).contains("timed out"));
    assert!(start.elapsed() < Duration::from_millis(1600));
    assert_eq!(watch.request("ping", json!({}))["result"], json!({}));
    assert!(
        watch
            .frames
            .recv_timeout(Duration::from_millis(2300))
            .is_err(),
        "late query output must be discarded"
    );
}

#[test]
fn watch_output_limit_is_an_error_without_partial_output() {
    let root = Fixture::new();
    std::fs::write(
        root.0.join("large.rs"),
        format!(
            "pub fn large() {{ let _ = \"{}\"; }}",
            "x".repeat(4 * 1024 * 1024)
        ),
    )
    .unwrap();
    let mut watch = Session::start(&root.0, true, true, &[]);
    let response = watch.call("get_source", json!({"nodes":["crate::large::large"]}));
    assert_eq!(response["result"]["isError"], true);
    assert!(text(&response).contains("output limit"));
    assert!(text(&response).len() < 1000);
    assert_eq!(watch.request("ping", json!({}))["result"], json!({}));
}

#[test]
fn watch_journal_interruption_and_restart_recover_at_each_commit_stage() {
    for fault in [
        "after-staging",
        "after-manifest",
        "mid-apply",
        "pre-cleanup",
    ] {
        let root = Fixture::new();
        let mut interrupted = Session::start(&root.0, true, true, &[("GIRDER_FAULT_EXIT", fault)]);
        let deadline = Instant::now() + Duration::from_secs(40);
        loop {
            if let Some(status) = interrupted.child.try_wait().unwrap() {
                assert_eq!(status.code(), Some(87), "fault {fault}");
                break;
            }
            assert!(Instant::now() < deadline, "fault did not trigger: {fault}");
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(interrupted);
        let mut recovered = Session::start(&root.0, true, true, &[]);
        let response = recovered.call("get_source", json!({"nodes":["crate::math::add"]}));
        assert_eq!(response["result"]["isError"], false, "{fault}");
        assert!(text(&response).contains("a+b"));
        let mut cold = Session::start(&root.0, false, true, &[]);
        assert_eq!(
            response["result"],
            cold.call("get_source", json!({"nodes":["crate::math::add"]}))["result"]
        );
    }
}

#[test]
fn watch_pipelined_calls_and_an_existing_graph_writer_remain_consistent() {
    for nested in [false, true] {
        let root = Fixture::new();
        let writer_root = if nested {
            let child = root.0.join("sub");
            std::fs::create_dir_all(&child).unwrap();
            std::fs::write(child.join("child.rs"), "pub fn child() {}\n").unwrap();
            std::fs::write(
                root.0.join("girder.toml"),
                "[graph]\npath = \"sub/project.aether\"\n",
            )
            .unwrap();
            child
        } else {
            root.0.clone()
        };
        let mut watch = Session::start(&root.0, true, true, &[]);
        watch.published();
        std::fs::write(
            root.0.join("math.rs"),
            "pub fn add(a:i32,b:i32)->i32 { a+b+99 }",
        )
        .unwrap();
        for _ in 0..10 {
            watch.send(
                "tools/call",
                json!({"name":"get_source","arguments":{"nodes":["crate::math::add"]}}),
            );
        }
        let writer = Command::new(env!("CARGO_BIN_EXE_girder"))
            .arg("analyze")
            .arg(&writer_root)
            .arg("--json")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        for id in 1..=10 {
            let response = watch.receive();
            assert_eq!(response["id"], id);
            assert_eq!(response["result"]["isError"], false);
            assert!(text(&response).contains("a+b+99"));
        }
        let written = writer.wait_with_output().unwrap();
        assert!(
            written.status.success(),
            "concurrent graph writer failed: {}",
            String::from_utf8_lossy(&written.stderr)
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn blocked_diagnostics_do_not_block_the_graph_timeout() {
    use std::os::unix::fs::OpenOptionsExt;
    let root = Fixture::new();
    let mut watch = Session::start_with_stderr(
        &root.0,
        true,
        false,
        &[("GIRDER_MCP_TIMEOUT_SECONDS", "1")],
        false,
    );
    let request = json!({"name":"get_source","arguments":{"nodes":["crate::math::add"]}});
    let ready_by = Instant::now() + Duration::from_secs(10);
    loop {
        watch.send("tools/call", request.clone());
        let frame = watch.frames.recv_timeout(Duration::from_secs(3)).unwrap();
        if frame["result"]["isError"] == false {
            break;
        }
        assert!(
            Instant::now() < ready_by,
            "watcher did not finish initial construction"
        );
    }
    // Fill this child fixture's diagnostics pipe without draining it. The next
    // worker diagnostic must block, independently of filesystem/event timing.
    let mut fill = std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(format!("/proc/{}/fd/2", watch.child.id()))
        .unwrap();
    loop {
        match fill.write(&[b'x'; 4096]) {
            Ok(0) => panic!("diagnostics pipe closed"),
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) => panic!("cannot fill diagnostics pipe: {error}"),
        }
    }
    std::fs::write(root.0.join("girder.toml"), "[invalid").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let started = Instant::now();
    watch.send("tools/call", request);
    let frame = watch
        .frames
        .recv_timeout(Duration::from_secs(3))
        .expect("blocked diagnostics held the generation mutex");
    assert_eq!(frame["result"]["isError"], true);
    assert!(text(&frame).contains("timeout"));
    assert!(started.elapsed() < Duration::from_millis(1700));
    watch.child.kill().unwrap();
    watch.child.wait().unwrap();
}
