use crate::project::config::ProjectConfig;
use crate::project::source::{collect_sources_with_config, is_configured_source_path};
use aether_builder::GraphBuilder;
use aether_graph::{NodeId, NodeKind, SemanticGraph};
use std::collections::{BTreeSet, HashSet};
use std::path::Path;

pub(crate) fn git_is_repo(root: &Path) -> bool {
    std::process::Command::new("git")
        .args([
            "-C",
            root.to_str().unwrap_or("."),
            "rev-parse",
            "--is-inside-work-tree",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

fn git_prefix(root: &Path) -> String {
    std::process::Command::new("git")
        .args([
            "-C",
            root.to_str().unwrap_or("."),
            "rev-parse",
            "--show-prefix",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn strip_git_prefix(path: &str, prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        Some(path.to_string())
    } else {
        path.strip_prefix(prefix).map(str::to_string)
    }
}

fn git_object_path(root: &Path, rel: &str) -> String {
    format!("{}{}", git_prefix(root), rel)
}

pub(crate) fn git_tracked_sources_at(
    root: &Path,
    git_ref: &str,
    config: &ProjectConfig,
) -> Vec<String> {
    let prefix = git_prefix(root);
    std::process::Command::new("git")
        .args([
            "-C",
            root.to_str().unwrap_or("."),
            "ls-tree",
            "-r",
            "--name-only",
            git_ref,
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|line| strip_git_prefix(line, &prefix))
                .filter(|rel| is_configured_source_path(config, rel).unwrap_or(false))
                .collect()
        })
        .unwrap_or_default()
}
pub(crate) fn build_baseline_graph(root: &Path, git_ref: &str) -> std::io::Result<SemanticGraph> {
    let config = ProjectConfig::load(root)?;
    let mut sources: BTreeSet<String> = collect_sources_with_config(root, &config)?
        .into_iter()
        .map(|(_, rel)| rel)
        .collect();
    sources.extend(git_tracked_sources_at(root, git_ref, &config));

    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();

    for rel in &sources {
        let object_path = git_object_path(root, rel);
        let output = std::process::Command::new("git")
            .args([
                "-C",
                root.to_str().unwrap_or("."),
                "show",
                &format!("{git_ref}:{object_path}"),
            ])
            .output();
        match output {
            Ok(o) if o.status.success() => {
                if let Ok(text) = String::from_utf8(o.stdout) {
                    builder.load_file(&mut graph, rel, &text);
                }
            }
            // File didn't exist at that ref (new file) — skip silently.
            _ => {}
        }
    }
    Ok(graph)
}

/// `bitcode test-impact <dir> [--run] [node::path...]`
///
/// Finds the minimal set of tests that cover any changed (or specified) functions,
/// using the call graph to determine reachability. Runs them if `--run` is passed.
///
pub(crate) fn git_changed_files(root: &Path) -> std::io::Result<Vec<String>> {
    let config = ProjectConfig::load(root)?;
    let prefix = git_prefix(root);
    let run = |extra_args: &[&str]| -> Vec<String> {
        let mut cmd_args = vec!["-C", root.to_str().unwrap_or("."), "diff", "--name-only"];
        cmd_args.extend_from_slice(extra_args);
        std::process::Command::new("git")
            .args(&cmd_args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter_map(|l| strip_git_prefix(l, &prefix))
                    .filter(|l| is_configured_source_path(&config, l).unwrap_or(false))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for f in run(&["--cached"])
        .into_iter()
        .chain(run(&[]))
        .chain(git_untracked_sources(root, &config))
    {
        if seen.insert(f.clone()) {
            out.push(f);
        }
    }
    Ok(out)
}

fn git_untracked_sources(root: &Path, config: &ProjectConfig) -> Vec<String> {
    let prefix = git_prefix(root);
    std::process::Command::new("git")
        .args([
            "-C",
            root.to_str().unwrap_or("."),
            "ls-files",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| strip_git_prefix(l, &prefix))
                .filter(|l| is_configured_source_path(config, l).unwrap_or(false))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) struct ChangedImpact {
    pub(crate) origin_ids: Vec<NodeId>,
    pub(crate) baseline_test_paths: Vec<String>,
    pub(crate) changed_files: Vec<String>,
}

pub(crate) fn semantic_changed_impact(
    root: &Path,
    current: &SemanticGraph,
) -> std::io::Result<Option<ChangedImpact>> {
    if !git_is_repo(root) {
        return Ok(None);
    }

    let baseline = build_baseline_graph(root, "HEAD")?;
    let diff = current.diff_from(&baseline);
    if diff.is_empty() {
        return Ok(Some(ChangedImpact {
            origin_ids: Vec::new(),
            baseline_test_paths: Vec::new(),
            changed_files: git_changed_files(root)?,
        }));
    }

    let mut seen_origins = HashSet::new();
    let origin_ids: Vec<NodeId> = diff
        .changed_node_ids()
        .into_iter()
        .filter(|id| {
            current
                .get(*id)
                .map(|n| n.kind == NodeKind::Function)
                .unwrap_or(false)
                && seen_origins.insert(*id)
        })
        .collect();

    let mut seen_tests = HashSet::new();
    let baseline_test_paths: Vec<String> = diff
        .removed
        .iter()
        .filter(|c| c.kind == NodeKind::Function)
        .flat_map(|c| baseline.tests_for(c.id))
        .filter_map(|id| baseline.get(id).map(|n| n.path.clone()))
        .filter(|path| seen_tests.insert(path.clone()))
        .collect();

    Ok(Some(ChangedImpact {
        origin_ids,
        baseline_test_paths,
        changed_files: git_changed_files(root)?,
    }))
}
