use crate::project::config::ProjectConfig;
use aether_builder::GraphBuilder;
use aether_graph::SemanticGraph;
use globset::GlobSet;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_TEMP_FILE: AtomicUsize = AtomicUsize::new(0);
const TRANSACTION_ROOT: &str = ".girder/transactions";
const TRANSACTION_MANIFEST: &str = "manifest.json";
const TRANSACTION_COMMITTED: &str = "COMMITTED";

/// Which interrupted journals a recovery pass may reclaim.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RecoveryScope {
    /// Roll back every uncommitted journal. Requires the exclusive journal
    /// lock, under which any remaining uncommitted journal is orphaned.
    All,
    /// Roll back only journals whose owning process is no longer alive.
    /// Used by read-only analysis so it never destroys a live writer's
    /// in-flight transaction.
    DeadOwnersOnly,
}

enum LockWait {
    Block,
    NonBlock,
}

/// Advisory exclusive lock on the `.girder` directory file descriptor.
///
/// Locking the directory itself, rather than a file inside it, preserves the
/// invariant that a completed commit removes `.girder` entirely. The held fd
/// stays valid after unlink, so cleanup under the lock is safe; acquisition
/// re-checks directory identity and retries because a lock on an unlinked
/// inode excludes nobody.
struct JournalLock {
    _dir: std::fs::File,
}

impl JournalLock {
    /// Lock an existing `.girder` directory. `Ok(None)` when the directory
    /// does not exist (nothing to recover) or, in `NonBlock` mode, when a
    /// writer currently holds the lock.
    fn acquire(root: &Path, wait: LockWait) -> std::io::Result<Option<Self>> {
        Self::acquire_inner(root, wait, false)
    }

    /// Create `.girder/transactions` and take the exclusive lock, waiting
    /// for any active writer. Keeping `transactions` present makes
    /// `.girder` non-empty, so unlocked best-effort pruners cannot remove
    /// it during the commit critical section.
    fn create_and_acquire(root: &Path) -> std::io::Result<Self> {
        match Self::acquire_inner(root, LockWait::Block, true)? {
            Some(lock) => {
                std::fs::create_dir_all(root.join(TRANSACTION_ROOT))?;
                Ok(lock)
            }
            None => Err(std::io::Error::other(
                "could not lock the project journal directory",
            )),
        }
    }

    fn acquire_inner(root: &Path, wait: LockWait, create: bool) -> std::io::Result<Option<Self>> {
        let path = root.join(".girder");
        for _ in 0..5 {
            if create {
                std::fs::create_dir_all(&path)?;
            }
            let dir = match std::fs::File::open(&path) {
                Ok(dir) => dir,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if create {
                        continue;
                    }
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };
            #[cfg(unix)]
            {
                use std::os::unix::io::AsRawFd;
                let flags = match wait {
                    LockWait::Block => libc::LOCK_EX,
                    LockWait::NonBlock => libc::LOCK_EX | libc::LOCK_NB,
                };
                if unsafe { libc::flock(dir.as_raw_fd(), flags) } != 0 {
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
                        return Ok(None);
                    }
                    return Err(error);
                }
                use std::os::unix::fs::MetadataExt;
                match std::fs::metadata(&path) {
                    Ok(current) => {
                        let held = dir.metadata()?;
                        if current.dev() == held.dev() && current.ino() == held.ino() {
                            return Ok(Some(JournalLock { _dir: dir }));
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        if !create {
                            return Ok(None);
                        }
                    }
                    Err(error) => return Err(error),
                }
                // The directory was removed or replaced while we waited for
                // the lock; retry against the current inode.
            }
            #[cfg(not(unix))]
            {
                // No advisory directory locking on this platform; preserve
                // the previous unlocked behavior. `wait` only means
                // something to `flock`, so it is unused here by design.
                let _ = wait;
                return Ok(Some(JournalLock { _dir: dir }));
            }
        }
        Err(std::io::Error::other(
            "could not lock the project journal directory",
        ))
    }
}

/// Whether the process that owns a `{pid}-{counter}` journal directory is
/// still alive. Unparseable names and our own pid are treated as abandoned:
/// no *other* live writer can be mid-commit while the journal lock is held.
fn journal_owner_is_alive(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let Some(pid) = name
        .split('-')
        .next()
        .and_then(|pid| pid.parse::<u32>().ok())
    else {
        return false;
    };
    if pid == 0 || pid == std::process::id() {
        return false;
    }
    #[cfg(unix)]
    {
        // `libc::pid_t` exists only on unix, so the narrowing belongs with
        // the probe that needs it rather than above this cfg split.
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        // Signal 0 probes existence: EPERM still means the process exists.
        if unsafe { libc::kill(pid, 0) } == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        // Cannot probe liveness: leave the journal for exclusive recovery.
        true
    }
}

#[derive(Debug)]
pub(crate) struct GraphSnapshot {
    pub(crate) graph: Option<SemanticGraph>,
    pub(crate) bytes: Option<Vec<u8>>,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectWrite {
    relative: PathBuf,
    expected: Option<Vec<u8>>,
    contents: Option<Vec<u8>>,
}

impl ProjectWrite {
    pub(crate) fn bytes(
        relative: impl Into<PathBuf>,
        expected: Option<Vec<u8>>,
        contents: Vec<u8>,
    ) -> Self {
        Self {
            relative: relative.into(),
            expected,
            contents: Some(contents),
        }
    }

    pub(crate) fn text(
        relative: impl Into<PathBuf>,
        expected: Option<Vec<u8>>,
        contents: impl Into<String>,
    ) -> Self {
        Self::bytes(relative, expected, contents.into().into_bytes())
    }

    pub(crate) fn relative(&self) -> &Path {
        &self.relative
    }

    pub(crate) fn contents(&self) -> Option<&[u8]> {
        self.contents.as_deref()
    }

    pub(crate) fn expected(&self) -> Option<&[u8]> {
        self.expected.as_deref()
    }

