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

// pub(crate) rather than private: `context_cmd`'s tests round-trip
// `authoring_context::plan_schema`-shaped steps through this exact loader
// to prove the schema `bitcode context` emits to an external model is one
// `load_plan` actually accepts (see gap in `docs/core-gap-analysis.md`).
pub(crate) fn load_plan(path: &Path) -> std::io::Result<Plan> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("could not read plan file {}: {error}", path.display()),
        )
    })?;
    parse_plan(&text)
}

/// The parse/validate half of [`load_plan`], split out so a caller that
/// already has plan JSON in memory (the GUI's "paste a plan back in" flow)
/// can reach the same parsing and `Plan::validate()` this loader uses
/// without writing a temp file first.
pub(crate) fn parse_plan(text: &str) -> std::io::Result<Plan> {
    let plan: Plan = serde_json::from_str(text).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("could not parse plan: {error}"),
        )
    })?;
    plan.validate().map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid plan: {error}"),
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

/// Refuses `root` if it is (or resolves to) `sample-project/`: that
/// directory is a pinned baseline `tools/authoring_task_check.py` and
/// `tools/plan_executor_oracle.py::authoring_target_node_source` read
/// against a specific clean source commit, not a scratch target. A live
/// authored write there — real, not the harness's own plain `plan run`
/// invocations, which never pass `--authored` and are unaffected — is
/// exactly what gap 18 in `docs/core-gap-analysis.md` recorded twice, three
/// days apart, because the demo workflow had nowhere else to point.
/// `demo-project/` exists for that instead. Read-only commands
/// (`context`, `search`, `analyze`, `test-impact`) never call this and stay
/// unaffected — this only guards the two things that write:
/// [`apply_authored_guarantees`] (`plan run --authored`, and the GUI's "Run
/// authored") and `author::author` in `project::commands` (`bitcode do`,
/// and the GUI's local-model "Run").
pub(crate) fn reject_measurement_fixture_root(root: &Path) -> std::io::Result<()> {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if canonical.file_name().and_then(|name| name.to_str()) == Some("sample-project") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "sample-project/ is a pinned measurement fixture (see gap 18/21 in \
             docs/core-gap-analysis.md) and refuses authored writes; run demos \
             against demo-project/ instead",
        ));
    }
    Ok(())
}

/// Applies the guarantees `--authored` promises to a plan written outside
/// Bit Code, in place: force a clean revert on any failure, and close the
/// vacuous-check hole by guaranteeing at least one real test verification.
/// Also used, via [`run_for_authoring_with_plan`], by the GUI's "Run
/// authored" flow for a pasted external plan — the same guarantees, the
/// same function, so the two callers can never drift.
pub(crate) fn apply_authored_guarantees(
    root: &Path,
    plan: &mut schema::Plan,
) -> std::io::Result<()> {
    reject_measurement_fixture_root(root)?;
    // A zero-step plan has nowhere to inject the mandatory check below —
    // `steps.last_mut()` would silently no-op — and nothing else in this
    // codebase rejects it: `Plan::validate()` has no `steps`
    // non-emptiness check, `precondition::check_preconditions` and
    // `executor::run_plan_v2` both just iterate `&plan.steps` and fall
    // through to `RunOutcome::Passed` on zero iterations. Left unchecked,
    // `--authored` would "pass" a plan that verified nothing at all, which
    // is a worse hole than the one this flag exists to close. Fail closed
    // instead.
    if plan.steps.is_empty() {
        return Err(std::io::Error::other(format!(
            "plan {} has zero steps; --authored refuses to run a plan with nothing to verify",
            plan.plan_id
        )));
    }
    // The same harness guarantees `bitcode do` applies internally
    // (`author::wrap_step_into_plan`) to a plan written outside Bit Code:
    // force a clean revert on any failure, and close the vacuous-check
    // hole by guaranteeing at least one real test verification — but only
    // inject it if the plan doesn't already have one; unlike a local model
    // (schema-forbidden from ever emitting `tests.impacted` itself), an
    // external plan is free-form and may already carry a legitimate one.
    plan.on_failure = schema::OnFailure::RollbackPlan;
    let has_impacted_check = plan.steps.iter().any(|step| {
        step.checks
            .iter()
            .any(|check| matches!(check, schema::Check::TestsImpacted { .. }))
    });
    if !has_impacted_check {
        // Deliberately the *last* step only, not every step — a real
        // decision, not an accident of `last_mut()`. The guarantee this
        // exists to provide is "the tree, if this plan durably commits,
        // passes its impacted tests" — a property of the plan's final
        // cumulative state, not of every intermediate one. `run_plan_v2`
        // stops at the first failing step, and `on_failure` is forced to
        // `RollbackPlan` above, so nothing partial is ever durably
        // committed regardless of where the check lands: either every
        // step (including this injected one) passes and the full set of
        // edits commits, or the first failure anywhere rolls the whole
        // plan back to `base_commit`. Injecting per-step would enforce a
        // *stronger* and often wrong property instead — that every
        // intermediate step independently passes tests — which rejects
        // legitimate staged edits by design (e.g. a rename split across
        // two steps: step 1 renames the definition, step 2 updates the
        // caller; running impacted tests after step 1 alone would see a
        // stale caller and fail on an inconsistency the plan was always
        // going to resolve by its next step). It would also run the
        // impacted-test command once per step instead of once per plan,
        // which is real cost for no additional guarantee on a plan that
        // passes.
        let last_step = plan.steps.last_mut().expect("checked non-empty above");
        last_step.checks.push(schema::Check::TestsImpacted {
            expect: schema::TestExpect::AllPass,
        });
    }
    Ok(())
}

