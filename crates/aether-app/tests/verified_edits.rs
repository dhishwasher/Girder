//! Stage 5 verified edits: every frozen fixture plan is run through the real CLI and held to its
//! expected outcome. Policy: docs/verified-edits-policy.md. Fixtures: fixtures/verified-edits/v1/.
//!
//! For every refusal the project tree (all files except `.git` and `.girder/reports`, which `plan
//! run` always writes) is hashed before and after: no source file, no saved graph, and no
//! transaction journal may change.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/verified-edits/v1")
}

struct Repo(PathBuf);

impl Repo {
    fn new(fixture: &str) -> (Self, String) {
        let root = std::env::temp_dir().join(format!(
            "girder-verified-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        copy(&fixtures().join(fixture), &root.join("w"));
        let work = root.join("w");
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .args(["-c", "user.email=f@x", "-c", "user.name=fixture"])
                .args(args)
                .current_dir(&work)
                .output()
                .unwrap();
            assert!(status.status.success(), "git {args:?}");
            String::from_utf8(status.stdout).unwrap()
        };
        git(&["init", "-q", "."]);
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "fixture"]);
        let base = git(&["rev-parse", "HEAD"]).trim().to_string();
        (Self(root), base)
    }

    fn work(&self) -> PathBuf {
        self.0.join("w")
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Hash of every file in the project except `.git` and `.girder/reports`.
fn snapshot(root: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel == ".git" || rel == ".girder/reports" {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                walk(root, &path, out);
            } else {
                let digest = Sha256::digest(std::fs::read(&path).unwrap());
                out.insert(rel, digest.iter().map(|b| format!("{b:02x}")).collect());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn sha(path: &Path) -> String {
    Sha256::digest(std::fs::read(path).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn frozen_fixture_and_plan_files_match_their_pinned_hashes() {
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(fixtures().join("manifest.json")).unwrap())
            .unwrap();
    let pinned = manifest["files_sha256"].as_object().unwrap();
    assert!(pinned.len() >= 26);
    for (relative, digest) in pinned {
        assert_eq!(
            sha(&fixtures().join(relative)),
            digest.as_str().unwrap(),
            "{relative} drifted"
        );
    }
}

#[test]
fn every_frozen_plan_meets_its_expected_outcome() {
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(fixtures().join("manifest.json")).unwrap())
            .unwrap();
    let mut failures = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let expected = &case["expected"];
        let outcome = expected["outcome"].as_str().unwrap();
        let (repo, base) = Repo::new(case["fixture"].as_str().unwrap());
        let plan_text = std::fs::read_to_string(fixtures().join(case["plan"].as_str().unwrap()))
            .unwrap()
            .replace("<HEAD>", &base);
        let plan_path = repo.0.join("plan.json");
        std::fs::write(&plan_path, &plan_text).unwrap();
        let plan: Value = serde_json::from_str(&plan_text).unwrap();
        let before = snapshot(&repo.work());
        let run = Command::new(env!("CARGO_BIN_EXE_girder"))
            .args(["plan", "run"])
            .arg(&plan_path)
            .current_dir(repo.work())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&run.stdout).to_string();
        let after = snapshot(&repo.work());
        let mut problems = Vec::new();
        match outcome {
            "refused" => {
                let category = expected["category"].as_str().unwrap();
                let step = expected["failed_step"].as_str().unwrap();
                if run.status.success() {
                    problems.push("the plan should have failed".to_string());
                }
                if !stdout.contains(&format!("[{step}] FAILED")) {
                    problems.push(format!("step {step} was not reported FAILED"));
                }
                if !stdout.contains(&format!("certification: REFUSED [{category}]")) {
                    problems.push(format!("expected REFUSED [{category}] in the output"));
                }
                if after != before {
                    let changed: Vec<_> = after
                        .iter()
                        .filter(|(k, v)| before.get(*k) != Some(v))
                        .map(|(k, _)| k.clone())
                        .chain(before.keys().filter(|k| !after.contains_key(*k)).cloned())
                        .collect();
                    problems.push(format!("the project changed on refusal: {changed:?}"));
                }
                if repo.work().join("project.aether").exists() {
                    problems.push("a saved graph appeared".to_string());
                }
                // Which phase refused matters: a baseline that is a sibling's fingerprint is caught
                // before anything is applied; a wrong declared delta is caught after the edit.
                if (id.ends_with("wrong-by-fingerprint") || id == "wrong-overload-by-fingerprint")
                    && !stdout.contains("is the fingerprint of")
                {
                    problems.push("expected the pre-apply sibling-fingerprint refusal".to_string());
                }
                if (id.ends_with("wrong-by-delta") || id == "wrong-overload-by-delta")
                    && !stdout.contains("the edit changed")
                {
                    problems.push("expected the post-apply wrong-delta refusal".to_string());
                }
            }
            "committed" | "committed-uncertified" => {
                if !run.status.success() {
                    problems.push(format!("the plan should have passed: {stdout}"));
                }
                if outcome == "committed" && !stdout.contains("certification: certified") {
                    problems.push("expected certification: certified".to_string());
                }
                if outcome == "committed-uncertified"
                    && !stdout.contains("uncertified (no verify block)")
                {
                    problems.push("expected the step to be reported uncertified".to_string());
                }
                for step in plan["steps"].as_array().unwrap() {
                    for edit in step["edits"].as_array().unwrap() {
                        if let (Some(node), Some(text)) =
                            (edit["node"].as_str(), edit["replace_node"].as_str())
                        {
                            let file = if node.starts_with("crate::app::") {
                                "app.py"
                            } else {
                                "src/lib.rs"
                            };
                            let content = std::fs::read_to_string(repo.work().join(file)).unwrap();
                            if !content.contains(text) {
                                problems.push(format!(
                                    "{file} does not contain the replacement for {node}"
                                ));
                            }
                        }
                    }
                }
                if id == "no-op-replacement-applies" && after != before {
                    problems.push("an identical replacement changed the tree".to_string());
                }
            }
            other => problems.push(format!("unknown outcome {other}")),
        }
        if !problems.is_empty() {
            failures.push(format!("{id}: {problems:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn a_committed_correct_overload_edit_leaves_the_other_overload_untouched() {
    let (repo, base) = Repo::new("overload");
    let plan = std::fs::read_to_string(fixtures().join("plans/correct-overload-applies.json"))
        .unwrap()
        .replace("<HEAD>", &base);
    let plan_path = repo.0.join("plan.json");
    std::fs::write(&plan_path, plan).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_girder"))
        .args(["plan", "run"])
        .arg(&plan_path)
        .current_dir(repo.work())
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stdout)
    );
    let source = std::fs::read_to_string(repo.work().join("src/lib.rs")).unwrap();
    assert!(source.contains("String::from(\"trait v2\")"));
    assert!(
        source.contains("String::from(\"inherent\")"),
        "the inherent overload must be untouched"
    );
    assert!(!source.contains("String::from(\"trait\")"));
}

#[test]
fn the_report_separates_certification_and_predicted_impact_from_executed_checks() {
    let (repo, base) = Repo::new("overload");
    let plan = std::fs::read_to_string(fixtures().join("plans/correct-overload-applies.json"))
        .unwrap()
        .replace("<HEAD>", &base);
    let plan_path = repo.0.join("plan.json");
    std::fs::write(&plan_path, plan).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_girder"))
        .args(["plan", "run"])
        .arg(&plan_path)
        .current_dir(repo.work())
        .output()
        .unwrap();
    assert!(run.status.success());
    let reports = repo.work().join(".girder/reports");
    let report_file = std::fs::read_dir(&reports)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let report: Value =
        serde_json::from_str(&std::fs::read_to_string(report_file).unwrap()).unwrap();
    let step = &report["steps"][0];
    assert_eq!(step["certification"]["certified"], true);
    assert_eq!(
        step["certification"]["delta"]["nodes_changed"][0],
        "crate::lib::Widget::render@Render"
    );
    assert_eq!(
        step["certification"]["impact"]["label"],
        "predicted, not execution evidence"
    );
    let impact = &step["certification"]["impact"];
    for key in [
        "tests_before",
        "tests_after",
        "tests_after_listed",
        "newly_reachable",
        "no_longer_reachable",
        "truncated",
    ] {
        assert!(!impact[key].is_null(), "impact is missing {key}");
    }
    // Executed checks are a separate list; predicted reachability is never placed in it.
    assert!(step["checks"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["kind"] != "impact"));
}
