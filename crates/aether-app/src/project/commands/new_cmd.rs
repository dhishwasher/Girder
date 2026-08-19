//! `bitcode new <dir> "<description>" --language rust|python --json` —
//! read-only, `context`-shaped context for authoring a program that does
//! not exist yet: project root, declared language, whatever files already
//! exist, the real `base_commit`, and a schema/skeleton whose edits are
//! `create_file` rather than node-pinned (see
//! [`authoring_context::creation_plan_schema`]). No model call, no
//! network, no writes.
//!
//! Unlike `context`, this command never builds or searches the semantic
//! graph: `context`/`do` pin a model to an enum of nodes already present
//! in the graph, and there is no such enum here — the whole point of
//! `bitcode new` is authoring a path the graph doesn't know about yet.
//! `--language` is required, not guessed: a `create` edit's content only
//! ever becomes graph-editable later if it is Rust or Python (the two
//! languages `aether-builder` parses), so the model needs to be told
//! which dialect to write rather than have it inferred from an empty or
//! ambiguous directory.

use super::authoring_context::{
    collect_words, creation_plan_schema, creation_plan_skeleton, generate_plan_id, invalid_input,
};
use crate::project::config::ProjectConfig;
use crate::project::git::git_head_commit;
use crate::project::source::collect_project_files_with_config;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: bitcode new <dir> \"<description>\" --language rust|python --json";

pub fn new(args: &[String]) -> std::io::Result<()> {
    let Some(root_arg) = args.first() else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let root = PathBuf::from(root_arg);

    // `--json` is the only output mode implemented, same as `context`: this
    // command exists to emit exactly one pipeable/pasteable JSON object.
    if !args.iter().any(|arg| arg == "--json") {
        return Err(invalid_input("bitcode new requires --json"));
    }

    let output = build_output(&root, args)?;
    let rendered = serde_json::to_string_pretty(&output)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    println!("{rendered}");
    Ok(())
}

/// Everything `new()` does once `--json` is confirmed present, factored out
/// so tests can inspect the emitted JSON object's structure directly.
fn build_output(root: &Path, args: &[String]) -> std::io::Result<Value> {
    let language = parse_language(args)?;
    let intent = collect_words(args, &["--json"], &["--language"]);
    build_new_json(root, &intent, &language)
}

/// `--language rust|python`, required — never guessed. Parsed the same way
/// [`super::authoring_context::parse_pinned_nodes`] parses `--nodes`.
fn parse_language(args: &[String]) -> std::io::Result<String> {
    let value = args
        .windows(2)
        .find(|window| window[0] == "--language")
        .map(|window| window[1].as_str());
    if args.iter().any(|arg| arg == "--language") && value.is_none() {
        return Err(invalid_input("--language requires a value"));
    }
    match value {
        Some("rust") => Ok("rust".to_string()),
        Some("python") => Ok("python".to_string()),
        Some(other) => Err(invalid_input(&format!(
            "--language {other:?} is not supported; bitcode new requires \
             --language rust or --language python"
        ))),
        None => Err(invalid_input(
            "bitcode new requires --language rust or --language python (not guessed \
             from an empty or ambiguous directory)",
        )),
    }
}

