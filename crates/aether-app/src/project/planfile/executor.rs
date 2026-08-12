//! The step loop: apply each step's edits into a disposable copy, run its
//! checks there, and only commit to the real tree once every check passes.
//! This makes `rollback_step` nearly free (a failing step's edits were
//! never written to the real tree) and makes `--dry` almost the same code
//! path with the final commit skipped (Phase 6).

use crate::project::config::ProjectConfig;
use crate::project::git::git_checkout_paths;
use crate::project::planfile::checks::command::run_command_check;
use crate::project::planfile::checks::graph as graph_checks;
use crate::project::planfile::checks::test_checks::{
    changed_node_ids, run_tests_full, run_tests_impacted, run_tests_named,
};
use crate::project::planfile::checks::CheckOutcome;
use crate::project::planfile::edit::{apply_edit, write_into};
use crate::project::planfile::schema::{Check, Edit, OnFailure, Plan, TestExpect};
use crate::project::source::{build_from_dir_with_config, commit_project_writes};
use crate::project::validation::CandidateWorkspace;
use aether_graph::SemanticGraph;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct StepOutcome {
    pub(crate) id: String,
    pub(crate) passed: bool,
    pub(crate) committed: bool,
    pub(crate) files_changed: Vec<PathBuf>,
    pub(crate) checks: Vec<CheckOutcome>,
}

#[derive(Debug)]
pub(crate) enum RunOutcome {
    Passed,
    RolledBackStep { at_step: String },
    RolledBackPlan { at_step: String },
    Stopped { at_step: String },
}

#[derive(Debug)]
pub(crate) struct PlanRunOutcome {
    pub(crate) outcome: RunOutcome,
    pub(crate) steps: Vec<StepOutcome>,
}

pub(crate) fn run_plan(
    root: &Path,
    config: &ProjectConfig,
    plan: &Plan,
    dry: bool,
) -> std::io::Result<PlanRunOutcome> {
    let mut steps = Vec::new();
    let mut committed_paths: Vec<PathBuf> = Vec::new();
    let mut created_paths: Vec<PathBuf> = Vec::new();
    let mut dry_overlay = DryOverlay::default();

    for step in &plan.steps {
        let cancel = Arc::new(AtomicBool::new(false));
        let candidate = CandidateWorkspace::create(root, config, &cancel)?;
        if dry {
            dry_overlay.replay(candidate.workspace_path())?;
        }

        let mut writes = Vec::new();
        let mut created_this_step = Vec::new();
        let mut apply_error = None;
        for edit in &step.edits {
            match apply_edit(candidate.workspace_path(), edit) {
                Ok(write) => {
                    if matches!(edit, Edit::Create { .. }) {
                        created_this_step.push(write.relative().to_path_buf());
                    }
                    writes.push(write);
                }
                Err(error) => {
                    apply_error = Some(error);
                    break;
                }
            }
        }

        if let Some(error) = apply_error {
            steps.push(StepOutcome {
                id: step.id.clone(),
                passed: false,
                committed: false,
                files_changed: Vec::new(),
                checks: vec![CheckOutcome {
                    kind: "edit".to_string(),
                    passed: false,
                    detail: error.to_string(),
                }],
            });
            return finish_run(
                root,
                plan,
                steps,
                &step.id,
                &committed_paths,
                &created_paths,
                dry,
            );
        }

        // The graph "before" snapshot must be taken before this step's
        // edits land, so `tests.impacted`'s changed-node detection (and
        // Phase 4's graph.edge_delta) reflect exactly what this step did.
        let need_graph = checks_need_graph(&step.checks);
        let before_graph = need_graph
            .then(|| {
                build_from_dir_with_config(candidate.workspace_path(), config)
                    .map(|(graph, _, _)| graph)
            })
            .transpose()?;

        for write in &writes {
            write_into(candidate.workspace_path(), write)?;
        }
        let files_changed: Vec<PathBuf> =
            writes.iter().map(|w| w.relative().to_path_buf()).collect();

        let after_graph = need_graph
            .then(|| build_from_dir_with_config(candidate.workspace_path(), config))
            .transpose()?
            .map(|(graph, _, _)| graph);
        let changed: Vec<aether_graph::NodeId> = match (&before_graph, &after_graph) {
            (Some(before), Some(after)) => changed_node_ids(before, after),
            _ => Vec::new(),
        };

        let mut check_outcomes = Vec::new();
        let mut all_passed = true;
        for check in &step.checks {
            let outcome = run_check(
                candidate.workspace_path(),
                check,
                config,
                before_graph.as_ref(),
                after_graph.as_ref(),
                &changed,
            );
            if !outcome.passed {
                all_passed = false;
            }
            check_outcomes.push(outcome);
            if !all_passed {
                break;
            }
        }

        if !all_passed {
            steps.push(StepOutcome {
                id: step.id.clone(),
                passed: false,
                committed: false,
                files_changed,
                checks: check_outcomes,
            });
            return finish_run(
                root,
                plan,
                steps,
                &step.id,
                &committed_paths,
                &created_paths,
                dry,
            );
        }

        if dry {
            dry_overlay.record(&writes);
            steps.push(StepOutcome {
                id: step.id.clone(),
                passed: true,
                committed: false,
                files_changed,
                checks: check_outcomes,
            });
            continue;
        }

        // Drop the disposable copy before the real-tree commit: a crash
        // inside `commit_project_writes` (fault injection, or a real
        // crash) exits via `std::process::exit`, which never runs `Drop` —
        // if the copy were still alive at that point, its `.bitcode/
        // validation/...` directory would leak forever.
        drop(candidate);

        let written = commit_project_writes(root, writes)?;
        committed_paths.extend(written.iter().cloned());
        created_paths.extend(created_this_step);

        steps.push(StepOutcome {
            id: step.id.clone(),
            passed: true,
            committed: true,
            files_changed: written,
            checks: check_outcomes,
        });
    }

    Ok(PlanRunOutcome {
        outcome: RunOutcome::Passed,
        steps,
    })
}

