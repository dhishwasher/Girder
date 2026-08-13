//! Bit Code Plan Format v1/v2: an external author writes an exact, literal JSON
//! plan (text edits plus verification checks) and Bit Code executes it
//! deterministically — no inference, no fuzzy retries. See the plan file
//! `now-create-a-plan-optimized-spark.md` in this repository's planning
//! history for the full design rationale.

mod checks;
mod edit;
mod executor;
mod precondition;
mod report;
mod schema;

use crate::project::config::ProjectConfig;
use schema::Plan;
use std::path::Path;

fn load_plan(path: &Path) -> std::io::Result<Plan> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("could not read plan file {}: {error}", path.display()),
        )
    })?;
    let plan: Plan = serde_json::from_str(&text).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("could not parse plan file {}: {error}", path.display()),
        )
    })?;
    plan.validate().map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid plan file {}: {error}", path.display()),
        )
    })?;
    Ok(plan)
}

/// `bitcode plan validate <plan.json>` — checks preconditions and match
/// counts only. Never writes a byte.
pub(crate) fn validate(root: &Path, plan_path: &Path) -> std::io::Result<()> {
    let plan = load_plan(plan_path)?;
    match precondition::check_preconditions(root, &plan)? {
        Ok(()) => {
            println!(
                "plan {} — preconditions OK ({} step(s))",
                plan.plan_id,
                plan.steps.len()
            );
            Ok(())
        }
        Err(failures) => {
            println!(
                "plan {} — {} precondition failure(s):",
                plan.plan_id,
                failures.len()
            );
            for failure in &failures {
                println!("  ! {}", failure.reason);
            }
            Err(std::io::Error::other(format!(
                "{} precondition failure(s)",
                failures.len()
            )))
        }
    }
}

/// `bitcode plan explain <plan.json>` — a human-readable summary of what a
/// plan would do. No execution, no preconditions.
pub(crate) fn explain(plan_path: &Path) -> std::io::Result<()> {
    let plan = load_plan(plan_path)?;
    println!("Plan: {}", plan.plan_id);
    println!("Intent: {}", plan.intent);
    if let Some(author) = &plan.author {
        println!("Author: {author}");
    }
    println!("Base commit: {}", plan.base_commit);
    println!("On failure: {:?}", plan.on_failure);
    println!("\nSteps ({}):", plan.steps.len());
    for step in &plan.steps {
        println!("  [{}] {}", step.id, step.description);
        if !step.edits.is_empty() {
            println!("      {} edit(s):", step.edits.len());
            for edit in &step.edits {
                match edit {
                    schema::Edit::Substitute {
                        path, occurrences, ..
                    } => println!("        substitute in {path} ({occurrences} occurrence(s))"),
                    schema::Edit::Create { path, .. } => println!("        create {path}"),
                    schema::Edit::Delete { path, .. } => println!("        delete {path}"),
                    schema::Edit::ReplaceNode { node, replacement } => {
                        println!(
                            "        replace full node projection {node} ({} byte(s))",
                            replacement.len()
                        )
                    }
                    schema::Edit::RenameNode { node, new_name } => {
                        println!("        rename {node} to {new_name}")
                    }
                    schema::Edit::DeleteNode { node, delete } => {
                        println!("        delete node {node} (delete={delete})")
                    }
                    schema::Edit::InsertIntoModule { node, insertion } => {
                        println!(
                            "        insert into module {node} ({} byte(s))",
                            insertion.len()
                        )
                    }
                }
            }
        }
        if !step.checks.is_empty() {
            let kinds: Vec<&str> = step.checks.iter().map(schema::Check::kind).collect();
            println!("      {} check(s): {}", step.checks.len(), kinds.join(", "));
        }
    }
    Ok(())
}

/// `bitcode plan run <plan.json> [--dry]` — execute a plan step by step.
/// Preconditions run against the real tree even in `--dry` mode; only the
/// final real-tree commit is skipped when `dry` is set.
pub(crate) fn run(
    root: &Path,
    plan_path: &Path,
    dry: bool,
    authoring_receipt: Option<&Path>,
) -> std::io::Result<()> {
    let plan = load_plan(plan_path)?;
    let authoring_calls = authoring_receipt
        .map(report::load_authoring_receipt)
        .transpose()?;
    if let Err(failures) = precondition::check_preconditions(root, &plan)? {
        println!(
            "plan {} — {} precondition failure(s):",
            plan.plan_id,
            failures.len()
        );
        for failure in &failures {
            println!("  ! {}", failure.reason);
        }
        return Err(std::io::Error::other(format!(
            "{} precondition failure(s)",
            failures.len()
        )));
    }

    let config = ProjectConfig::load(root)?;
    let result = executor::run_plan(root, &config, &plan, dry)?;

    for step in &result.steps {
        let status = if step.passed { "passed" } else { "FAILED" };
        println!("[{}] {status} — {} check(s)", step.id, step.checks.len());
        for check in &step.checks {
            let mark = if check.passed { "ok" } else { "FAIL" };
            println!("    {mark} {}: {}", check.kind, check.detail);
        }
    }

    let built_report = report::build_report(&plan, &result, dry, authoring_calls.as_deref());
    if dry {
        // A dry run must never write to the real tree, including the
        // report itself — print it instead of persisting it under
        // `.bitcode/reports/`.
        let rendered = serde_json::to_string_pretty(&built_report)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        println!("\nreport (dry run, not written to disk):\n{rendered}");
    } else {
        let report_path = report::write_report(root, &built_report)?;
        println!("\nreport: {}", report_path.display());
    }

    match result.outcome {
        executor::RunOutcome::Passed => {
            let suffix = if dry { " (dry run — real tree untouched)" } else { "" };
            println!(
                "plan {} — all {} step(s) passed{suffix}",
                plan.plan_id,
                result.steps.len()
            );
            Ok(())
        }
        executor::RunOutcome::RolledBackStep { at_step } => Err(std::io::Error::other(format!(
            "plan {} failed at step {at_step}; that step's edits were never committed to the real tree",
            plan.plan_id
        ))),
        executor::RunOutcome::RolledBackPlan { at_step } => Err(std::io::Error::other(format!(
            "plan {} failed at step {at_step}; rolled back to base_commit {}",
            plan.plan_id, plan.base_commit
        ))),
        executor::RunOutcome::Stopped { at_step } => Err(std::io::Error::other(format!(
            "plan {} stopped at step {at_step}; real tree left exactly as committed through the prior step",
            plan.plan_id
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    fn write_plan(name: &str, json: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "bitcode-planfile-mod-{name}-{}-{}.json",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, json).unwrap();
        path
    }

    #[test]
    fn explain_reads_a_plan_without_touching_the_project() {
        let path = write_plan(
            "explain",
            r#"{"plan_version":1,"plan_id":"p","intent":"do a thing","base_commit":"abc",
                "steps":[{"id":"s1","description":"step one","edits":[],
                          "checks":[{"kind":"command","run":"true","expect_exit":0}]}]}"#,
        );
        assert!(explain(&path).is_ok());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_plan_rejects_invalid_json_with_a_clear_error() {
        let path = write_plan("invalid", "not json");
        let error = load_plan(&path).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        let _ = std::fs::remove_file(&path);
    }
}
