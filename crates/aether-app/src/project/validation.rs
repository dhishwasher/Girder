use crate::project::config::{ConfiguredCommand, ProjectConfig};
use crate::project::process::{run_diagnostic, BoundedOutput, BoundedStatus};
use crate::project::source::{verify_project_writes, ProjectWrite};
use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

static NEXT_VALIDATION: AtomicUsize = AtomicUsize::new(0);
const VALIDATION_ROOT: &str = ".bitcode/validation";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValidationPolicy {
    Project,
    #[cfg(any(feature = "gui", test))]
    Extension,
}

impl ValidationPolicy {
    fn is_extension(self) -> bool {
        #[cfg(any(feature = "gui", test))]
        {
            self == Self::Extension
        }
        #[cfg(not(any(feature = "gui", test)))]
        {
            false
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValidationStatus {
    Passed,
    Failed,
    TimedOut,
    Cancelled,
    Skipped,
}

impl ValidationStatus {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::TimedOut => "timed out",
            Self::Cancelled => "cancelled",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ValidationStep {
    pub(crate) label: String,
    pub(crate) command: Option<String>,
    pub(crate) status: ValidationStatus,
    pub(crate) duration: Duration,
    pub(crate) output: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ValidationReport {
    pub(crate) steps: Vec<ValidationStep>,
}

impl ValidationReport {
    pub(crate) fn passed(&self) -> bool {
        self.steps.iter().all(|step| {
            matches!(
                step.status,
                ValidationStatus::Passed | ValidationStatus::Skipped
            )
        })
    }

    pub(crate) fn summary(&self) -> String {
        let passed = self
            .steps
            .iter()
            .filter(|step| step.status == ValidationStatus::Passed)
            .count();
        if self.passed() {
            format!("validation passed ({passed} check(s))")
        } else {
            let failure = self
                .steps
                .iter()
                .find(|step| {
                    !matches!(
                        step.status,
                        ValidationStatus::Passed | ValidationStatus::Skipped
                    )
                })
                .map(|step| format!("{} {}", step.label, step.status.label()))
                .unwrap_or_else(|| "validation failed".to_string());
            format!("{failure} ({passed} check(s) passed)")
        }
    }
}

pub(crate) fn validate_candidate(
    root: &Path,
    config: &ProjectConfig,
    writes: &[ProjectWrite],
    cancel: &Arc<AtomicBool>,
) -> std::io::Result<ValidationReport> {
    validate_candidate_with_policy(root, config, writes, cancel, ValidationPolicy::Project)
}

#[cfg(any(feature = "gui", test))]
pub(crate) fn validate_extension_command(
    root: &Path,
    config: &ProjectConfig,
    argv: Vec<String>,
    cancel: &Arc<AtomicBool>,
) -> std::io::Result<ValidationReport> {
    if bubblewrap_path().is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "extension commands require Bubblewrap filesystem and network isolation",
        ));
    }
    let mut config = config.clone();
    config.validation.enabled = true;
    config.validation.run_tests = false;
    config.validation.commands = vec![argv];
    validate_candidate_with_policy(root, &config, &[], cancel, ValidationPolicy::Extension)
}

fn validate_candidate_with_policy(
    root: &Path,
    config: &ProjectConfig,
    writes: &[ProjectWrite],
    cancel: &Arc<AtomicBool>,
    policy: ValidationPolicy,
) -> std::io::Result<ValidationReport> {
    verify_project_writes(root, writes)?;
    if !config.validation.enabled {
        return Ok(ValidationReport {
            steps: vec![ValidationStep {
                label: "Project validation".to_string(),
                command: None,
                status: ValidationStatus::Skipped,
                duration: Duration::ZERO,
                output: "disabled by bitcode.toml".to_string(),
            }],
        });
    }
    check_cancelled(cancel)?;

    let started = Instant::now();
    let sandbox = CandidateWorkspace::create(root, config, cancel)?;
    for write in writes {
        check_cancelled(cancel)?;
        let target = sandbox.workspace.join(write.relative());
        if let Some(contents) = write.contents() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::fs::File::create(&target)?;
            file.write_all(contents)?;
        } else {
            std::fs::remove_file(&target)?;
        }
    }

    let os_sandbox = bubblewrap_path().is_some();
    let mut steps = vec![ValidationStep {
        label: "Isolated candidate".to_string(),
        command: None,
        status: ValidationStatus::Passed,
        duration: started.elapsed(),
        output: format!(
            "prepared {} candidate write(s); filesystem sandbox: {}",
            writes.len(),
            if os_sandbox {
                "bubblewrap"
            } else {
                "candidate copy only"
            }
        ),
    }];
    let commands = validation_commands(&sandbox.workspace, config, policy);
    if commands.is_empty() {
        steps.push(ValidationStep {
            label: "Build and tests".to_string(),
            command: None,
            status: ValidationStatus::Skipped,
            duration: Duration::ZERO,
            output: "no Cargo.toml or configured validation command; parser validation applies"
                .to_string(),
        });
        return Ok(ValidationReport { steps });
    }
    let mut candidate_snapshot = snapshot_candidate(&sandbox.workspace)?;

    for (label, command) in commands {
        check_cancelled(cancel)?;
        let step = run_validation_command(
            &sandbox.workspace,
            root,
            &label,
            &command,
            config,
            cancel,
            policy,
        )?;
        let passed = step.status == ValidationStatus::Passed;
        steps.push(step);
        if !passed {
            break;
        }
        let current_snapshot = snapshot_candidate(&sandbox.workspace)?;
        if current_snapshot != candidate_snapshot {
            let changed = snapshot_changes(&candidate_snapshot, &current_snapshot);
            let created_lockfile = command.program == "cargo"
                && !candidate_snapshot.contains_key(Path::new("Cargo.lock"))
                && current_snapshot.contains_key(Path::new("Cargo.lock"))
                && changed == [PathBuf::from("Cargo.lock")];
            if created_lockfile {
                candidate_snapshot = current_snapshot;
                continue;
            }
            steps.push(ValidationStep {
                label: "Candidate integrity".to_string(),
                command: None,
                status: ValidationStatus::Failed,
                duration: Duration::ZERO,
                output: render_snapshot_diff(&candidate_snapshot, &current_snapshot),
            });
            break;
        }
    }
    Ok(ValidationReport { steps })
}

fn validation_commands(
    candidate: &Path,
    config: &ProjectConfig,
    policy: ValidationPolicy,
) -> Vec<(String, ConfiguredCommand)> {
    let mut commands = Vec::new();
    if !policy.is_extension() && candidate.join("Cargo.toml").is_file() {
        commands.push((
            "Rust build".to_string(),
            ConfiguredCommand {
                program: "cargo".to_string(),
                args: ["check", "--workspace", "--all-targets"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            },
        ));
        if config.validation.run_tests {
            commands.push((
                "Rust tests".to_string(),
                ConfiguredCommand {
                    program: "cargo".to_string(),
                    args: ["test", "--workspace"]
                        .into_iter()
                        .map(str::to_string)
                        .collect(),
                },
            ));
        }
    }
    commands.extend(
        config
            .validation
            .commands
            .iter()
            .enumerate()
            .filter_map(|(index, parts)| {
                let (program, args) = parts.split_first()?;
                Some((
                    format!("Configured check {}", index + 1),
                    ConfiguredCommand {
                        program: program.clone(),
                        args: args.to_vec(),
                    },
                ))
            }),
    );
    commands
}

fn run_validation_command(
    candidate: &Path,
    project_root: &Path,
    label: &str,
    configured: &ConfiguredCommand,
    project_config: &ProjectConfig,
    cancel: &Arc<AtomicBool>,
    policy: ValidationPolicy,
) -> std::io::Result<ValidationStep> {
    let started = Instant::now();
    let target_dir =
        (configured.program == "cargo").then(|| validation_target_dir(project_root, candidate));
    let mut command = sandboxed_command(candidate, configured, target_dir.as_deref(), policy)?;
    command
        .current_dir(candidate)
        .env("BITCODE_VALIDATION", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1");
    if let Some(target) = &target_dir {
        command.env("CARGO_TARGET_DIR", target);
    }

    let run = run_diagnostic(
        command,
        Duration::from_secs(project_config.validation.timeout_seconds),
        project_config.validation.max_output_bytes,
        cancel,
    )
    .map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("could not start {}: {error}", configured.display()),
        )
    })?;
    let status = match run.status {
        BoundedStatus::Completed(exit) if exit.success() => ValidationStatus::Passed,
        BoundedStatus::Completed(_) | BoundedStatus::OutputLimited => ValidationStatus::Failed,
        BoundedStatus::TimedOut => ValidationStatus::TimedOut,
        BoundedStatus::Cancelled => ValidationStatus::Cancelled,
    };
    let output = render_output(run.exit, run.stdout, run.stderr);
    Ok(ValidationStep {
        label: label.to_string(),
        command: Some(configured.display()),
        status,
        duration: started.elapsed(),
        output,
    })
}

