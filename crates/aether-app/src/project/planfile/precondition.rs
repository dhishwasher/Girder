//! Plan-wide preconditions, checked once before any edit is applied.
//!
//! All failures are collected rather than short-circuited, so `plan
//! validate`/`plan run` reports everything wrong with a plan at once. Only
//! read-only filesystem/git operations happen here — `safe_project_input_path`
//! (not the output variant) is used deliberately so checking a plan never
//! creates a directory as a side effect.

use crate::project::config::ProjectConfig;
use crate::project::git::{git_head_commit, git_worktree_clean};
use crate::project::planfile::edit::{apply_step_edits_v2, count_occurrences, EditState};
use crate::project::planfile::schema::{Edit, Plan};
use crate::project::source::safe_project_input_path;
use crate::project::validation::CandidateWorkspace;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub(crate) struct PreconditionFailure {
    pub(crate) reason: String,
}

pub(crate) fn check_preconditions(
    root: &Path,
    plan: &Plan,
) -> std::io::Result<Result<(), Vec<PreconditionFailure>>> {
    let mut failures = Vec::new();

    match git_worktree_clean(root) {
        Ok(true) => {}
        Ok(false) => failures.push(PreconditionFailure {
            reason: "worktree is not clean (git status --porcelain reported changes)".into(),
        }),
        Err(error) => failures.push(PreconditionFailure {
            reason: format!("could not check worktree cleanliness: {error}"),
        }),
    }

    match git_head_commit(root) {
        Ok(head) if head == plan.base_commit => {}
        Ok(head) => failures.push(PreconditionFailure {
            reason: format!(
                "HEAD is {head} but the plan's base_commit is {}",
                plan.base_commit
            ),
        }),
        Err(error) => failures.push(PreconditionFailure {
            reason: format!("could not resolve HEAD: {error}"),
        }),
    }

    if plan.plan_version == 2 && !failures.is_empty() {
        return Ok(Err(failures));
    }
    if plan.plan_version == 2 {
        let config = ProjectConfig::load(root)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let candidate = CandidateWorkspace::create(root, &config, &cancel)?;
        let mut state = EditState::default();
        for step in &plan.steps {
            if let Err(error) = apply_step_edits_v2(
                candidate.workspace_path(),
                &config,
                &step.id,
                &step.edits,
                &mut state,
            ) {
                failures.push(PreconditionFailure {
                    reason: error.to_string(),
                });
                break;
            }
        }
        return if failures.is_empty() {
            Ok(Ok(()))
        } else {
            Ok(Err(failures))
        };
    }

    // A later step's edit is validated against the state earlier steps in
    // *this plan* would leave behind, not against the pristine on-disk
    // file — otherwise a plan whose step 2 edits text that only exists
    // after step 1 runs would always fail preconditions before step 1 ever
    // executes. `None` means "does not exist" (deleted, or never created).
    let mut virtual_files: HashMap<String, Option<String>> = HashMap::new();
    for step in &plan.steps {
        for edit in &step.edits {
            check_edit(root, &step.id, edit, &mut failures, &mut virtual_files);
        }
    }

    if failures.is_empty() {
        Ok(Ok(()))
    } else {
        Ok(Err(failures))
    }
}