    pub(crate) fn delete(relative: impl Into<PathBuf>, expected: Vec<u8>) -> Self {
        Self {
            relative: relative.into(),
            expected: Some(expected),
            contents: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TransactionManifest {
    version: u32,
    entries: Vec<TransactionEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TransactionEntry {
    relative: String,
    had_original: bool,
    #[serde(default)]
    staged: Option<String>,
    backup: String,
}

pub(crate) fn is_supported_source_path(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("rs") | Some("py")
    )
}

pub(crate) fn is_configured_source_path(
    config: &ProjectConfig,
    path: &str,
) -> std::io::Result<bool> {
    let relative = path.trim_start_matches("./").replace('\\', "/");
    if !is_supported_source_path(&relative) || config.source_excludes()?.is_match(&relative) {
        return Ok(false);
    }

    Ok(config.source.roots.iter().any(|root| {
        let normalized = root.trim_start_matches("./").trim_end_matches('/');
        normalized.is_empty()
            || normalized == "."
            || relative == normalized
            || relative.starts_with(&format!("{normalized}/"))
    }))
}

/// Recursively collect configured source files, returning project-relative paths.
///
/// Canonical paths are constrained to the project root and de-duplicated. By
/// default symlinks are not followed; when enabled, visited-directory tracking
/// prevents cycles and links outside the project are rejected.
pub(crate) fn collect_sources_with_config(
    root: &Path,
    config: &ProjectConfig,
) -> std::io::Result<Vec<(PathBuf, String)>> {
    collect_configured_files(root, config, true)
}

/// Recursively collect every configured, non-excluded project file. Graph
/// lowering uses this to distinguish an unknown semantic path from one that
/// names a projection in an unsupported language.
pub(crate) fn collect_project_files_with_config(
    root: &Path,
    config: &ProjectConfig,
) -> std::io::Result<Vec<(PathBuf, String)>> {
    collect_configured_files(root, config, false)
}

fn collect_configured_files(
    root: &Path,
    config: &ProjectConfig,
    supported_sources_only: bool,
) -> std::io::Result<Vec<(PathBuf, String)>> {
    let canonical_root = std::fs::canonicalize(root)?;
    if !canonical_root.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("project root is not a directory: {}", root.display()),
        ));
    }

    let excludes = config.source_excludes()?;
    let mut stack = Vec::new();
    for source_root in &config.source.roots {
        let configured = canonical_root.join(source_root);
        let canonical = std::fs::canonicalize(&configured).map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!(
                    "could not resolve configured source root {}: {error}",
                    configured.display()
                ),
            )
        })?;
        ensure_inside_root(&canonical_root, &canonical)?;
        if !canonical.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "configured source root is not a directory: {}",
                    configured.display()
                ),
            ));
        }
        stack.push(canonical);
    }

    let mut visited_dirs = HashSet::new();
    let mut visited_files = HashSet::new();
    let mut out = Vec::new();

    while let Some(dir) = stack.pop() {
        let canonical_dir = std::fs::canonicalize(&dir)?;
        ensure_inside_root(&canonical_root, &canonical_dir)?;
        if !visited_dirs.insert(canonical_dir.clone()) {
            continue;
        }

        let mut entries = std::fs::read_dir(&canonical_dir)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let entry_path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_symlink() && !config.source.follow_symlinks {
                continue;
            }

            let canonical = if file_type.is_symlink() {
                let canonical = std::fs::canonicalize(&entry_path)?;
                ensure_inside_root(&canonical_root, &canonical)?;
                canonical
            } else {
                entry_path
            };
            let relative = canonical.strip_prefix(&canonical_root).map_err(|_| {
                std::io::Error::other(format!(
                    "source path escaped project root: {}",
                    canonical.display()
                ))
            })?;
            let relative_text = relative.to_string_lossy().replace('\\', "/");

            if is_excluded(&excludes, &relative_text) {
                continue;
            }

            if canonical.is_dir() {
                stack.push(canonical);
            } else if canonical.is_file()
                && (!supported_sources_only || is_supported_source_path(&relative_text))
                && visited_files.insert(std::fs::canonicalize(&canonical)?)
            {
                out.push((canonical, relative_text));
            }
        }
    }

    out.sort_by(|left, right| left.1.cmp(&right.1));
    Ok(out)
}

fn ensure_inside_root(root: &Path, candidate: &Path) -> std::io::Result<()> {
    if candidate.starts_with(root) {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "configured source path escapes project root: {}",
                candidate.display()
            ),
        ))
    }
}

fn is_excluded(excludes: &GlobSet, relative: &str) -> bool {
    excludes.is_match(relative)
}

/// Exact Cargo binary target names from the project's root Cargo.toml, as a
/// normalized project-relative file path -> declared target name map:
/// `[[bin]] path = "…"` overrides, plus the package's own name for the
/// conventional `src/main.rs` binary (Cargo's default target name when no
/// override names that exact path).
///
/// Measured gap: Cargo target metadata is not otherwise indexed. A binary at
/// a custom path has no discoverable target name and its
/// `CARGO_BIN_EXE_<target>` subprocess entrypoint stays unresolved; more
/// subtly, `src/main.rs`'s real target name (the package name, not
/// necessarily knowable from source) only disambiguates against a same-named
/// launch once a second binary exists in the project. A missing, unreadable,
/// or unparseable manifest — or a `[[bin]]` entry without an explicit `path`
/// — contributes nothing and convention remains the only source, exactly as
/// before.
fn cargo_bin_targets(root: &Path) -> HashMap<String, String> {
    let Ok(text) = std::fs::read_to_string(root.join("Cargo.toml")) else {
        return HashMap::new();
    };
    let Ok(manifest) = text.parse::<toml::Value>() else {
        return HashMap::new();
    };
    let mut targets: HashMap<String, String> = manifest
        .get("bin")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|bin| {
            let name = bin.get("name")?.as_str()?;
            let path = bin.get("path")?.as_str()?;
            (!name.is_empty() && !path.is_empty())
                .then(|| (path.replace('\\', "/"), name.to_string()))
        })
        .collect();
    if let Some(package_name) = manifest
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .filter(|name| !name.is_empty())
    {
        targets
            .entry("src/main.rs".to_string())
            .or_insert_with(|| package_name.to_string());
    }
    targets
}

/// Build a graph from the configured source files.
pub(crate) fn build_from_dir(root: &Path) -> std::io::Result<(SemanticGraph, GraphBuilder, usize)> {
    let config = ProjectConfig::load(root)?;
    build_from_dir_with_config(root, &config)
}

pub(crate) fn build_from_dir_with_config(
    root: &Path,
    config: &ProjectConfig,
) -> std::io::Result<(SemanticGraph, GraphBuilder, usize)> {
    recover_project_transactions_read_only(root)?;
    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();
    builder.set_bin_targets(cargo_bin_targets(root));
    let sources = collect_sources_with_config(root, config)?;
    let mut contents = Vec::with_capacity(sources.len());
    for (absolute, relative) in &sources {
        let text = std::fs::read_to_string(absolute).map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("failed to read source {relative}: {error}"),
            )
        })?;
        contents.push((relative.clone(), text));
    }
    builder.load_files(
        &mut graph,
        contents
            .iter()
            .map(|(relative, text)| (relative.as_str(), text.as_str())),
    );
    Ok((graph, builder, sources.len()))
}

pub(crate) fn save_graph(
    root: &Path,
    config: &ProjectConfig,
    graph: &SemanticGraph,
) -> std::io::Result<PathBuf> {
    let output = safe_project_output_path(root, &config.graph.path)?;
    let bytes = encode_graph(&output, graph)?;

    atomic_write(&output, &bytes)?;
    Ok(output)
}

pub(crate) fn graph_project_write(
    root: &Path,
    config: &ProjectConfig,
    graph: &SemanticGraph,
    expected: Option<Vec<u8>>,
) -> std::io::Result<ProjectWrite> {
    let output = safe_project_input_path(root, &config.graph.path)?;
    let contents = encode_graph(&output, graph)?;
    Ok(ProjectWrite::bytes(
        PathBuf::from(&config.graph.path),
        expected,
        contents,
    ))
}