#[derive(Default)]
struct DryOverlay {
    files: BTreeMap<PathBuf, Option<Vec<u8>>>,
}

impl DryOverlay {
    fn replay(&self, workspace: &Path) -> std::io::Result<()> {
        for (relative, contents) in &self.files {
            let target = workspace.join(relative);
            match contents {
                Some(contents) => {
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(target, contents)?;
                }
                None => match std::fs::remove_file(target) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                },
            }
        }
        Ok(())
    }

    fn record(&mut self, writes: &[crate::project::source::ProjectWrite]) {
        for write in writes {
            self.files.insert(
                write.relative().to_path_buf(),
                write.contents().map(|contents| contents.to_vec()),
            );
        }
    }
}

fn checks_need_graph(checks: &[Check]) -> bool {
    checks.iter().any(|check| {
        matches!(
            check,
            Check::GraphCallersOf { .. }
                | Check::GraphCalleesOf { .. }
                | Check::GraphTestsFor { .. }
                | Check::GraphNodeExists { .. }
                | Check::GraphNodeAbsent { .. }
                | Check::GraphNoNewEdgesInto { .. }
                | Check::GraphEdgeDelta { .. }
                | Check::GraphUnresolved { .. }
                | Check::TestsImpacted { .. }
                | Check::TestsNamed { .. }
        )
    })
}

fn apply_test_expect(mut outcome: CheckOutcome, expect: TestExpect) -> CheckOutcome {
    if expect == TestExpect::AllFail {
        outcome.passed = !outcome.passed;
        outcome.detail = format!("expect_result=all_fail: {}", outcome.detail);
    }
    outcome
}

