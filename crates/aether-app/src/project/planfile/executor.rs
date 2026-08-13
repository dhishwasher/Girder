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
use crate::project::planfile::edit::{apply_edit, apply_step_edits_v2, write_into, EditState};
use crate::project::planfile::schema::{Check, Edit, OnFailure, Plan, TestExpect};
use crate::project::source::{build_from_dir_with_config, commit_project_writes};
use crate::project::validation::CandidateWorkspace;
use aether_graph::SemanticGraph;
use sha2::{Digest, Sha256};
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
    pub(crate) write_fingerprints: Option<Vec<WriteFingerprint>>,
}

#[derive(Debug, Clone)]
pub(crate) struct WriteFingerprint {
    pub(crate) path: PathBuf,
    pub(crate) before_bytes: Option<usize>,
    pub(crate) before_sha256: Option<String>,
    pub(crate) after_bytes: Option<usize>,
    pub(crate) after_sha256: Option<String>,
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
    if plan.plan_version == 2 {
        return run_plan_v2(root, config, plan, dry);
    }
    let mut steps = Vec::new();
    let mut base_existence = BaseExistenceLedger::default();
    let mut dry_overlay = DryOverlay::default();
    let mut cached_graph: Option<SemanticGraph> = None;

    for step in &plan.steps {
        let cancel = Arc::new(AtomicBool::new(false));
        let candidate = CandidateWorkspace::create(root, config, &cancel)?;
        if dry {
            dry_overlay.replay(candidate.workspace_path())?;
        }

        let mut writes = Vec::new();
        let mut apply_error = None;
        for edit in &step.edits {
            match apply_edit(candidate.workspace_path(), edit) {
                Ok(write) => {
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
                write_fingerprints: None,
            });
            return finish_run(root, plan, steps, &step.id, &base_existence, dry);
        }

        // The graph "before" snapshot must be taken before this step's
        // edits land, so `tests.impacted`'s changed-node detection (and
        // Phase 4's graph.edge_delta) reflect exactly what this step did.
        let need_graph = checks_need_graph(&step.checks);
        let before_graph = need_graph
            .then(|| {
                take_or_build_graph(&mut cached_graph, || {
                    build_from_dir_with_config(candidate.workspace_path(), config)
                        .map(|(graph, _, _)| graph)
                })
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
                write_fingerprints: None,
            });
            return finish_run(root, plan, steps, &step.id, &base_existence, dry);
        }

        if dry {
            dry_overlay.record(&writes);
            update_graph_cache(
                &mut cached_graph,
                need_graph,
                !writes.is_empty(),
                after_graph,
            );
            steps.push(StepOutcome {
                id: step.id.clone(),
                passed: true,
                committed: false,
                files_changed,
                checks: check_outcomes,
                write_fingerprints: None,
            });
            continue;
        }

        // Drop the disposable copy before the real-tree commit: a crash
        // inside `commit_project_writes` (fault injection, or a real
        // crash) exits via `std::process::exit`, which never runs `Drop` —
        // if the copy were still alive at that point, its `.bitcode/
        // validation/...` directory would leak forever.
        drop(candidate);

        base_existence.observe(&writes);
        let written = commit_project_writes(root, writes)?;
        update_graph_cache(
            &mut cached_graph,
            need_graph,
            !written.is_empty(),
            after_graph,
        );
        steps.push(StepOutcome {
            id: step.id.clone(),
            passed: true,
            committed: true,
            files_changed: written,
            checks: check_outcomes,
            write_fingerprints: None,
        });
    }

    Ok(PlanRunOutcome {
        outcome: RunOutcome::Passed,
        steps,
    })
}

