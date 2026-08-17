//! `bitcode context <dir> [--nodes <path>[,<path>...]] ["<intent>"] --json`
//! — read-only: builds the graph, selects nodes exactly as `bitcode do`
//! would (same [`TOP_K`](super::authoring_context::TOP_K)/
//! [`NODE_SCORE_FLOOR_RATIO`](super::authoring_context::NODE_SCORE_FLOOR_RATIO)),
//! and prints the same `{path, language, source}` node context and a real
//! Plan Format v2 authoring schema, plus a plan skeleton, as one JSON
//! object on stdout. No model call, no network, no writes — this is `do`
//! split at the model boundary, for pasting into an external chat model
//! that isn't wired in as a provider.

use super::authoring_context::{
    build_authoring_context, collect_words, generate_plan_id, invalid_input, parse_pinned_nodes,
    plan_schema, plan_skeleton, SelectionError,
};
use crate::project::config::ProjectConfig;
use crate::project::git::git_head_commit;
use crate::project::source::build_from_dir_with_config;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const USAGE: &str =
    "usage: bitcode context <dir> [--nodes <path>[,<path>...]] [\"<intent>\"] --json";

pub fn context(args: &[String]) -> std::io::Result<()> {
    let Some(root_arg) = args.first() else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let root = PathBuf::from(root_arg);

    // `--json` is the only output mode implemented: this command exists to
    // emit exactly one pipeable/pasteable JSON object, so there is no
    // human-readable fallback to silently produce instead.
    if !args.iter().any(|arg| arg == "--json") {
        return Err(invalid_input("bitcode context requires --json"));
    }

    let output = build_output(&root, args)?;
    let rendered = serde_json::to_string_pretty(&output)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    println!("{rendered}");
    Ok(())
}

