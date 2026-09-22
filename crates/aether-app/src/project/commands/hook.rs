//! Fail-open structured advisories for agent PreToolUse and PostToolUse hooks.
//!
//! Read events get a small source-only context suggestion. Successful edit
//! events get a bounded blast-radius advisory from the saved graph snapshot.
//! The edit path never scans or reconciles source files.

use crate::project::source;
use aether_graph::{NodeId, NodeKind, SemanticGraph};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_INPUT_BYTES: usize = 64 * 1024;
const COMPUTATION_TIMEOUT: Duration = Duration::from_millis(20);
const MAX_REPORT_BYTES: usize = 8 * 1024;
const MAX_ORIGINS: usize = 8;
const MAX_REACHED: usize = 32;
const MAX_UNCOVERED: usize = 16;
const IMPACT_CACHE_HEADER: &[u8] = b"GIRDER_HOOK_IMPACT_V2\n";
const ADVISORY: &str = "Girder: consider `girder context . --nodes <node::path> --json --source-only` before this whole-file read.";
const HOOK_LOG_ENV: &str = "GIRDER_HOOK_LOG";
const HOOK_LOG_PATH: &str = ".girder/hook-log.jsonl";
const LOG_WAIT_TIMEOUT: Duration = Duration::from_millis(200);

static PANIC_HOOK_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Eq, PartialEq)]
struct ReadRequest {
    cwd: PathBuf,
    file_path: String,
}

/// Handles one structured hook payload. Every failure is intentionally
/// ignored: callers must be able to continue their read when this helper is
/// unavailable or receives an unfamiliar payload.
pub fn hook(args: &[String]) -> std::io::Result<()> {
    if args.iter().any(|arg| arg == "--edit-blast-radius")
        && args.iter().any(|arg| arg == "--observation")
    {
        return hook_observation();
    }
    let logging_enabled = std::env::var_os(HOOK_LOG_ENV).is_some();
    let (sender, receiver) = mpsc::sync_channel(1);
    let (log_sender, log_receiver) = mpsc::sync_channel::<(Option<PathBuf>, Vec<HookLogEntry>)>(1);
    let _ = std::thread::Builder::new().spawn(move || {
        let started = Instant::now();
        let combined = catch_silently(|| {
            let payload = read_payload();
            let output = payload.as_ref().and_then(compute_hook_output);
            let (log_cwd, log_entries) = if logging_enabled {
                match &payload {
                    Some(value) => (
                        value.as_object().and_then(hook_cwd),
                        build_log_entries(value, started.elapsed()),
                    ),
                    None => (None, Vec::new()),
                }
            } else {
                (None, Vec::new())
            };
            Some((output, log_cwd, log_entries))
        })
        .flatten();
        let (output, log_cwd, log_entries) = combined.unwrap_or((None, None, Vec::new()));
        let _ = sender.send(output);
        if logging_enabled {
            let _ = log_sender.send((log_cwd, log_entries));
        }
    });
    if let Ok(Some(output)) = receiver.recv_timeout(COMPUTATION_TIMEOUT) {
        if !output.stdout.is_empty() {
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(&output.stdout);
            let _ = stdout.flush();
        }
        if !output.stderr.is_empty() {
            let mut stderr = std::io::stderr().lock();
            let _ = stderr.write_all(&output.stderr);
            let _ = stderr.flush();
        }
    }
    // Deliberately after the primary response is already written: this wait
    // (and the file write it guards) must never affect the timing the caller
    // observes for stdout/stderr, only add best-effort work once that's done.
    if logging_enabled {
        if let Ok((Some(cwd), entries)) = log_receiver.recv_timeout(LOG_WAIT_TIMEOUT) {
            if !entries.is_empty() {
                append_hook_log(&cwd, &entries);
            }
        }
    }
    Ok(())
}

fn hook_observation() -> std::io::Result<()> {
    // Measure the computation synchronously so scheduler delay is not counted
    // as graph work. Production mode above retains the hard timeout.
    let observation = catch_silently(observation_output)
        .flatten()
        .unwrap_or_else(ObservationRecord::failed);
    let mut stdout = std::io::stdout().lock();
    let _ = serde_json::to_writer(&mut stdout, &observation);
    let _ = stdout.write_all(b"\n");
    let _ = stdout.flush();
    Ok(())
}

