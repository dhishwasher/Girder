//! The plan run report: a JSON document stating exactly what changed and
//! what was proven, written unconditionally to `.bitcode/reports/` — a
//! natural sibling of the existing `.bitcode/transactions/` and
//! `.bitcode/validation/` directories. `detail` on each check carries the
//! precise mismatch text (e.g. a `graph.callers_of` failure's expected vs.
//! actual path lists) that a plan's author reads to write the next plan.

use crate::project::planfile::executor::{PlanRunOutcome, RunOutcome};
use crate::project::planfile::schema::Plan;
use crate::project::source::safe_project_output_path;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub(crate) struct CheckReport {
    pub(crate) kind: String,
    pub(crate) result: &'static str,
    pub(crate) detail: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct StepReport {
    pub(crate) id: String,
    pub(crate) result: &'static str,
    pub(crate) committed: bool,
    pub(crate) files_changed: Vec<PathBuf>,
    pub(crate) checks: Vec<CheckReport>,
}

#[derive(Debug, Serialize)]
pub(crate) struct FinalState {
    pub(crate) dry_run: bool,
    pub(crate) description: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct PlanReport {
    pub(crate) plan_id: String,
    pub(crate) base_commit: String,
    pub(crate) result: &'static str,
    pub(crate) failed_at: Option<String>,
    pub(crate) steps: Vec<StepReport>,
    pub(crate) final_state: FinalState,
}

pub(crate) fn build_report(plan: &Plan, run: &PlanRunOutcome, dry: bool) -> PlanReport {
    let (result, failed_at, description) = match &run.outcome {
        RunOutcome::Passed => (
            "passed",
            None,
            if dry {
                "dry run — real tree untouched".to_string()
            } else {
                "committed through the final step".to_string()
            },
        ),
        RunOutcome::RolledBackStep { at_step } => (
            "rolled_back_step",
            Some(at_step.clone()),
            format!("failed at step {at_step}; that step's edits were never committed"),
        ),
        RunOutcome::RolledBackPlan { at_step } => (
            "rolled_back_plan",
            Some(at_step.clone()),
            format!(
                "failed at step {at_step}; reverted to base_commit {}",
                plan.base_commit
            ),
        ),
        RunOutcome::Stopped { at_step } => (
            "stopped",
            Some(at_step.clone()),
            format!("stopped at step {at_step}; tree left as committed through the prior step"),
        ),
    };

    let steps = run
        .steps
        .iter()
        .map(|step| StepReport {
            id: step.id.clone(),
            result: if step.passed { "passed" } else { "failed" },
            committed: step.committed,
            files_changed: step.files_changed.clone(),
            checks: step
                .checks
                .iter()
                .map(|check| CheckReport {
                    kind: check.kind.clone(),
                    result: if check.passed { "passed" } else { "failed" },
                    detail: check.detail.clone(),
                })
                .collect(),
        })
        .collect();

    PlanReport {
        plan_id: plan.plan_id.clone(),
        base_commit: plan.base_commit.clone(),
        result,
        failed_at,
        steps,
        final_state: FinalState {
            dry_run: dry,
            description,
        },
    }
}

pub(crate) fn write_report(root: &Path, report: &PlanReport) -> std::io::Result<PathBuf> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let relative = format!(
        ".bitcode/reports/{}-{}-{timestamp}.json",
        sanitize_for_filename(&report.plan_id),
        std::process::id()
    );
    let path = safe_project_output_path(root, &relative)?;
    let bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// `plan_id` is author-controlled free text; keep only characters that are
/// safe as a single path component so a plan cannot influence where its
/// report lands (`safe_project_output_path` also rejects `..`/absolute
/// paths, but sanitizing here keeps report filenames readable).
fn sanitize_for_filename(plan_id: &str) -> String {
    let cleaned: String = plan_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "plan".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::planfile::checks::CheckOutcome;
    use crate::project::planfile::executor::StepOutcome;
    use crate::project::planfile::schema::OnFailure;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    fn temp_root(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "bitcode-planfile-report-{name}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn plan() -> Plan {
        Plan {
            plan_version: 1,
            plan_id: "demo-plan".into(),
            intent: "demo".into(),
            author: None,
            base_commit: "abc123".into(),
            on_failure: OnFailure::RollbackPlan,
            steps: Vec::new(),
        }
    }

    #[test]
    fn a_failing_graph_check_carries_its_mismatch_detail_into_the_report() {
        let root = temp_root("write");
        let run = PlanRunOutcome {
            outcome: RunOutcome::RolledBackPlan {
                at_step: "s2".to_string(),
            },
            steps: vec![StepOutcome {
                id: "s2".to_string(),
                passed: false,
                committed: false,
                files_changed: Vec::new(),
                checks: vec![CheckOutcome {
                    kind: "graph.callers_of".to_string(),
                    passed: false,
                    detail: "mode Exact: expected [\"a\"], actual [\"a\", \"b\"]".to_string(),
                }],
            }],
        };
        let report = build_report(&plan(), &run, false);
        assert_eq!(report.result, "rolled_back_plan");
        assert_eq!(report.failed_at.as_deref(), Some("s2"));
        assert!(report.steps[0].checks[0].detail.contains("expected"));

        let path = write_report(&root, &report).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(parsed["result"], "rolled_back_plan");
        assert_eq!(parsed["failed_at"], "s2");
        assert_eq!(parsed["steps"][0]["checks"][0]["result"], "failed");
        assert!(parsed["steps"][0]["checks"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("expected"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stop_and_rollback_plan_both_produce_a_correct_failed_report() {
        let root = temp_root("stop-vs-rollback");
        let stop_run = PlanRunOutcome {
            outcome: RunOutcome::Stopped {
                at_step: "s1".to_string(),
            },
            steps: Vec::new(),
        };
        let stop_report = build_report(&plan(), &stop_run, false);
        assert_eq!(stop_report.result, "stopped");
        assert_eq!(stop_report.failed_at.as_deref(), Some("s1"));

        let rollback_run = PlanRunOutcome {
            outcome: RunOutcome::RolledBackPlan {
                at_step: "s1".to_string(),
            },
            steps: Vec::new(),
        };
        let rollback_report = build_report(&plan(), &rollback_run, false);
        assert_eq!(rollback_report.result, "rolled_back_plan");
        assert_eq!(rollback_report.failed_at.as_deref(), Some("s1"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