fn run_plan_v2(
    root: &Path,
    config: &ProjectConfig,
    plan: &Plan,
    dry: bool,
) -> std::io::Result<PlanRunOutcome> {
    let mut steps = Vec::new();
    let mut base_existence = BaseExistenceLedger::default();
    let mut dry_overlay = DryOverlay::default();
    let mut edit_state = EditState::default();

    for step in &plan.steps {
        let cancel = Arc::new(AtomicBool::new(false));
        let candidate = CandidateWorkspace::create(root, config, &cancel)?;
        if dry {
            dry_overlay.replay(candidate.workspace_path())?;
        }

        let need_graph = checks_need_graph(&step.checks);
        if need_graph || step.edits.iter().any(Edit::is_graph_addressed) {
            edit_state.ensure_graph(candidate.workspace_path(), config)?;
        }
        let before_graph = need_graph.then(|| edit_state.graph().cloned()).flatten();

        let writes = match apply_step_edits_v2(
            candidate.workspace_path(),
            config,
            &step.id,
            &step.edits,
            &mut edit_state,
        ) {
            Ok(writes) => writes,
            Err(error) => {
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
                    write_fingerprints: Some(Vec::new()),
                });
                return finish_run(root, plan, steps, &step.id, &base_existence, dry);
            }
        };

        let files_changed: Vec<PathBuf> = writes
            .iter()
            .map(|write| write.relative().to_path_buf())
            .collect();
        let write_fingerprints = fingerprint_writes(&writes);
        let after_graph = need_graph.then(|| edit_state.graph().cloned()).flatten();
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
            all_passed &= outcome.passed;
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
                write_fingerprints: Some(write_fingerprints),
            });
            return finish_run(root, plan, steps, &step.id, &base_existence, dry);
        }

        if dry {
            dry_overlay.record(&writes);
            steps.push(StepOutcome {
                id: step.id.clone(),
                passed: true,
                committed: false,
                files_changed,
                checks: check_outcomes,
                write_fingerprints: Some(write_fingerprints),
            });
            continue;
        }

        drop(candidate);
        base_existence.observe(&writes);
        let written = commit_project_writes(root, writes)?;
        steps.push(StepOutcome {
            id: step.id.clone(),
            passed: true,
            committed: true,
            files_changed: written,
            checks: check_outcomes,
            write_fingerprints: Some(write_fingerprints),
        });
    }

    Ok(PlanRunOutcome {
        outcome: RunOutcome::Passed,
        steps,
    })
}

fn fingerprint_writes(writes: &[crate::project::source::ProjectWrite]) -> Vec<WriteFingerprint> {
    writes
        .iter()
        .map(|write| WriteFingerprint {
            path: write.relative().to_path_buf(),
            before_bytes: write.expected().map(<[u8]>::len),
            before_sha256: write.expected().map(sha256_hex),
            after_bytes: write.contents().map(<[u8]>::len),
            after_sha256: write.contents().map(sha256_hex),
        })
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn take_or_build_graph(
    cache: &mut Option<SemanticGraph>,
    build: impl FnOnce() -> std::io::Result<SemanticGraph>,
) -> std::io::Result<SemanticGraph> {
    match cache.take() {
        Some(graph) => Ok(graph),
        None => build(),
    }
}

fn update_graph_cache(
    cache: &mut Option<SemanticGraph>,
    step_built_graph: bool,
    content_changed: bool,
    after_graph: Option<SemanticGraph>,
) {
    if step_built_graph {
        *cache = after_graph;
    } else if content_changed {
        *cache = None;
    }
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
    base_existence: &BaseExistenceLedger,
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
                rollback_to_base(root, plan, base_existence)?;
            }
            RunOutcome::RolledBackPlan {
                at_step: failed_step_id.to_string(),
            }
        }
    };
    Ok(PlanRunOutcome { outcome, steps })
}

#[derive(Default)]
struct BaseExistenceLedger {
    paths: BTreeMap<PathBuf, bool>,
}

impl BaseExistenceLedger {
    fn observe(&mut self, writes: &[crate::project::source::ProjectWrite]) {
        for write in writes {
            self.paths
                .entry(write.relative().to_path_buf())
                .or_insert_with(|| write.expected().is_some());
        }
    }
}

