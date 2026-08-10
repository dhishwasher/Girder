use crate::project::config::ProjectConfig;
use crate::project::process::{run_captured, BoundedStatus, CapturedRun};
use crate::project::source::is_configured_source_path;
use aether_builder::GraphBuilder;
use aether_graph::{NodeId, NodeKind, SemanticGraph};
use std::collections::{BTreeSet, HashSet};
use std::io::{Error, ErrorKind};
use std::path::Path;
use std::time::Duration;

/// Hard bounds for every Git subprocess. Timeout and overflow kill the whole
/// process group and fail the command with an explicit classification —
/// captured output is either complete or an error, never truncated data.
const GIT_TIMEOUT: Duration = Duration::from_secs(120);
const GIT_MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

fn git_timeout() -> Duration {
    std::env::var("BITCODE_GIT_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .unwrap_or(GIT_TIMEOUT)
}

fn root_argument(root: &Path) -> std::io::Result<&str> {
    root.to_str().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("project path is not valid UTF-8: {}", root.display()),
        )
    })
}

fn git_captured(root: &Path, args: &[&str]) -> std::io::Result<CapturedRun> {
    let mut command = std::process::Command::new("git");
    command
        .arg("-C")
        .arg(root_argument(root)?)
        .args(args)
        // Analysis commands only read repository state; skipping optional
        // index refreshes keeps concurrent invocations from contending on
        // `.git/index.lock`.
        .env("GIT_OPTIONAL_LOCKS", "0");
    run_captured(command, git_timeout(), GIT_MAX_OUTPUT_BYTES)
        .map_err(|error| Error::new(error.kind(), format!("failed to run git: {error}")))
}

fn git_output(root: &Path, args: &[&str]) -> std::io::Result<Vec<u8>> {
    let run = git_captured(root, args)?;
    let rendered = args.join(" ");
    match run.status {
        BoundedStatus::Completed(status) if status.success() => Ok(run.stdout),
        BoundedStatus::Completed(status) => {
            let stderr = String::from_utf8_lossy(&run.stderr);
            Err(Error::other(format!(
                "git {rendered} failed with {}: {}",
                status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "a signal".into()),
                stderr.trim()
            )))
        }
        BoundedStatus::TimedOut => Err(Error::new(
            ErrorKind::TimedOut,
            format!(
                "git {rendered} timed out after {}s; its process tree was killed",
                git_timeout().as_secs()
            ),
        )),
        BoundedStatus::OutputLimited => Err(Error::other(format!(
            "git {rendered} produced more than {GIT_MAX_OUTPUT_BYTES} bytes of output; \
             its process tree was killed"
        ))),
        BoundedStatus::Cancelled => Err(Error::new(
            ErrorKind::Interrupted,
            format!("git {rendered} was cancelled"),
        )),
    }
}

fn git_text(root: &Path, args: &[&str]) -> std::io::Result<String> {
    String::from_utf8(git_output(root, args)?).map_err(|error| {
        Error::new(
            ErrorKind::InvalidData,
            format!("git {} returned non-UTF-8 output: {error}", args.join(" ")),
        )
    })
}

pub(crate) fn git_is_repo(root: &Path) -> std::io::Result<bool> {
    let run = git_captured(root, &["rev-parse", "--is-inside-work-tree"])?;
    let stdout = match run.status {
        BoundedStatus::Completed(status) if status.success() => run.stdout,
        BoundedStatus::Completed(_) => return Ok(false),
        BoundedStatus::TimedOut => {
            return Err(Error::new(
                ErrorKind::TimedOut,
                format!(
                    "git rev-parse timed out after {}s; its process tree was killed",
                    git_timeout().as_secs()
                ),
            ))
        }
        BoundedStatus::OutputLimited | BoundedStatus::Cancelled => {
            return Err(Error::other("git rev-parse exceeded its output bounds"))
        }
    };
    let stdout = String::from_utf8(stdout).map_err(|error| {
        Error::new(
            ErrorKind::InvalidData,
            format!("git rev-parse returned non-UTF-8 output: {error}"),
        )
    })?;
    Ok(stdout.trim() == "true")
}

