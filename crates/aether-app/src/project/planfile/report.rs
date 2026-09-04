//! The plan run report: a JSON document stating exactly what changed and
//! what was proven, written unconditionally to `.girder/reports/` — a
//! natural sibling of the existing `.girder/transactions/` and
//! `.girder/validation/` directories. `detail` on each check carries the
//! precise mismatch text (e.g. a `graph.callers_of` failure's expected vs.
//! actual path lists) that a plan's author reads to write the next plan.

use crate::project::planfile::executor::{PlanRunOutcome, RunOutcome};
use crate::project::planfile::schema::Plan;
use crate::project::source::safe_project_output_path;
use serde::{Deserialize, Serialize};
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) writes: Option<Vec<WriteFingerprintReport>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WriteFingerprintReport {
    pub(crate) path: PathBuf,
    pub(crate) before_bytes: Option<usize>,
    pub(crate) before_sha256: Option<String>,
    pub(crate) after_bytes: Option<usize>,
    pub(crate) after_sha256: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tokens: Option<TokenLedger>,
    /// Set only by `plan run --authored --authored-by <name>`: the
    /// externally authoring model's name, so a report (and git history)
    /// distinguishes a locally-authored plan from a remotely-authored one.
    /// Absent (not merely null) for every other run, so this field never
    /// appears in a report `girder do`/plain `plan run` produces.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) authored_by: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthoringCall {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) tokens: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoringReceipt {
    schema_version: u32,
    calls: Vec<AuthoringCall>,
}

#[derive(Debug, Serialize)]
pub(crate) struct TokenCall {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) tokens: u32,
    pub(crate) remote: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct TokenLedger {
    pub(crate) authoring_tokens: u64,
    pub(crate) calls: Vec<TokenCall>,
    pub(crate) remote_call_count: usize,
    pub(crate) zero_remote: bool,
}

pub(crate) fn load_authoring_receipt(path: &Path) -> std::io::Result<Vec<AuthoringCall>> {
    let bytes = std::fs::read(path).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!(
                "could not read authoring receipt {}: {error}",
                path.display()
            ),
        )
    })?;
    let receipt: AuthoringReceipt = serde_json::from_slice(&bytes).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "could not parse authoring receipt {}: {error}",
                path.display()
            ),
        )
    })?;
    if receipt.schema_version != 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "unsupported authoring receipt schema_version {} (expected 1)",
                receipt.schema_version
            ),
        ));
    }
    if receipt.calls.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "authoring receipt must contain at least one model call",
        ));
    }
    for (index, call) in receipt.calls.iter().enumerate() {
        if call.provider.trim().is_empty() || call.model.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("authoring receipt call {index} must name a non-empty provider and model"),
            ));
        }
    }
    Ok(receipt.calls)
}

pub(crate) fn build_report(
    plan: &Plan,
    run: &PlanRunOutcome,
    dry: bool,
    authoring_calls: Option<&[AuthoringCall]>,
    authored_by: Option<&str>,
) -> PlanReport {
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
            writes: step.write_fingerprints.as_ref().map(|writes| {
                writes
                    .iter()
                    .map(|write| WriteFingerprintReport {
                        path: write.path.clone(),
                        before_bytes: write.before_bytes,
                        before_sha256: write.before_sha256.clone(),
                        after_bytes: write.after_bytes,
                        after_sha256: write.after_sha256.clone(),
                    })
                    .collect()
            }),
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
        tokens: authoring_calls.map(build_token_ledger),
        authored_by: authored_by.map(String::from),
    }
}

fn build_token_ledger(calls: &[AuthoringCall]) -> TokenLedger {
    let calls: Vec<TokenCall> = calls
        .iter()
        .map(|call| TokenCall {
            provider: call.provider.clone(),
            model: call.model.clone(),
            tokens: call.tokens,
            remote: !is_local_provider(&call.provider),
        })
        .collect();
    let authoring_tokens = calls.iter().map(|call| u64::from(call.tokens)).sum();
    let remote_call_count = calls.iter().filter(|call| call.remote).count();
    TokenLedger {
        authoring_tokens,
        calls,
        remote_call_count,
        zero_remote: remote_call_count == 0,
    }
}

