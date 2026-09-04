//! The `command{run, expect_exit, timeout_secs}` check kind: a thin wrapper
//! over the shared bounded-subprocess engine. `TimedOut`/`OutputLimited`/
//! `Cancelled` are always a failure regardless of `expect_exit` — there is
//! no exit code to compare in those cases.

use super::CheckOutcome;
use crate::project::process::{run_captured, BoundedStatus};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const COMMAND_MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

pub(crate) fn run_command_check(
    workspace: &Path,
    run: &str,
    expect_exit: i32,
    timeout_secs: u64,
) -> CheckOutcome {
    let mut command = Command::new("sh");
    command.arg("-c").arg(run).current_dir(workspace);
    let run_result = run_captured(
        command,
        Duration::from_secs(timeout_secs),
        COMMAND_MAX_OUTPUT_BYTES,
    );
    match run_result {
        Ok(run_result) => match run_result.status {
            BoundedStatus::Completed(status) => {
                let code = status.code().unwrap_or(-1);
                CheckOutcome {
                    kind: "command".to_string(),
                    passed: code == expect_exit,
                    detail: format!("`{run}` exited {code} (expected {expect_exit})"),
                }
            }
            BoundedStatus::TimedOut => CheckOutcome {
                kind: "command".to_string(),
                passed: false,
                detail: format!("`{run}` timed out after {timeout_secs}s; its process tree was killed"),
            },
            BoundedStatus::OutputLimited => CheckOutcome {
                kind: "command".to_string(),
                passed: false,
                detail: format!(
                    "`{run}` produced more than {COMMAND_MAX_OUTPUT_BYTES} bytes of output; its process tree was killed"
                ),
            },
            BoundedStatus::Cancelled => CheckOutcome {
                kind: "command".to_string(),
                passed: false,
                detail: format!("`{run}` was cancelled"),
            },
        },
        Err(error) => CheckOutcome {
            kind: "command".to_string(),
            passed: false,
            detail: format!("could not run `{run}`: {error}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "girder-planfile-command-check-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn matching_exit_code_passes() {
        let dir = temp_dir();
        let outcome = run_command_check(&dir, "exit 0", 0, 5);
        assert!(outcome.passed, "{}", outcome.detail);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mismatched_exit_code_fails() {
        let dir = temp_dir();
        let outcome = run_command_check(&dir, "exit 3", 0, 5);
        assert!(!outcome.passed);
        assert!(outcome.detail.contains("exited 3"), "{}", outcome.detail);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn timeout_is_always_a_failure_even_if_expect_exit_is_never_reached() {
        let dir = temp_dir();
        let outcome = run_command_check(&dir, "sleep 30", 0, 1);
        assert!(!outcome.passed);
        assert!(outcome.detail.contains("timed out"), "{}", outcome.detail);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn command_runs_inside_the_given_workspace() {
        let dir = temp_dir();
        std::fs::write(dir.join("marker"), b"present\n").unwrap();
        let outcome = run_command_check(&dir, "test -f marker", 0, 5);
        assert!(outcome.passed, "{}", outcome.detail);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