pub(crate) fn load_graph_snapshot(
    root: &Path,
    config: &ProjectConfig,
) -> std::io::Result<GraphSnapshot> {
    let path = safe_project_input_path(root, &config.graph.path)?;
    let bytes = read_optional_bytes(&path)?;
    let Some(bytes) = bytes else {
        return Ok(GraphSnapshot {
            graph: None,
            bytes: None,
            error: None,
        });
    };
    let decoded = if path.extension().and_then(|extension| extension.to_str()) == Some("aetherb") {
        SemanticGraph::from_bytes(&bytes)
    } else {
        std::str::from_utf8(&bytes)
            .map_err(|error| aether_graph::GraphError::Deserialize(error.to_string()))
            .and_then(SemanticGraph::from_ron)
    };
    match decoded {
        Ok(graph) => Ok(GraphSnapshot {
            graph: Some(graph),
            bytes: Some(bytes),
            error: None,
        }),
        Err(error) => Ok(GraphSnapshot {
            graph: None,
            bytes: Some(bytes),
            error: Some(error.to_string()),
        }),
    }
}

/// Rebuild source projections and merge them with the validated durable graph.
///
/// Commands that mutate graph-native state use this strict path so corrupt
/// persistence cannot be silently replaced and agent/extension-owned nodes are
/// not lost by a source-only rebuild.
pub(crate) fn load_reconciled_graph(
    root: &Path,
    config: &ProjectConfig,
) -> std::io::Result<(SemanticGraph, Option<Vec<u8>>, usize)> {
    let (source, _, files) = build_from_dir_with_config(root, config)?;
    let persisted = load_graph_snapshot(root, config)?;
    if let Some(error) = persisted.error {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("persisted graph is invalid; refusing graph changes: {error}"),
        ));
    }
    let graph = match persisted.graph {
        Some(durable) => SemanticGraph::reconcile_persisted(source, &durable).0,
        None => source,
    };
    Ok((graph, persisted.bytes, files))
}

fn encode_graph(path: &Path, graph: &SemanticGraph) -> std::io::Result<Vec<u8>> {
    if path.extension().and_then(|extension| extension.to_str()) == Some("aetherb") {
        graph
            .to_bytes()
            .map_err(|error| std::io::Error::other(error.to_string()))
    } else {
        graph
            .to_ron()
            .map(String::into_bytes)
            .map_err(|error| std::io::Error::other(error.to_string()))
    }
}

pub(crate) fn read_project_bytes(
    root: &Path,
    relative: impl AsRef<Path>,
) -> std::io::Result<Option<Vec<u8>>> {
    let path = safe_project_input_path(root, relative)?;
    read_optional_bytes(&path)
}

pub(crate) fn read_project_bytes_bounded(
    root: &Path,
    relative: impl AsRef<Path>,
    max_bytes: usize,
) -> std::io::Result<Option<Vec<u8>>> {
    let path = safe_project_input_path(root, relative)?;
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let limit = u64::try_from(max_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut bytes = Vec::with_capacity(max_bytes.min(64 * 1024));
    file.take(limit).read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("project file {} exceeds {max_bytes} bytes", path.display()),
        ));
    }
    Ok(Some(bytes))
}

fn read_optional_bytes(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Commit source projections and graph state as one recoverable transaction.
///
/// Every candidate is staged before any target is replaced. A durable manifest
/// and backups let startup roll back a crash that happens before the committed
/// marker is synced.
pub(crate) fn commit_project_writes(
    root: &Path,
    writes: Vec<ProjectWrite>,
) -> std::io::Result<Vec<PathBuf>> {
    if writes.is_empty() {
        return Ok(Vec::new());
    }
    // Hold the exclusive journal lock for the entire critical section:
    // recovery, baseline checks, staging, renames, marker, and cleanup.
    // Concurrent writers serialize here and read-only analysis skips
    // recovery instead of rolling back this in-flight journal.
    let _journal_lock = JournalLock::create_and_acquire(root)?;
    let result = recover_transactions_locked(root, RecoveryScope::All)
        .and_then(|_| commit_project_writes_locked(root, writes));
    if result.is_err() {
        let _ = cleanup_empty_transaction_roots(root);
    }
    result
}

fn commit_project_writes_locked(
    root: &Path,
    writes: Vec<ProjectWrite>,
) -> std::io::Result<Vec<PathBuf>> {
    let mut targets = BTreeSet::new();
    let mut prepared = Vec::new();
    for write in writes {
        let target = safe_project_output_path(root, &write.relative)?;
        if !targets.insert(target.clone()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "transaction contains duplicate target {}",
                    write.relative.display()
                ),
            ));
        }
        let current = read_optional_bytes(&target)?;
        if current != write.expected {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "{} changed on disk; reload before committing",
                    write.relative.display()
                ),
            ));
        }
        if current.as_deref() == write.contents.as_deref() {
            continue;
        }
        prepared.push((write, target, current));
    }
    if prepared.is_empty() {
        cleanup_empty_transaction_roots(root)?;
        return Ok(Vec::new());
    }

    let transaction_id = format!(
        "{}-{}",
        std::process::id(),
        NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
    );
    let manifest_path = safe_project_output_path(
        root,
        Path::new(TRANSACTION_ROOT)
            .join(&transaction_id)
            .join(TRANSACTION_MANIFEST),
    )?;
    let transaction_dir = manifest_path
        .parent()
        .ok_or_else(|| std::io::Error::other("transaction manifest has no parent"))?
        .to_path_buf();

    let mut entries = Vec::with_capacity(prepared.len());
    for (index, (write, target, current)) in prepared.iter().enumerate() {
        let staged = write.contents.as_ref().map(|_| format!("{index}.staged"));
        let backup = format!("{index}.backup");
        if let (Some(staged), Some(contents)) = (&staged, &write.contents) {
            write_synced_file(
                &transaction_dir.join(staged),
                contents,
                std::fs::metadata(target)
                    .ok()
                    .map(|metadata| metadata.permissions()),
            )?;
        }
        if let Some(bytes) = current {
            write_synced_file(&transaction_dir.join(&backup), bytes, None)?;
        }
        entries.push(TransactionEntry {
            relative: write.relative.to_string_lossy().replace('\\', "/"),
            had_original: current.is_some(),
            staged,
            backup,
        });
    }

    maybe_fault_exit("after-staging");
    let manifest = TransactionManifest {
        version: 1,
        entries,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    atomic_write(&manifest_path, &manifest_bytes)?;
    sync_dir(&transaction_dir)?;
    maybe_fault_exit("after-manifest");

    let commit_result: std::io::Result<()> = (|| {
        for (index, entry) in manifest.entries.iter().enumerate() {
            let target = safe_project_output_path(root, &entry.relative)?;
            if let Some(staged) = &entry.staged {
                std::fs::rename(transaction_member(&transaction_dir, staged)?, &target)?;
            } else {
                std::fs::remove_file(&target)?;
            }
            if let Some(parent) = target.parent() {
                sync_dir(parent)?;
            }
            if index == 0 {
                maybe_fault_exit("mid-apply");
            }
        }
        write_synced_file(
            &transaction_dir.join(TRANSACTION_COMMITTED),
            b"committed\n",
            None,
        )?;
        sync_dir(&transaction_dir)
    })();

    if let Err(commit_error) = commit_result {
        if let Err(rollback_error) = rollback_transaction(root, &transaction_dir, &manifest) {
            return Err(std::io::Error::other(format!(
                "transaction failed: {commit_error}; rollback also failed: {rollback_error}"
            )));
        }
        cleanup_transaction(root, &transaction_dir)?;
        return Err(commit_error);
    }

    maybe_fault_exit("pre-cleanup");
    let written = prepared
        .iter()
        .map(|(write, _, _)| write.relative.clone())
        .collect();
    // The durable marker is the commit point. Cleanup is best-effort here;
    // startup removes any committed journal left behind by interruption.
    let _ = cleanup_transaction(root, &transaction_dir);
    Ok(written)
}

