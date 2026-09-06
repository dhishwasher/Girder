//! `girder orient <dir> [--nodes <path>[,<path>...]] ["<intent>"] [--depth N] --json`
//! — read-only composite orientation. Resolves the target exactly as
//! `girder context` does (same [`build_authoring_context`]/
//! [`super::authoring_context::TOP_K`]/
//! [`super::authoring_context::NODE_SCORE_FLOOR_RATIO`] selection), then in
//! the same graph build reports its source, direct callers/callees to
//! `--depth` (default 1), the tests that cover it
//! ([`aether_graph::SemanticGraph::tests_for`], the same machinery
//! `test-impact` uses), and its impact set
//! ([`aether_graph::SemanticGraph::impact_of`], the same machinery `review`
//! uses) — one call in place of the `context` + `query` (callers) + `query`
//! (callees) + `test-impact` + `query` (impact) chain the `orient` MCP tool
//! otherwise requires. No writes, no model call, no network.

use super::authoring_context::{
    build_authoring_context, collect_words, invalid_input, parse_pinned_nodes, SelectedNode,
    SelectionError,
};
use crate::project::config::ProjectConfig;
use crate::project::source::build_from_dir_with_config;
use aether_graph::{EdgeKind, NodeId, SemanticGraph};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const USAGE: &str =
    "usage: girder orient <dir> [--nodes <path>[,<path>...]] [\"<intent>\"] [--depth N] --json";

/// Cap on how many paths one section (callers/callees/tests/impact) lists in
/// full before degrading to a count. Keeps one high-fan-out node from
/// producing an unbounded response on its own; `count` is always the real
/// total regardless of this cap, so nothing is silently dropped — only the
/// path *list* is capped, never the fact that more exist.
const MAX_LISTED_PER_SECTION: usize = 50;

/// Hops of callers/callees ever walked, regardless of a higher `--depth`
/// request.
///
/// // EXTENSION POINT: transitive caller/callee depth beyond 2 hops is not
/// computed. A request for more is capped here and the cap is reported back
/// (`depth_requested` vs `depth_applied`) rather than silently honored or
/// rejected — walking an unbounded number of hops on a dense graph is a
/// different cost profile than the one-call-instead-of-several trade this
/// tool exists to make.
const MAX_DEPTH: u32 = 2;

/// Absolute similarity-score floor below which an intent-resolved node is
/// reported with `"confidence": "low"` rather than presented as a plain
/// answer. Not relative to the top hit — [`super::authoring_context::NODE_SCORE_FLOOR_RATIO`]
/// already discards hits scoring below half of the top hit, so a low top hit
/// score would otherwise pass through unflagged. 0.12 sits just under the
/// 0.17 top-hit score `docs/core-gap-analysis.md` gap #15 recorded as a real
/// match, with 0.05-0.09 recorded as noise on that same run — the same
/// evidence `NODE_SCORE_FLOOR_RATIO`'s doc comment cites.
const LOW_CONFIDENCE_SCORE_FLOOR: f32 = 0.12;

pub fn orient(args: &[String]) -> std::io::Result<()> {
    let Some(root_arg) = args.first() else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let root = PathBuf::from(root_arg);

    // Like `girder context`: this command exists to emit exactly one
    // pipeable JSON object, so there is no human-readable fallback mode.
    if !args.iter().any(|arg| arg == "--json") {
        return Err(invalid_input("girder orient requires --json"));
    }

    let output = build_output(&root, args)?;
    let rendered = serde_json::to_string_pretty(&output)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    println!("{rendered}");
    Ok(())
}

/// Everything `orient()` does once `--json` is confirmed present, factored
/// out so tests can inspect the emitted JSON object directly.
fn build_output(root: &Path, args: &[String]) -> std::io::Result<Value> {
    let pinned_node_paths = parse_pinned_nodes(args)?;
    let intent = collect_words(args, &["--json"], &["--nodes", "--depth"]);
    let (depth_requested, depth_applied) = parse_depth(args)?;

    let config = ProjectConfig::load(root)?;
    let (graph, _builder, _files) = build_from_dir_with_config(root, &config)?;

    let ctx = match build_authoring_context(&graph, &intent, pinned_node_paths.as_deref()) {
        Ok(ctx) => ctx,
        Err(SelectionError::NoMatches) => {
            return Err(invalid_input(&format!("no nodes matched \"{intent}\"")));
        }
        Err(SelectionError::UnknownPath(path)) => {
            return Err(invalid_input(&format!(
                "--nodes references a path not present in the graph: {path}"
            )));
        }
    };

    let nodes: Vec<Value> = ctx
        .nodes
        .iter()
        .map(|selected| orient_one(&graph, selected, depth_applied))
        .collect();

    Ok(json!({
        "intent": intent,
        "depth_requested": depth_requested,
        "depth_applied": depth_applied,
        "nodes": nodes,
    }))
}

