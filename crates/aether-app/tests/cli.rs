use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

struct TempRepo {
    root: PathBuf,
}

impl TempRepo {
    fn new(name: &str) -> Self {
        let unique = format!(
            "bitcode-cli-{name}-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&root).unwrap();
        let repo = Self { root };
        repo.git(&["init"]);
        repo.git(&["config", "user.email", "bitcode@example.invalid"]);
        repo.git(&["config", "user.name", "Bit Code Test"]);
        repo
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, text).unwrap();
    }

    fn remove(&self, rel: &str) {
        std::fs::remove_file(self.root.join(rel)).unwrap();
    }

    fn git(&self, args: &[&str]) -> Output {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn commit_all(&self, message: &str) {
        self.git(&["add", "."]);
        self.git(&["commit", "-m", message]);
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn run_bitcode_output(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bitcode"))
        .args(args)
        .output()
        .unwrap()
}

fn run_bitcode(args: &[&str]) -> String {
    let output = run_bitcode_output(args);
    assert!(
        output.status.success(),
        "bitcode {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn review_reports_added_modified_removed_functions() {
    let repo = TempRepo::new("review-node-kinds");
    repo.write(
        "src/lib.rs",
        r#"
pub fn add(a: i64, b: i64) -> i64 { a + b }
pub fn removed(x: i64) -> i64 { x + 1 }

#[test]
fn test_removed() {
    assert_eq!(removed(1), 2);
}

#[test]
fn test_add() {
    assert_eq!(add(2, 3), 5);
}
"#,
    );
    repo.commit_all("baseline");

    repo.write(
        "src/lib.rs",
        r#"
pub fn add(a: i64, b: i64) -> i64 { a - b }
pub fn added(x: i64) -> i64 { add(x, 2) }

#[test]
fn test_add() {
    assert_eq!(add(2, 3), -1);
}
"#,
    );

    let stdout = run_bitcode(&["review", repo.path().to_str().unwrap(), "--since", "HEAD"]);

    assert!(stdout.contains("Semantic Review"), "{stdout}");
    assert!(stdout.contains("Added ["), "{stdout}");
    assert!(stdout.contains("Modified ["), "{stdout}");
    assert!(stdout.contains("Removed ["), "{stdout}");
    assert!(stdout.contains("crate::lib::added"), "{stdout}");
    assert!(stdout.contains("crate::lib::add"), "{stdout}");
    assert!(stdout.contains("crate::lib::removed"), "{stdout}");
    assert!(stdout.contains("baseline tests: test_removed"), "{stdout}");
}

#[test]
fn review_reports_nodes_removed_with_deleted_files() {
    let repo = TempRepo::new("review-deleted-file");
    repo.write("src/lib.rs", "pub fn keep() -> i64 { 1 }\n");
    repo.write(
        "src/obsolete.rs",
        r#"
pub fn obsolete(x: i64) -> i64 { x + 1 }

#[test]
fn test_obsolete() {
    assert_eq!(obsolete(1), 2);
}
"#,
    );
    repo.commit_all("baseline");

    repo.remove("src/obsolete.rs");

    let stdout = run_bitcode(&["review", repo.path().to_str().unwrap(), "--since", "HEAD"]);

    assert!(stdout.contains("Removed ["), "{stdout}");
    assert!(stdout.contains("crate::obsolete::obsolete"), "{stdout}");
    assert!(stdout.contains("baseline tests: test_obsolete"), "{stdout}");
}

#[test]
fn test_impact_selects_macro_contained_rust_tests() {
    let repo = TempRepo::new("test-impact-macro");
    repo.write(
        "src/lib.rs",
        r#"
pub fn add(a: i64, b: i64) -> i64 { a + b }

#[test]
fn test_add() {
    assert_eq!(add(2, 3), 5);
}
"#,
    );
    repo.commit_all("baseline");

    repo.write(
        "src/lib.rs",
        r#"
pub fn add(a: i64, b: i64) -> i64 { a - b }

#[test]
fn test_add() {
    assert_eq!(add(2, 3), -1);
}
"#,
    );

    let stdout = run_bitcode(&["test-impact", repo.path().to_str().unwrap()]);

    assert!(stdout.contains("Changed functions"), "{stdout}");
    assert!(stdout.contains("crate::lib::add"), "{stdout}");
    assert!(stdout.contains("Impacted tests (1)"), "{stdout}");
    assert!(stdout.contains("crate::lib::test_add"), "{stdout}");
    assert!(
        stdout.contains("cargo test --workspace test_add"),
        "{stdout}"
    );
}

#[test]
fn test_impact_includes_untracked_source_files() {
    let repo = TempRepo::new("test-impact-untracked");
    repo.write("src/lib.rs", "pub fn keep() -> i64 { 1 }\n");
    repo.commit_all("baseline");

    repo.write(
        "src/new.rs",
        r#"
pub fn fresh(x: i64) -> i64 { x + 10 }

#[test]
fn test_fresh() {
    assert_eq!(fresh(1), 11);
}
"#,
    );

    let stdout = run_bitcode(&["test-impact", repo.path().to_str().unwrap()]);

    assert!(stdout.contains("changed files: src/new.rs"), "{stdout}");
    assert!(stdout.contains("crate::new::fresh"), "{stdout}");
    assert!(stdout.contains("Impacted tests (1)"), "{stdout}");
    assert!(stdout.contains("crate::new::test_fresh"), "{stdout}");
}

#[test]
fn test_impact_uses_baseline_tests_for_removed_functions() {
    let repo = TempRepo::new("test-impact-removed");
    repo.write(
        "src/lib.rs",
        r#"
pub fn old_fn(x: i64) -> i64 { x + 1 }

#[test]
fn test_old_fn() {
    assert_eq!(old_fn(1), 2);
}
"#,
    );
    repo.commit_all("baseline");

    repo.write(
        "src/lib.rs",
        r#"
#[test]
fn test_old_fn() {
    assert_eq!(old_fn(1), 2);
}
"#,
    );

    let stdout = run_bitcode(&["test-impact", repo.path().to_str().unwrap()]);

    assert!(
        stdout.contains("Removed functions were detected; using their baseline test coverage."),
        "{stdout}"
    );
    assert!(stdout.contains("Impacted tests (1)"), "{stdout}");
    assert!(stdout.contains("crate::lib::test_old_fn"), "{stdout}");
    assert!(
        stdout.contains("cargo test --workspace test_old_fn"),
        "{stdout}"
    );
}

#[test]
fn forge_projects_generated_functions_to_source_file() {
    let repo = TempRepo::new("forge-writeback");
    repo.write("src/lib.rs", "pub fn existing() -> i64 { 1 }\n");

    let stdout = run_bitcode(&[
        "forge",
        repo.path().to_str().unwrap(),
        "add user authentication",
    ]);

    let generated = std::fs::read_to_string(repo.path().join("src/forge.rs")).unwrap();
    assert!(stdout.contains("projected source file(s):"), "{stdout}");
    assert!(stdout.contains("src/forge.rs"), "{stdout}");
    assert!(generated.contains("fn validate_credentials"), "{generated}");
    assert!(generated.contains("fn generate_token"), "{generated}");
    assert!(generated.contains("fn authenticate"), "{generated}");
    assert!(
        generated.contains("validate_credentials(username, password)"),
        "{generated}"
    );
    assert!(generated.contains("generate_token(0)"), "{generated}");
}

#[test]
fn forge_validation_failure_leaves_project_unchanged() {
    let repo = TempRepo::new("forge-validation-failure");
    repo.write("src/lib.rs", "pub fn existing() -> i64 { 1 }\n");
    repo.write(
        "bitcode.toml",
        r#"
version = 1

[validation]
commands = [["sh", "-c", "printf validation-broke >&2; exit 9"]]
"#,
    );

    let output = run_bitcode_output(&[
        "forge",
        repo.path().to_str().unwrap(),
        "add user authentication",
    ]);

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("[failed] Configured check 1"), "{stdout}");
    assert!(stdout.contains("validation-broke"), "{stdout}");
    assert!(stderr.contains("project was not modified"), "{stderr}");
    assert!(!repo.path().join("src/forge.rs").exists());
    assert!(!repo.path().join("project.aether").exists());
    assert!(!repo.path().join(".bitcode").exists());
}

#[test]
fn refactor_rename_projects_graph_changes_to_source_file() {
    let repo = TempRepo::new("refactor-writeback");
    repo.write(
        "src/lib.rs",
        r#"
pub fn add(a: i64, b: i64) -> i64 {
    a + b
}

pub fn sum_list(xs: &[i64]) -> i64 {
    let mut total = 0;
    for x in xs {
        total = add(total, *x);
    }
    total
}
"#,
    );

    let stdout = run_bitcode(&[
        "refactor",
        repo.path().to_str().unwrap(),
        "rename",
        "crate::lib::add",
        "plus",
    ]);

    let source = std::fs::read_to_string(repo.path().join("src/lib.rs")).unwrap();
    assert!(
        stdout.contains("Renamed crate::lib::add -> crate::lib::plus"),
        "{stdout}"
    );
    assert!(stdout.contains("projected source file(s):"), "{stdout}");
    assert!(
        source.contains("pub fn plus(a: i64, b: i64) -> i64"),
        "{source}"
    );
    assert!(source.contains("total = plus(total, *x);"), "{source}");
    assert!(!source.contains("pub fn add(a: i64, b: i64)"), "{source}");
    assert!(!source.contains("total = add(total, *x);"), "{source}");
}

#[test]
fn config_init_creates_a_valid_project_contract_without_overwriting() {
    let repo = TempRepo::new("config-init");

    let stdout = run_bitcode(&["config", repo.path().to_str().unwrap(), "--init"]);
    let config = std::fs::read_to_string(repo.path().join("bitcode.toml")).unwrap();

    assert!(stdout.contains("Created"), "{stdout}");
    assert!(stdout.contains("version = 1"), "{stdout}");
    assert!(config.contains("roots = [\".\"]"), "{config}");
    assert!(
        config.contains("output_file = \"src/forge.rs\""),
        "{config}"
    );
    assert!(config.contains("[validation]"), "{config}");
    assert!(config.contains("run_tests = true"), "{config}");

    let output = run_bitcode_output(&["config", repo.path().to_str().unwrap(), "--init"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("refusing to overwrite"), "{stderr}");
}

#[test]
fn analyze_honors_configured_source_roots_and_excludes() {
    let repo = TempRepo::new("config-source-scope");
    repo.write("src/lib.rs", "pub fn included() -> i64 { 1 }\n");
    repo.write("src/generated.rs", "pub fn generated() -> i64 { 2 }\n");
    repo.write("examples/demo.rs", "pub fn outside_root() -> i64 { 3 }\n");
    repo.write(
        "bitcode.toml",
        r#"
version = 1

[source]
roots = ["src"]
exclude = ["src/generated.rs"]
"#,
    );

    let stdout = run_bitcode(&["analyze", repo.path().to_str().unwrap()]);

    assert!(stdout.contains("loaded 1 source file(s)"), "{stdout}");
    assert!(stdout.contains("1 functions"), "{stdout}");
}

#[test]
fn test_impact_run_propagates_configured_runner_failure() {
    let repo = TempRepo::new("test-runner-failure");
    repo.write(
        "src/lib.rs",
        r#"
pub fn add(a: i64, b: i64) -> i64 { a + b }

#[test]
fn test_add() {
    assert_eq!(add(2, 3), 5);
}
"#,
    );
    repo.write(
        "bitcode.toml",
        r#"
version = 1

[tests]
rust = ["false", "{test}"]
"#,
    );

    let output = run_bitcode_output(&[
        "test-impact",
        repo.path().to_str().unwrap(),
        "--run",
        "crate::lib::add",
    ]);

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("false test_add"), "{stdout}");
    assert!(stderr.contains("Rust command exited with 1"), "{stderr}");
}