/// Crash injection for recovery testing: aborts the process at a named
/// transaction transition when `GIRDER_FAULT_EXIT` names it. Inert unless
/// that variable is set, so production behavior is unchanged.
fn maybe_fault_exit(point: &str) {
    if std::env::var("GIRDER_FAULT_EXIT").is_ok_and(|value| value == point) {
        std::process::exit(87);
    }
}

/// Check every transaction baseline without writing anything.
///
/// Validation runs against a candidate copy first; the real commit repeats this
/// check so an external edit made during validation still blocks replacement.
pub(crate) fn verify_project_writes(root: &Path, writes: &[ProjectWrite]) -> std::io::Result<()> {
    let mut targets = BTreeSet::new();
    for write in writes {
        let target = safe_project_input_path(root, &write.relative)?;
        if !targets.insert(target) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "transaction contains duplicate target {}",
                    write.relative.display()
                ),
            ));
        }
        if read_project_bytes(root, &write.relative)? != write.expected {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "{} changed on disk; reload before validating",
                    write.relative.display()
                ),
            ));
        }
    }
    Ok(())
}

/// Recover transactions interrupted before their committed marker was synced.
///
/// Takes the exclusive journal lock, waiting for any active writer; every
/// uncommitted journal found under the lock is orphaned and rolled back.
/// Headless CLI writers recover through `commit_project_writes`, which locks
/// internally; this entry point serves workspace open and tests.
#[cfg(any(feature = "gui", test))]
pub(crate) fn recover_project_transactions(root: &Path) -> std::io::Result<usize> {
    let Some(_lock) = JournalLock::acquire(root, LockWait::Block)? else {
        return Ok(0);
    };
    recover_transactions_locked(root, RecoveryScope::All)
}

/// Recovery for read-only analysis commands.
///
/// Never blocks behind a writer (contention means the journal is live) and
/// never rolls back a journal whose owning process is still running, so a
/// concurrent `review`/`test-impact` cannot destroy an in-flight commit.
pub(crate) fn recover_project_transactions_read_only(root: &Path) -> std::io::Result<usize> {
    let Some(_lock) = JournalLock::acquire(root, LockWait::NonBlock)? else {
        return Ok(0);
    };
    recover_transactions_locked(root, RecoveryScope::DeadOwnersOnly)
}

fn recover_transactions_locked(root: &Path, scope: RecoveryScope) -> std::io::Result<usize> {
    let transaction_root = safe_project_input_path(root, TRANSACTION_ROOT)?;
    let mut dirs = match std::fs::read_dir(&transaction_root) {
        Ok(entries) => entries.collect::<Result<Vec<_>, _>>()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    dirs.sort_by_key(|entry| entry.file_name());

    let mut recovered = 0;
    let mut skipped_live = false;
    for entry in dirs {
        let file_type = entry.file_type()?;
        if file_type.is_symlink() || !file_type.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "invalid project transaction entry: {}",
                    entry.path().display()
                ),
            ));
        }
        let transaction_dir = entry.path();
        let manifest_path = transaction_dir.join(TRANSACTION_MANIFEST);
        let manifest_bytes = match std::fs::read(&manifest_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if scope == RecoveryScope::DeadOwnersOnly
                    && journal_owner_is_alive(&entry.file_name())
                {
                    skipped_live = true;
                    continue;
                }
                std::fs::remove_dir_all(&transaction_dir)?;
                recovered += 1;
                continue;
            }
            Err(error) => return Err(error),
        };
        let manifest: TransactionManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid transaction manifest: {error}"),
                )
            })?;
        if manifest.version != 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unsupported transaction version {}", manifest.version),
            ));
        }
        if !transaction_dir.join(TRANSACTION_COMMITTED).exists() {
            if scope == RecoveryScope::DeadOwnersOnly && journal_owner_is_alive(&entry.file_name())
            {
                skipped_live = true;
                continue;
            }
            rollback_transaction(root, &transaction_dir, &manifest)?;
            recovered += 1;
        }
        cleanup_transaction(root, &transaction_dir)?;
    }
    if !skipped_live {
        cleanup_empty_transaction_roots(root)?;
    }
    Ok(recovered)
}

fn rollback_transaction(
    root: &Path,
    transaction_dir: &Path,
    manifest: &TransactionManifest,
) -> std::io::Result<()> {
    for entry in &manifest.entries {
        let target = safe_project_output_path(root, &entry.relative)?;
        if entry.had_original {
            let backup = transaction_member(transaction_dir, &entry.backup)?;
            atomic_write(&target, &std::fs::read(backup)?)?;
        } else {
            match std::fs::remove_file(&target) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

fn transaction_member(transaction_dir: &Path, name: &str) -> std::io::Result<PathBuf> {
    let path = Path::new(name);
    if path.components().count() != 1
        || !matches!(
            path.components().next(),
            Some(std::path::Component::Normal(_))
        )
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid transaction member {name:?}"),
        ));
    }
    Ok(transaction_dir.join(path))
}

fn cleanup_transaction(root: &Path, transaction_dir: &Path) -> std::io::Result<()> {
    std::fs::remove_dir_all(transaction_dir)?;
    cleanup_empty_transaction_roots(root)
}

fn cleanup_empty_transaction_roots(root: &Path) -> std::io::Result<()> {
    let transactions = root.join(TRANSACTION_ROOT);
    match std::fs::remove_dir(&transactions) {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
            ) => {}
        Err(error) => return Err(error),
    }
    let girder = root.join(".girder");
    match std::fs::remove_dir(&girder) {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
            ) => {}
        Err(error) => return Err(error),
    }
    Ok(())
}