#[allow(clippy::too_many_arguments)]
fn run_check(
    workspace: &Path,
    check: &Check,
    config: &ProjectConfig,
    before: Option<&SemanticGraph>,
    after: Option<&SemanticGraph>,
    changed: &[aether_graph::NodeId],
) -> CheckOutcome {
    match check {
        Check::Command {
            run,
            expect_exit,
            timeout_secs,
        } => run_command_check(workspace, run, *expect_exit, *timeout_secs),
        Check::TestsImpacted { expect } => match after {
            Some(graph) => apply_test_expect(
                run_tests_impacted(workspace, config, graph, changed),
                *expect,
            ),
            None => CheckOutcome::not_yet_implemented("tests.impacted"),
        },
        Check::TestsNamed { tests, expect } => match after {
            Some(graph) => {
                apply_test_expect(run_tests_named(workspace, config, graph, tests), *expect)
            }
            None => CheckOutcome::not_yet_implemented("tests.named"),
        },
        Check::TestsFull { expect } => {
            apply_test_expect(run_tests_full(workspace, config), *expect)
        }
        Check::GraphCallersOf { node, expect, mode } => match after {
            Some(graph) => graph_checks::callers_of(graph, node, expect, *mode),
            None => CheckOutcome::not_yet_implemented("graph.callers_of"),
        },
        Check::GraphCalleesOf { node, expect, mode } => match after {
            Some(graph) => graph_checks::callees_of(graph, node, expect, *mode),
            None => CheckOutcome::not_yet_implemented("graph.callees_of"),
        },
        Check::GraphTestsFor { node, expect, mode } => match after {
            Some(graph) => graph_checks::tests_for(graph, node, expect, *mode),
            None => CheckOutcome::not_yet_implemented("graph.tests_for"),
        },
        Check::GraphNodeExists { node } => match after {
            Some(graph) => graph_checks::node_exists(graph, node),
            None => CheckOutcome::not_yet_implemented("graph.node_exists"),
        },
        Check::GraphNodeAbsent { node } => match after {
            Some(graph) => graph_checks::node_absent(graph, node),
            None => CheckOutcome::not_yet_implemented("graph.node_absent"),
        },
        Check::GraphNoNewEdgesInto { node } => match (before, after) {
            (Some(before), Some(after)) => graph_checks::no_new_edges_into(before, after, node),
            _ => CheckOutcome::not_yet_implemented("graph.no_new_edges_into"),
        },
        Check::GraphEdgeDelta {
            max_added,
            max_removed,
        } => match (before, after) {
            (Some(before), Some(after)) => {
                graph_checks::edge_delta(before, after, *max_added, *max_removed)
            }
            _ => CheckOutcome::not_yet_implemented("graph.edge_delta"),
        },
        Check::GraphUnresolved {
            node,
            from,
            expect_result,
        } => match after {
            Some(graph) => graph_checks::unresolved(graph, node, from, *expect_result),
            None => CheckOutcome::not_yet_implemented("graph.unresolved"),
        },
        other => CheckOutcome::not_yet_implemented(other.kind()),
    }
}

fn finish_run(
    root: &Path,
    plan: &Plan,
    steps: Vec<StepOutcome>,
    failed_step_id: &str,
    committed_paths: &[PathBuf],
    created_paths: &[PathBuf],
    dry: bool,
) -> std::io::Result<PlanRunOutcome> {
    let outcome = match plan.on_failure {
        OnFailure::Stop => RunOutcome::Stopped {
            at_step: failed_step_id.to_string(),
        },
        OnFailure::RollbackStep => RunOutcome::RolledBackStep {
            at_step: failed_step_id.to_string(),
        },
        OnFailure::RollbackPlan => {
            if !dry {
                rollback_to_base(root, plan, committed_paths, created_paths)?;
            }
            RunOutcome::RolledBackPlan {
                at_step: failed_step_id.to_string(),
            }
        }
    };
    Ok(PlanRunOutcome { outcome, steps })
}

/// Restore every path a prior step in this plan committed back to its
/// `base_commit` content, then delete every path a prior step's `create`
/// edit introduced — `git checkout` alone leaves untracked-at-base files
/// in place, so created paths need an explicit delete.
fn rollback_to_base(
    root: &Path,
    plan: &Plan,
    committed_paths: &[PathBuf],
    created_paths: &[PathBuf],
) -> std::io::Result<()> {
    let restore: Vec<&Path> = committed_paths
        .iter()
        .filter(|path| !created_paths.contains(path))
        .map(PathBuf::as_path)
        .collect();
    if !restore.is_empty() {
        git_checkout_paths(root, &plan.base_commit, &restore)?;
    }
    for created in created_paths {
        match std::fs::remove_file(root.join(created)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::source::ProjectWrite;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "bitcode-planfile-executor-{name}-{}-{}",
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
    fn dry_overlay_replays_substitutions_creates_and_deletes() {
        let workspace = TempDir::new("overlay");
        std::fs::write(workspace.0.join("changed.rs"), "pristine\n").unwrap();
        std::fs::write(workspace.0.join("deleted.rs"), "remove me\n").unwrap();
        let mut overlay = DryOverlay::default();
        overlay.record(&[
            ProjectWrite::text("changed.rs", Some(b"pristine\n".to_vec()), "planned\n"),
            ProjectWrite::text("nested/created.rs", None, "created\n"),
            ProjectWrite::delete("deleted.rs", b"remove me\n".to_vec()),
        ]);

        overlay.replay(&workspace.0).unwrap();

        assert_eq!(
            std::fs::read_to_string(workspace.0.join("changed.rs")).unwrap(),
            "planned\n"
        );
        assert_eq!(
            std::fs::read_to_string(workspace.0.join("nested/created.rs")).unwrap(),
            "created\n"
        );
        assert!(!workspace.0.join("deleted.rs").exists());
    }
}