fn check_edit(
    root: &Path,
    step_id: &str,
    edit: &Edit,
    failures: &mut Vec<PreconditionFailure>,
    virtual_files: &mut HashMap<String, Option<String>>,
) {
    let Some(edit_path) = edit.path() else {
        failures.push(PreconditionFailure {
            reason: format!(
                "step {step_id}: graph-addressed edit for node {:?} requires plan_version 2 lowering",
                edit.node().unwrap_or("<missing node>")
            ),
        });
        return;
    };
    let validated = match safe_project_input_path(root, edit_path) {
        Ok(path) => path,
        Err(error) => {
            failures.push(PreconditionFailure {
                reason: format!(
                    "step {step_id}: edit path {:?} does not resolve inside the project: {error}",
                    edit_path
                ),
            });
            return;
        }
    };

    let path = edit_path.to_string();
    let current: Option<String> = match virtual_files.get(&path) {
        Some(state) => state.clone(),
        None => match std::fs::read(&validated) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => Some(text),
                Err(error) => {
                    failures.push(PreconditionFailure {
                        reason: format!("step {step_id}: {path} is not valid UTF-8: {error}"),
                    });
                    return;
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                failures.push(PreconditionFailure {
                    reason: format!("step {step_id}: could not read {path}: {error}"),
                });
                return;
            }
        },
    };

    match edit {
        Edit::Substitute {
            match_text,
            replace,
            occurrences,
            ..
        } => {
            let Some(contents) = current else {
                failures.push(PreconditionFailure {
                    reason: format!(
                        "step {step_id}: {path} does not exist at this point in the plan"
                    ),
                });
                return;
            };
            let found = count_occurrences(&contents, match_text);
            let expected = *occurrences as usize;
            if found != expected {
                failures.push(PreconditionFailure {
                    reason: format!(
                        "step {step_id}: {path} expects {expected} occurrence(s) of the given \
                         match text, found {found}"
                    ),
                });
                // Keep validating downstream steps against the unmodified
                // state rather than aborting the whole precondition pass.
                virtual_files.insert(path, Some(contents));
                return;
            }
            let updated = contents.replace(match_text.as_str(), replace);
            virtual_files.insert(path, Some(updated));
        }
        Edit::Create { create, .. } => {
            if current.is_some() {
                failures.push(PreconditionFailure {
                    reason: format!(
                        "step {step_id}: {path} already exists at this point in the plan; \
                         create edits never overwrite"
                    ),
                });
                return;
            }
            virtual_files.insert(path, Some(create.clone()));
        }
        Edit::Delete { delete, .. } => {
            if !*delete {
                failures.push(PreconditionFailure {
                    reason: format!(
                        "step {step_id}: {path}: a delete edit must set \"delete\": true"
                    ),
                });
                return;
            }
            if current.is_none() {
                failures.push(PreconditionFailure {
                    reason: format!(
                        "step {step_id}: {path} does not exist at this point in the plan; \
                         nothing to delete"
                    ),
                });
                return;
            }
            virtual_files.insert(path, None);
        }
        Edit::ReplaceNode { .. }
        | Edit::RenameNode { .. }
        | Edit::DeleteNode { .. }
        | Edit::InsertIntoModule { .. } => unreachable!("graph edits returned above"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::planfile::schema::OnFailure;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "bitcode-planfile-precondition-{name}-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn git(&self, args: &[&str]) {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(&self.0)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn init_repo(name: &str) -> (TempDir, String) {
        let dir = TempDir::new(name);
        dir.git(&["init", "-q"]);
        dir.git(&["config", "user.email", "bitcode@example.invalid"]);
        dir.git(&["config", "user.name", "Bit Code Test"]);
        std::fs::write(dir.0.join("src.rs"), "fn old() {}\n").unwrap();
        dir.git(&["add", "."]);
        dir.git(&["commit", "-q", "-m", "init"]);
        let head = String::from_utf8(
            std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&dir.0)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();
        (dir, head)
    }

    fn plan_with(base_commit: &str, edits: Vec<Edit>) -> Plan {
        Plan {
            plan_version: 1,
            plan_id: "test-plan".into(),
            intent: "test".into(),
            author: None,
            base_commit: base_commit.into(),
            on_failure: OnFailure::RollbackPlan,
            steps: vec![crate::project::planfile::schema::Step {
                id: "s1".into(),
                description: String::new(),
                edits,
                checks: Vec::new(),
            }],
        }
    }

    #[test]
    fn passes_on_a_clean_matching_plan() {
        let (dir, head) = init_repo("clean-pass");
        let plan = plan_with(
            &head,
            vec![Edit::Substitute {
                path: "src.rs".into(),
                match_text: "fn old() {}\n".into(),
                replace: "fn new() {}\n".into(),
                occurrences: 1,
            }],
        );
        let result = check_preconditions(&dir.0, &plan).unwrap();
        assert!(
            result.is_ok(),
            "expected pass, got {:?}",
            result
                .err()
                .map(|f| f.into_iter().map(|f| f.reason).collect::<Vec<_>>())
        );
    }

    #[test]
    fn reports_all_failures_together_not_just_the_first() {
        let (dir, _head) = init_repo("multi-failure");
        std::fs::write(dir.0.join("dirty.rs"), "uncommitted\n").unwrap();

        let plan = plan_with(
            "0000000000000000000000000000000000000000",
            vec![
                Edit::Substitute {
                    path: "src.rs".into(),
                    match_text: "does not exist anywhere".into(),
                    replace: "x".into(),
                    occurrences: 1,
                },
                Edit::Substitute {
                    path: "../escape.rs".into(),
                    match_text: "x".into(),
                    replace: "y".into(),
                    occurrences: 1,
                },
            ],
        );
        let failures = check_preconditions(&dir.0, &plan).unwrap().unwrap_err();
        // worktree dirty + wrong base_commit + match-count mismatch + path escape
        assert_eq!(
            failures.len(),
            4,
            "{:?}",
            failures.iter().map(|f| &f.reason).collect::<Vec<_>>()
        );
    }

    #[test]
    fn match_count_mismatch_is_reported_precisely() {
        let (dir, head) = init_repo("match-mismatch");
        let plan = plan_with(
            &head,
            vec![Edit::Substitute {
                path: "src.rs".into(),
                match_text: "fn old() {}\n".into(),
                replace: "x".into(),
                occurrences: 2,
            }],
        );
        let failures = check_preconditions(&dir.0, &plan).unwrap().unwrap_err();
        assert_eq!(failures.len(), 1);
        assert!(
            failures[0].reason.contains("expects 2"),
            "{}",
            failures[0].reason
        );
        assert!(
            failures[0].reason.contains("found 1"),
            "{}",
            failures[0].reason
        );
    }
}