fn write_synced_file(
    path: &Path,
    bytes: &[u8],
    permissions: Option<std::fs::Permissions>,
) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    if let Some(permissions) = permissions {
        file.set_permissions(permissions)?;
    }
    file.sync_all()
}

fn sync_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn atomic_write(output: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = output.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("output path has no parent: {}", output.display()),
        )
    })?;
    let file_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("girder-output");
    let temp = parent.join(format!(
        ".{file_name}.girder-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let permissions = std::fs::metadata(output)
        .ok()
        .map(|metadata| metadata.permissions());

    let result: std::io::Result<()> = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        file.sync_all()?;
        std::fs::rename(&temp, output)?;
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result?;
    Ok(())
}

pub(crate) fn safe_project_output_path(
    root: &Path,
    relative: impl AsRef<Path>,
) -> std::io::Result<PathBuf> {
    safe_project_path(root, relative.as_ref(), true)
}

pub(crate) fn safe_project_input_path(
    root: &Path,
    relative: impl AsRef<Path>,
) -> std::io::Result<PathBuf> {
    safe_project_path(root, relative.as_ref(), false)
}

fn safe_project_path(
    root: &Path,
    relative: &Path,
    create_parents: bool,
) -> std::io::Result<PathBuf> {
    let canonical_root = std::fs::canonicalize(root)?;
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "output path must stay inside the project: {}",
                relative.display()
            ),
        ));
    }

    let mut current = canonical_root.clone();
    let mut parent_missing = false;
    if let Some(parent) = relative.parent() {
        for component in parent.components() {
            if matches!(component, std::path::Component::CurDir) {
                continue;
            }
            current.push(component.as_os_str());
            if parent_missing {
                if create_parents {
                    std::fs::create_dir(&current)?;
                }
                continue;
            }
            match std::fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        format!(
                            "output directory must not be a symlink: {}",
                            current.display()
                        ),
                    ));
                }
                Ok(metadata) if !metadata.is_dir() => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::NotADirectory,
                        format!("output parent is not a directory: {}", current.display()),
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if create_parents {
                        std::fs::create_dir(&current)?;
                    } else {
                        parent_missing = true;
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }

    let output = canonical_root.join(relative);
    if let Ok(metadata) = std::fs::symlink_metadata(&output) {
        if metadata.file_type().is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("output file must not be a symlink: {}", output.display()),
            ));
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "girder-source-{name}-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn loads_configured_roots_and_excludes() {
        let dir = TempDir::new("roots");
        std::fs::create_dir_all(dir.0.join("src")).unwrap();
        std::fs::create_dir_all(dir.0.join("vendor")).unwrap();
        std::fs::write(dir.0.join("src/lib.rs"), "fn keep() {}\n").unwrap();
        std::fs::write(dir.0.join("src/legacy.js"), "function keep() {}\n").unwrap();
        std::fs::write(dir.0.join("src/generated.rs"), "fn skip() {}\n").unwrap();
        std::fs::write(dir.0.join("vendor/third_party.rs"), "fn vendor() {}\n").unwrap();

        let mut config = ProjectConfig::default();
        config.source.roots = vec!["src".into()];
        config.source.exclude = vec!["src/generated.rs".into()];
        let sources = collect_sources_with_config(&dir.0, &config).unwrap();

        assert_eq!(
            sources
                .iter()
                .map(|(_, relative)| relative.as_str())
                .collect::<Vec<_>>(),
            ["src/lib.rs"]
        );
        assert_eq!(
            collect_project_files_with_config(&dir.0, &config)
                .unwrap()
                .into_iter()
                .map(|(_, relative)| relative)
                .collect::<Vec<_>>(),
            vec!["src/legacy.js".to_string(), "src/lib.rs".to_string()]
        );
    }

    #[test]
    fn configured_source_filter_matches_roots_and_excludes() {
        let mut config = ProjectConfig::default();
        config.source.roots = vec!["src".into(), "tests".into()];
        config.source.exclude = vec!["src/generated/**".into()];

        assert!(is_configured_source_path(&config, "src/lib.rs").unwrap());
        assert!(is_configured_source_path(&config, "tests/cli.rs").unwrap());
        assert!(!is_configured_source_path(&config, "examples/demo.rs").unwrap());
        assert!(!is_configured_source_path(&config, "src/generated/api.rs").unwrap());
    }

    #[test]
    fn loads_a_directory_and_resolves_across_files() {
        let dir = TempDir::new("cross-file");
        std::fs::create_dir_all(dir.0.join("src/util")).unwrap();
        std::fs::write(
            dir.0.join("src/util/math.rs"),
            "fn double(x: i64) -> i64 { x + x }\n",
        )
        .unwrap();
        std::fs::write(dir.0.join("src/app.rs"), "fn run() -> i64 { double(21) }\n").unwrap();

        let (graph, _builder, files) = build_from_dir(&dir.0).unwrap();
        assert_eq!(files, 2);
        assert!(graph.find_by_path("crate::util::math::double").is_some());
        assert!(graph.find_by_path("crate::app::run").is_some());
        let run = aether_graph::NodeId::from_path("crate::app::run");
        let double = aether_graph::NodeId::from_path("crate::util::math::double");
        let calls: Vec<_> = graph
            .neighbors(run, Some(aether_graph::EdgeKind::Calls))
            .into_iter()
            .map(|node| node.id)
            .collect();
        assert!(calls.contains(&double));
    }

    #[test]
    fn directory_build_rejects_invalid_utf8_instead_of_returning_a_partial_graph() {
        let dir = TempDir::new("invalid-utf8");
        std::fs::create_dir_all(dir.0.join("src")).unwrap();
        std::fs::write(dir.0.join("src/lib.rs"), "fn readable() {}\n").unwrap();
        std::fs::write(dir.0.join("src/invalid.rs"), b"fn invalid() {}\n\xff\n").unwrap();

        let error = match build_from_dir(&dir.0) {
            Ok(_) => panic!("invalid UTF-8 source unexpectedly produced a graph"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(
            error
                .to_string()
                .contains("failed to read source src/invalid.rs"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn followed_symlinks_cannot_escape_the_project() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new("symlink");
        let outside = TempDir::new("outside");
        std::fs::write(outside.0.join("secret.rs"), "fn secret() {}\n").unwrap();
        symlink(&outside.0, dir.0.join("linked")).unwrap();

        let mut config = ProjectConfig::default();
        config.source.follow_symlinks = true;
        let error = collect_sources_with_config(&dir.0, &config).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn graph_save_creates_configured_parent_and_round_trips() {
        let dir = TempDir::new("graph-save");
        let mut config = ProjectConfig::default();
        config.graph.path = ".girder/semantic.aether".into();
        let mut graph = SemanticGraph::new();
        graph.upsert_node(aether_graph::Node::new(
            aether_graph::NodeKind::Function,
            "work",
            "crate::work",
        ));

        let output = save_graph(&dir.0, &config, &graph).unwrap();
        let loaded = SemanticGraph::load(&output).unwrap();

        assert_eq!(output, dir.0.join(".girder/semantic.aether"));
        assert!(loaded.find_by_path("crate::work").is_some());
    }

    #[cfg(unix)]
    #[test]
    fn graph_output_cannot_follow_a_directory_symlink() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new("graph-symlink");
        let outside = TempDir::new("graph-outside");
        symlink(&outside.0, dir.0.join("artifacts")).unwrap();
        let mut config = ProjectConfig::default();
        config.graph.path = "artifacts/project.aether".into();

        let error = save_graph(&dir.0, &config, &SemanticGraph::new()).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(!outside.0.join("project.aether").exists());
    }

    #[test]
    fn project_writes_commit_as_a_batch_and_clean_the_journal() {
        let dir = TempDir::new("transaction");
        std::fs::create_dir_all(dir.0.join("src")).unwrap();
        std::fs::write(dir.0.join("src/lib.rs"), b"old\n").unwrap();

        let written = commit_project_writes(
            &dir.0,
            vec![
                ProjectWrite::bytes("src/lib.rs", Some(b"old\n".to_vec()), b"new\n".to_vec()),
                ProjectWrite::bytes("project.aether", None, b"graph\n".to_vec()),
            ],
        )
        .unwrap();

        assert_eq!(
            written,
            [PathBuf::from("src/lib.rs"), PathBuf::from("project.aether")]
        );
        assert_eq!(std::fs::read(dir.0.join("src/lib.rs")).unwrap(), b"new\n");
        assert_eq!(
            std::fs::read(dir.0.join("project.aether")).unwrap(),
            b"graph\n"
        );
        assert!(!dir.0.join(".girder").exists());
    }

    #[test]
    fn project_writes_delete_as_part_of_the_atomic_batch() {
        let dir = TempDir::new("transaction-delete");
        std::fs::write(dir.0.join("remove.rs"), b"generated\n").unwrap();
        std::fs::write(dir.0.join("keep.rs"), b"old\n").unwrap();

        let written = commit_project_writes(
            &dir.0,
            vec![
                ProjectWrite::delete("remove.rs", b"generated\n".to_vec()),
                ProjectWrite::bytes("keep.rs", Some(b"old\n".to_vec()), b"new\n".to_vec()),
            ],
        )
        .unwrap();

        assert_eq!(
            written,
            [PathBuf::from("remove.rs"), PathBuf::from("keep.rs")]
        );
        assert!(!dir.0.join("remove.rs").exists());
        assert_eq!(std::fs::read(dir.0.join("keep.rs")).unwrap(), b"new\n");
        assert!(!dir.0.join(".girder").exists());
    }

    #[test]
    fn project_writes_reject_stale_inputs_before_replacing_anything() {
        let dir = TempDir::new("transaction-conflict");
        std::fs::write(dir.0.join("a.rs"), b"external\n").unwrap();

        let error = commit_project_writes(
            &dir.0,
            vec![
                ProjectWrite::bytes("a.rs", Some(b"old\n".to_vec()), b"local\n".to_vec()),
                ProjectWrite::bytes("b.rs", None, b"new\n".to_vec()),
            ],
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(dir.0.join("a.rs")).unwrap(), b"external\n");
        assert!(!dir.0.join("b.rs").exists());
        assert!(!dir.0.join(".girder").exists());
    }

    #[test]
    fn startup_rolls_back_an_interrupted_transaction() {
        let dir = TempDir::new("transaction-recovery");
        let transaction_dir = dir.0.join(TRANSACTION_ROOT).join("interrupted");
        std::fs::create_dir_all(&transaction_dir).unwrap();
        std::fs::write(dir.0.join("existing.rs"), b"partially committed\n").unwrap();
        std::fs::write(dir.0.join("created.rs"), b"partially created\n").unwrap();
        std::fs::write(transaction_dir.join("0.backup"), b"original\n").unwrap();
        std::fs::write(transaction_dir.join("0.staged"), b"candidate\n").unwrap();
        std::fs::write(transaction_dir.join("1.staged"), b"candidate\n").unwrap();
        let manifest = TransactionManifest {
            version: 1,
            entries: vec![
                TransactionEntry {
                    relative: "existing.rs".into(),
                    had_original: true,
                    staged: Some("0.staged".into()),
                    backup: "0.backup".into(),
                },
                TransactionEntry {
                    relative: "created.rs".into(),
                    had_original: false,
                    staged: Some("1.staged".into()),
                    backup: "1.backup".into(),
                },
            ],
        };
        std::fs::write(
            transaction_dir.join(TRANSACTION_MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        assert_eq!(recover_project_transactions(&dir.0).unwrap(), 1);
        assert_eq!(
            std::fs::read(dir.0.join("existing.rs")).unwrap(),
            b"original\n"
        );
        assert!(!dir.0.join("created.rs").exists());
        assert!(!dir.0.join(".girder").exists());
    }

    #[test]
    fn startup_restores_a_file_deleted_by_an_interrupted_transaction() {
        let dir = TempDir::new("transaction-delete-recovery");
        let transaction_dir = dir.0.join(TRANSACTION_ROOT).join("interrupted");
        std::fs::create_dir_all(&transaction_dir).unwrap();
        std::fs::write(transaction_dir.join("0.backup"), b"original\n").unwrap();
        let manifest = TransactionManifest {
            version: 1,
            entries: vec![TransactionEntry {
                relative: "deleted.rs".into(),
                had_original: true,
                staged: None,
                backup: "0.backup".into(),
            }],
        };
        std::fs::write(
            transaction_dir.join(TRANSACTION_MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        assert_eq!(recover_project_transactions(&dir.0).unwrap(), 1);
        assert_eq!(
            std::fs::read(dir.0.join("deleted.rs")).unwrap(),
            b"original\n"
        );
        assert!(!dir.0.join(".girder").exists());
    }

    #[test]
    fn cargo_bin_targets_reads_exact_manifest_path_overrides() {
        let dir = TempDir::new("bin-targets");
        std::fs::write(
            dir.0.join("Cargo.toml"),
            br#"
[package]
name = "demo"
version = "0.1.0"

[[bin]]
name = "trust-custom"
path = "tools/entry.rs"

[[bin]]
name = "no-path-declared"

[[bin]]
path = "tools/anonymous.rs"
"#,
        )
        .unwrap();

        let targets = cargo_bin_targets(&dir.0);
        assert_eq!(
            targets.get("tools/entry.rs").map(String::as_str),
            Some("trust-custom")
        );
        assert_eq!(
            targets.get("src/main.rs").map(String::as_str),
            Some("demo"),
            "the package name backs the conventional src/main.rs target"
        );
        assert_eq!(
            targets.len(),
            2,
            "entries missing name or path contribute nothing"
        );
    }

    #[test]
    fn cargo_bin_targets_prefers_an_explicit_override_for_src_main_rs() {
        let dir = TempDir::new("bin-targets-main-override");
        std::fs::write(
            dir.0.join("Cargo.toml"),
            br#"
[package]
name = "demo"
version = "0.1.0"

[[bin]]
name = "renamed-main"
path = "src/main.rs"
"#,
        )
        .unwrap();

        let targets = cargo_bin_targets(&dir.0);
        assert_eq!(
            targets.get("src/main.rs").map(String::as_str),
            Some("renamed-main"),
            "an explicit override for src/main.rs must win over the package name"
        );
    }

    #[test]
    fn cargo_bin_targets_is_empty_without_a_readable_manifest() {
        let dir = TempDir::new("bin-targets-missing");
        assert!(cargo_bin_targets(&dir.0).is_empty());

        std::fs::write(dir.0.join("Cargo.toml"), b"not valid toml =").unwrap();
        assert!(cargo_bin_targets(&dir.0).is_empty());
    }

    #[test]
    fn directory_build_resolves_a_custom_binary_path_via_the_manifest() {
        let dir = TempDir::new("build-custom-bin");
        std::fs::write(
            dir.0.join("Cargo.toml"),
            br#"
[package]
name = "demo"
version = "0.1.0"

[[bin]]
name = "trust-custom"
path = "tools/entry.rs"
"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.0.join("tools")).unwrap();
        std::fs::write(dir.0.join("tools/entry.rs"), b"fn main() {}\n").unwrap();
        std::fs::create_dir_all(dir.0.join("tests")).unwrap();
        std::fs::write(
            dir.0.join("tests/cli.rs"),
            br#"
use std::process::Command;

#[test]
fn cli_route() {
    Command::new(env!("CARGO_BIN_EXE_trust-custom")).output().unwrap();
}
"#,
        )
        .unwrap();

        let (graph, _, _) = build_from_dir(&dir.0).unwrap();
        let main = aether_graph::NodeId::from_path("crate::tools::entry::main");
        assert_eq!(
            graph.tests_for(main).len(),
            1,
            "the project loader must apply the manifest's exact bin path"
        );
    }

    #[test]
    fn directory_build_recovers_before_parsing_sources() {
        let dir = TempDir::new("build-recovery");
        std::fs::create_dir_all(dir.0.join("src")).unwrap();
        std::fs::write(dir.0.join("src/lib.rs"), b"fn partial() {}\n").unwrap();
        let transaction_dir = dir.0.join(TRANSACTION_ROOT).join("interrupted");
        std::fs::create_dir_all(&transaction_dir).unwrap();
        std::fs::write(transaction_dir.join("0.backup"), b"fn original() {}\n").unwrap();
        std::fs::write(transaction_dir.join("0.staged"), b"fn candidate() {}\n").unwrap();
        let manifest = TransactionManifest {
            version: 1,
            entries: vec![TransactionEntry {
                relative: "src/lib.rs".into(),
                had_original: true,
                staged: Some("0.staged".into()),
                backup: "0.backup".into(),
            }],
        };
        std::fs::write(
            transaction_dir.join(TRANSACTION_MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let (graph, _, _) = build_from_dir(&dir.0).unwrap();

        assert!(graph.find_by_path("crate::lib::original").is_some());
        assert!(graph.find_by_path("crate::lib::partial").is_none());
        assert!(!dir.0.join(".girder").exists());
    }

    #[test]
    fn startup_keeps_a_committed_transaction_and_cleans_its_journal() {
        let dir = TempDir::new("transaction-committed");
        let transaction_dir = dir.0.join(TRANSACTION_ROOT).join("committed");
        std::fs::create_dir_all(&transaction_dir).unwrap();
        std::fs::write(dir.0.join("file.rs"), b"committed\n").unwrap();
        std::fs::write(transaction_dir.join("0.backup"), b"old\n").unwrap();
        let manifest = TransactionManifest {
            version: 1,
            entries: vec![TransactionEntry {
                relative: "file.rs".into(),
                had_original: true,
                staged: Some("0.staged".into()),
                backup: "0.backup".into(),
            }],
        };
        std::fs::write(
            transaction_dir.join(TRANSACTION_MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(transaction_dir.join(TRANSACTION_COMMITTED), b"committed\n").unwrap();

        assert_eq!(recover_project_transactions(&dir.0).unwrap(), 0);
        assert_eq!(
            std::fs::read(dir.0.join("file.rs")).unwrap(),
            b"committed\n"
        );
        assert!(!dir.0.join(".girder").exists());
    }

    fn write_interrupted_journal(root: &Path, name: &str) {
        let transaction_dir = root.join(TRANSACTION_ROOT).join(name);
        std::fs::create_dir_all(&transaction_dir).unwrap();
        std::fs::write(root.join("existing.rs"), b"partially committed\n").unwrap();
        std::fs::write(transaction_dir.join("0.backup"), b"original\n").unwrap();
        std::fs::write(transaction_dir.join("0.staged"), b"candidate\n").unwrap();
        let manifest = TransactionManifest {
            version: 1,
            entries: vec![TransactionEntry {
                relative: "existing.rs".into(),
                had_original: true,
                staged: Some("0.staged".into()),
                backup: "0.backup".into(),
            }],
        };
        std::fs::write(
            transaction_dir.join(TRANSACTION_MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn recovery_removes_an_empty_journal_directory() {
        let dir = TempDir::new("empty-journal");
        std::fs::create_dir_all(dir.0.join(TRANSACTION_ROOT).join("interrupted")).unwrap();

        assert_eq!(recover_project_transactions(&dir.0).unwrap(), 1);
        assert!(!dir.0.join(".girder").exists());
        // A second pass finds nothing: recovery is idempotent.
        assert_eq!(recover_project_transactions(&dir.0).unwrap(), 0);
    }

    #[test]
    fn recovery_fails_closed_on_a_torn_manifest() {
        let dir = TempDir::new("torn-manifest");
        let transaction_dir = dir.0.join(TRANSACTION_ROOT).join("interrupted");
        std::fs::create_dir_all(&transaction_dir).unwrap();
        std::fs::write(dir.0.join("existing.rs"), b"partially committed\n").unwrap();
        std::fs::write(transaction_dir.join("0.backup"), b"original\n").unwrap();
        std::fs::write(transaction_dir.join(TRANSACTION_MANIFEST), b"{ torn").unwrap();

        let error = recover_project_transactions(&dir.0).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        // Nothing was modified and the evidence is preserved for inspection.
        assert_eq!(
            std::fs::read(dir.0.join("existing.rs")).unwrap(),
            b"partially committed\n"
        );
        assert!(transaction_dir.join("0.backup").exists());
        // The failure is stable, not destructive, on retry.
        let retry = recover_project_transactions(&dir.0).unwrap_err();
        assert_eq!(retry.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn recovery_rolls_back_a_partially_applied_transaction() {
        let dir = TempDir::new("partial-apply");
        let transaction_dir = dir.0.join(TRANSACTION_ROOT).join("interrupted");
        std::fs::create_dir_all(&transaction_dir).unwrap();
        // Entry 0 was already renamed into place (its staged file is gone);
        // entry 1 never applied and still has its staged candidate.
        std::fs::write(dir.0.join("first.rs"), b"candidate one\n").unwrap();
        std::fs::write(dir.0.join("second.rs"), b"original two\n").unwrap();
        std::fs::write(transaction_dir.join("0.backup"), b"original one\n").unwrap();
        std::fs::write(transaction_dir.join("1.backup"), b"original two\n").unwrap();
        std::fs::write(transaction_dir.join("1.staged"), b"candidate two\n").unwrap();
        let manifest = TransactionManifest {
            version: 1,
            entries: vec![
                TransactionEntry {
                    relative: "first.rs".into(),
                    had_original: true,
                    staged: Some("0.staged".into()),
                    backup: "0.backup".into(),
                },
                TransactionEntry {
                    relative: "second.rs".into(),
                    had_original: true,
                    staged: Some("1.staged".into()),
                    backup: "1.backup".into(),
                },
            ],
        };
        std::fs::write(
            transaction_dir.join(TRANSACTION_MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        assert_eq!(recover_project_transactions(&dir.0).unwrap(), 1);
        assert_eq!(
            std::fs::read(dir.0.join("first.rs")).unwrap(),
            b"original one\n"
        );
        assert_eq!(
            std::fs::read(dir.0.join("second.rs")).unwrap(),
            b"original two\n"
        );
        assert!(!dir.0.join(".girder").exists());
        assert_eq!(recover_project_transactions(&dir.0).unwrap(), 0);
    }

    #[test]
    fn recovery_rolls_back_a_fully_applied_uncommitted_transaction() {
        let dir = TempDir::new("applied-uncommitted");
        let transaction_dir = dir.0.join(TRANSACTION_ROOT).join("interrupted");
        std::fs::create_dir_all(&transaction_dir).unwrap();
        // Every rename landed but the COMMITTED marker never synced: the
        // marker is the commit point, so this must still become all-old.
        std::fs::write(dir.0.join("replaced.rs"), b"candidate\n").unwrap();
        std::fs::write(dir.0.join("created.rs"), b"created\n").unwrap();
        std::fs::write(transaction_dir.join("0.backup"), b"original\n").unwrap();
        let manifest = TransactionManifest {
            version: 1,
            entries: vec![
                TransactionEntry {
                    relative: "replaced.rs".into(),
                    had_original: true,
                    staged: Some("0.staged".into()),
                    backup: "0.backup".into(),
                },
                TransactionEntry {
                    relative: "created.rs".into(),
                    had_original: false,
                    staged: Some("1.staged".into()),
                    backup: "1.backup".into(),
                },
            ],
        };
        std::fs::write(
            transaction_dir.join(TRANSACTION_MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        assert_eq!(recover_project_transactions(&dir.0).unwrap(), 1);
        assert_eq!(
            std::fs::read(dir.0.join("replaced.rs")).unwrap(),
            b"original\n"
        );
        assert!(!dir.0.join("created.rs").exists());
        assert!(!dir.0.join(".girder").exists());
        assert_eq!(recover_project_transactions(&dir.0).unwrap(), 0);
    }

    #[test]
    fn read_only_recovery_preserves_a_live_writers_journal() {
        let dir = TempDir::new("live-journal");
        let mut writer = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let journal = format!("{}-0", writer.id());
        write_interrupted_journal(&dir.0, &journal);

        assert_eq!(recover_project_transactions_read_only(&dir.0).unwrap(), 0);
        assert!(dir.0.join(TRANSACTION_ROOT).join(&journal).exists());
        assert_eq!(
            std::fs::read(dir.0.join("existing.rs")).unwrap(),
            b"partially committed\n"
        );

        // Exclusive recovery reclaims the journal regardless of liveness:
        // under the lock no writer can be mid-commit.
        assert_eq!(recover_project_transactions(&dir.0).unwrap(), 1);
        assert_eq!(
            std::fs::read(dir.0.join("existing.rs")).unwrap(),
            b"original\n"
        );
        assert!(!dir.0.join(".girder").exists());

        let _ = writer.kill();
        let _ = writer.wait();
    }

    #[test]
    fn read_only_recovery_reclaims_a_dead_owners_journal() {
        let dir = TempDir::new("dead-journal");
        let mut exited = std::process::Command::new("true").spawn().unwrap();
        let dead_pid = exited.id();
        exited.wait().unwrap();
        write_interrupted_journal(&dir.0, &format!("{dead_pid}-0"));

        assert_eq!(recover_project_transactions_read_only(&dir.0).unwrap(), 1);
        assert_eq!(
            std::fs::read(dir.0.join("existing.rs")).unwrap(),
            b"original\n"
        );
        assert!(!dir.0.join(".girder").exists());
    }

    #[test]
    fn read_only_recovery_skips_while_the_journal_lock_is_held() {
        let dir = TempDir::new("locked-journal");
        write_interrupted_journal(&dir.0, "interrupted");

        let lock = JournalLock::acquire(&dir.0, LockWait::Block)
            .unwrap()
            .expect("journal directory exists");
        assert_eq!(recover_project_transactions_read_only(&dir.0).unwrap(), 0);
        assert!(dir.0.join(TRANSACTION_ROOT).join("interrupted").exists());
        drop(lock);

        // Unparseable journal names are treated as abandoned once unlocked.
        assert_eq!(recover_project_transactions_read_only(&dir.0).unwrap(), 1);
        assert_eq!(
            std::fs::read(dir.0.join("existing.rs")).unwrap(),
            b"original\n"
        );
        assert!(!dir.0.join(".girder").exists());
    }

    #[test]
    fn graph_snapshot_reports_corruption_without_discarding_bytes() {
        let dir = TempDir::new("graph-corrupt");
        std::fs::write(dir.0.join("project.aether"), b"not a graph").unwrap();

        let snapshot = load_graph_snapshot(&dir.0, &ProjectConfig::default()).unwrap();

        assert!(snapshot.graph.is_none());
        assert_eq!(snapshot.bytes.as_deref(), Some(b"not a graph".as_slice()));
        assert!(snapshot.error.is_some());
    }

    #[test]
    fn reading_a_missing_nested_graph_does_not_create_directories() {
        let dir = TempDir::new("graph-read");
        let mut config = ProjectConfig::default();
        config.graph.path = ".cache/semantic/project.aether".into();

        let snapshot = load_graph_snapshot(&dir.0, &config).unwrap();

        assert!(snapshot.graph.is_none());
        assert!(!dir.0.join(".cache").exists());
    }

    #[test]
    fn bounded_project_reads_stop_at_the_limit() {
        let dir = TempDir::new("bounded-read");
        std::fs::write(dir.0.join("large.txt"), b"12345").unwrap();

        assert_eq!(
            read_project_bytes_bounded(&dir.0, "large.txt", 5)
                .unwrap()
                .unwrap(),
            b"12345"
        );
        let error = read_project_bytes_bounded(&dir.0, "large.txt", 4).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }
}