fn validation_target_dir(project_root: &Path, candidate: &Path) -> PathBuf {
    if let Some(configured) = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from) {
        return if configured.is_absolute() {
            configured
        } else {
            project_root.join(configured)
        };
    }
    let target = project_root.join("target");
    if target.exists() {
        target
    } else {
        candidate
            .parent()
            .unwrap_or(candidate)
            .join("validation-target")
    }
}

fn sandboxed_command(
    candidate: &Path,
    configured: &ConfiguredCommand,
    target_dir: Option<&Path>,
    policy: ValidationPolicy,
) -> std::io::Result<Command> {
    let Some(bwrap) = bubblewrap_path() else {
        let mut command = Command::new(&configured.program);
        command.args(&configured.args);
        return Ok(command);
    };

    let mut command = Command::new(bwrap);
    command.args([
        "--die-with-parent",
        "--ro-bind",
        "/",
        "/",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--tmpfs",
        "/tmp",
        "--bind",
    ]);
    command.arg(candidate).arg(candidate);
    if policy.is_extension() {
        command.arg("--unshare-net");
        let preserved = [
            "PATH",
            "CARGO_HOME",
            "RUSTUP_HOME",
            "CARGO_TARGET_DIR",
            "TMPDIR",
            "LANG",
            "LC_ALL",
        ]
        .into_iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (key, value)))
        .collect::<Vec<_>>();
        command.env_clear();
        command.envs(preserved);
    }
    if let Some(target) = target_dir {
        std::fs::create_dir_all(target)?;
        if !target.starts_with(candidate) {
            command.arg("--bind").arg(target).arg(target);
        }
    }
    if let Some(cargo_home) = std::env::var_os("CARGO_HOME").map(PathBuf::from) {
        if cargo_home.is_dir()
            && !cargo_home.starts_with(candidate)
            && target_dir != Some(cargo_home.as_path())
        {
            command.arg("--bind").arg(&cargo_home).arg(&cargo_home);
        }
    }
    command
        .arg("--chdir")
        .arg(candidate)
        .arg("--setenv")
        .arg("BITCODE_VALIDATION")
        .arg("1");
    if policy.is_extension() {
        command.arg("--setenv").arg("HOME").arg("/tmp");
    }
    command
        .arg("--")
        .arg(&configured.program)
        .args(&configured.args);
    Ok(command)
}