fn is_local_provider(provider: &str) -> bool {
    matches!(provider, "ollama:local" | "mock")
}

pub(crate) fn write_report(root: &Path, report: &PlanReport) -> std::io::Result<PathBuf> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let relative = format!(
        ".girder/reports/{}-{}-{timestamp}.json",
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
            "girder-planfile-report-{name}-{}-{}",
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
                write_fingerprints: None,
            }],
        };
        let report = build_report(&plan(), &run, false, None, None);
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
        let stop_report = build_report(&plan(), &stop_run, false, None, None);
        assert_eq!(stop_report.result, "stopped");
        assert_eq!(stop_report.failed_at.as_deref(), Some("s1"));

        let rollback_run = PlanRunOutcome {
            outcome: RunOutcome::RolledBackPlan {
                at_step: "s1".to_string(),
            },
            steps: Vec::new(),
        };
        let rollback_report = build_report(&plan(), &rollback_run, false, None, None);
        assert_eq!(rollback_report.result, "rolled_back_plan");
        assert_eq!(rollback_report.failed_at.as_deref(), Some("s1"));
        let _ = std::fs::remove_dir_all(&root);
    }

    fn passed_run() -> PlanRunOutcome {
        PlanRunOutcome {
            outcome: RunOutcome::Passed,
            steps: Vec::new(),
        }
    }

    #[test]
    fn local_only_authoring_sets_zero_remote_and_names_every_call() {
        let calls = vec![AuthoringCall {
            provider: "ollama:local".to_string(),
            model: "qwen2.5-coder:7b".to_string(),
            tokens: 83,
        }];
        let report = build_report(&plan(), &passed_run(), false, Some(&calls), None);
        let tokens = report.tokens.unwrap();
        assert_eq!(tokens.authoring_tokens, 83);
        assert_eq!(tokens.remote_call_count, 0);
        assert!(tokens.zero_remote);
        assert_eq!(tokens.calls[0].provider, "ollama:local");
        assert_eq!(tokens.calls[0].model, "qwen2.5-coder:7b");
        assert!(!tokens.calls[0].remote);
    }

    #[test]
    fn mixed_or_unknown_authoring_fails_closed_as_remote() {
        let calls = vec![
            AuthoringCall {
                provider: "ollama:local".to_string(),
                model: "local-model".to_string(),
                tokens: 10,
            },
            AuthoringCall {
                provider: "unrecognized-provider".to_string(),
                model: "mystery-model".to_string(),
                tokens: 20,
            },
        ];
        let report = build_report(&plan(), &passed_run(), false, Some(&calls), None);
        let tokens = report.tokens.unwrap();
        assert_eq!(tokens.authoring_tokens, 30);
        assert_eq!(tokens.remote_call_count, 1);
        assert!(!tokens.zero_remote);
        assert!(!tokens.calls[0].remote);
        assert!(tokens.calls[1].remote);
    }

    #[test]
    fn absent_authoring_provenance_omits_the_tokens_block() {
        let report = build_report(&plan(), &passed_run(), false, None, None);
        let value = serde_json::to_value(report).unwrap();
        assert!(value.get("tokens").is_none());
    }

    #[test]
    fn absent_authored_by_omits_the_field_entirely() {
        let report = build_report(&plan(), &passed_run(), false, None, None);
        let value = serde_json::to_value(report).unwrap();
        assert!(value.get("authored_by").is_none());
    }

    #[test]
    fn authored_by_is_recorded_when_given() {
        let report = build_report(&plan(), &passed_run(), false, None, Some("claude-sonnet-5"));
        assert_eq!(report.authored_by.as_deref(), Some("claude-sonnet-5"));
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["authored_by"], "claude-sonnet-5");
    }
}