fn parse_depth(args: &[String]) -> std::io::Result<(u32, u32)> {
    let requested = match args.windows(2).find(|window| window[0] == "--depth") {
        Some(window) => window[1]
            .parse::<u32>()
            .map_err(|_| invalid_input("--depth must be a non-negative integer"))?,
        None => 1,
    };
    let applied = requested.min(MAX_DEPTH);
    Ok((requested, applied))
}

/// One selected node's full orientation entry: source plus every section a
/// chained `get_source` + `ask_codebase` (callers/callees/impact) +
/// `impacted_tests` call sequence would answer.
fn orient_one(graph: &SemanticGraph, selected: &SelectedNode, depth: u32) -> Value {
    let node = &selected.node;
    let id = node.id;

    let callers = traverse(graph, id, depth, true);
    let callees = traverse(graph, id, depth, false);
    let test_ids = graph.tests_for(id);
    // `impact_of` is the same unbounded backward-reachability BFS
    // `review`/`impacted_tests` use — it is not subject to `--depth`, which
    // only bounds the callers/callees sections above. It is also, like
    // those two callers, advisory where dynamic dispatch is involved:
    // // EXTENSION POINT: a call reached only through dynamic dispatch,
    // reflection, or another unproven indirection is not in `impact_of`'s
    // edge set (measured recall 0.000 on a polymorphic-dispatch case,
    // `docs/core-representative-mutations.md`) and so will not appear here
    // either, the same gap `impacted_tests`' MCP description already
    // discloses.
    let impact_ids: Vec<NodeId> = graph.impact_of(id).affected.keys().copied().collect();

    let mut entry = json!({
        "path": node.path,
        "language": node.language,
        "source": node.source,
        "callers": section(graph, callers, depth),
        "callees": section(graph, callees, depth),
        "tests": section(graph, test_ids, 0),
        "impact": section(graph, impact_ids, 0),
    });

    if let Some(score) = selected.score {
        if let Some(object) = entry.as_object_mut() {
            object.insert("score".to_string(), json!(score));
            object.insert(
                "confidence".to_string(),
                json!(if score < LOW_CONFIDENCE_SCORE_FLOOR {
                    "low"
                } else {
                    "high"
                }),
            );
        }
    }

    entry
}

/// Breadth-first callers (incoming `Calls` edges) or callees (outgoing
/// `Calls` edges) from `start`, up to `depth` hops, using the same
/// [`SemanticGraph::callers`]/[`SemanticGraph::neighbors`] the `ask_codebase`
/// MCP tool's Callers/Callees query shapes already use — no new graph
/// traversal is introduced here, only depth-bounded iteration of it.
///
/// // EXTENSION POINT: a call resolved only across a language boundary
/// (e.g. a Rust extractor's node calling into Python, or vice versa) is not
/// a `Calls` edge in this graph — project-wide resolution
/// (`aether-builder::sync::resolve_calls`) is per-language, so such a call
/// is invisible to this traversal the same way it is invisible to
/// `ask_codebase`.
fn traverse(graph: &SemanticGraph, start: NodeId, depth: u32, callers: bool) -> Vec<NodeId> {
    let mut visited: HashSet<NodeId> = HashSet::from([start]);
    let mut frontier = vec![start];
    let mut collected = Vec::new();
    for _ in 0..depth {
        let mut next_frontier = Vec::new();
        for &id in &frontier {
            let neighbors = if callers {
                graph.callers(id)
            } else {
                graph.neighbors(id, Some(EdgeKind::Calls))
            };
            for neighbor in neighbors {
                if visited.insert(neighbor.id) {
                    collected.push(neighbor.id);
                    next_frontier.push(neighbor.id);
                }
            }
        }
        if next_frontier.is_empty() {
            break;
        }
        frontier = next_frontier;
    }
    collected
}