fn snapshot_candidate(root: &Path) -> std::io::Result<BTreeMap<PathBuf, (u64, u64)>> {
    let mut snapshot = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let mut entries = std::fs::read_dir(&directory)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| std::io::Error::other("candidate snapshot escaped its root"))?
                    .to_path_buf();
                let bytes = std::fs::read(&path)?;
                snapshot.insert(relative, (bytes.len() as u64, stable_hash(&bytes)));
            }
        }
    }
    Ok(snapshot)
}

fn render_snapshot_diff(
    before: &BTreeMap<PathBuf, (u64, u64)>,
    after: &BTreeMap<PathBuf, (u64, u64)>,
) -> String {
    let mut changed = snapshot_changes(before, after)
        .into_iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    changed.truncate(20);
    format!(
        "validation command modified candidate input: {}",
        changed.join(", ")
    )
}

fn snapshot_changes(
    before: &BTreeMap<PathBuf, (u64, u64)>,
    after: &BTreeMap<PathBuf, (u64, u64)>,
) -> Vec<PathBuf> {
    let mut changed = before
        .keys()
        .chain(after.keys())
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|path| before.get(*path) != after.get(*path))
        .cloned()
        .collect::<Vec<_>>();
    changed.sort();
    changed
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn bubblewrap_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join("bwrap"))
        .find(|candidate| candidate.is_file())
}