/// The body of `build_output`, taking an already-resolved intent and
/// language instead of raw CLI args, mirroring
/// [`super::context_cmd::build_context_json`]'s split for the same reason:
/// a non-CLI caller (a future GUI panel, a test) can reach exactly this
/// JSON without building a fake args vector.
pub(crate) fn build_new_json(root: &Path, intent: &str, language: &str) -> std::io::Result<Value> {
    let config = ProjectConfig::load(root)?;
    let existing_files: Vec<String> = collect_project_files_with_config(root, &config)?
        .into_iter()
        .map(|(_, relative)| relative)
        .collect();

    let base_commit = git_head_commit(root).map_err(|error| {
        invalid_input(&format!(
            "bitcode new requires {} to be a git repository with at least one commit \
             (a plan's base_commit needs a resolvable HEAD); run `git init && git commit \
             --allow-empty -m init` first: {error}",
            root.display()
        ))
    })?;
    let plan_id = generate_plan_id();

    Ok(json!({
        "project_root": root.display().to_string(),
        "language": language,
        "existing_files": existing_files,
        "base_commit": base_commit,
        "schema": creation_plan_schema(),
        "plan_skeleton": creation_plan_skeleton(&base_commit, intent, &plan_id),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

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

    /// An empty git repository (one commit, no tracked files) — the shape
    /// `bitcode new` targets: a project that does not exist yet.
    fn empty_repo(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "bitcode-new-cmd-{name}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "--quiet"]);
        git(
            &root,
            &[
                "-c",
                "user.name=Bit Code Tests",
                "-c",
                "user.email=tests@bitcode.invalid",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "init",
            ],
        );
        root
    }

    /// A non-empty git repository, so `existing_files` has something real
    /// in it.
    fn repo_with_a_file(name: &str) -> PathBuf {
        let root = empty_repo(name);
        std::fs::write(root.join("README.md"), "hello\n").unwrap();
        git(&root, &["add", "README.md"]);
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
                "add readme",
            ],
        );
        root
    }

    #[test]
    fn requires_a_dir_argument() {
        assert!(new(&[]).is_ok());
    }

    #[test]
    fn requires_the_json_flag() {
        let root = empty_repo("requires-json");
        let args = vec![
            root.display().to_string(),
            "--language".to_string(),
            "python".to_string(),
        ];
        let error = new(&args).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("requires --json"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn requires_a_language_and_never_guesses_one() {
        let root = empty_repo("requires-language");
        let args = vec![root.display().to_string(), "--json".to_string()];
        let error = build_output(&root, &args).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("not guessed"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_an_unsupported_language() {
        let root = empty_repo("unsupported-language");
        let args = vec![
            root.display().to_string(),
            "--language".to_string(),
            "javascript".to_string(),
            "--json".to_string(),
        ];
        let error = build_output(&root, &args).unwrap_err();
        assert!(error.to_string().contains("javascript"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_directory_with_no_commits_gets_a_clear_error_not_a_downstream_git_failure() {
        let root = std::env::temp_dir().join(format!(
            "bitcode-new-cmd-no-commits-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "--quiet"]);
        // No commit at all: HEAD does not resolve.

        let error = build_new_json(&root, "a greeter", "python").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("at least one commit"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn emits_every_top_level_key_with_the_documented_shape() {
        let root = repo_with_a_file("top-level-keys");
        let args = vec![
            root.display().to_string(),
            "--language".to_string(),
            "python".to_string(),
            "a greeter script".to_string(),
            "--json".to_string(),
        ];
        let output = build_output(&root, &args).unwrap();

        let object = output.as_object().unwrap();
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "base_commit",
                "existing_files",
                "language",
                "plan_skeleton",
                "project_root",
                "schema",
            ]
        );

        assert_eq!(output["language"], "python");
        assert_eq!(output["project_root"], root.display().to_string());
        assert_eq!(output["base_commit"].as_str().unwrap().len(), 40);
        assert_eq!(output["existing_files"], serde_json::json!(["README.md"]));

        assert!(output["schema"]["properties"]["edits"]["items"]["oneOf"].is_array());
        assert_eq!(output["schema"]["properties"]["checks"]["minItems"], 1);

        let skeleton = &output["plan_skeleton"];
        assert_eq!(skeleton["plan_version"], 2);
        assert_eq!(skeleton["base_commit"], output["base_commit"]);
        assert_eq!(skeleton["intent"], "a greeter script");
        let steps = skeleton["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0]["checks"].as_array().unwrap().len(), 0);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_new_project_has_an_empty_existing_files_list() {
        let root = empty_repo("empty-project");
        let output = build_new_json(&root, "a greeter script", "rust").unwrap();
        assert_eq!(output["existing_files"], serde_json::json!([]));
        let _ = std::fs::remove_dir_all(&root);
    }

    // The bug this guards against, mirroring context_cmd's own test of the
    // same shape: a step literally satisfying `creation_plan_schema` must
    // be accepted by the real loader, including gap 24's rule — since the
    // schema's checks.minItems/kind-enum already force a command check,
    // this also proves the schema never hands a model a shape
    // `Plan::validate()` would reject.
    #[test]
    fn a_step_satisfying_creation_plan_schema_round_trips_through_load_plan() {
        let root = repo_with_a_file("schema-round-trip");
        let output = build_new_json(&root, "add a greeter script", "python").unwrap();
        let base_commit = output["base_commit"].as_str().unwrap().to_string();

        let step = serde_json::json!({
            "id": "step-1",
            "description": "create greeter.py",
            "edits": [{
                "path": "greeter.py",
                "create": "def greet(name):\n    return f\"Hello, {name}!\"\n"
            }],
            "checks": [{
                "kind": "command",
                "run": "python3 -c \"import greeter; assert greeter.greet('x') == 'Hello, x!'\""
            }]
        });
        let plan = serde_json::json!({
            "plan_version": 2,
            "plan_id": "schema-round-trip",
            "intent": "add a greeter script",
            "base_commit": base_commit,
            "on_failure": "rollback_plan",
            "steps": [step]
        });
        let plan_path = std::env::temp_dir().join(format!(
            "bitcode-new-cmd-schema-round-trip-{}-{}.json",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();

        let loaded = crate::project::planfile::load_plan(&plan_path)
            .expect("a step satisfying creation_plan_schema must be accepted by load_plan");
        assert_eq!(loaded.steps.len(), 1);
        assert_eq!(loaded.steps[0].edits.len(), 1);
        assert_eq!(loaded.steps[0].checks.len(), 1);

        let _ = std::fs::remove_file(&plan_path);
        let _ = std::fs::remove_dir_all(&root);
    }
}