/// Restore every base-existing path a prior step committed, then delete every
/// base-new path. The ledger records existence on first touch, before later
/// steps can make a deleted base path look newly created.
fn rollback_to_base(
    root: &Path,
    plan: &Plan,
    base_existence: &BaseExistenceLedger,
) -> std::io::Result<()> {
    let restore: Vec<&Path> = base_existence
        .paths
        .iter()
        .filter_map(|(path, existed)| existed.then_some(path.as_path()))
        .collect();
    if !restore.is_empty() {
        git_checkout_paths(root, &plan.base_commit, &restore)?;
    }
    for (path, existed) in &base_existence.paths {
        if *existed {
            continue;
        }
        match std::fs::remove_file(root.join(path)) {
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
    use crate::project::planfile::schema::Plan;
    use crate::project::source::ProjectWrite;
    use std::collections::BTreeSet;
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    type NodeSnapshot = BTreeSet<(aether_graph::NodeId, String)>;
    type EdgeSnapshot = BTreeSet<(
        aether_graph::NodeId,
        aether_graph::NodeId,
        aether_graph::EdgeKind,
    )>;

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

    fn git(root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
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

    #[test]
    fn rollback_plan_restores_base_file_deleted_then_recreated_across_steps() {
        let repository = TempDir::new("delete-recreate-rollback");
        let tracked = repository.0.join("tracked.rs");
        std::fs::write(&tracked, "pub fn base_value() -> i32 { 1 }\n").unwrap();
        git(&repository.0, &["init", "--quiet"]);
        git(&repository.0, &["add", "tracked.rs"]);
        git(
            &repository.0,
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
        let base_commit = git(&repository.0, &["rev-parse", "HEAD"]);
        let plan: Plan = serde_json::from_value(serde_json::json!({
            "plan_version": 1,
            "plan_id": "delete-recreate-rollback",
            "intent": "restore a base-existing path after cross-step recreation",
            "base_commit": base_commit,
            "on_failure": "rollback_plan",
            "steps": [
                {
                    "id": "delete-tracked",
                    "edits": [{"path": "tracked.rs", "delete": true}]
                },
                {"id": "intervening-step"},
                {
                    "id": "recreate-tracked",
                    "edits": [{
                        "path": "tracked.rs",
                        "create": "pub fn recreated_value() -> i32 { 3 }\n"
                    }]
                },
                {
                    "id": "force-rollback",
                    "checks": [{"kind": "command", "run": "exit 9"}]
                }
            ]
        }))
        .unwrap();

        let result = run_plan(&repository.0, &ProjectConfig::default(), &plan, false).unwrap();

        assert!(matches!(
            result.outcome,
            RunOutcome::RolledBackPlan { ref at_step } if at_step == "force-rollback"
        ));
        assert_eq!(
            std::fs::read_to_string(tracked).unwrap(),
            "pub fn base_value() -> i32 { 1 }\n"
        );
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

    fn graph_snapshot(graph: &SemanticGraph) -> (NodeSnapshot, EdgeSnapshot) {
        let nodes = graph
            .nodes()
            .map(|node| (node.id, node.path.clone()))
            .collect();
        let edges = graph.edges().into_iter().collect();
        (nodes, edges)
    }

    #[test]
    fn graph_identity_is_independent_of_candidate_root() {
        let first = TempDir::new("identity-first");
        let second = TempDir::new("identity-second");
        for root in [&first.0, &second.0] {
            std::fs::create_dir_all(root.join("src")).unwrap();
            std::fs::write(
                root.join("src/lib.rs"),
                "pub fn target() {}\npub fn caller() { target(); }\n",
            )
            .unwrap();
        }
        let config = ProjectConfig::default();
        let (first_graph, _, _) = build_from_dir_with_config(&first.0, &config).unwrap();
        let (second_graph, _, _) = build_from_dir_with_config(&second.0, &config).unwrap();

        assert_eq!(graph_snapshot(&first_graph), graph_snapshot(&second_graph));
        assert!(first_graph.find_by_path("crate::lib::caller").is_some());
    }

    #[test]
    fn cached_after_graph_becomes_the_next_steps_before_graph_without_rebuilding() {
        let mut graph = SemanticGraph::new();
        let node =
            aether_graph::Node::new(aether_graph::NodeKind::Function, "cached", "crate::cached")
                .with_language("rust");
        graph.upsert_node(node.clone());
        let mut cache = Some(graph);
        let mut builds = 0;

        let reused = take_or_build_graph(&mut cache, || {
            builds += 1;
            Ok(SemanticGraph::new())
        })
        .unwrap();

        assert_eq!(builds, 0);
        assert!(reused.contains(node.id));
        assert!(
            cache.is_none(),
            "the cached snapshot is consumed exactly once"
        );
    }

    #[test]
    fn content_changing_non_graph_steps_invalidate_the_cache() {
        let mut cache = Some(SemanticGraph::new());
        update_graph_cache(&mut cache, false, true, None);
        assert!(cache.is_none());

        let replacement = SemanticGraph::new();
        update_graph_cache(&mut cache, true, true, Some(replacement));
        assert!(cache.is_some());
    }
}