#[derive(Debug, Default)]
struct HookOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Rust invokes the process-wide panic hook before unwinding. This command is
/// deliberately silent on malformed hook data, so temporarily replace that
/// hook while running the isolated worker and restore it before returning.
fn catch_silently<T>(work: impl FnOnce() -> Option<T>) -> Option<Option<T>> {
    let Ok(_lock) = PANIC_HOOK_LOCK.lock() else {
        return None;
    };
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).ok();
    std::panic::set_hook(previous);
    result
}

fn compute_hook_output(payload: &Value) -> Option<HookOutput> {
    if let Some(response) = advisory_response_for_payload(payload) {
        let mut stdout = Vec::new();
        serde_json::to_writer(&mut stdout, &response).ok()?;
        stdout.push(b'\n');
        return Some(HookOutput {
            stdout,
            stderr: Vec::new(),
        });
    }
    let report = post_edit_report_for_payload(payload)?;
    Some(HookOutput {
        stdout: Vec::new(),
        stderr: report.into_bytes(),
    })
}

#[derive(Debug, Clone, Default, serde::Serialize)]
struct ObservationRecord {
    graph_ready: bool,
    origins: usize,
    reached: usize,
    uncovered: usize,
    origins_omitted: usize,
    reached_omitted: usize,
    uncovered_omitted: usize,
    emitted_bytes: usize,
    duration_us: u128,
    emitted: bool,
    timed_out: bool,
    failed: bool,
}

impl ObservationRecord {
    fn failed() -> Self {
        Self {
            failed: true,
            ..Self::default()
        }
    }
}

fn observation_output() -> Option<ObservationRecord> {
    let started = Instant::now();
    let Some(payload) = read_payload() else {
        return Some(ObservationRecord {
            duration_us: started.elapsed().as_micros(),
            failed: true,
            ..ObservationRecord::default()
        });
    };
    let mut record = compute_post_edit(&payload).observation();
    if let Some(request) = eligible_edit_request(&payload) {
        if request.paths.len() != 1 {
            record.failed = true;
            record.emitted = false;
            record.emitted_bytes = 0;
        }
    }
    Some(record)
}

fn advisory_response_for_payload(payload: &Value) -> Option<Value> {
    let request = eligible_read_request(payload)?;
    if ready_whole_file_read(&request.cwd, &request.file_path) {
        Some(advisory_output())
    } else {
        None
    }
}