/// Renders one section (callers/callees/tests/impact) as `{count, paths,
/// truncated}`: every path sorted for determinism, capped at
/// [`MAX_LISTED_PER_SECTION`] entries, with `count` always the true total —
/// the degrade-deliberately contract for output-cap safety. `depth` is
/// carried through only for the callers/callees sections (tests/impact pass
/// `0`, meaning "not depth-bounded").
fn section(graph: &SemanticGraph, ids: Vec<NodeId>, depth: u32) -> Value {
    let mut paths: Vec<String> = ids
        .iter()
        .filter_map(|&id| graph.get(id).map(|n| n.path.clone()))
        .collect();
    paths.sort();
    let count = paths.len();
    let truncated = count > MAX_LISTED_PER_SECTION;
    if truncated {
        paths.truncate(MAX_LISTED_PER_SECTION);
    }
    if depth > 0 {
        json!({"count": count, "paths": paths, "truncated": truncated, "depth": depth})
    } else {
        json!({"count": count, "paths": paths, "truncated": truncated})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(root: &Path, args: &[&str]) {
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
    }

    /// A real git project: `add` calls `helper`, and `caller` calls `add`,
    /// plus one `#[test]` that covers `add` — enough to exercise callers,
    /// callees, tests, and impact all at once.
    fn fixture_project(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "girder-orient-fixture-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("calc.rs"),
            "pub fn helper() -> i32 {\n    1\n}\n\npub fn add(a: i32, b: i32) -> i32 {\n    a + b + helper()\n}\n\npub fn caller() -> i32 {\n    add(1, 2)\n}\n\n#[test]\nfn test_add() {\n    assert_eq!(add(1, 2), 4);\n}\n",
        )
        .unwrap();
        git(&root, &["init", "--quiet"]);
        git(&root, &["add", "calc.rs"]);
        git(
            &root,
            &[
                "-c",
                "user.name=Girder Tests",
                "-c",
                "user.email=tests@girder.invalid",
                "commit",
                "--quiet",
                "-m",
                "base",
            ],
        );
        root
    }

    #[test]
    fn requires_a_dir_argument() {
        assert!(orient(&[]).is_ok());
    }

    #[test]
    fn requires_the_json_flag() {
        let root = fixture_project("requires-json");
        let args = vec![
            root.display().to_string(),
            "--nodes".to_string(),
            "crate::calc::add".to_string(),
        ];
        let error = orient(&args).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("requires --json"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reports_source_callers_callees_tests_and_impact_for_a_pinned_node() {
        let root = fixture_project("full-shape");
        let args = vec![
            root.display().to_string(),
            "--nodes".to_string(),
            "crate::calc::add".to_string(),
            "--json".to_string(),
        ];
        let output = build_output(&root, &args).unwrap();
        let node = &output["nodes"].as_array().unwrap()[0];

        assert_eq!(node["path"], "crate::calc::add");
        assert!(node["source"].as_str().unwrap().contains("fn add"));

        // Both `caller` and `test_add` directly call `add` (the test calls
        // it inside its own `assert_eq!`), so both are direct callers, not
        // just the one non-test caller.
        assert_eq!(node["callers"]["count"], 2);
        let caller_paths: Vec<&str> = node["callers"]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(caller_paths.contains(&"crate::calc::caller"));
        assert!(caller_paths.contains(&"crate::calc::test_add"));

        assert_eq!(node["callees"]["count"], 1);
        assert_eq!(
            node["callees"]["paths"].as_array().unwrap()[0],
            "crate::calc::helper"
        );

        assert_eq!(node["tests"]["count"], 1);
        assert_eq!(
            node["tests"]["paths"].as_array().unwrap()[0],
            "crate::calc::test_add"
        );

        // `caller` and `test_add` both call/cover `add`, so both are in its
        // impact set.
        let impact_paths: Vec<&str> = node["impact"]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(impact_paths.contains(&"crate::calc::caller"));
        assert!(impact_paths.contains(&"crate::calc::test_add"));

        assert!(node.get("confidence").is_none(), "a pinned node was never scored: {node}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn pinned_selection_never_reports_a_score_or_confidence() {
        let root = fixture_project("pinned-no-score");
        let args = vec![
            root.display().to_string(),
            "--nodes".to_string(),
            "crate::calc::helper".to_string(),
            "--json".to_string(),
        ];
        let output = build_output(&root, &args).unwrap();
        let node = &output["nodes"].as_array().unwrap()[0];
        assert!(node.get("score").is_none(), "{node}");
        assert!(node.get("confidence").is_none(), "{node}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn depth_two_reaches_a_second_hop_caller() {
        let root = fixture_project("depth-two");
        let args = vec![
            root.display().to_string(),
            "--nodes".to_string(),
            "crate::calc::helper".to_string(),
            "--json".to_string(),
            "--depth".to_string(),
            "2".to_string(),
        ];
        let output = build_output(&root, &args).unwrap();
        let node = &output["nodes"].as_array().unwrap()[0];
        // helper's direct caller is add; add's caller is caller — only
        // reachable at depth 2.
        let paths: Vec<&str> = node["callers"]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(paths.contains(&"crate::calc::add"));
        assert!(
            paths.contains(&"crate::calc::caller"),
            "depth 2 must reach add's own caller: {paths:?}"
        );
        assert_eq!(output["depth_applied"], 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_depth_beyond_the_cap_is_capped_and_reported() {
        let root = fixture_project("depth-capped");
        let args = vec![
            root.display().to_string(),
            "--nodes".to_string(),
            "crate::calc::add".to_string(),
            "--json".to_string(),
            "--depth".to_string(),
            "50".to_string(),
        ];
        let output = build_output(&root, &args).unwrap();
        assert_eq!(output["depth_requested"], 50);
        assert_eq!(output["depth_applied"], MAX_DEPTH);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unknown_pinned_path_selection_error_names_the_path() {
        let root = fixture_project("unknown-path");
        let args = vec![
            root.display().to_string(),
            "--nodes".to_string(),
            "crate::nonexistent::path".to_string(),
            "--json".to_string(),
        ];
        let error = build_output(&root, &args).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(
            error.to_string().contains("crate::nonexistent::path"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