/// `true` when `git status --porcelain` reports nothing outstanding — no
/// staged, unstaged, or untracked changes. Used by the plan executor's
/// preconditions and by `rollback_plan`'s reliance on a known-clean
/// `base_commit` state.
pub(crate) fn git_worktree_clean(root: &Path) -> std::io::Result<bool> {
    let status = git_text(root, &["status", "--porcelain"])?;
    Ok(status.trim().is_empty())
}

/// The full object id of `HEAD`, used to enforce a plan's `base_commit`
/// precondition.
pub(crate) fn git_head_commit(root: &Path) -> std::io::Result<String> {
    let oid = git_text(root, &["rev-parse", "HEAD"])?;
    Ok(oid.trim().to_string())
}

/// Restore `paths` to their exact contents at `commit`, deleting any path
/// that did not exist there. `git checkout` alone does not remove files
/// that are untracked at `commit` (e.g. a file a plan step created), so
/// callers doing a full plan rollback must additionally delete paths they
/// know a step created — this function only restores tracked history.
pub(crate) fn git_checkout_paths(
    root: &Path,
    commit: &str,
    paths: &[&Path],
) -> std::io::Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut args: Vec<&str> = vec!["checkout", commit, "--"];
    let rendered: Vec<String> = paths
        .iter()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect();
    args.extend(rendered.iter().map(String::as_str));
    git_output(root, &args)?;
    Ok(())
}

fn git_prefix(root: &Path) -> std::io::Result<String> {
    let output = git_text(root, &["rev-parse", "--show-prefix"])?;
    let output = output.strip_suffix('\n').unwrap_or(&output);
    Ok(output.strip_suffix('\r').unwrap_or(output).to_string())
}

fn strip_git_prefix(path: &str, prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        Some(path.to_string())
    } else {
        path.strip_prefix(prefix).map(str::to_string)
    }
}

fn git_object_path(prefix: &str, rel: &str) -> String {
    format!("{prefix}{rel}")
}

fn nul_delimited_paths(bytes: &[u8], command: &str) -> std::io::Result<Vec<String>> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            std::str::from_utf8(path)
                .map(str::to_owned)
                .map_err(|error| {
                    Error::new(
                        ErrorKind::InvalidData,
                        format!("{command} returned a non-UTF-8 path: {error}"),
                    )
                })
        })
        .collect()
}

fn resolve_git_commit(root: &Path, git_ref: &str) -> std::io::Result<String> {
    let revision = format!("{git_ref}^{{commit}}");
    let oid = git_text(
        root,
        &["rev-parse", "--verify", "--end-of-options", &revision],
    )?;
    let oid = oid.trim();
    if oid.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("git resolved {git_ref:?} to an empty object id"),
        ));
    }
    Ok(oid.to_string())
}

pub(crate) fn git_tracked_sources_at(
    root: &Path,
    commit_oid: &str,
    config: &ProjectConfig,
) -> std::io::Result<Vec<String>> {
    let prefix = git_prefix(root)?;
    let output = git_output(
        root,
        &[
            "ls-tree",
            "--full-tree",
            "-r",
            "-z",
            "--name-only",
            commit_oid,
            "--",
        ],
    )?;
    let mut sources = Vec::new();
    for path in nul_delimited_paths(&output, "git ls-tree")? {
        let Some(relative) = strip_git_prefix(&path, &prefix) else {
            continue;
        };
        if is_configured_source_path(config, &relative)? {
            sources.push(relative);
        }
    }
    Ok(sources)
}

