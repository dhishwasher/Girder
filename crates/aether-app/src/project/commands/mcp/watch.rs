//! Opt-in owner of cached extraction and atomically published graph generations.
mod queries;
mod snapshot;

use super::{
    error_response, prepare_tool, tool_error, tool_text, tool_timeout, MAX_TOOL_OUTPUT_BYTES,
};
use crate::project::config::ProjectConfig;
use crate::project::source::{self, locks::ProcessLock, CachedProject, JournalLock, OutputLocks};
use aether_builder::{FullRebuildReason, UpdateReport};
use aether_graph::SemanticGraph;
use notify::{
    event::{ModifyKind, RemoveKind, RenameMode},
    Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use serde_json::{json, Value};
use snapshot::{exclusions, graph_identity, Snapshot};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const QUIET: Duration = Duration::from_millis(250);
const STABILITY: Duration = Duration::from_millis(100);
const QUEUE_CAPACITY: usize = 4096;

struct Generation {
    number: u64,
    graph: SemanticGraph,
    files: usize,
    config: ProjectConfig,
    snapshot: Snapshot,
    owner: Arc<ProcessLock>,
}

#[derive(Default, Clone)]
struct Metrics {
    attempts: u64,
    initial_attempts: u64,
    failed_attempts: u64,
    builds: u64,
    published: u64,
    discarded: u64,
    fallbacks: u64,
    reasons: BTreeMap<String, u64>,
    parsed: usize,
    reused: usize,
}

struct State {
    epoch: u64,
    stale: bool,
    stop: bool,
    pending: BTreeSet<String>,
    reasons: BTreeSet<String>,
    queued: usize,
    first_event: Instant,
    last_event: Instant,
    generation: Option<Arc<Generation>>,
    config: ProjectConfig,
    error: Option<String>,
    metrics: Metrics,
}

struct Shared {
    root: PathBuf,
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn invalidate(state: &mut State, reason: Option<&str>) {
        let now = Instant::now();
        if !state.stale {
            state.first_event = now;
        }
        state.stale = true;
        state.epoch += 1;
        state.last_event = now;
        if let Some(reason) = reason {
            state.reasons.insert(reason.to_string());
        }
    }

    fn event(&self, result: notify::Result<Event>) {
        // Keep filesystem calls outside the mutex used by MCP timeout handling.
        let directories: BTreeSet<_> = result
            .as_ref()
            .ok()
            .filter(|event| !event.need_rescan() && !matches!(event.kind, EventKind::Access(_)))
            .into_iter()
            .flat_map(|event| &event.paths)
            .filter(|path| path.is_dir())
            .cloned()
            .collect();
        let mut state = self.state.lock().unwrap();
        if state.stop {
            return;
        }
        let event = match result {
            Ok(event) => event,
            Err(error) => {
                Self::invalidate(&mut state, Some("notification_error"));
                state.error = Some(error.to_string());
                self.changed.notify_all();
                return;
            }
        };
        if event.need_rescan() {
            Self::invalidate(&mut state, Some("overflow"));
            self.changed.notify_all();
            return;
        }
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        let mut relevant = Vec::new();
        let mut removed_directory = false;
        for path in &event.paths {
            let Ok(relative) = path.strip_prefix(&self.root) else {
                continue;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            // A parent's generic metadata event is also emitted by our graph
            // replacement. Root removal/rename remains a structural event.
            if relative.is_empty()
                && matches!(
                    event.kind,
                    EventKind::Modify(ModifyKind::Any | ModifyKind::Metadata(_))
                )
            {
                continue;
            }
            if ignored(&relative) {
                continue;
            }
            let graph_relative: PathBuf =
                Path::new(&state.config.graph.path).components().collect();
            if relative == graph_relative.to_string_lossy().replace('\\', "/") {
                // Audits and pre-response snapshots detect external graph
                // replacement under the output lock. Do not read persistence
                // while holding the invalidation mutex.
                continue;
            }
            let metadata = ["girder.toml", "Cargo.toml", "go.mod"].contains(&relative.as_str());
            let was_directory = state.generation.as_ref().is_some_and(|g| {
                g.snapshot
                    .files
                    .keys()
                    .any(|p| p.starts_with(&format!("{relative}/")))
            });
            removed_directory |= was_directory && matches!(event.kind, EventKind::Remove(_));
            let directory_event = (matches!(event.kind, EventKind::Remove(RemoveKind::Folder))
                || directories.contains(path)
                || relative.is_empty()
                || was_directory)
                && directory_in_scope(&state.config, &relative);
            let owned = state
                .generation
                .as_ref()
                .is_some_and(|g| g.snapshot.files.contains_key(&relative));
            if metadata || owned || directory_event || configured(&state.config, &relative) {
                relevant.push(relative);
            }
        }
        if relevant.is_empty() {
            return;
        }
        let reason = if removed_directory {
            Some("directory_deletion")
        } else {
            match event.kind {
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if event.paths.len() == 2 => {
                    None
                }
                EventKind::Modify(ModifyKind::Name(_)) => Some("unknown_rename"),
                EventKind::Remove(RemoveKind::Folder) => Some("directory_deletion"),
                EventKind::Any | EventKind::Other => Some("uncertain_event_mapping"),
                _ => None,
            }
        };
        Self::invalidate(&mut state, reason);
        state.queued += 1;
        if state.queued > QUEUE_CAPACITY || state.pending.len() + relevant.len() > QUEUE_CAPACITY {
            state.pending.clear();
            state.reasons.insert("overflow".to_string());
        } else {
            state.pending.extend(relevant);
        }
        self.changed.notify_all();
    }
}

fn ignored(relative: &str) -> bool {
    relative.split('/').any(|part| {
        ["target", ".git", "node_modules", "__pycache__", ".girder"].contains(&part)
            || (part.starts_with('.') && part.contains(".girder-") && part.ends_with(".tmp"))
    })
}

fn directory_in_scope(config: &ProjectConfig, relative: &str) -> bool {
    if relative.is_empty() {
        return true;
    }
    if config.source_excludes().is_ok_and(|e| e.is_match(relative)) {
        return false;
    }
    config.source.roots.iter().any(|root| {
        let root = root.trim_start_matches("./").trim_end_matches('/');
        root.is_empty()
            || root == "."
            || relative == root
            || relative.starts_with(&format!("{root}/"))
            || root.starts_with(&format!("{relative}/"))
    })
}

fn configured(config: &ProjectConfig, relative: &str) -> bool {
    if !source::is_configured_source_path(config, relative).unwrap_or(true) {
        return false;
    }
    let Ok(excludes) = config.source_excludes() else {
        return true;
    };
    let mut path = Path::new(relative);
    loop {
        if excludes.is_match(path) {
            return false;
        }
        let Some(parent) = path.parent() else {
            break;
        };
        if parent.as_os_str().is_empty() {
            break;
        }
        path = parent;
    }
    true
}

struct CheckedAnswer {
    answer: io::Result<String>,
    snapshot: Snapshot,
    _writer_guard: ProcessLock,
    _output_guard: OutputLocks,
}

struct QueryJob {
    generation: Arc<Generation>,
    argv: Vec<String>,
    reply: mpsc::SyncSender<io::Result<CheckedAnswer>>,
}

pub(super) struct Server {
    shared: Arc<Shared>,
    worker: Option<thread::JoinHandle<()>>,
    query: mpsc::SyncSender<QueryJob>,
}

impl Server {
    pub(super) fn start(root: &Path) -> io::Result<Self> {
        let root = root.canonicalize()?;
        let config = CachedProject::configuration(&root, &exclusions())?;
        let identity = graph_identity(&root, &config)?;
        let owner = ProcessLock::acquire("graph-owner", &identity, false)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "another MCP watcher already owns this graph",
            )
        })?;
        if !owner.outside(&root) {
            return Err(io::Error::other("watch requires a narrower project root: graph ownership storage must remain outside it"));
        }
        let now = Instant::now();
        let shared = Arc::new(Shared {
            root: root.clone(),
            state: Mutex::new(State {
                epoch: 0,
                stale: true,
                stop: false,
                pending: BTreeSet::new(),
                reasons: BTreeSet::new(),
                queued: 0,
                first_event: now,
                last_event: now,
                generation: None,
                config,
                error: None,
                metrics: Metrics::default(),
            }),
            changed: Condvar::new(),
        });
        let events = shared.clone();
        let mut watcher = notify::recommended_watcher(move |event| events.event(event))
            .map_err(io::Error::other)?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(io::Error::other)?;
        if let Some(parent) = root.parent() {
            watcher
                .watch(parent, RecursiveMode::NonRecursive)
                .map_err(io::Error::other)?;
        }
        let updates = shared.clone();
        let worker = thread::spawn(move || update_loop(updates, watcher, identity, owner));
        // At most one computation and one queued job. A timed-out job's result
        // channel is dropped; it cannot emit later or spawn unbounded threads.
        let (query, requests) = mpsc::sync_channel::<QueryJob>(1);
        thread::spawn(move || {
            while let Ok(job) = requests.recv() {
                let result = (|| {
                    let answer = queries::execute(&root, &job.generation, &job.argv);
                    #[cfg(debug_assertions)]
                    if let Ok(delay) = std::env::var("GIRDER_WATCH_TEST_QUERY_DELAY_MS") {
                        if let Ok(delay) = delay.parse::<u64>() {
                            eprintln!(
                                "girder watch: {}",
                                json!({"event":"query_computed", "generation":job.generation.number})
                            );
                            thread::sleep(Duration::from_millis(delay.min(10_000)));
                        }
                    }

                    let guard = ProcessLock::acquire("journal", &root, false)?
                        .ok_or_else(|| io::Error::from(io::ErrorKind::WouldBlock))?;
                    let output_guard = OutputLocks::acquire(
                        &root,
                        &[PathBuf::from(&job.generation.config.graph.path)],
                        false,
                    )?
                    .ok_or_else(|| io::Error::from(io::ErrorKind::WouldBlock))?;
                    if !job.generation.owner.is_current()? {
                        return Err(io::Error::other(
                            "graph owner lock was replaced; restart watching",
                        ));
                    }
                    let snapshot = Snapshot::capture(&root, &job.generation.config)?;
                    Ok(CheckedAnswer {
                        answer,
                        snapshot,
                        _writer_guard: guard,
                        _output_guard: output_guard,
                    })
                })();
                let _ = job.reply.send(result);
            }
        });
        Ok(Self {
            shared,
            worker: Some(worker),
            query,
        })
    }

    /// Render and emit under the same generation guard used by invalidation.
    /// No caller may receive a clean-result value and emit it later unchecked.
    pub(super) fn respond(
        &self,
        line: &str,
        executable: &Path,
        output: &mut impl Write,
    ) -> io::Result<()> {
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(_) => {
                return emit_optional(
                    output,
                    super::handle_line(line, &self.shared.root, executable),
                )
            }
        };
        if message.get("method").and_then(Value::as_str) != Some("tools/call")
            || message.get("id").is_none()
        {
            return emit_optional(
                output,
                super::handle_line(line, &self.shared.root, executable),
            );
        }
        let id = message["id"].clone();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let argv = match prepare_tool(&params, &self.shared.root) {
            Ok(argv) => argv,
            Err(error) => {
                return emit_optional(output, Some(error_response(id, error.code, &error.message)))
            }
        };
        let paid = match argv[0].as_str() {
            "orient" => Some("orient"),
            "test-impact" => Some("impacted_tests"),
            _ => None,
        };
        if let Some(tool) = paid {
            if let Err(error) = crate::project::require_paid(tool) {
                return emit_result(
                    output,
                    id,
                    tool_error(format!(
                        "`girder {}` failed: error: {error}",
                        argv.join(" ")
                    )),
                );
            }
        }
        let deadline = Instant::now() + tool_timeout();
        loop {
            let generation = match self.clean_generation(deadline) {
                Ok(generation) => generation,
                Err(error) => return emit_result(output, id, tool_error(error.to_string())),
            };
            let (reply, result) = mpsc::sync_channel(1);
            let mut job = QueryJob {
                generation: generation.clone(),
                argv: argv.clone(),
                reply,
            };
            loop {
                match self.query.try_send(job) {
                    Ok(()) => break,
                    Err(mpsc::TrySendError::Full(returned)) => {
                        job = returned;
                    }
                    Err(mpsc::TrySendError::Disconnected(_)) => {
                        return emit_result(output, id, tool_error("cached query worker stopped"))
                    }
                }
                if Instant::now() >= deadline {
                    return emit_result(
                        output,
                        id,
                        tool_error("cached graph query timed out waiting for the query worker"),
                    );
                }
                thread::sleep(Duration::from_millis(10));
            }
            let answer =
                match result.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(answer) => answer,
                    Err(_) => {
                        return emit_result(
                            output,
                            id,
                            tool_error("cached graph query timed out; its result was discarded"),
                        )
                    }
                };
            if Instant::now() >= deadline {
                return emit_result(
                    output,
                    id,
                    tool_error("cached graph query timed out; its result was discarded"),
                );
            }
            let checked = match answer {
                Ok(checked) => checked,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(error) => {
                    let mut state = self.shared.state.lock().unwrap();
                    Shared::invalidate(&mut state, Some("validation_mismatch"));
                    state.error = Some(error.to_string());
                    self.shared.changed.notify_all();
                    continue;
                }
            };
            let mut state = self.shared.state.lock().unwrap();
            let current = !state.stale
                && state
                    .generation
                    .as_ref()
                    .is_some_and(|g| g.number == generation.number);
            if !current {
                continue;
            }
            if checked.snapshot != generation.snapshot {
                Shared::invalidate(&mut state, Some("validation_mismatch"));
                self.shared.changed.notify_all();
                continue;
            }
            if Instant::now() >= deadline {
                return emit_result(
                    output,
                    id,
                    tool_error("graph validation exceeded the tool timeout"),
                );
            }
            if let Some(tool) = paid {
                if let Err(error) = crate::project::require_paid(tool) {
                    return emit_result(
                        output,
                        id,
                        tool_error(format!(
                            "`girder {}` failed: error: {error}",
                            argv.join(" ")
                        )),
                    );
                }
            }
            let ownership =
                generation.owner.is_current().and_then(|current| {
                    if current {
                        Ok(checked._writer_guard.is_current()?
                            && checked._output_guard.is_current()?)
                    } else {
                        Ok(false)
                    }
                });
            let ownership_error = match ownership {
                Ok(true) => None,
                Ok(false) => Some("process ownership lock was replaced".to_string()),
                Err(error) => Some(error.to_string()),
            };
            if let Some(error) = ownership_error {
                Shared::invalidate(&mut state, Some("ownership_lock_replaced"));
                state.error = Some(error);
                self.shared.changed.notify_all();
                continue;
            }
            let answer = match checked.answer {
                Ok(text) if text.len() > MAX_TOOL_OUTPUT_BYTES => tool_error(format!(
                    "cached graph query exceeded the {MAX_TOOL_OUTPUT_BYTES}-byte output limit"
                )),
                Ok(text) => tool_text(if text.trim().is_empty() {
                    "(no results)"
                } else {
                    &text
                }),
                Err(error) => tool_error(format!(
                    "`girder {}` failed: error: {error}",
                    argv.join(" ")
                )),
            };
            // This mutex is the linearization point: an invalidation must happen
            // before this check or after this complete frame has been emitted.
            return emit_result(output, id, answer);
        }
    }

    fn clean_generation(&self, deadline: Instant) -> io::Result<Arc<Generation>> {
        let mut state = self.shared.state.lock().unwrap();
        loop {
            if !state.stale {
                if let Some(generation) = &state.generation {
                    return Ok(generation.clone());
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "graph remained stale or rebuilding until the tool timeout{}",
                        state
                            .error
                            .as_ref()
                            .map(|e| format!(": {e}"))
                            .unwrap_or_default()
                    ),
                ));
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, remaining)
                .unwrap()
                .0;
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shared.state.lock().unwrap().stop = true;
        self.shared.changed.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        // Native backends may retain their callback briefly after unwatching.
        // Release the published generation explicitly instead of letting that
        // callback's Shared reference keep the owner lease after shutdown.
        self.shared.state.lock().unwrap().generation = None;
    }
}