/// Everything `context()` does once `--json` is confirmed present, factored
/// out so tests can inspect the emitted JSON object's structure directly
/// instead of scraping stdout. `context()` is the sole non-test caller and
/// still prints exactly what this returns, unchanged.
fn build_output(root: &Path, args: &[String]) -> std::io::Result<Value> {
    let pinned_node_paths = parse_pinned_nodes(args)?;
    let intent = collect_words(args, &["--json"], &["--nodes"]);

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

    let base_commit = git_head_commit(root)?;
    let plan_id = generate_plan_id();
    Ok(json!({
        "intent": intent,
        "base_commit": base_commit,
        "nodes": ctx.node_context,
        // Real Plan Format v2 (see `plan_schema`'s doc comment), not
        // `ctx.schema` — this JSON goes to an external model that writes a
        // plan file directly, straight into `load_plan`, with no
        // translation layer the way `bitcode do`'s local path has
        // (`author::convert_edit`/`convert_check`).
        "schema": plan_schema(&ctx.node_paths),
        "plan_skeleton": plan_skeleton(&base_commit, &intent, &plan_id),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    fn write_temp_plan(name: &str, plan: &Value) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "bitcode-context-cmd-{name}-{}-{}.json",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, serde_json::to_vec_pretty(plan).unwrap()).unwrap();
        path
    }

    fn plan_with_step(step: Value) -> Value {
        json!({
            "plan_version": 2,
            "plan_id": "schema-shape-test",
            "intent": "test",
            "base_commit": "deadbeef",
            "on_failure": "rollback_plan",
            "steps": [step]
        })
    }

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

    /// A tiny real git project with one function to select against, so
    /// `build_output()` can run end to end (graph build, `git_head_commit`,
    /// node selection) without a model, a network, or any writes.
    fn fixture_project(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "bitcode-context-cmd-fixture-{name}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("calc.rs"),
            "pub fn greet() -> String {\n    \"hi\".to_string()\n}\n",
        )
        .unwrap();
        git(&root, &["init", "--quiet"]);
        git(&root, &["add", "calc.rs"]);
        git(
            &root,
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
        root
    }

    #[test]
    fn requires_a_dir_argument() {
        assert!(context(&[]).is_ok());
    }

    #[test]
    fn requires_the_json_flag() {
        let root = fixture_project("requires-json");
        let args = vec![
            root.display().to_string(),
            "--nodes".to_string(),
            "crate::calc::greet".to_string(),
        ];
        let error = context(&args).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("requires --json"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_matches_selection_error_is_reported_and_read_only() {
        let root = fixture_project("no-matches");
        let args = vec![
            root.display().to_string(),
            "nothing on this earth will match this intent string".to_string(),
            "--json".to_string(),
        ];
        let error = build_output(&root, &args).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("no nodes matched"), "{error}");
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

    #[test]
    fn emits_every_top_level_key_with_the_documented_shape() {
        let root = fixture_project("top-level-keys");
        let args = vec![
            root.display().to_string(),
            "--nodes".to_string(),
            "crate::calc::greet".to_string(),
            "--json".to_string(),
        ];
        let output = build_output(&root, &args).unwrap();

        let object = output.as_object().unwrap();
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["base_commit", "intent", "nodes", "plan_skeleton", "schema"]
        );

        assert!(output["intent"].is_string());
        assert!(output["base_commit"].is_string());
        assert_eq!(output["base_commit"].as_str().unwrap().len(), 40);

        let nodes = output["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 1);
        let node = &nodes[0];
        assert_eq!(node["path"], "crate::calc::greet");
        assert_eq!(node["language"], "rust");
        assert!(node["source"].as_str().unwrap().contains("fn greet"));

        // `schema` is real Plan Format v2 (see `plan_schema`), not the flat
        // local-authoring shape.
        assert!(output["schema"]["properties"]["edits"]["items"]["oneOf"].is_array());
        assert!(output["schema"]["properties"]["checks"]["items"]["oneOf"].is_array());

        let skeleton = &output["plan_skeleton"];
        assert_eq!(skeleton["plan_version"], 2);
        assert_eq!(skeleton["base_commit"], output["base_commit"]);
        assert_eq!(skeleton["on_failure"], "rollback_plan");
        let steps = skeleton["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 1);
        let checks = steps[0]["checks"].as_array().unwrap();
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0]["kind"], "tests.impacted");
        assert_eq!(checks[0]["expect"], "all_pass");

        let _ = std::fs::remove_dir_all(&root);
    }

    // The bug this closes: `bitcode context` used to hand an external model
    // `authoring_context::step_schema` — the flat shape `bitcode do`'s local
    // path translates via `convert_edit`/`convert_check` before it ever
    // reaches a plan file. An external model has no such translation layer;
    // its output goes straight to `load_plan`. This test proves a step
    // literally satisfying that flat schema is rejected by the real loader,
    // which is why `context` must emit `plan_schema` instead (see the test
    // below).
    #[test]
    fn a_step_satisfying_the_flat_local_authoring_schema_is_rejected_by_load_plan() {
        let node_paths = vec!["crate::calc::greet".to_string()];
        let step = json!({
            "id": "step-1",
            "description": "",
            "edits": [{
                "node": "crate::calc::greet",
                "operation": "replace_node",
                "replace_node": "fn greet() {}",
                "rename_node": "",
                "delete_node": false,
                "insert_into_module": ""
            }],
            "checks": [{
                "kind": "graph.node_exists",
                "node": "crate::calc::greet",
                "expect": [],
                "run": "-",
                "expect_exit": 0
            }]
        });
        // `edits[0]` and `checks[0]` above are built to satisfy
        // `authoring_context::step_schema(&node_paths)` exactly.
        let _ = crate::project::commands::authoring_context::step_schema(&node_paths);
        let path = write_temp_plan("flat-schema-rejected", &plan_with_step(step));

        let error = crate::project::planfile::load_plan(&path).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_step_satisfying_plan_schema_round_trips_through_load_plan() {
        let node_paths = vec!["crate::calc::greet".to_string()];
        let schema = plan_schema(&node_paths);
        // Sanity on the schema shape itself: an edit is a discriminated
        // union (`oneOf`), not the flat all-fields-required shape.
        assert!(schema["properties"]["edits"]["items"]["oneOf"].is_array());
        assert!(schema["properties"]["checks"]["items"]["oneOf"].is_array());

        let step = json!({
            "id": "step-1",
            "description": "uppercase greet's greeting",
            "edits": [{"node": "crate::calc::greet", "replace_node": "fn greet() {}"}],
            "checks": [{"kind": "graph.node_exists", "node": "crate::calc::greet"}]
        });
        let path = write_temp_plan("plan-schema-round-trip", &plan_with_step(step));

        let loaded = crate::project::planfile::load_plan(&path)
            .expect("a step satisfying plan_schema must be accepted by load_plan");

        assert_eq!(loaded.steps.len(), 1);
        assert_eq!(loaded.steps[0].edits.len(), 1);
        assert_eq!(loaded.steps[0].checks.len(), 1);
        let _ = std::fs::remove_file(&path);
    }
}
