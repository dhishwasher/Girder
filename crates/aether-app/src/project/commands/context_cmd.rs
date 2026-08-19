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
    build_authoring_context, collect_words, generate_plan_id, invalid_input, node_context_entry,
    parse_pinned_nodes, plan_schema, plan_skeleton, SelectionError,
};
use crate::project::config::ProjectConfig;
use crate::project::git::git_head_commit;
use crate::project::source::build_from_dir_with_config;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: bitcode context <dir> [--nodes <path>[,<path>...]] [\"<intent>\"] --json [--with-tests]";

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
    let intent = collect_words(args, &["--json", "--with-tests"], &["--nodes"]);
    let pinned = pinned_node_paths.as_deref();
    if args.iter().any(|arg| arg == "--with-tests") {
        build_context_json_inner(root, &intent, pinned, true)
    } else {
        // Routes through the same `build_context_json` the GUI calls, so
        // that function keeps a real, always-compiled (non-gui, non-test)
        // caller instead of only test-module and gui-gated ones.
        build_context_json(root, &intent, pinned)
    }
}

/// The body of `build_output`, taking an already-resolved intent and pinned
/// node paths instead of raw CLI args. Split out so the GUI's "Copy context
/// JSON" button can produce exactly the same object `bitcode context --json`
/// prints without building a fake args vector — same node selection, same
/// schema, same plan skeleton, no reimplementation. Always builds without
/// `--with-tests` (the GUI button doesn't expose that flag yet) — the CLI's
/// `build_output` is the only caller that can request it, via
/// `build_context_json_inner`.
pub(crate) fn build_context_json(
    root: &Path,
    intent: &str,
    pinned_node_paths: Option<&[String]>,
) -> std::io::Result<Value> {
    build_context_json_inner(root, intent, pinned_node_paths, false)
}