#[derive(Debug, Eq, PartialEq)]
struct EditRequest {
    cwd: PathBuf,
    paths: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
struct ReachedNode {
    distance: u32,
    path: String,
    id: NodeId,
}

#[derive(Debug, Default)]
struct ImpactComputation {
    report: Option<String>,
    graph_ready: bool,
    origins: usize,
    reached: usize,
    uncovered: usize,
    origins_omitted: usize,
    reached_omitted: usize,
    uncovered_omitted: usize,
    emitted_bytes: usize,
    duration_us: u128,
    failed: bool,
}

impl ImpactComputation {
    fn observation(self) -> ObservationRecord {
        let timed_out = self.duration_us >= COMPUTATION_TIMEOUT.as_micros();
        ObservationRecord {
            graph_ready: self.graph_ready,
            origins: self.origins,
            reached: self.reached,
            uncovered: self.uncovered,
            origins_omitted: self.origins_omitted,
            reached_omitted: self.reached_omitted,
            uncovered_omitted: self.uncovered_omitted,
            emitted_bytes: if timed_out { 0 } else { self.emitted_bytes },
            duration_us: self.duration_us,
            emitted: self.report.is_some() && !timed_out,
            timed_out,
            failed: self.failed,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct PathReport {
    file_path: String,
    origin_count: usize,
    origins: Vec<String>,
    reached_count: usize,
    reached: Vec<ReachedNode>,
    uncovered_count: usize,
    uncovered: Vec<ReachedNode>,
}

impl PathReport {
    fn origin_count(&self) -> usize {
        self.origin_count
    }

    fn reached_count(&self) -> usize {
        self.reached_count
    }

    fn uncovered_count(&self) -> usize {
        self.uncovered_count
    }
}

pub(crate) fn encode_impact_cache(graph: &SemanticGraph) -> std::io::Result<Vec<u8>> {
    let files: BTreeSet<_> = graph
        .nodes()
        .filter(|node| node.kind == NodeKind::Function && node.attr("is_test").is_none())
        .filter_map(|node| node.file.as_deref().and_then(normalize_node_file))
        .collect();
    let paths: Vec<_> = files
        .into_iter()
        .filter_map(|path| report_for_path_stats(graph, &path))
        .collect();
    let mut body = Vec::new();
    let mut index = Vec::with_capacity(paths.len());
    for report in paths {
        let offset = body.len();
        serde_json::to_writer(&mut body, &report).map_err(|error| {
            std::io::Error::other(format!("could not encode hook cache: {error}"))
        })?;
        index.push((
            format!("{:x}", Sha256::digest(report.file_path.as_bytes())),
            offset,
            body.len() - offset,
        ));
    }
    let mut encoded = Vec::with_capacity(IMPACT_CACHE_HEADER.len() + index.len() * 91 + body.len());
    encoded.extend_from_slice(IMPACT_CACHE_HEADER);
    for (digest, offset, length) in index {
        writeln!(encoded, "{digest} {offset:016x} {length:08x}")?;
    }
    encoded.push(b'\n');
    encoded.extend_from_slice(&body);
    Ok(encoded)
}

fn decode_impact_cache(bytes: &[u8], paths: &[String]) -> Option<Vec<PathReport>> {
    let rest = bytes.strip_prefix(IMPACT_CACHE_HEADER)?;
    let (index_bytes, body) = if let Some(body) = rest.strip_prefix(b"\n") {
        (&[][..], body)
    } else {
        let index_end = rest.windows(2).position(|window| window == b"\n\n")?;
        (&rest[..index_end], &rest[index_end + 2..])
    };
    let index = std::str::from_utf8(index_bytes).ok()?;
    let wanted: BTreeMap<_, _> = paths
        .iter()
        .map(|path| (format!("{:x}", Sha256::digest(path.as_bytes())), path))
        .collect();
    let mut reports = Vec::new();
    for line in index.lines() {
        let mut fields = line.split(' ');
        let digest = fields.next()?;
        let offset = usize::from_str_radix(fields.next()?, 16).ok()?;
        let length = usize::from_str_radix(fields.next()?, 16).ok()?;
        if fields.next().is_some() {
            return None;
        }
        let Some(path) = wanted.get(digest) else {
            continue;
        };
        let end = offset.checked_add(length)?;
        let report: PathReport = serde_json::from_slice(body.get(offset..end)?).ok()?;
        if report.file_path.as_str() != path.as_str() {
            return None;
        }
        reports.push(report);
    }
    reports.sort_by(|left, right| left.file_path.cmp(&right.file_path));
    Some(reports)
}

#[derive(Debug)]
struct ReportEntries {
    edited: Vec<String>,
    origin_count: usize,
    origins: Vec<(String, String)>,
    reached_count: usize,
    reached: Vec<(String, ReachedNode)>,
    uncovered_count: usize,
    uncovered: Vec<(String, ReachedNode)>,
}

impl ReportEntries {
    fn from_paths(paths: &[PathReport]) -> Self {
        let mut entries = Self {
            edited: paths.iter().map(|path| path.file_path.clone()).collect(),
            origin_count: paths.iter().map(PathReport::origin_count).sum(),
            origins: Vec::new(),
            reached_count: paths.iter().map(PathReport::reached_count).sum(),
            reached: Vec::new(),
            uncovered_count: paths.iter().map(PathReport::uncovered_count).sum(),
            uncovered: Vec::new(),
        };
        for path in paths {
            entries.origins.extend(
                path.origins
                    .iter()
                    .cloned()
                    .map(|origin| (path.file_path.clone(), origin)),
            );
            entries.reached.extend(
                path.reached
                    .iter()
                    .cloned()
                    .map(|reached| (path.file_path.clone(), reached)),
            );
            entries.uncovered.extend(
                path.uncovered
                    .iter()
                    .cloned()
                    .map(|uncovered| (path.file_path.clone(), uncovered)),
            );
        }
        entries.origins.sort();
        entries.reached.sort_by(|left, right| {
            left.1
                .distance
                .cmp(&right.1.distance)
                .then_with(|| left.1.path.cmp(&right.1.path))
                .then_with(|| left.0.cmp(&right.0))
        });
        entries.uncovered.sort_by(|left, right| {
            left.1
                .distance
                .cmp(&right.1.distance)
                .then_with(|| left.1.path.cmp(&right.1.path))
                .then_with(|| left.0.cmp(&right.0))
        });
        entries
    }
}

#[derive(Debug)]
struct RenderedReport {
    text: String,
}

/// Build all post-edit advice inside the worker deadline. A missing, invalid,
/// or stale snapshot is deliberately indistinguishable from no advice.
fn post_edit_report_for_payload(payload: &Value) -> Option<String> {
    compute_post_edit(payload).report
}

fn compute_post_edit(payload: &Value) -> ImpactComputation {
    let started = Instant::now();
    let Some(request) = eligible_edit_request(payload) else {
        return ImpactComputation {
            duration_us: started.elapsed().as_micros(),
            failed: true,
            ..ImpactComputation::default()
        };
    };
    let Ok(snapshot) = source::load_hook_snapshot(&request.cwd) else {
        return ImpactComputation {
            duration_us: started.elapsed().as_micros(),
            failed: true,
            ..ImpactComputation::default()
        };
    };
    let Some(snapshot) = snapshot else {
        return ImpactComputation {
            duration_us: started.elapsed().as_micros(),
            ..ImpactComputation::default()
        };
    };
    let Some(reports) = decode_impact_cache(&snapshot, &request.paths) else {
        return ImpactComputation {
            duration_us: started.elapsed().as_micros(),
            failed: true,
            ..ImpactComputation::default()
        };
    };
    let mut result = ImpactComputation {
        graph_ready: true,
        ..ImpactComputation::default()
    };
    for path_report in &reports {
        result.origins += path_report.origin_count();
        result.reached += path_report.reached_count();
        result.uncovered += path_report.uncovered_count();
    }
    result.origins_omitted = result.origins.saturating_sub(MAX_ORIGINS);
    result.reached_omitted = result.reached.saturating_sub(MAX_REACHED);
    result.uncovered_omitted = result.uncovered.saturating_sub(MAX_UNCOVERED);
    if !reports.is_empty() {
        if let Some(rendered) = render_report(&reports) {
            result.emitted_bytes = rendered.text.len();
            result.report = Some(rendered.text);
        } else {
            result.failed = true;
        }
    }
    result.duration_us = started.elapsed().as_micros();
    result
}

#[cfg(test)]
fn report_for_path(graph: &SemanticGraph, file_path: &str) -> Option<String> {
    report_for_path_stats(graph, file_path)
        .and_then(|report| render_report(&[report]))
        .map(|report| report.text)
}

fn report_for_path_stats(graph: &SemanticGraph, file_path: &str) -> Option<PathReport> {
    let mut origins: Vec<_> = graph
        .nodes()
        .filter(|node| {
            node.kind == NodeKind::Function
                && node.attr("is_test").is_none()
                && node
                    .file
                    .as_deref()
                    .and_then(normalize_node_file)
                    .as_deref()
                    == Some(file_path)
        })
        .map(|node| (node.id, node.path.clone()))
        .collect();
    origins.sort_by(|left, right| left.1.cmp(&right.1));
    if origins.is_empty() {
        return None;
    }

    let mut reached: BTreeMap<NodeId, ReachedNode> = BTreeMap::new();
    for (id, _) in &origins {
        reached.entry(*id).or_insert_with(|| ReachedNode {
            distance: 0,
            path: graph
                .get(*id)
                .map(|node| node.path.clone())
                .unwrap_or_default(),
            id: *id,
        });
        for (target, distance) in graph.impact_of(*id).affected {
            let Some(node) = graph.get(target) else {
                continue;
            };
            let candidate = ReachedNode {
                distance,
                path: node.path.clone(),
                id: target,
            };
            match reached.get(&target) {
                Some(existing) if existing.distance <= candidate.distance => {}
                _ => {
                    reached.insert(target, candidate);
                }
            }
        }
    }
    let mut reached: Vec<_> = reached.into_values().collect();
    reached.sort_by(|left, right| {
        left.distance
            .cmp(&right.distance)
            .then_with(|| left.path.cmp(&right.path))
    });

    let mut uncovered: Vec<_> = reached
        .iter()
        .filter(|entry| {
            graph.get(entry.id).is_some_and(|node| {
                node.kind == NodeKind::Function
                    && node.attr("is_test").is_none()
                    && graph.tests_for(entry.id).is_empty()
            })
        })
        .cloned()
        .collect();

    let origin_count = origins.len();
    let reached_count = reached.len();
    let uncovered_count = uncovered.len();
    origins.truncate(MAX_ORIGINS);
    reached.truncate(MAX_REACHED);
    uncovered.truncate(MAX_UNCOVERED);
    Some(PathReport {
        file_path: file_path.to_owned(),
        origin_count,
        origins: origins.into_iter().map(|(_, path)| path).collect(),
        reached_count,
        reached,
        uncovered_count,
        uncovered,
    })
}

fn render_report(paths: &[PathReport]) -> Option<RenderedReport> {
    let entries = ReportEntries::from_paths(paths);
    let origins = entries.origin_count;
    let reached = entries.reached_count;
    let uncovered = entries.uncovered_count;
    let mut output = String::new();
    push_report_line(
        &mut output,
        "Girder saved-graph impact advisory (edit-blast-radius-v1)\n",
    )?;
    push_report_line(&mut output, "edited paths:\n")?;
    for path in &entries.edited {
        push_report_line(&mut output, &format!("  {path}\n"))?;
    }
    push_report_line(
        &mut output,
        &format!(
            "origins: {origins} ({} omitted)\n",
            origins.saturating_sub(MAX_ORIGINS)
        ),
    )?;
    for (file, path) in entries.origins.iter().take(MAX_ORIGINS) {
        push_report_line(&mut output, &format!("  {file}: {path}\n"))?;
    }
    push_report_line(
        &mut output,
        &format!(
            "reached: {reached} ({} omitted)\n",
            reached.saturating_sub(MAX_REACHED)
        ),
    )?;
    for (file, entry) in entries.reached.iter().take(MAX_REACHED) {
        push_report_line(
            &mut output,
            &format!("  {file}: {} {}\n", entry.distance, entry.path),
        )?;
    }
    push_report_line(
        &mut output,
        &format!(
            "uncovered: {uncovered} ({} omitted)\n",
            uncovered.saturating_sub(MAX_UNCOVERED)
        ),
    )?;
    for (file, entry) in entries.uncovered.iter().take(MAX_UNCOVERED) {
        push_report_line(
            &mut output,
            &format!("  {file}: {} {}\n", entry.distance, entry.path),
        )?;
    }
    Some(RenderedReport { text: output })
}

fn push_report_line(output: &mut String, line: &str) -> Option<()> {
    if output.len().saturating_add(line.len()) > MAX_REPORT_BYTES {
        return None;
    }
    output.push_str(line);
    Some(())
}

fn eligible_edit_request(payload: &Value) -> Option<EditRequest> {
    let object = payload.as_object()?;
    let event = object
        .get("hook_event_name")
        .or_else(|| object.get("hookEventName"))
        .and_then(Value::as_str)?;
    if event != "PostToolUse" {
        return None;
    }
    let tool_name = object.get("tool_name")?.as_str()?.to_ascii_lowercase();
    if !matches!(
        tool_name.as_str(),
        "edit" | "write" | "apply_patch" | "applypatch"
    ) {
        return None;
    }
    if indicates_failure(object.get("success")) {
        return None;
    }
    let response = object
        .get("tool_response")
        .or_else(|| object.get("tool_result"))?;
    if indicates_failure(Some(response)) {
        return None;
    }
    let input = object.get("tool_input")?;
    let input_object = input.as_object();
    let cwd = hook_cwd(object)?;
    let mut paths = BTreeSet::new();
    if let Some(input_object) = input_object {
        for key in ["file_path", "path"] {
            if let Some(path) = input_object.get(key).and_then(Value::as_str) {
                if let Some(path) = normalize_event_path(&cwd, path) {
                    if is_supported_source_path(&path) {
                        paths.insert(path);
                    }
                }
            }
        }
        for key in ["patch", "input", "content"] {
            if let Some(text) = input_object.get(key).and_then(Value::as_str) {
                extract_patch_paths(text, &cwd, &mut paths);
            }
        }
    } else if let Some(text) = input.as_str() {
        extract_patch_paths(text, &cwd, &mut paths);
    }
    if paths.is_empty() {
        return None;
    }
    Some(EditRequest {
        cwd,
        paths: paths.into_iter().collect(),
    })
}

fn indicates_failure(value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    match value {
        Value::Bool(false) => true,
        Value::Object(object) => {
            object
                .get("success")
                .is_some_and(|v| v == &Value::Bool(false))
                || object.get("ok").is_some_and(|v| v == &Value::Bool(false))
                || object
                    .get("is_error")
                    .is_some_and(|v| v == &Value::Bool(true))
                || object
                    .get("isError")
                    .is_some_and(|v| v == &Value::Bool(true))
                || object
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| {
                        matches!(
                            status.to_ascii_lowercase().as_str(),
                            "error" | "failed" | "failure"
                        )
                    })
        }
        _ => false,
    }
}

fn extract_patch_paths(text: &str, cwd: &Path, paths: &mut BTreeSet<String>) {
    for line in text.lines() {
        let path = [
            "*** Update File:",
            "*** Add File:",
            "*** Delete File:",
            "*** Move to:",
            "+++ ",
        ]
        .iter()
        .find_map(|prefix| line.strip_prefix(prefix).map(str::trim));
        let Some(path) = path else {
            continue;
        };
        let path = path.split_once('\t').map_or(path, |(path, _)| path);
        if path == "/dev/null" {
            continue;
        }
        let path = path
            .strip_prefix("a/")
            .or_else(|| path.strip_prefix("b/"))
            .unwrap_or(path);
        if let Some(path) = normalize_event_path(cwd, path) {
            if is_supported_source_path(&path) {
                paths.insert(path);
            }
        }
    }
}

fn normalize_event_path(cwd: &Path, raw: &str) -> Option<String> {
    let raw = raw.replace('\\', "/");
    let path = Path::new(&raw);
    let relative = if path.is_absolute() {
        path.strip_prefix(cwd).ok()?.to_path_buf()
    } else {
        path.to_path_buf()
    };
    let mut clean = Vec::new();
    for component in relative.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(value) => clean.push(value.to_str()?.to_owned()),
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => return None,
        }
    }
    (!clean.is_empty()).then(|| clean.join("/"))
}

fn normalize_node_file(file: &str) -> Option<String> {
    normalize_event_path(Path::new("."), file)
}

fn is_supported_source_path(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("rs" | "py" | "ts" | "tsx" | "go")
    )
}