pub(crate) fn build_baseline_graph(root: &Path, git_ref: &str) -> std::io::Result<SemanticGraph> {
    let config = ProjectConfig::load(root)?;
    let commit_oid = resolve_git_commit(root, git_ref)?;
    let sources: BTreeSet<String> = git_tracked_sources_at(root, &commit_oid, &config)?
        .into_iter()
        .collect();
    let prefix = git_prefix(root)?;

    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();
    let mut contents = Vec::with_capacity(sources.len());

    for rel in &sources {
        let object_path = git_object_path(&prefix, rel);
        let object = format!("{commit_oid}:{object_path}");
        let output = git_output(root, &["show", &object])?;
        let text = String::from_utf8(output).map_err(|error| {
            Error::new(
                ErrorKind::InvalidData,
                format!("baseline source {rel} is not valid UTF-8: {error}"),
            )
        })?;
        contents.push((rel.clone(), text));
    }
    builder.load_files(
        &mut graph,
        contents
            .iter()
            .map(|(relative, text)| (relative.as_str(), text.as_str())),
    );
    Ok(graph)
}

/// `bitcode test-impact <dir> [--run] [node::path...]`
///
/// Finds the minimal set of tests that cover any changed (or specified) functions,
/// using the call graph to determine reachability. Runs them if `--run` is passed.
///
pub(crate) fn git_changed_files(root: &Path) -> std::io::Result<Vec<String>> {
    let config = ProjectConfig::load(root)?;
    let run = |extra_args: &[&str]| -> std::io::Result<Vec<String>> {
        let mut cmd_args = vec!["diff", "--relative", "--name-only", "-z"];
        cmd_args.extend_from_slice(extra_args);
        let output = git_output(root, &cmd_args)?;
        let mut paths = Vec::new();
        for relative in nul_delimited_paths(&output, "git diff")? {
            if is_configured_source_path(&config, &relative)? {
                paths.push(relative);
            }
        }
        Ok(paths)
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for f in run(&["--cached"])?
        .into_iter()
        .chain(run(&[])?)
        .chain(git_untracked_sources(root, &config)?)
    {
        if seen.insert(f.clone()) {
            out.push(f);
        }
    }
    Ok(out)
}

fn git_untracked_sources(root: &Path, config: &ProjectConfig) -> std::io::Result<Vec<String>> {
    let output = git_output(root, &["ls-files", "-z", "--others", "--exclude-standard"])?;
    let mut sources = Vec::new();
    for relative in nul_delimited_paths(&output, "git ls-files")? {
        if is_configured_source_path(config, &relative)? {
            sources.push(relative);
        }
    }
    Ok(sources)
}

pub(crate) struct ChangedImpact {
    pub(crate) origin_ids: Vec<NodeId>,
    pub(crate) baseline_test_paths: Vec<String>,
    pub(crate) changed_files: Vec<String>,
}

pub(crate) fn semantic_changed_impact(
    root: &Path,
    current: &SemanticGraph,
) -> std::io::Result<Option<ChangedImpact>> {
    if !git_is_repo(root)? {
        return Ok(None);
    }

    let baseline = build_baseline_graph(root, "HEAD")?;
    let diff = current.diff_from(&baseline);
    if diff.is_empty() {
        return Ok(Some(ChangedImpact {
            origin_ids: Vec::new(),
            baseline_test_paths: Vec::new(),
            changed_files: git_changed_files(root)?,
        }));
    }

    let mut seen_origins = HashSet::new();
    let origin_ids: Vec<NodeId> = diff
        .changed_node_ids()
        .into_iter()
        .filter(|id| {
            current
                .get(*id)
                .map(|n| n.kind == NodeKind::Function)
                .unwrap_or(false)
                && seen_origins.insert(*id)
        })
        .collect();

    let mut seen_tests = HashSet::new();
    let baseline_test_paths: Vec<String> = diff
        .removed
        .iter()
        .filter(|c| c.kind == NodeKind::Function)
        .flat_map(|c| baseline.tests_for(c.id))
        .filter_map(|id| baseline.get(id).map(|n| n.path.clone()))
        .filter(|path| seen_tests.insert(path.clone()))
        .collect();

    Ok(Some(ChangedImpact {
        origin_ids,
        baseline_test_paths,
        changed_files: git_changed_files(root)?,
    }))
}