/// Real body shared by both `build_context_json` (GUI, `--with-tests`
/// always off) and the CLI's `build_output` (honors `--with-tests`). When
/// `with_tests` is set, each selected node gets a `"tests"` object: full
/// `{path, language, source}` context for at most 3 covering tests (reusing
/// the same reachability `test-impact` uses via `graph.tests_for`), and
/// `{"path": ...}`-only entries for the rest, so the payload cannot blow up
/// on a heavily tested node. When unset, no extra graph traversal happens
/// and no `"tests"` key is added — default `context` cost is unchanged.
fn build_context_json_inner(
    root: &Path,
    intent: &str,
    pinned_node_paths: Option<&[String]>,
    with_tests: bool,
) -> std::io::Result<Value> {
    let config = ProjectConfig::load(root)?;
    let (graph, _builder, _files) = build_from_dir_with_config(root, &config)?;

    let ctx = match build_authoring_context(&graph, intent, pinned_node_paths) {
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

    let mut node_context = ctx.node_context;
    if with_tests {
        const MAX_FULL_TESTS: usize = 3;
        for (selected, entry) in ctx.nodes.iter().zip(node_context.iter_mut()) {
            let test_ids = graph.tests_for(selected.node.id);
            let full: Vec<Value> = test_ids
                .iter()
                .take(MAX_FULL_TESTS)
                .filter_map(|&id| graph.get(id))
                .map(node_context_entry)
                .collect();
            let names_only: Vec<Value> = test_ids
                .iter()
                .skip(MAX_FULL_TESTS)
                .filter_map(|&id| graph.get(id))
                .map(|node| json!({"path": node.path}))
                .collect();
            if let Some(object) = entry.as_object_mut() {
                object.insert(
                    "tests".to_string(),
                    json!({"full": full, "names_only": names_only}),
                );
            }
        }
    }

    let base_commit = git_head_commit(root)?;
    let plan_id = generate_plan_id();
    Ok(json!({
        "intent": intent,
        "base_commit": base_commit,
        "nodes": node_context,
        // Real Plan Format v2 (see `plan_schema`'s doc comment), not
        // `ctx.schema` — this JSON goes to an external model that writes a
        // plan file directly, straight into `load_plan`, with no
        // translation layer the way `bitcode do`'s local path has
        // (`author::convert_edit`/`convert_check`).
        "schema": plan_schema(&ctx.node_paths),
        "plan_skeleton": plan_skeleton(&base_commit, intent, &plan_id),
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

    /// Same fixture shape as `fixture_project`, but with two distinct
    /// functions so a test can pin either one and compare.
    fn two_node_fixture_project(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "bitcode-context-cmd-fixture-{name}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("calc.rs"),
            "pub fn greet() -> String {\n    \"hi\".to_string()\n}\n\npub fn hello() -> String {\n    \"hello\".to_string()\n}\n",
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

    // The GUI's "Copy context JSON" button (see `app.rs::copy_author_context_json`)
    // recomputes `pinned_node_paths` from live checkbox state on every click
    // and calls `build_context_json` directly — this pins that contract at
    // the function boundary the GUI actually calls: two calls that differ
    // only in which node is pinned must differ in `nodes` (obviously) and
    // in `plan_skeleton.plan_id` (since a real second build was performed,
    // not a cached one — `plan_id` embeds a fresh millisecond timestamp
    // every call, so identical output across two different-selection calls
    // is a caching/staleness bug, not a coincidence).
    #[test]
    fn build_context_json_with_different_pinned_nodes_differs_in_nodes_and_plan_id() {
        let root = two_node_fixture_project("pinned-selection-differs");

        let first = build_context_json(
            &root,
            "make hello end with an exclamation mark",
            Some(&["crate::calc::greet".to_string()]),
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = build_context_json(
            &root,
            "make hello end with an exclamation mark",
            Some(&["crate::calc::hello".to_string()]),
        )
        .unwrap();

        assert_ne!(first["nodes"], second["nodes"], "{first} vs {second}");
        assert_eq!(
            first["nodes"].as_array().unwrap()[0]["path"],
            "crate::calc::greet"
        );
        assert_eq!(
            second["nodes"].as_array().unwrap()[0]["path"],
            "crate::calc::hello"
        );
        assert_ne!(
            first["plan_skeleton"]["plan_id"], second["plan_skeleton"]["plan_id"],
            "two separate builds must never share a plan_id: {first} vs {second}"
        );

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

    /// A real git project with one function and one real `#[test]` that
    /// calls it, so `graph.tests_for` has a genuine covering test to find.
    fn fixture_project_with_one_covering_test(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "bitcode-context-cmd-fixture-{name}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("calc.rs"),
            "pub fn greet() -> String {\n    \"hi\".to_string()\n}\n\n#[test]\nfn test_greet() {\n    assert_eq!(greet(), \"hi\");\n}\n",
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
    fn without_with_tests_flag_no_tests_key_is_present() {
        let root = fixture_project_with_one_covering_test("no-with-tests-flag");
        let args = vec![
            root.display().to_string(),
            "--nodes".to_string(),
            "crate::calc::greet".to_string(),
            "--json".to_string(),
        ];
        let output = build_output(&root, &args).unwrap();
        let node = &output["nodes"].as_array().unwrap()[0];
        assert!(
            node.get("tests").is_none(),
            "no \"tests\" key must appear without --with-tests: {node}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn with_tests_flag_attaches_full_source_of_the_covering_test() {
        let root = fixture_project_with_one_covering_test("with-tests-flag");
        let args = vec![
            root.display().to_string(),
            "--nodes".to_string(),
            "crate::calc::greet".to_string(),
            "--json".to_string(),
            "--with-tests".to_string(),
        ];
        let output = build_output(&root, &args).unwrap();
        let node = &output["nodes"].as_array().unwrap()[0];
        let full = node["tests"]["full"].as_array().unwrap();
        assert_eq!(full.len(), 1, "{node}");
        assert_eq!(full[0]["path"], "crate::calc::test_greet");
        assert_eq!(full[0]["language"], "rust");
        assert!(full[0]["source"]
            .as_str()
            .unwrap()
            .contains("fn test_greet"));
        assert!(node["tests"]["names_only"].as_array().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn with_tests_flag_caps_full_source_at_three_and_names_only_beyond() {
        let root = std::env::temp_dir().join(format!(
            "bitcode-context-cmd-fixture-many-covering-tests-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut source = String::from("pub fn greet() -> String {\n    \"hi\".to_string()\n}\n\n");
        for i in 0..5 {
            source.push_str(&format!(
                "#[test]\nfn test_greet_{i}() {{\n    assert_eq!(greet(), \"hi\");\n}}\n\n"
            ));
        }
        std::fs::write(root.join("calc.rs"), source).unwrap();
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

        let args = vec![
            root.display().to_string(),
            "--nodes".to_string(),
            "crate::calc::greet".to_string(),
            "--json".to_string(),
            "--with-tests".to_string(),
        ];
        let output = build_output(&root, &args).unwrap();
        let node = &output["nodes"].as_array().unwrap()[0];
        let full = node["tests"]["full"].as_array().unwrap();
        let names_only = node["tests"]["names_only"].as_array().unwrap();
        assert_eq!(full.len(), 3, "{node}");
        assert_eq!(names_only.len(), 2, "{node}");
        for entry in full {
            assert!(entry["source"].as_str().unwrap().contains("fn test_greet"));
        }
        for entry in names_only {
            assert!(
                entry.get("source").is_none(),
                "names_only entries must not carry source: {entry}"
            );
            assert!(entry.get("path").is_some());
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