fn render_output(exit: Option<ExitStatus>, stdout: BoundedOutput, stderr: BoundedOutput) -> String {
    let mut sections = Vec::new();
    if let Some(exit) = exit {
        sections.push(format!("exit: {exit}"));
    }
    let stdout = stdout.render();
    if !stdout.trim().is_empty() {
        sections.push(format!("stdout:\n{stdout}"));
    }
    let stderr = stderr.render();
    if !stderr.trim().is_empty() {
        sections.push(format!("stderr:\n{stderr}"));
    }
    sections.join("\n")
}

fn check_cancelled(cancel: &Arc<AtomicBool>) -> std::io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "candidate validation was cancelled",
        ))
    } else {
        Ok(())
    }
}

struct CandidateWorkspace {
    workspace: PathBuf,
    allocation: PathBuf,
}

impl CandidateWorkspace {
    fn create(
        root: &Path,
        config: &ProjectConfig,
        cancel: &Arc<AtomicBool>,
    ) -> std::io::Result<Self> {
        let canonical_root = std::fs::canonicalize(root)?;
        let validation_root = canonical_root.join(VALIDATION_ROOT);
        let allocation = validation_root.join(format!(
            "{}-{}",
            std::process::id(),
            NEXT_VALIDATION.fetch_add(1, Ordering::Relaxed)
        ));
        let workspace = allocation.join("workspace");
        // A concurrent transaction cleanup or workspace drop may prune an
        // empty `.bitcode` between these creations; retry once on ENOENT.
        let mut attempts = 0;
        loop {
            attempts += 1;
            let created = std::fs::create_dir_all(&validation_root)
                .and_then(|()| std::fs::create_dir(&allocation))
                .and_then(|()| std::fs::create_dir(&workspace));
            match created {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && attempts < 3 => {
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        let mut copied = 0_u64;
        let mut visited = HashSet::new();
        let candidate = Self {
            workspace,
            allocation,
        };
        copy_directory(
            &canonical_root,
            &canonical_root,
            &candidate.workspace,
            config,
            &mut copied,
            &mut visited,
            cancel,
        )?;
        Ok(candidate)
    }
}

impl Drop for CandidateWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.allocation);
        if let Some(validation_root) = self.allocation.parent() {
            let _ = std::fs::remove_dir(validation_root);
            if let Some(bitcode) = validation_root.parent() {
                let _ = std::fs::remove_dir(bitcode);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn copy_directory(
    project_root: &Path,
    source: &Path,
    destination: &Path,
    config: &ProjectConfig,
    copied: &mut u64,
    visited: &mut HashSet<PathBuf>,
    cancel: &Arc<AtomicBool>,
) -> std::io::Result<()> {
    check_cancelled(cancel)?;
    let canonical_source = std::fs::canonicalize(source)?;
    if !canonical_source.starts_with(project_root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "validation input escapes project root: {}",
                canonical_source.display()
            ),
        ));
    }
    if !visited.insert(canonical_source.clone()) {
        return Ok(());
    }

    let mut entries = std::fs::read_dir(&canonical_source)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        check_cancelled(cancel)?;
        let source_path = entry.path();
        let relative = source_path.strip_prefix(project_root).map_err(|_| {
            std::io::Error::other(format!(
                "validation input escaped project root: {}",
                source_path.display()
            ))
        })?;
        if should_skip(relative) {
            continue;
        }
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        let resolved = if file_type.is_symlink() {
            if !config.source.follow_symlinks {
                continue;
            }
            let resolved = std::fs::canonicalize(&source_path)?;
            if !resolved.starts_with(project_root) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!(
                        "validation symlink escapes project root: {}",
                        source_path.display()
                    ),
                ));
            }
            resolved
        } else {
            source_path
        };

        if resolved.is_dir() {
            std::fs::create_dir_all(&destination_path)?;
            copy_directory(
                project_root,
                &resolved,
                &destination_path,
                config,
                copied,
                visited,
                cancel,
            )?;
        } else if resolved.is_file() {
            let metadata = std::fs::metadata(&resolved)?;
            *copied = copied
                .checked_add(metadata.len())
                .ok_or_else(|| std::io::Error::other("validation copy size overflow"))?;
            if *copied > config.validation.max_copy_bytes {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::StorageFull,
                    format!(
                        "validation copy exceeds validation.max_copy_bytes ({})",
                        config.validation.max_copy_bytes
                    ),
                ));
            }
            std::fs::copy(&resolved, &destination_path)?;
            std::fs::set_permissions(&destination_path, metadata.permissions())?;
        }
    }
    Ok(())
}