fn eligible_read_request(payload: &Value) -> Option<ReadRequest> {
    let object = payload.as_object()?;
    if object
        .get("hook_event_name")
        .or_else(|| object.get("hookEventName"))
        .and_then(Value::as_str)
        .is_some_and(|event| event != "PreToolUse")
    {
        return None;
    }
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

/// One opt-in diagnostic line per `girder hook` invocation. Never carries
/// source content — only the requested path and outcome booleans/reason.
#[derive(Debug, Clone, serde::Serialize)]
struct HookLogEntry {
    timestamp: u64,
    event: &'static str,
    file_path: String,
    snapshot_available: bool,
    advice_emitted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
}

/// Mirrors `compute_hook_output`'s own dispatch order (try the read advisory
/// path, fall back to the edit path) rather than switching on
/// `hook_event_name` directly: `eligible_read_request` already tolerates a
/// missing event name, so a strict top-level match here would silently drop
/// entries `compute_hook_output` itself would have answered.
fn build_log_entries(payload: &Value, duration: Duration) -> Vec<HookLogEntry> {
    let read_entries = log_read_invocation(payload, duration);
    if !read_entries.is_empty() {
        return read_entries;
    }
    log_edit_invocation(payload, duration)
}

fn log_read_invocation(payload: &Value, duration: Duration) -> Vec<HookLogEntry> {
    let Some(object) = payload.as_object() else {
        return Vec::new();
    };
    let Some(tool_name) = object.get("tool_name").and_then(Value::as_str) else {
        return Vec::new();
    };
    if !is_whole_file_read(tool_name) {
        return Vec::new();
    }
    let Some(tool_input) = object.get("tool_input").and_then(Value::as_object) else {
        return Vec::new();
    };
    let Some(file_path) = tool_input
        .get("file_path")
        .and_then(Value::as_str)
        .or_else(|| tool_input.get("path").and_then(Value::as_str))
    else {
        return Vec::new();
    };
    let Some(cwd) = hook_cwd(object) else {
        return Vec::new();
    };

    let bounded = ["offset", "limit", "start_line", "end_line"]
        .iter()
        .any(|key| tool_input.contains_key(*key));
    let timed_out = duration >= COMPUTATION_TIMEOUT;
    let (eligible_path, snapshot_available) = read_eligibility_snapshot(&cwd, file_path);
    let advice_emitted = !timed_out && !bounded && eligible_path && snapshot_available;
    let reason = if advice_emitted {
        None
    } else if timed_out {
        Some("timeout")
    } else if bounded || !eligible_path {
        Some("ineligible_path")
    } else {
        Some("no_snapshot")
    };

    vec![HookLogEntry {
        timestamp: unix_timestamp(),
        event: "read",
        file_path: file_path.to_owned(),
        snapshot_available,
        advice_emitted,
        reason,
    }]
}

fn log_edit_invocation(payload: &Value, duration: Duration) -> Vec<HookLogEntry> {
    let Some(object) = payload.as_object() else {
        return Vec::new();
    };
    let Some(tool_name) = object.get("tool_name").and_then(Value::as_str) else {
        return Vec::new();
    };
    let tool_name = tool_name.to_ascii_lowercase();
    if !matches!(
        tool_name.as_str(),
        "edit" | "write" | "apply_patch" | "applypatch"
    ) {
        return Vec::new();
    }
    let timed_out = duration >= COMPUTATION_TIMEOUT;

    match eligible_edit_request(payload) {
        None => {
            let file_path = object
                .get("tool_input")
                .and_then(Value::as_object)
                .and_then(|input| {
                    input
                        .get("file_path")
                        .and_then(Value::as_str)
                        .or_else(|| input.get("path").and_then(Value::as_str))
                })
                .unwrap_or_default()
                .to_owned();
            vec![HookLogEntry {
                timestamp: unix_timestamp(),
                event: "edit",
                file_path,
                snapshot_available: false,
                advice_emitted: false,
                reason: Some(if timed_out {
                    "timeout"
                } else {
                    "ineligible_path"
                }),
            }]
        }
        Some(request) => {
            let snapshot = source::load_hook_snapshot(&request.cwd).ok().flatten();
            let snapshot_available = snapshot.is_some();
            let reports = snapshot
                .as_deref()
                .and_then(|bytes| decode_impact_cache(bytes, &request.paths));
            request
                .paths
                .iter()
                .map(|path| {
                    let has_node = reports
                        .as_ref()
                        .is_some_and(|reports| reports.iter().any(|r| &r.file_path == path));
                    let advice_emitted = !timed_out && snapshot_available && has_node;
                    let reason = if advice_emitted {
                        None
                    } else if timed_out {
                        Some("timeout")
                    } else if !snapshot_available {
                        Some("no_snapshot")
                    } else {
                        Some("no_matching_node")
                    };
                    HookLogEntry {
                        timestamp: unix_timestamp(),
                        event: "edit",
                        file_path: path.clone(),
                        snapshot_available,
                        advice_emitted,
                        reason,
                    }
                })
                .collect()
        }
    }
}

/// Mirrors `ready_whole_file_read`'s checks but reports eligibility and
/// snapshot presence separately, since logging distinguishes them.
fn read_eligibility_snapshot(cwd: &Path, file_path: &str) -> (bool, bool) {
    let Ok(root) = cwd.canonicalize() else {
        return (false, false);
    };
    let snapshot_available = root.join("project.aether").is_file();
    let requested = Path::new(file_path);
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let eligible = candidate.canonicalize().is_ok_and(|candidate| {
        candidate.starts_with(&root)
            && matches!(
                candidate
                    .extension()
                    .and_then(|extension| extension.to_str()),
                Some("rs" | "py" | "ts" | "tsx" | "go")
            )
    });
    (eligible, snapshot_available)
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Best-effort JSONL append; a write failure (missing `.girder`, permissions,
/// full disk) is silently ignored to keep this diagnostic strictly opt-in and
/// never fail-closed.
fn append_hook_log(cwd: &Path, entries: &[HookLogEntry]) {
    let path = cwd.join(HOOK_LOG_PATH);
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    for entry in entries {
        let Ok(mut line) = serde_json::to_vec(entry) else {
            continue;
        };
        line.push(b'\n');
        let _ = file.write_all(&line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_graph::{Edge, EdgeKind, Node};
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
    fn edit_events_deduplicate_paths_and_accept_patch_headers() {
        let root = TempRoot::new("edit-parse");
        let edit = json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Edit",
            "tool_input": {
                "file_path": root.0.join("src/lib.rs"),
                "patch": "*** Update File: src/lib.rs\n*** Update File: src/lib.rs\n*** Add File: src/new.py\n"
            },
            "tool_response": {"success": true},
            "cwd": root.0,
        });
        assert_eq!(
            eligible_edit_request(&edit),
            Some(EditRequest {
                cwd: root.0.clone(),
                paths: vec!["src/lib.rs".into(), "src/new.py".into()],
            })
        );

        let mut failed = edit;
        failed["tool_response"] = json!({"success": false});
        assert!(eligible_edit_request(&failed).is_none());
        failed["hook_event_name"] = json!("PreToolUse");
        assert!(eligible_edit_request(&failed).is_none());
    }

    #[test]
    fn report_uses_propagating_reverse_impact_and_existing_test_relation() {
        let mut graph = SemanticGraph::new();
        let mut origin = Node::new(NodeKind::Function, "origin", "crate::lib::origin");
        origin.file = Some("src/lib.rs".into());
        let origin = graph.upsert_node(origin);
        let mut caller = Node::new(NodeKind::Function, "caller", "crate::lib::caller");
        caller.file = Some("src/main.rs".into());
        let caller = graph.upsert_node(caller);
        let mut test = Node::new(NodeKind::Function, "test_origin", "crate::lib::test_origin");
        test.file = Some("src/lib.rs".into());
        test.set_attr("is_test", "true");
        let test = graph.upsert_node(test);
        let module = graph.upsert_node(Node::new(NodeKind::Module, "module", "crate::lib"));
        graph
            .add_edge(caller, origin, Edge::new(EdgeKind::Calls))
            .unwrap();
        graph
            .add_edge(test, origin, Edge::new(EdgeKind::Calls))
            .unwrap();
        graph
            .add_edge(module, origin, Edge::new(EdgeKind::Contains))
            .unwrap();

        let report = report_for_path(&graph, "src/lib.rs").unwrap();
        assert!(report.contains("saved-graph impact"));
        assert!(report.contains("0 crate::lib::origin"));
        assert!(report.contains("1 crate::lib::caller"));
        assert!(!report.contains("crate::lib\n"));
        assert!(report.contains("uncovered: 1 (0 omitted)"));
    }

    #[test]
    fn report_caps_lists_and_records_omissions() {
        let mut graph = SemanticGraph::new();
        for index in 0..10 {
            let mut node = Node::new(
                NodeKind::Function,
                format!("origin_{index}"),
                format!("crate::lib::origin_{index}"),
            );
            node.file = Some("src/lib.rs".into());
            graph.upsert_node(node);
        }
        let report = report_for_path(&graph, "src/lib.rs").unwrap();
        assert!(report.len() <= MAX_REPORT_BYTES);
        assert!(report.contains("origins: 10 (2 omitted)"));
        assert!(report.contains("reached: 10 (0 omitted)"));
        assert!(report.contains("uncovered: 10 (0 omitted)"));
    }

    #[test]
    fn uncovered_entries_keep_reached_distance_then_path_order() {
        let mut graph = SemanticGraph::new();
        let mut origin = Node::new(NodeKind::Function, "origin", "crate::z::origin");
        origin.file = Some("src/lib.rs".into());
        let origin = graph.upsert_node(origin);
        let mut near = Node::new(NodeKind::Function, "near", "crate::a::near");
        near.file = Some("src/main.rs".into());
        let near = graph.upsert_node(near);
        let mut far = Node::new(NodeKind::Function, "far", "crate::zero::far");
        far.file = Some("src/main.rs".into());
        let far = graph.upsert_node(far);
        graph
            .add_edge(near, origin, Edge::new(EdgeKind::Calls))
            .unwrap();
        graph
            .add_edge(far, near, Edge::new(EdgeKind::Calls))
            .unwrap();

        let report = report_for_path(&graph, "src/lib.rs").unwrap();
        let origin_at = report.find("src/lib.rs: 0 crate::z::origin").unwrap();
        let near_at = report.find("src/lib.rs: 1 crate::a::near").unwrap();
        let far_at = report.find("src/lib.rs: 2 crate::zero::far").unwrap();
        assert!(origin_at < near_at && near_at < far_at, "{report}");
    }

    #[test]
    fn multi_path_report_applies_caps_globally_and_stays_bounded() {
        let mut graph = SemanticGraph::new();
        for (file, prefix) in [("src/one.rs", "one"), ("src/two.rs", "two")] {
            for index in 0..20 {
                let mut node = Node::new(
                    NodeKind::Function,
                    format!("{prefix}_{index}"),
                    format!("crate::{prefix}::{index}"),
                );
                node.file = Some(file.into());
                graph.upsert_node(node);
            }
        }
        let reports = [
            report_for_path_stats(&graph, "src/one.rs").unwrap(),
            report_for_path_stats(&graph, "src/two.rs").unwrap(),
        ];
        let rendered = render_report(&reports).unwrap();
        assert!(rendered.text.len() <= MAX_REPORT_BYTES);
        assert!(rendered.text.contains("origins: 40 (32 omitted)"));
        assert!(rendered.text.contains("reached: 40 (8 omitted)"));
        assert!(rendered.text.contains("uncovered: 40 (24 omitted)"));
        assert_eq!(rendered.text.matches("crate::").count(), 8 + 32 + 16);
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