/// `bitcode plan run <plan.json> [--dry] [--authored [--authored-by <name>]]`
/// — execute a plan step by step. Preconditions run against the real tree
/// even in `--dry` mode; only the final real-tree commit is skipped when
/// `dry` is set.
///
/// `authored` and `authored_by` default to `false`/`None` at the CLI's only
/// call site unless `--authored`/`--authored-by` are passed, so every
/// existing invocation (`plan run <plan.json>`, `--dry`,
/// `--authoring-receipt <r>`) takes the same `plan`, the same preconditions,
/// the same executor call, the same printed lines, and the same report
/// shape as before this parameter existed.
pub(crate) fn run(
    root: &Path,
    plan_path: &Path,
    dry: bool,
    authoring_receipt: Option<&Path>,
    authored: bool,
    authored_by: Option<&str>,
) -> std::io::Result<()> {
    let mut plan = load_plan(plan_path)?;
    let authoring_calls = authoring_receipt
        .map(report::load_authoring_receipt)
        .transpose()?;

    if authored {
        apply_authored_guarantees(root, &mut plan)?;
    }

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

    let built_report =
        report::build_report(&plan, &result, dry, authoring_calls.as_deref(), authored_by);
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

/// The result of one `run_for_authoring` attempt: enough to decide whether
/// `bitcode do` should stop, and — when it should keep going — the same
/// per-check pass/fail detail `run()` already prints, as data instead of
/// stdout, for a repair prompt.
///
/// This is deliberately a separate entry point from `run()` rather than a
/// refactor of it: `run()`'s printed output is depended on byte-for-byte
/// (the P1-P5 oracle drives `plan run` through the CLI), so this function
/// re-orchestrates the same precondition/executor/report calls `run()` makes
/// instead of sharing code with it, and touches none of their internals.
pub(crate) struct AuthoringRunResult {
    pub(crate) passed: bool,
    pub(crate) diagnostic: String,
    /// Set only for a non-dry run: where the (unconditionally written)
    /// report landed.
    pub(crate) report_path: Option<std::path::PathBuf>,
    /// Set only for a dry run: the report that would have been written,
    /// rendered exactly as `run()`'s `--dry` path prints it.
    pub(crate) report_json: Option<String>,
}

pub(crate) fn run_for_authoring(
    root: &Path,
    plan_path: &Path,
    dry: bool,
    authoring_receipt: Option<&Path>,
) -> std::io::Result<AuthoringRunResult> {
    run_for_authoring_with_plan(root, load_plan(plan_path)?, dry, authoring_receipt, None)
}

/// The body of [`run_for_authoring`], taking an already-loaded `Plan`
/// directly instead of a path, and threading through an `authored_by` the
/// report can record. Split out so the GUI's "Run authored" flow — a plan
/// pasted into a text box and mutated in memory by
/// [`apply_authored_guarantees`] — can reach the same execution path with
/// no temp file and no re-parsing. `bitcode do`'s call site above (the sole
/// path-based caller) always passes `None` for `authored_by`; it has no
/// such concept, and its outcome is already named by `provider`/`model` in
/// the success message.
pub(crate) fn run_for_authoring_with_plan(
    root: &Path,
    plan: Plan,
    dry: bool,
    authoring_receipt: Option<&Path>,
    authored_by: Option<&str>,
) -> std::io::Result<AuthoringRunResult> {
    let authoring_calls = authoring_receipt
        .map(report::load_authoring_receipt)
        .transpose()?;

    if let Err(failures) = precondition::check_preconditions(root, &plan)? {
        let diagnostic = failures
            .iter()
            .map(|failure| format!("precondition failed: {}", failure.reason))
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(AuthoringRunResult {
            passed: false,
            diagnostic,
            report_path: None,
            report_json: None,
        });
    }

    let config = ProjectConfig::load(root)?;
    let result = executor::run_plan(root, &config, &plan, dry)?;

    let mut diagnostic_lines = Vec::new();
    for step in &result.steps {
        let status = if step.passed { "passed" } else { "FAILED" };
        diagnostic_lines.push(format!(
            "[{}] {status} — {} check(s)",
            step.id,
            step.checks.len()
        ));
        for check in &step.checks {
            let mark = if check.passed { "ok" } else { "FAIL" };
            diagnostic_lines.push(format!("    {mark} {}: {}", check.kind, check.detail));
        }
    }

    let built_report =
        report::build_report(&plan, &result, dry, authoring_calls.as_deref(), authored_by);
    let (report_path, report_json) = if dry {
        let rendered = serde_json::to_string_pretty(&built_report)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        (None, Some(rendered))
    } else {
        (Some(report::write_report(root, &built_report)?), None)
    };

    let passed = matches!(result.outcome, executor::RunOutcome::Passed);
    if !passed {
        diagnostic_lines.push(match &result.outcome {
            executor::RunOutcome::RolledBackStep { at_step } => format!(
                "plan {} failed at step {at_step}; that step's edits were never committed to the real tree",
                plan.plan_id
            ),
            executor::RunOutcome::RolledBackPlan { at_step } => format!(
                "plan {} failed at step {at_step}; rolled back to base_commit {}",
                plan.plan_id, plan.base_commit
            ),
            executor::RunOutcome::Stopped { at_step } => format!(
                "plan {} stopped at step {at_step}; real tree left exactly as committed through the prior step",
                plan.plan_id
            ),
            executor::RunOutcome::Passed => unreachable!("passed is false in this branch"),
        });
    }

    Ok(AuthoringRunResult {
        passed,
        diagnostic: diagnostic_lines.join("\n"),
        report_path,
        report_json,
    })
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

    #[test]
    fn reject_measurement_fixture_root_rejects_a_directory_named_sample_project() {
        let parent = std::env::temp_dir().join(format!(
            "bitcode-planfile-mod-fixture-guard-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let root = parent.join("sample-project");
        std::fs::create_dir_all(&root).unwrap();

        let error = reject_measurement_fixture_root(&root).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("demo-project"), "{error}");
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn reject_measurement_fixture_root_allows_other_directories() {
        let root = std::env::temp_dir().join(format!(
            "bitcode-planfile-mod-fixture-guard-allowed-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();

        assert!(reject_measurement_fixture_root(&root).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_with_authored_rejects_sample_project_as_a_target() {
        let parent = std::env::temp_dir().join(format!(
            "bitcode-planfile-mod-authored-fixture-guard-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let root = parent.join("sample-project");
        std::fs::create_dir_all(&root).unwrap();
        let base_commit = init_git_repo(&root);
        let path = write_plan(
            "authored-fixture-guard",
            &format!(
                r#"{{"plan_version":2,"plan_id":"p","intent":"i",
                    "base_commit":"{base_commit}",
                    "steps":[{{"id":"s1","checks":[{{"kind":"command","run":"true"}}]}}]}}"#
            ),
        );

        let error = run(&root, &path, false, None, true, None).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("demo-project"), "{error}");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
    }

    fn temp_git_root(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "bitcode-planfile-mod-authoring-{name}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn git(root: &std::path::Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn init_git_repo(root: &std::path::Path) -> String {
        std::fs::write(root.join("tracked.txt"), "base\n").unwrap();
        git(root, &["init", "--quiet"]);
        git(root, &["add", "tracked.txt"]);
        git(
            root,
            &[
                "-c",
                "user.name=Bit Code Tests",
                "-c",
                "user.email=tests@bitcode.invalid",
                "commit",
                "--quiet",
                "-m",
                "base",
            ],
        );
        git(root, &["rev-parse", "HEAD"])
    }

    #[test]
    fn run_for_authoring_reports_precondition_failures_as_diagnostic_without_executing() {
        let root = temp_git_root("precondition");
        init_git_repo(&root);
        let path = write_plan(
            "authoring-precondition",
            r#"{"plan_version":1,"plan_id":"p","intent":"i","base_commit":"not-the-real-head",
                "steps":[]}"#,
        );

        let result = run_for_authoring(&root, &path, false, None).unwrap();

        assert!(!result.passed);
        assert!(
            result.diagnostic.contains("precondition failed"),
            "{}",
            result.diagnostic
        );
        assert!(result.report_path.is_none());
        assert!(result.report_json.is_none());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_for_authoring_reports_a_passing_plan_with_a_written_report() {
        let root = temp_git_root("passing");
        let base_commit = init_git_repo(&root);
        let path = write_plan(
            "authoring-passing",
            &format!(
                r#"{{"plan_version":1,"plan_id":"authoring-passing","intent":"i",
                    "base_commit":"{base_commit}",
                    "steps":[{{"id":"s1","checks":[{{"kind":"command","run":"true"}}]}}]}}"#
            ),
        );

        let result = run_for_authoring(&root, &path, false, None).unwrap();

        assert!(result.passed, "{}", result.diagnostic);
        assert!(
            result.diagnostic.contains("passed"),
            "{}",
            result.diagnostic
        );
        assert!(result.report_path.as_ref().is_some_and(|p| p.exists()));
        assert!(result.report_json.is_none());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_for_authoring_dry_run_renders_report_json_without_writing_it() {
        let root = temp_git_root("dry");
        let base_commit = init_git_repo(&root);
        let path = write_plan(
            "authoring-dry",
            &format!(
                r#"{{"plan_version":1,"plan_id":"authoring-dry","intent":"i",
                    "base_commit":"{base_commit}",
                    "steps":[{{"id":"s1","checks":[{{"kind":"command","run":"true"}}]}}]}}"#
            ),
        );

        let result = run_for_authoring(&root, &path, true, None).unwrap();

        assert!(result.passed, "{}", result.diagnostic);
        assert!(result.report_path.is_none());
        assert!(result
            .report_json
            .as_ref()
            .is_some_and(|json| json.contains("\"passed\"")));
        assert!(!root.join(".bitcode/reports").exists());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_for_authoring_with_plan_records_authored_by_and_needs_no_temp_file() {
        // Mirrors the GUI's "Run authored" flow exactly: parse pasted plan
        // JSON, apply the same guarantees `plan run --authored` applies,
        // then execute — all without ever writing the plan to a temp file.
        let root = temp_git_root("with-plan-authored-by");
        let base_commit = init_git_repo(&root);
        let mut plan = parse_plan(&format!(
            r#"{{"plan_version":2,"plan_id":"with-plan-authored-by","intent":"i",
                "base_commit":"{base_commit}",
                "steps":[{{"id":"s1","checks":[{{"kind":"command","run":"true"}}]}}]}}"#
        ))
        .unwrap();
        apply_authored_guarantees(&root, &mut plan).unwrap();
        assert_eq!(plan.on_failure, schema::OnFailure::RollbackPlan);

        let result =
            run_for_authoring_with_plan(&root, plan, false, None, Some("claude-sonnet-5")).unwrap();

        assert!(result.passed, "{}", result.diagnostic);
        let report_path = result.report_path.expect("non-dry run writes a report");
        let written = std::fs::read_to_string(report_path).unwrap();
        let report: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(report["authored_by"], "claude-sonnet-5", "{written}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_with_authored_rejects_a_zero_step_plan() {
        let root = temp_git_root("authored-zero-step");
        let base_commit = init_git_repo(&root);
        let path = write_plan(
            "authored-zero-step",
            &format!(
                r#"{{"plan_version":2,"plan_id":"p","intent":"i",
                    "base_commit":"{base_commit}","steps":[]}}"#
            ),
        );

        let error = run(&root, &path, false, None, true, None).unwrap_err();
        assert!(error.to_string().contains("zero steps"), "{error}");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_with_authored_injects_the_mandatory_check_when_absent() {
        let root = temp_git_root("authored-inject");
        let base_commit = init_git_repo(&root);
        let path = write_plan(
            "authored-inject",
            &format!(
                r#"{{"plan_version":2,"plan_id":"authored-inject","intent":"i",
                    "base_commit":"{base_commit}",
                    "steps":[{{"id":"s1","checks":[{{"kind":"command","run":"true"}}]}}]}}"#
            ),
        );

        assert!(run(&root, &path, false, None, true, None).is_ok());

        let mut entries = std::fs::read_dir(root.join(".bitcode/reports")).unwrap();
        let report_path = entries.next().unwrap().unwrap().path();
        let written = std::fs::read_to_string(report_path).unwrap();
        let report: serde_json::Value = serde_json::from_str(&written).unwrap();
        let checks = report["steps"][0]["checks"].as_array().unwrap();
        assert!(
            checks.iter().any(|check| check["kind"] == "tests.impacted"),
            "{written}"
        );
        assert!(report.get("authored_by").is_none(), "{written}");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_with_authored_injects_the_mandatory_check_on_the_last_step_only() {
        // Deliberate: the guarantee is "the final cumulative state passes,"
        // not "every intermediate step independently passes" (see the
        // comment at the injection site in `run`). A two-step plan with no
        // existing `tests.impacted` check should get exactly one injected
        // check, on step two, and none on step one.
        let root = temp_git_root("authored-inject-multi-step");
        let base_commit = init_git_repo(&root);
        let path = write_plan(
            "authored-inject-multi-step",
            &format!(
                r#"{{"plan_version":2,"plan_id":"authored-inject-multi-step","intent":"i",
                    "base_commit":"{base_commit}",
                    "steps":[
                        {{"id":"s1","checks":[{{"kind":"command","run":"true"}}]}},
                        {{"id":"s2","checks":[{{"kind":"command","run":"true"}}]}}
                    ]}}"#
            ),
        );

        assert!(run(&root, &path, false, None, true, None).is_ok());

        let mut entries = std::fs::read_dir(root.join(".bitcode/reports")).unwrap();
        let report_path = entries.next().unwrap().unwrap().path();
        let written = std::fs::read_to_string(report_path).unwrap();
        let report: serde_json::Value = serde_json::from_str(&written).unwrap();
        let step1_checks = report["steps"][0]["checks"].as_array().unwrap();
        let step2_checks = report["steps"][1]["checks"].as_array().unwrap();
        assert!(
            step1_checks
                .iter()
                .all(|check| check["kind"] != "tests.impacted"),
            "{written}"
        );
        assert!(
            step2_checks
                .iter()
                .any(|check| check["kind"] == "tests.impacted"),
            "{written}"
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_with_authored_forces_rollback_plan_even_when_the_plan_declares_stop() {
        let root = temp_git_root("authored-force-rollback");
        let base_commit = init_git_repo(&root);
        let path = write_plan(
            "authored-force-rollback",
            &format!(
                r#"{{"plan_version":2,"plan_id":"authored-force-rollback","intent":"i",
                    "base_commit":"{base_commit}","on_failure":"stop",
                    "steps":[{{"id":"s1","checks":[{{"kind":"command","run":"false"}}]}}]}}"#
            ),
        );

        let error = run(&root, &path, false, None, true, None).unwrap_err();
        assert!(
            error.to_string().contains("rolled back to base_commit"),
            "{error}"
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_records_authored_by_in_the_written_report() {
        let root = temp_git_root("authored-by");
        let base_commit = init_git_repo(&root);
        let path = write_plan(
            "authored-by",
            &format!(
                r#"{{"plan_version":2,"plan_id":"authored-by","intent":"i",
                    "base_commit":"{base_commit}",
                    "steps":[{{"id":"s1","checks":[{{"kind":"command","run":"true"}}]}}]}}"#
            ),
        );

        assert!(run(&root, &path, false, None, true, Some("claude-sonnet-5")).is_ok());

        let mut entries = std::fs::read_dir(root.join(".bitcode/reports")).unwrap();
        let report_path = entries.next().unwrap().unwrap().path();
        let written = std::fs::read_to_string(report_path).unwrap();
        let report: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(report["authored_by"], "claude-sonnet-5", "{written}");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&root);
    }
}
