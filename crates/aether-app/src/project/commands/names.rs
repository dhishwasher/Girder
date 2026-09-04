//! `girder names <dir> <identifier> [--kind function|type|all] --json` —
//! exact name match across the workspace. `girder query` answers
//! who-calls-X but cannot answer "how many things are named X"; `girder
//! search` answers that only approximately (substring, over both name and
//! path). This exists so an agent doesn't fall back to `grep` for an
//! exact-name lookup.

use super::authoring_context::invalid_input;
use crate::project::config::ProjectConfig;
use crate::project::source::build_from_dir_with_config;
use aether_graph::NodeKind;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: girder names <dir> <identifier> [--kind function|type|all] --json";

pub fn names(args: &[String]) -> std::io::Result<()> {
    let Some(root_arg) = args.first() else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let Some(identifier) = args.get(1) else {
        eprintln!("{USAGE}");
        return Ok(());
    };

    // `--json` is the only output mode: this command exists to emit one
    // pipeable JSON array, so there is no human-readable fallback to
    // silently produce instead.
    if !args.iter().any(|arg| arg == "--json") {
        return Err(invalid_input("girder names requires --json"));
    }

    let root = PathBuf::from(root_arg);
    let results = find_names(&root, identifier, args)?;
    let rendered = serde_json::to_string_pretty(&results)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    println!("{rendered}");
    Ok(())
}

/// Everything `names()` does once `--json` is confirmed present, factored
/// out so tests can inspect the returned JSON array directly instead of
/// scraping stdout.
fn find_names(root: &Path, identifier: &str, args: &[String]) -> std::io::Result<Vec<Value>> {
    let kind_filter = parse_kind_filter(args)?;

    let config = ProjectConfig::load(root)?;
    let (graph, _builder, _files) = build_from_dir_with_config(root, &config)?;

    let mut matches: Vec<_> = graph
        .nodes()
        .filter(|node| node.name == identifier)
        .filter(|node| kind_filter.is_none_or(|kind| node.kind == kind))
        .collect();
    matches.sort_by(|left, right| left.path.cmp(&right.path));

    Ok(matches
        .into_iter()
        .map(|node| {
            json!({
                "path": node.path,
                "language": node.language,
                "kind": format!("{:?}", node.kind),
            })
        })
        .collect())
}

fn parse_kind_filter(args: &[String]) -> std::io::Result<Option<NodeKind>> {
    match args
        .windows(2)
        .find(|window| window[0] == "--kind")
        .map(|window| window[1].as_str())
    {
        Some("function") => Ok(Some(NodeKind::Function)),
        Some("type") => Ok(Some(NodeKind::Type)),
        Some("all") => Ok(None),
        Some(other) => Err(invalid_input(&format!(
            "--kind must be function, type, or all (got \"{other}\")"
        ))),
        None => Ok(None),
    }
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

    /// A tiny real git project with two same-named-prefix functions and a
    /// same-named type, so exact-match and `--kind` filtering both have
    /// something real to distinguish.
    fn fixture_project(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "girder-names-fixture-{name}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("calc.rs"),
            "pub struct Greet;\n\npub fn greet() -> String {\n    \"hi\".to_string()\n}\n\npub fn greeting() -> String {\n    \"hi\".to_string()\n}\n",
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
        assert!(names(&[]).is_ok());
    }

    #[test]
    fn requires_an_identifier_argument() {
        assert!(names(&["some-dir".to_string()]).is_ok());
    }

    #[test]
    fn requires_the_json_flag() {
        let root = fixture_project("requires-json");
        let args = vec![root.display().to_string(), "greet".to_string()];
        let error = names(&args).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("requires --json"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_an_unknown_kind_value() {
        let root = fixture_project("bad-kind");
        let args = vec![
            root.display().to_string(),
            "greet".to_string(),
            "--json".to_string(),
            "--kind".to_string(),
            "bogus".to_string(),
        ];
        let error = names(&args).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("--kind"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn exact_match_excludes_a_substring_near_miss() {
        let root = fixture_project("exact-not-substring");
        let args = vec![root.display().to_string(), "greet".to_string()];
        let results = find_names(&root, "greet", &args).unwrap();
        assert_eq!(
            results.len(),
            1,
            "greeting must not match greet: {results:?}"
        );
        assert_eq!(results[0]["path"], "crate::calc::greet");
        assert_eq!(results[0]["language"], "rust");
        assert_eq!(results[0]["kind"], "Function");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_match_returns_an_empty_array_not_an_error() {
        let root = fixture_project("no-match");
        let args = vec![root.display().to_string(), "does_not_exist".to_string()];
        let results = find_names(&root, "does_not_exist", &args).unwrap();
        assert!(results.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn kind_filter_excludes_a_same_named_type() {
        let root = fixture_project("kind-filter");
        let base_args = vec![root.display().to_string(), "Greet".to_string()];

        let all = find_names(&root, "Greet", &base_args).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0]["kind"], "Type");

        let mut function_args = base_args.clone();
        function_args.extend(["--kind".to_string(), "function".to_string()]);
        let functions_only = find_names(&root, "Greet", &function_args).unwrap();
        assert!(functions_only.is_empty(), "{functions_only:?}");

        let mut type_args = base_args;
        type_args.extend(["--kind".to_string(), "type".to_string()]);
        let types_only = find_names(&root, "Greet", &type_args).unwrap();
        assert_eq!(types_only.len(), 1);
        assert_eq!(types_only[0]["kind"], "Type");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn results_are_sorted_by_path() {
        let root = fixture_project("sorted");
        std::fs::write(
            root.join("other.rs"),
            "pub fn greet() -> String {\n    \"bye\".to_string()\n}\n",
        )
        .unwrap();
        git(&root, &["add", "other.rs"]);
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
                "add other",
            ],
        );

        let args = vec![root.display().to_string(), "greet".to_string()];
        let results = find_names(&root, "greet", &args).unwrap();
        assert_eq!(results.len(), 2, "{results:?}");
        let paths: Vec<&str> = results
            .iter()
            .map(|entry| entry["path"].as_str().unwrap())
            .collect();
        let mut sorted = paths.clone();
        sorted.sort_unstable();
        assert_eq!(paths, sorted, "results must be sorted by path");

        let _ = std::fs::remove_dir_all(&root);
    }
}