fn should_skip(relative: &Path) -> bool {
    relative.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some(
                ".git" | ".bitcode" | "target" | "node_modules" | ".venv" | "venv" | "__pycache__"
            )
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEST: AtomicUsize = AtomicUsize::new(0);

    struct TempProject(PathBuf);

    impl TempProject {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "bitcode-validation-{label}-{}-{}",
                std::process::id(),
                NEXT_TEST.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, relative: &str, contents: &str) {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, contents).unwrap();
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn command_config(script: &str) -> ProjectConfig {
        let mut config = ProjectConfig::default();
        config.validation.commands =
            vec![vec!["sh".to_string(), "-c".to_string(), script.to_string()]];
        config.validation.timeout_seconds = 2;
        config
    }

    #[test]
    fn candidate_commands_see_new_bytes_without_mutating_project() {
        let project = TempProject::new("isolation");
        project.write("src/lib.rs", "old\n");
        let write = ProjectWrite::text("src/lib.rs", Some(b"old\n".to_vec()), "new candidate\n");
        let config = command_config("grep -q 'new candidate' src/lib.rs && printf candidate-ok");

        let report = validate_candidate(
            &project.0,
            &config,
            &[write],
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        assert!(report.passed(), "{:?}", report.steps);
        assert_eq!(
            std::fs::read_to_string(project.0.join("src/lib.rs")).unwrap(),
            "old\n"
        );
        assert!(!project.0.join("marker").exists());
        assert!(!project.0.join(".bitcode").exists());
    }

    #[test]
    fn failed_command_returns_bounded_diagnostics_and_keeps_project_clean() {
        let project = TempProject::new("failure");
        project.write("src/lib.rs", "old\n");
        let write =
            ProjectWrite::text("src/lib.rs", Some(b"old\n".to_vec()), "invalid candidate\n");
        let config = command_config("printf 'compile-broke' >&2; exit 7");

        let report = validate_candidate(
            &project.0,
            &config,
            &[write],
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        assert!(!report.passed());
        let failed = report.steps.last().unwrap();
        assert_eq!(failed.status, ValidationStatus::Failed);
        assert!(failed.output.contains("compile-broke"));
        assert_eq!(
            std::fs::read_to_string(project.0.join("src/lib.rs")).unwrap(),
            "old\n"
        );
    }

    #[test]
    fn bubblewrap_blocks_absolute_project_writes() {
        if bubblewrap_path().is_none() {
            return;
        }
        let project = TempProject::new("filesystem-sandbox");
        project.write("src/lib.rs", "old\n");
        let escaped = project.0.join("escaped");
        let config = command_config(&format!("touch '{}'", escaped.display()));

        let report = validate_candidate(
            &project.0,
            &config,
            &[ProjectWrite::text(
                "src/lib.rs",
                Some(b"old\n".to_vec()),
                "candidate\n",
            )],
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        assert!(report.passed(), "{:?}", report.steps);
        assert!(!escaped.exists());
    }

    #[test]
    fn bubblewrap_makes_the_host_filesystem_read_only() {
        if bubblewrap_path().is_none() {
            return;
        }
        let project = TempProject::new("read-only-host");
        project.write("src/lib.rs", "old\n");
        let escaped = std::env::current_dir()
            .unwrap()
            .join(format!(".bitcode-validation-escape-{}", std::process::id()));
        let _ = std::fs::remove_file(&escaped);
        let _cleanup = HostMarker(escaped.clone());
        let config = command_config(&format!("touch '{}'", escaped.display()));

        let report = validate_candidate(
            &project.0,
            &config,
            &[ProjectWrite::text(
                "src/lib.rs",
                Some(b"old\n".to_vec()),
                "candidate\n",
            )],
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        assert!(!report.passed(), "{:?}", report.steps);
        assert!(!escaped.exists());
    }

    #[test]
    fn validation_commands_cannot_rewrite_candidate_inputs() {
        let project = TempProject::new("candidate-read-only");
        project.write("src/lib.rs", "old\n");
        let config = command_config("printf rewritten > src/lib.rs");

        let report = validate_candidate(
            &project.0,
            &config,
            &[ProjectWrite::text(
                "src/lib.rs",
                Some(b"old\n".to_vec()),
                "candidate\n",
            )],
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        assert!(!report.passed());
        assert_eq!(
            std::fs::read_to_string(project.0.join("src/lib.rs")).unwrap(),
            "old\n"
        );
    }

    #[test]
    fn timed_out_command_is_terminated() {
        let project = TempProject::new("timeout");
        project.write("src/lib.rs", "old\n");
        let write = ProjectWrite::text("src/lib.rs", Some(b"old\n".to_vec()), "candidate\n");
        let mut config = command_config("sleep 30");
        config.validation.timeout_seconds = 1;
        let started = Instant::now();

        let report = validate_candidate(
            &project.0,
            &config,
            &[write],
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        assert_eq!(
            report.steps.last().unwrap().status,
            ValidationStatus::TimedOut
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn pre_cancelled_validation_writes_nothing() {
        let project = TempProject::new("cancel");
        project.write("src/lib.rs", "old\n");
        let cancel = Arc::new(AtomicBool::new(true));
        let error = validate_candidate(
            &project.0,
            &ProjectConfig::default(),
            &[ProjectWrite::text(
                "src/lib.rs",
                Some(b"old\n".to_vec()),
                "candidate\n",
            )],
            &cancel,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
        assert_eq!(
            std::fs::read_to_string(project.0.join("src/lib.rs")).unwrap(),
            "old\n"
        );
    }

    #[test]
    fn failed_candidate_copy_cleans_validation_directory() {
        let project = TempProject::new("copy-cleanup");
        project.write("src/lib.rs", "larger than one byte\n");
        let mut config = ProjectConfig::default();
        config.validation.max_copy_bytes = 1;

        let error = validate_candidate(
            &project.0,
            &config,
            &[ProjectWrite::text(
                "src/lib.rs",
                Some(b"larger than one byte\n".to_vec()),
                "candidate\n",
            )],
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::StorageFull);
        assert!(!project.0.join(".bitcode").exists());
    }

    #[test]
    fn cargo_validation_commands_are_ordered_build_then_test() {
        let project = TempProject::new("cargo-command-order");
        project.write("Cargo.toml", "[workspace]\n");
        let mut config = ProjectConfig::default();
        config.validation.commands = vec![vec!["custom-check".to_string()]];

        let commands = validation_commands(&project.0, &config, ValidationPolicy::Project);

        assert_eq!(
            commands
                .iter()
                .map(|(label, _)| label.as_str())
                .collect::<Vec<_>>(),
            ["Rust build", "Rust tests", "Configured check 1"]
        );
        assert_eq!(commands[0].1.program, "cargo");
        assert_eq!(
            commands[0].1.args,
            ["check", "--workspace", "--all-targets"]
        );
        assert_eq!(commands[1].1.args, ["test", "--workspace"]);
    }

    #[test]
    fn extension_policy_runs_only_the_declared_command() {
        let project = TempProject::new("extension-command-order");
        project.write("Cargo.toml", "[workspace]\n");
        let mut config = ProjectConfig::default();
        config.validation.commands = vec![vec![
            "cargo".to_string(),
            "fmt".to_string(),
            "--check".to_string(),
        ]];

        let commands = validation_commands(&project.0, &config, ValidationPolicy::Extension);

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].0, "Configured check 1");
        assert_eq!(commands[0].1.program, "cargo");
        assert_eq!(commands[0].1.args, ["fmt", "--check"]);
    }

    #[test]
    fn extension_command_does_not_inherit_ambient_secrets() {
        if bubblewrap_path().is_none() {
            return;
        }
        const SECRET: &str = "BITCODE_EXTENSION_SECRET_TEST";
        let project = TempProject::new("extension-secret-isolation");
        project.write("src/lib.rs", "pub fn example() {}\n");
        std::env::set_var(SECRET, "must-not-cross-sandbox");

        let result = validate_extension_command(
            &project.0,
            &ProjectConfig::default(),
            vec![
                "sh".to_string(),
                "-c".to_string(),
                format!("test -z \"${{{SECRET}+x}}\""),
            ],
            &Arc::new(AtomicBool::new(false)),
        );
        std::env::remove_var(SECRET);

        let report = result.unwrap();
        assert!(report.passed(), "{:?}", report.steps);
    }

    struct HostMarker(PathBuf);

    impl Drop for HostMarker {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}