fn emit_optional(output: &mut impl Write, response: Option<Value>) -> io::Result<()> {
    if let Some(response) = response {
        serde_json::to_writer(&mut *output, &response).map_err(io::Error::other)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(())
}

fn emit_result(output: &mut impl Write, id: Value, result: Value) -> io::Result<()> {
    emit_optional(
        output,
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result})),
    )
}

fn rebuild_reasons(reasons: &BTreeSet<String>) -> Vec<FullRebuildReason> {
    if reasons.is_empty() {
        return Vec::new();
    }
    if reasons
        .iter()
        .any(|r| r == "directory_deletion" || r == "root_removal")
    {
        vec![FullRebuildReason::OwnershipChanged]
    } else {
        vec![FullRebuildReason::UncertainEventMapping]
    }
}

fn update_loop(
    shared: Arc<Shared>,
    mut watcher: RecommendedWatcher,
    identity: PathBuf,
    owner: ProcessLock,
) {
    let mut current_owner = (identity, Arc::new(owner));
    let mut cache: Option<CachedProject> = None;
    let mut root_present = true;
    loop {
        let mut state = shared.state.lock().unwrap();
        if state.stop {
            break;
        }
        if !state.stale {
            let generation = state.generation.clone().unwrap();
            drop(state);
            // Native events can be lost. A bounded-frequency reconciliation
            // check also detects root removal and external graph replacement.
            let current = Snapshot::capture(&shared.root, &generation.config);
            state = shared.state.lock().unwrap();
            if current.as_ref().ok() != Some(&generation.snapshot) && !state.stale {
                Shared::invalidate(&mut state, Some("validation_mismatch"));
            }
            if !state.stale {
                drop(shared.changed.wait_timeout(state, QUIET).unwrap());
                continue;
            }
        }
        let quiet_left = QUIET.saturating_sub(state.last_event.elapsed());
        if !quiet_left.is_zero() {
            drop(shared.changed.wait_timeout(state, quiet_left).unwrap());
            continue;
        }
        let epoch = state.epoch;
        let dirty: Vec<_> = state.pending.iter().cloned().collect();
        let reasons = state.reasons.clone();
        let started = state.first_event;
        let initial = cache.is_none();
        if initial {
            state.metrics.initial_attempts += 1;
        } else {
            state.metrics.attempts += 1;
        }
        drop(state);
        let mut built = false;
        let attempt = (|| -> io::Result<(CachedProject, Snapshot, Option<UpdateReport>)> {
            if !shared.root.is_dir() {
                root_present = false;
                return Err(io::Error::other("watched root was removed"));
            }
            if !root_present
                || dirty.iter().any(|p| p.is_empty())
                || reasons.contains("root_removal")
            {
                let _ = watcher.unwatch(&shared.root);
                watcher
                    .watch(&shared.root, RecursiveMode::Recursive)
                    .map_err(io::Error::other)?;
                root_present = true;
            }
            let writer_guard = JournalLock::create_and_acquire(&shared.root)?;
            source::recover_under_lock(&shared.root, &writer_guard)?;
            let config = CachedProject::configuration(&shared.root, &exclusions())?;
            let output_guard =
                OutputLocks::acquire(&shared.root, &[PathBuf::from(&config.graph.path)], true)?
                    .ok_or_else(|| io::Error::other("could not lock graph output"))?;
            let before = Snapshot::capture(&shared.root, &config)?;
            thread::sleep(STABILITY);
            if Snapshot::capture(&shared.root, &config)? != before {
                return Err(io::Error::other(
                    "unstable reads during the 100 ms stability check",
                ));
            }
            if shared.state.lock().unwrap().epoch != epoch {
                return Err(io::Error::other(
                    "events arrived during the stability check",
                ));
            }
            let (mut candidate, report) = if let Some(cache) = &cache {
                let mut dirty = dirty.clone();
                if let Some(generation) = &shared.state.lock().unwrap().generation {
                    dirty.extend(before.dirty_since(&generation.snapshot));
                }
                let (candidate, report) = cache.updated(&dirty, &rebuild_reasons(&reasons))?;
                (candidate, Some(report))
            } else {
                (
                    CachedProject::open_with_exclusions(&shared.root, &exclusions())?,
                    None,
                )
            };
            built = true;
            if let Some(report) = &report {
                let mut state = shared.state.lock().unwrap();
                state.metrics.builds += 1;
                state.metrics.parsed += report.parsed_files.len();
                state.metrics.reused += report.reused_files.len();
                if !report.full_rebuild_reasons.is_empty() {
                    state.metrics.fallbacks += 1;
                    for reason in &report.full_rebuild_reasons {
                        *state
                            .metrics
                            .reasons
                            .entry(reason.as_str().to_string())
                            .or_default() += 1;
                    }
                }
            }
            let after = Snapshot::capture(&shared.root, &candidate.config)?;
            if after != before || !after.matches_candidate(&candidate) {
                return Err(io::Error::other(
                    "unstable read: candidate does not match validated inputs",
                ));
            }
            let identity = graph_identity(&shared.root, &candidate.config)?;
            let owner = if current_owner.0 == identity {
                current_owner.1.clone()
            } else {
                let owner =
                    ProcessLock::acquire("graph-owner", &identity, false)?.ok_or_else(|| {
                        io::Error::other("another watcher owns the newly configured graph")
                    })?;
                if !owner.outside(&shared.root) {
                    return Err(io::Error::other(
                        "graph ownership storage must remain outside the watched root",
                    ));
                }
                Arc::new(owner)
            };
            if !owner.is_current()? {
                return Err(io::Error::other(
                    "graph owner lock was replaced; restart watching",
                ));
            }
            let write = source::graph_project_write(
                &shared.root,
                &candidate.config,
                &candidate.graph,
                candidate.persisted_bytes.clone(),
            )?;
            {
                let mut state = shared.state.lock().unwrap();
                if state.epoch != epoch {
                    return Err(io::Error::other(
                        "events invalidated the candidate before persistence",
                    ));
                }
                state.config = candidate.config.clone();
            }
            source::commit_under_lock(&shared.root, vec![write], &writer_guard, &output_guard)?;
            candidate.persisted_bytes =
                source::read_project_bytes(&shared.root, &candidate.config.graph.path)?;
            let published = Snapshot::capture(&shared.root, &candidate.config)?;
            if !published.sources_equal(&before) || !published.matches_candidate(&candidate) {
                return Err(io::Error::other("inputs changed during graph persistence"));
            }
            if !owner.is_current()? || !writer_guard.is_current()? || !output_guard.is_current()? {
                return Err(io::Error::other(
                    "process ownership lock changed during persistence",
                ));
            }
            let graph = candidate.source_graph.clone();
            let files = candidate.builder.source_files().len();
            let config = candidate.config.clone();
            // Keep the writer guard until the complete generation is published.
            let mut state = shared.state.lock().unwrap();
            if state.epoch != epoch {
                return Err(io::Error::other(
                    "events invalidated the candidate before publication",
                ));
            }
            let number = state.generation.as_ref().map_or(1, |g| g.number + 1);
            state.generation = Some(Arc::new(Generation {
                number,
                graph,
                files,
                config,
                snapshot: published.clone(),
                owner: owner.clone(),
            }));
            // Outstanding queries keep the prior generation's Arc alive. Once
            // no response uses it, release an obsolete graph-path lease rather
            // than reserving every historical configuration for the session.
            current_owner = (identity, owner);
            if report.is_some() {
                state.metrics.published += 1;
            }
            state.stale = false;
            state.error = None;
            state.pending.clear();
            state.reasons.clear();
            state.queued = 0;
            Ok((candidate, published, report))
        })();
        let root_removed = attempt.is_err() && !shared.root.is_dir();
        let mut state = shared.state.lock().unwrap();
        let diagnostic = match attempt {
            Ok((candidate, _snapshot, report)) => {
                let metrics = &state.metrics;
                let diagnostic = json!({"event":"published", "initial":initial, "generation":state.generation.as_ref().unwrap().number,
                    "update_latency_seconds":started.elapsed().as_secs_f64(), "parsed_files":report.as_ref().map_or(candidate.builder.source_files().len(), |r|r.parsed_files.len()),
                    "reused_files":report.as_ref().map_or(0, |r|r.reused_files.len()), "full_rebuild_reasons":report.as_ref().map(|r|r.full_rebuild_reasons.iter().map(|r|r.as_str()).collect::<Vec<_>>()).unwrap_or_default(),
                    "update_attempts":metrics.attempts, "initial_attempts":metrics.initial_attempts,"failed_attempts":metrics.failed_attempts, "published_updates":metrics.published, "discarded_candidates":metrics.discarded,
                    "full_parsing_count":metrics.fallbacks, "fallback_denominator":metrics.builds, "fallback_percentage":if metrics.builds==0 {None} else {Some(100.0*metrics.fallbacks as f64/metrics.builds as f64)},
                    "fallback_reasons":metrics.reasons, "event_reasons":reasons, "total_parsed_files":metrics.parsed,"total_reused_files":metrics.reused});
                cache = Some(candidate);
                diagnostic
            }
            Err(error) => {
                if built {
                    state.metrics.discarded += 1;
                } else {
                    state.metrics.failed_attempts += 1;
                }
                let reason = if root_removed {
                    "root_removal"
                } else {
                    "unstable_reads"
                };
                Shared::invalidate(&mut state, Some(reason));
                state.error = Some(error.to_string());
                json!({"event":"discarded","reason":reason,"error":error.to_string(),"update_attempts":state.metrics.attempts,"initial":initial,"initial_attempts":state.metrics.initial_attempts,"failed_attempts":state.metrics.failed_attempts,"discarded_candidates":state.metrics.discarded,"full_parsing_count":state.metrics.fallbacks,"fallback_denominator":state.metrics.builds,"fallback_reasons":state.metrics.reasons,"total_parsed_files":state.metrics.parsed,"total_reused_files":state.metrics.reused})
            }
        };
        shared.changed.notify_all();
        drop(state);
        // A blocked diagnostics pipe must not keep calls from their timeout.
        let _ = writeln!(io::stderr().lock(), "girder watch: {diagnostic}");
    }
}

#[cfg(test)]
mod tests;
