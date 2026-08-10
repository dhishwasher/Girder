//! `tests.impacted` / `tests.named` / `tests.full`. Changed-node detection
//! for `tests.impacted` reuses the exact mechanism `commands/review.rs`
//! already uses for precise per-node semantic diffing
//! (`SemanticGraph::diff_from` / `GraphDiff::changed_node_ids`) — not a
//! coarse "every function in an edited file" heuristic, which would select
//! most of the suite and defeat the point of the check.

use super::CheckOutcome;
use crate::project::config::{ConfiguredCommand, ProjectConfig};
use crate::project::process::{run_captured, BoundedStatus};
use aether_graph::{NodeId, SemanticGraph};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// Nodes changed by a step's edits: the set `after.diff_from(&before)`
/// reports as added or modified. Mirrors `review`'s before/after pattern
/// (`crates/aether-app/src/project/commands/review.rs`), just sourced from
/// two on-disk snapshots of the disposable copy instead of a git ref.
pub(crate) fn changed_node_ids(before: &SemanticGraph, after: &SemanticGraph) -> Vec<NodeId> {
    after.diff_from(before).changed_node_ids()
}

fn tests_by_language(graph: &SemanticGraph, ids: &[NodeId]) -> (Vec<String>, Vec<String>) {
    let mut rust = Vec::new();
    let mut python = Vec::new();
    for id in ids {
        if let Some(node) = graph.get(*id) {
            match node.language.as_str() {
                "rust" => rust.push(node.name.clone()),
                "python" => python.push(node.name.clone()),
                _ => {}
            }
        }
    }
    (rust, python)
}

fn run_commands(
    workspace: &Path,
    config: &ProjectConfig,
    commands: &[(&str, ConfiguredCommand)],
) -> Vec<String> {
    let mut failures = Vec::new();
    for (language, command) in commands {
        let mut child = Command::new(&command.program);
        child.args(&command.args).current_dir(workspace);
        let timeout = Duration::from_secs(config.tests.run_timeout_seconds);
        match run_captured(child, timeout, config.tests.run_max_output_bytes) {
            Ok(run) => match run.status {
                BoundedStatus::Completed(status) if status.success() => {}
                BoundedStatus::Completed(status) => failures.push(format!(
                    "{language} command exited with {}: {}",
                    status
                        .code()
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "a signal".into()),
                    command.display()
                )),
                BoundedStatus::TimedOut => failures.push(format!(
                    "{language} command `{}` timed out after {}s",
                    command.display(),
                    timeout.as_secs()
                )),
                BoundedStatus::OutputLimited => failures.push(format!(
                    "{language} command `{}` exceeded its output budget",
                    command.display()
                )),
                BoundedStatus::Cancelled => failures.push(format!(
                    "{language} command `{}` was cancelled",
                    command.display()
                )),
            },
            Err(error) => failures.push(format!(
                "could not start {language} command `{}`: {error}",
                command.display()
            )),
        }
    }
    failures
}

pub(crate) fn run_tests_impacted(
    workspace: &Path,
    config: &ProjectConfig,
    graph: &SemanticGraph,
    changed: &[NodeId],
) -> CheckOutcome {
    let impacted = graph.tests_for_nodes(changed);
    if impacted.is_empty() {
        return CheckOutcome {
            kind: "tests.impacted".to_string(),
            passed: true,
            detail: "no impacted tests for this step's changed nodes".to_string(),
        };
    }
    let (rust_tests, python_tests) = tests_by_language(graph, &impacted);
    run_selected(
        workspace,
        config,
        "tests.impacted",
        &rust_tests,
        &python_tests,
    )
}

pub(crate) fn run_tests_named(
    workspace: &Path,
    config: &ProjectConfig,
    graph: &SemanticGraph,
    tests: &[String],
) -> CheckOutcome {
    let mut ids = Vec::new();
    let mut missing = Vec::new();
    for path in tests {
        match graph.find_by_path(path) {
            Some(node) => ids.push(node.id),
            None => missing.push(path.clone()),
        }
    }
    if !missing.is_empty() {
        return CheckOutcome {
            kind: "tests.named".to_string(),
            passed: false,
            detail: format!("unknown test node path(s): {}", missing.join(", ")),
        };
    }
    let (rust_tests, python_tests) = tests_by_language(graph, &ids);
    run_selected(workspace, config, "tests.named", &rust_tests, &python_tests)
}

pub(crate) fn run_tests_full(workspace: &Path, config: &ProjectConfig) -> CheckOutcome {
    let mut commands: Vec<(&str, ConfiguredCommand)> = Vec::new();
    if workspace.join("Cargo.toml").is_file() {
        commands.push((
            "Rust",
            ConfiguredCommand {
                program: "cargo".to_string(),
                args: ["test", "--workspace"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            },
        ));
    }
    if let Some(command) = config.python_test_command("") {
        commands.push(("Python", command));
    }
    if commands.is_empty() {
        return CheckOutcome {
            kind: "tests.full".to_string(),
            passed: false,
            detail:
                "no full-suite test command available (no Cargo.toml, no tests.python configured)"
                    .to_string(),
        };
    }
    let failures = run_commands(workspace, config, &commands);
    CheckOutcome {
        kind: "tests.full".to_string(),
        passed: failures.is_empty(),
        detail: if failures.is_empty() {
            "full suite passed".to_string()
        } else {
            failures.join("; ")
        },
    }
}

fn run_selected(
    workspace: &Path,
    config: &ProjectConfig,
    kind: &str,
    rust_tests: &[String],
    python_tests: &[String],
) -> CheckOutcome {
    let mut commands: Vec<(&str, ConfiguredCommand)> = Vec::new();
    for name in rust_tests {
        if let Some(command) = config.rust_test_command(name) {
            commands.push(("Rust", command));
        }
    }
    if !python_tests.is_empty() {
        if let Some(command) = config.python_test_command(&python_tests.join(" or ")) {
            commands.push(("Python", command));
        }
    }
    if commands.is_empty() {
        return CheckOutcome {
            kind: kind.to_string(),
            passed: false,
            detail: "test commands are not configured in bitcode.toml".to_string(),
        };
    }
    let failures = run_commands(workspace, config, &commands);
    CheckOutcome {
        kind: kind.to_string(),
        passed: failures.is_empty(),
        detail: if failures.is_empty() {
            format!("{} test(s) passed", rust_tests.len() + python_tests.len())
        } else {
            failures.join("; ")
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_builder::GraphBuilder;

    fn graph_from(files: &[(&str, &str)]) -> SemanticGraph {
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files.iter().copied());
        graph
    }

    #[test]
    fn changed_node_ids_selects_only_the_edited_function_not_the_whole_file() {
        let before = graph_from(&[(
            "src/code.rs",
            "pub fn untouched() -> i64 { 1 }\npub fn edited() -> i64 { 2 }\n",
        )]);
        let after = graph_from(&[(
            "src/code.rs",
            "pub fn untouched() -> i64 { 1 }\npub fn edited() -> i64 { 3 }\n",
        )]);

        let changed = changed_node_ids(&before, &after);
        let paths: Vec<String> = changed
            .iter()
            .filter_map(|id| after.get(*id).map(|n| n.path.clone()))
            .collect();
        // The enclosing module node's content hash also changes (it stores
        // the whole file source), so it legitimately appears alongside the
        // edited function — the assertion that matters is that the
        // *untouched* sibling function is never selected.
        assert!(
            paths.contains(&"crate::code::edited".to_string()),
            "{paths:?}"
        );
        assert!(
            !paths.contains(&"crate::code::untouched".to_string()),
            "{paths:?}"
        );
    }
}
