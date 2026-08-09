use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
        self.write_bytes(rel, text.as_bytes());
    }

    fn write_bytes(&self, rel: &str, bytes: &[u8]) {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
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

fn generated_identity_fingerprint(output: &str) -> String {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("SHA-256 fingerprint: "))
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("identity output omitted its fingerprint:\n{output}"))
}

#[test]
fn analyze_and_inspect_export_exact_bounded_json() {
    let repo = TempRepo::new("graph-json");
    repo.write(
        "src/lib.rs",
        r#"
pub fn callee() -> i64 { 1 }
pub fn caller() -> i64 { callee() }
"#,
    );

    let analyze = run_bitcode(&["analyze", repo.path().to_str().unwrap(), "--json"]);
    let summary: serde_json::Value = serde_json::from_str(&analyze).unwrap();
    assert_eq!(summary["schema_version"], 1);
    assert_eq!(summary["source_files"], 1);
    assert!(summary["nodes"].as_u64().unwrap() >= 3);
    assert!(summary["edges"].as_u64().unwrap() >= 3);
    for field in ["build_ms", "similarity_ms", "save_ms"] {
        assert!(
            summary[field].is_u64(),
            "analysis summary omitted numeric {field}: {summary}"
        );
    }
    assert_eq!(
        summary["graph_path"],
        repo.path()
            .join("project.aether")
            .to_string_lossy()
            .as_ref()
    );

    let graph_path = repo.path().join("project.aether");
    let exported = run_bitcode(&["inspect", graph_path.to_str().unwrap(), "--json"]);
    let graph: serde_json::Value = serde_json::from_str(&exported).unwrap();
    assert_eq!(graph["schema_version"], 1);
    let nodes = graph["nodes"].as_array().unwrap();
    let paths = nodes
        .iter()
        .map(|node| node["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(paths.windows(2).all(|pair| pair[0] <= pair[1]));
    let caller = nodes
        .iter()
        .find(|node| node["path"] == "crate::lib::caller")
        .unwrap();
    assert_eq!(caller["kind"], "Function");
    assert_eq!(caller["language"], "rust");
    assert_eq!(caller["source_sha256"].as_str().unwrap().len(), 64);
    assert!(caller.get("source").is_none());

    let edges = graph["edges"].as_array().unwrap();
    assert!(edges.iter().any(|edge| {
        edge["source"] == "crate::lib::caller"
            && edge["target"] == "crate::lib::callee"
            && edge["kind"] == "Calls"
            && edge["weight_bits"] == 1.0_f32.to_bits()
    }));
    let edge_keys = edges
        .iter()
        .map(|edge| {
            (
                edge["source"].as_str().unwrap(),
                edge["target"].as_str().unwrap(),
                edge["kind"].as_str().unwrap(),
                edge["weight_bits"].as_u64().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert!(edge_keys.windows(2).all(|pair| pair[0] <= pair[1]));
}

#[test]
fn inspect_json_fails_closed_for_missing_or_corrupt_graphs() {
    let repo = TempRepo::new("graph-json-failure");
    let missing = repo.path().join("missing.aether");
    let missing_output = run_bitcode_output(&["inspect", missing.to_str().unwrap(), "--json"]);
    assert!(!missing_output.status.success());
    assert!(
        String::from_utf8_lossy(&missing_output.stderr).contains("could not load"),
        "{}",
        String::from_utf8_lossy(&missing_output.stderr)
    );

    repo.write("corrupt.aether", "not a semantic graph\n");
    let corrupt = repo.path().join("corrupt.aether");
    let corrupt_output = run_bitcode_output(&["inspect", corrupt.to_str().unwrap(), "--json"]);
    assert!(!corrupt_output.status.success());
    assert!(
        String::from_utf8_lossy(&corrupt_output.stderr).contains("could not load"),
        "{}",
        String::from_utf8_lossy(&corrupt_output.stderr)
    );
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
fn review_rejects_an_invalid_baseline_reference() {
    let repo = TempRepo::new("review-invalid-ref");
    repo.write("src/lib.rs", "pub fn stable() -> i64 { 1 }\n");
    repo.commit_all("baseline");

    let output = run_bitcode_output(&[
        "review",
        repo.path().to_str().unwrap(),
        "--since",
        "DOES_NOT_EXIST",
    ]);

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("Building baseline graph"), "{stdout}");
    assert!(stderr.contains("rev-parse --verify"), "{stderr}");
    assert!(stderr.contains("DOES_NOT_EXIST"), "{stderr}");
}

#[test]
fn review_preserves_leading_whitespace_in_a_nested_repository_root() {
    let repo = TempRepo::new("review-leading-space-root");
    repo.write(" pkg/src/lib.rs", "pub fn value() -> i64 { 1 }\n");
    repo.commit_all("baseline");
    repo.write(" pkg/src/lib.rs", "pub fn value() -> i64 { 2 }\n");
    let nested = repo.path().join(" pkg");

    let stdout = run_bitcode(&["review", nested.to_str().unwrap(), "--since", "HEAD"]);

    assert!(stdout.contains("Modified ["), "{stdout}");
    assert!(stdout.contains("crate::lib::value"), "{stdout}");
    assert!(!stdout.contains("Added ["), "{stdout}");
}

#[test]
fn automatic_test_impact_scopes_nested_repository_changes_to_the_project_root() {
    let repo = TempRepo::new("impact-nested-root");
    repo.write(
        "project/src/lib.rs",
        r#"
pub fn value() -> i64 { 1 }

#[test]
fn test_value() {
    assert_eq!(value(), 1);
}
"#,
    );
    repo.write("outside.rs", "pub fn outside() -> i64 { 1 }\n");
    repo.commit_all("baseline");
    repo.write(
        "project/src/lib.rs",
        r#"
pub fn value() -> i64 { 2 }

#[test]
fn test_value() {
    assert_eq!(value(), 2);
}
"#,
    );
    repo.write("outside.rs", "pub fn outside() -> i64 { 2 }\n");
    let nested = repo.path().join("project");

    let stdout = run_bitcode(&["test-impact", nested.to_str().unwrap()]);

    assert!(stdout.contains("changed files: src/lib.rs"), "{stdout}");
    assert!(!stdout.contains("outside.rs"), "{stdout}");
    assert!(stdout.contains("crate::lib::value"), "{stdout}");
    assert!(stdout.contains("crate::lib::test_value"), "{stdout}");
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
fn test_impact_does_not_count_cfg_test_helpers_as_tests() {
    let repo = TempRepo::new("test-impact-cfg-test-helper");
    let source = |value: i64| {
        format!(
            r#"
pub fn selected() -> i64 {{ {value} }}

#[cfg(test)]
fn test_support_only() {{
    let _ = selected();
}}

#[test]
fn selected_test() {{
    assert!(selected() > 0);
}}

#[test]
fn unrelated_test() {{
    assert_eq!(2 + 2, 4);
}}
"#
        )
    };
    repo.write("src/lib.rs", &source(1));
    repo.commit_all("baseline");
    repo.write("src/lib.rs", &source(2));

    let stdout = run_bitcode(&["test-impact", repo.path().to_str().unwrap()]);

    assert!(stdout.contains("Impacted tests (1)"), "{stdout}");
    assert!(stdout.contains("crate::lib::selected_test"), "{stdout}");
    assert!(
        !stdout.contains("crate::lib::test_support_only"),
        "{stdout}"
    );
    assert!(
        stdout.contains("(1 other test(s) not in impact set"),
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
fn test_impact_follows_if_let_narrowed_receivers() {
    let repo = TempRepo::new("test-impact-if-let-narrowing");
    let source = |verifier_body: &str| {
        format!(
            r#"
pub struct SessionIdentity;
impl SessionIdentity {{
    pub fn verify_selected_operation(&self) {{ {verifier_body} }}
}}

pub struct DecoyIdentity;
impl DecoyIdentity {{
    pub fn verify_selected_operation(&self) {{}}
}}

pub fn apply(identity: Option<&SessionIdentity>) {{
    if let Some(identity) = identity {{
        identity.verify_selected_operation();
    }}
}}

#[test]
fn provenance_test() {{
    apply(Some(&SessionIdentity));
}}
"#
        )
    };
    repo.write("src/lib.rs", &source(""));
    repo.commit_all("baseline");
    repo.write("src/lib.rs", &source("let _changed = true;"));

    let stdout = run_bitcode(&["test-impact", repo.path().to_str().unwrap()]);

    assert!(
        stdout.contains("crate::lib::SessionIdentity::verify_selected_operation"),
        "{stdout}"
    );
    assert!(stdout.contains("Impacted tests (1)"), "{stdout}");
    assert!(stdout.contains("crate::lib::provenance_test"), "{stdout}");
    assert!(
        stdout.contains("cargo test --workspace provenance_test"),
        "{stdout}"
    );
}

#[test]
fn test_impact_follows_match_arm_narrowed_receivers() {
    let repo = TempRepo::new("test-impact-match-arm-narrowing");
    let source = |verifier_body: &str| {
        format!(
            r#"
pub struct SessionIdentity;
impl SessionIdentity {{
    pub fn is_valid(&self) -> bool {{ true }}
    pub fn verify_selected_operation(&self) {{ {verifier_body} }}
}}

pub struct DecoyIdentity;
impl DecoyIdentity {{
    pub fn is_valid(&self) -> bool {{ false }}
    pub fn verify_selected_operation(&self) {{}}
}}

pub fn apply(identity: Option<&SessionIdentity>) {{
    match identity {{
        Some(identity) if identity.is_valid() => identity.verify_selected_operation(),
        _ => {{}},
    }}
}}

#[test]
fn provenance_test() {{
    apply(Some(&SessionIdentity));
}}
"#
        )
    };
    repo.write("src/lib.rs", &source(""));
    repo.commit_all("baseline");
    repo.write("src/lib.rs", &source("let _changed = true;"));

    let stdout = run_bitcode(&["test-impact", repo.path().to_str().unwrap()]);

    assert!(
        stdout.contains("crate::lib::SessionIdentity::verify_selected_operation"),
        "{stdout}"
    );
    assert!(stdout.contains("Impacted tests (1)"), "{stdout}");
    assert!(stdout.contains("crate::lib::provenance_test"), "{stdout}");
    assert!(
        stdout.contains("cargo test --workspace provenance_test"),
        "{stdout}"
    );
}

#[test]
fn test_impact_follows_let_else_narrowed_receivers() {
    let repo = TempRepo::new("test-impact-let-else-narrowing");
    let source = |verifier_body: &str| {
        format!(
            r#"
pub struct SessionIdentity;
impl SessionIdentity {{
    pub fn verify_selected_operation(&self) {{ {verifier_body} }}
}}

pub struct DecoyIdentity;
impl DecoyIdentity {{
    pub fn verify_selected_operation(&self) {{}}
}}

pub fn apply(identity: Option<&SessionIdentity>) {{
    let Some(identity) = identity else {{
        return;
    }};
    identity.verify_selected_operation();
}}

#[test]
fn provenance_test() {{
    apply(Some(&SessionIdentity));
}}
"#
        )
    };
    repo.write("src/lib.rs", &source(""));
    repo.commit_all("baseline");
    repo.write("src/lib.rs", &source("let _changed = true;"));

    let stdout = run_bitcode(&["test-impact", repo.path().to_str().unwrap()]);

    assert!(
        stdout.contains("crate::lib::SessionIdentity::verify_selected_operation"),
        "{stdout}"
    );
    assert!(stdout.contains("Impacted tests (1)"), "{stdout}");
    assert!(stdout.contains("crate::lib::provenance_test"), "{stdout}");
    assert!(
        stdout.contains("cargo test --workspace provenance_test"),
        "{stdout}"
    );
}

#[test]
fn test_impact_follows_let_chain_narrowed_receivers() {
    let repo = TempRepo::new("test-impact-let-chain-narrowing");
    let source = |verifier_body: &str| {
        format!(
            r#"
pub struct SessionIdentity;
impl SessionIdentity {{
    pub fn is_valid(&self) -> bool {{ true }}
    pub fn verify_selected_operation(&self) {{ {verifier_body} }}
}}

pub struct DecoyIdentity;
impl DecoyIdentity {{
    pub fn is_valid(&self) -> bool {{ false }}
    pub fn verify_selected_operation(&self) {{}}
}}

pub fn apply(identity: Option<&SessionIdentity>) {{
    if let Some(identity) = identity && identity.is_valid() {{
        identity.verify_selected_operation();
    }}
}}

#[test]
fn provenance_test() {{
    apply(Some(&SessionIdentity));
}}
"#
        )
    };
    repo.write("src/lib.rs", &source(""));
    repo.commit_all("baseline");
    repo.write("src/lib.rs", &source("let _changed = true;"));

    let stdout = run_bitcode(&["test-impact", repo.path().to_str().unwrap()]);

    assert!(
        stdout.contains("crate::lib::SessionIdentity::verify_selected_operation"),
        "{stdout}"
    );
    assert!(stdout.contains("Impacted tests (1)"), "{stdout}");
    assert!(stdout.contains("crate::lib::provenance_test"), "{stdout}");
    assert!(
        stdout.contains("cargo test --workspace provenance_test"),
        "{stdout}"
    );
}

#[test]
fn test_impact_follows_python_annotated_receivers() {
    let repo = TempRepo::new("test-impact-python-annotated-receiver");
    let models = |inspect_body: &str| {
        format!(
            r#"
class SessionIdentity:
    def inspect(self):
        {inspect_body}

class DecoyIdentity:
    def inspect(self):
        return False
"#
        )
    };
    repo.write("models.py", &models("return True"));
    repo.write(
        "service.py",
        r#"
from models import SessionIdentity

def apply(identity: SessionIdentity):
    identity.inspect()

def test_provenance():
    apply(SessionIdentity())
"#,
    );
    repo.commit_all("baseline");
    repo.write("models.py", &models("return 'changed'"));

    let stdout = run_bitcode(&["test-impact", repo.path().to_str().unwrap()]);

    assert!(
        stdout.contains("crate::models::SessionIdentity::inspect"),
        "{stdout}"
    );
    assert!(stdout.contains("Impacted tests (1)"), "{stdout}");
    assert!(
        stdout.contains("crate::service::test_provenance"),
        "{stdout}"
    );
    assert!(stdout.contains("pytest -k test_provenance"), "{stdout}");
}

#[test]
fn test_impact_follows_unambiguous_python_nullable_receivers() {
    let repo = TempRepo::new("test-impact-python-nullable-receiver");
    let models = |inspect_body: &str| {
        format!(
            r#"
class SessionIdentity:
    def inspect(self):
        {inspect_body}

class DecoyIdentity:
    def inspect(self):
        return False
"#
        )
    };
    repo.write("models.py", &models("return True"));
    repo.write(
        "service.py",
        r#"
from models import DecoyIdentity, SessionIdentity

class Optional:
    def __class_getitem__(cls, item):
        return cls

    def inspect(self):
        return False

class Box:
    def __init__(self):
        self.identity = DecoyIdentity()

def apply(identity: SessionIdentity | None):
    if identity is None:
        return False
    return identity.inspect()

def ambiguous(identity: SessionIdentity | DecoyIdentity):
    return identity.inspect()

def ambiguous_named(session_identity: SessionIdentity | DecoyIdentity):
    return session_identity.inspect()

def custom(identity: Optional[SessionIdentity]):
    return identity.inspect()

def custom_named(session_identity: Optional[SessionIdentity]):
    return session_identity.inspect()

def dotted_suffix(identity: SessionIdentity | None, box):
    return box.identity.inspect()

def test_provenance():
    apply(SessionIdentity())

def test_decoy():
    ambiguous(DecoyIdentity())

def test_ambiguous_named():
    ambiguous_named(DecoyIdentity())

def test_custom_wrapper():
    custom(Optional())

def test_custom_named_wrapper():
    custom_named(Optional())

def test_dotted_suffix():
    dotted_suffix(SessionIdentity(), Box())
"#,
    );
    repo.commit_all("baseline");
    repo.write("models.py", &models("return 'changed'"));

    let stdout = run_bitcode(&["test-impact", repo.path().to_str().unwrap()]);

    assert!(
        stdout.contains("crate::models::SessionIdentity::inspect"),
        "{stdout}"
    );
    assert!(stdout.contains("Impacted tests (1)"), "{stdout}");
    assert!(
        stdout.contains("crate::service::test_provenance"),
        "{stdout}"
    );
    assert!(!stdout.contains("crate::service::test_decoy"), "{stdout}");
    assert!(
        !stdout.contains("crate::service::test_ambiguous_named"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("crate::service::test_custom_wrapper"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("crate::service::test_custom_named_wrapper"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("crate::service::test_dotted_suffix"),
        "{stdout}"
    );
    assert!(stdout.contains("pytest -k test_provenance"), "{stdout}");
}

#[test]
fn test_impact_follows_python_constructor_assignments() {
    let repo = TempRepo::new("test-impact-python-constructor-assignment");
    let models = |inspect_body: &str| {
        format!(
            r#"
class SessionIdentity:
    def inspect(self):
        {inspect_body}

class DecoyIdentity:
    def inspect(self):
        return False
"#
        )
    };
    repo.write("models.py", &models("return True"));
    repo.write(
        "service.py",
        r#"
from models import SessionIdentity

def apply():
    identity = SessionIdentity()
    identity.inspect()

def test_provenance():
    apply()
"#,
    );
    repo.commit_all("baseline");
    repo.write("models.py", &models("return 'changed'"));

    let stdout = run_bitcode(&["test-impact", repo.path().to_str().unwrap()]);

    assert!(
        stdout.contains("crate::models::SessionIdentity::inspect"),
        "{stdout}"
    );
    assert!(stdout.contains("Impacted tests (1)"), "{stdout}");
    assert!(
        stdout.contains("crate::service::test_provenance"),
        "{stdout}"
    );
    assert!(stdout.contains("pytest -k test_provenance"), "{stdout}");
}

#[test]
fn test_impact_follows_python_aliased_receivers() {
    let repo = TempRepo::new("test-impact-python-aliased-receiver");
    let models = |inspect_body: &str| {
        format!(
            r#"
class SessionIdentity:
    def inspect(self):
        {inspect_body}

class DecoyIdentity:
    def inspect(self):
        return False
"#
        )
    };
    repo.write("models.py", &models("return True"));
    repo.write(
        "service.py",
        r#"
from models import SessionIdentity as Session

def apply(identity: Session):
    identity.inspect()

def test_provenance():
    apply(Session())
"#,
    );
    repo.commit_all("baseline");
    repo.write("models.py", &models("return 'changed'"));

    let stdout = run_bitcode(&["test-impact", repo.path().to_str().unwrap()]);

    assert!(
        stdout.contains("crate::models::SessionIdentity::inspect"),
        "{stdout}"
    );
    assert!(stdout.contains("Impacted tests (1)"), "{stdout}");
    assert!(
        stdout.contains("crate::service::test_provenance"),
        "{stdout}"
    );
    assert!(stdout.contains("pytest -k test_provenance"), "{stdout}");
}

#[test]
fn test_impact_follows_cargo_binary_subprocess_entrypoint() {
    let repo = TempRepo::new("test-impact-cargo-binary-entrypoint");
    let main = |dispatch_body: &str| {
        format!(
            r#"
fn main() {{
    dispatch();
}}

fn dispatch() {{
    {dispatch_body}
}}
"#
        )
    };
    repo.write("src/main.rs", &main("println!(\"ready\");"));
    repo.write(
        "tests/cli.rs",
        r#"
use std::process::Command;

fn run_binary() {
    Command::new(env!("CARGO_BIN_EXE_demo")).output().unwrap();
}

#[test]
fn cli_dispatch() {
    run_binary();
}

#[test]
fn unrelated_test() {
    assert_eq!(2 + 2, 4);
}
"#,
    );
    repo.commit_all("baseline");
    repo.write("src/main.rs", &main("println!(\"changed\");"));

    let stdout = run_bitcode(&["test-impact", repo.path().to_str().unwrap()]);

    assert!(stdout.contains("crate::main::dispatch"), "{stdout}");
    assert!(stdout.contains("Impacted tests (1)"), "{stdout}");
    assert!(
        stdout.contains("crate::tests::cli::cli_dispatch"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("crate::tests::cli::unrelated_test"),
        "{stdout}"
    );
    assert!(
        stdout.contains("cargo test --workspace cli_dispatch"),
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
fn source_read_failures_abort_analysis_without_replacing_the_durable_graph() {
    let repo = TempRepo::new("source-read-failure");
    repo.write("src/lib.rs", "pub fn stable() -> i64 { 1 }\n");
    repo.write("src/other.rs", "pub fn readable() -> i64 { 2 }\n");
    repo.commit_all("baseline");

    run_bitcode(&["analyze", repo.path().to_str().unwrap()]);
    let graph_path = repo.path().join("project.aether");
    let durable_before = std::fs::read(&graph_path).unwrap();
    repo.write_bytes("src/other.rs", b"pub fn unreadable() {}\n\xff\n");

    for args in [
        vec!["analyze", repo.path().to_str().unwrap()],
        vec!["review", repo.path().to_str().unwrap(), "--since", "HEAD"],
        vec![
            "test-impact",
            repo.path().to_str().unwrap(),
            "crate::lib::stable",
        ],
    ] {
        let output = run_bitcode_output(&args);
        assert!(
            !output.status.success(),
            "bitcode {args:?} unexpectedly passed"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("failed to read source src/other.rs"),
            "{stderr}"
        );
        assert_eq!(std::fs::read(&graph_path).unwrap(), durable_before);
    }
}

#[test]
fn test_impact_rejects_every_unknown_explicit_node() {
    let repo = TempRepo::new("test-impact-unknown-node");
    repo.write(
        "src/lib.rs",
        r#"
pub fn known() -> i64 { 1 }

#[test]
fn test_known() {
    assert_eq!(known(), 1);
}
"#,
    );

    let output = run_bitcode_output(&[
        "test-impact",
        repo.path().to_str().unwrap(),
        "crate::lib::known",
        "crate::lib::typo",
    ]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown explicit node path(s)"), "{stderr}");
    assert!(stderr.contains("crate::lib::typo"), "{stderr}");
}

#[test]
fn automatic_test_impact_rejects_a_non_git_project_explicitly() {
    let project = TempRepo::new("test-impact-non-git");
    std::fs::remove_dir_all(project.path().join(".git")).unwrap();
    project.write("src/lib.rs", "pub fn changed() -> i64 { 1 }\n");

    let output = run_bitcode_output(&["test-impact", project.path().to_str().unwrap()]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("automatic test-impact requires a Git repository"),
        "{stderr}"
    );
    assert!(stderr.contains("pass explicit node paths"), "{stderr}");
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

#[test]
fn collaboration_cli_forks_syncs_merges_and_materializes_graphs() {
    let repo = TempRepo::new("collaboration");
    repo.write("src/lib.rs", "pub fn run() -> i64 { 1 }\n");
    let mut durable = aether_graph::SemanticGraph::new();
    durable.upsert_node(aether_graph::Node::new(
        aether_graph::NodeKind::Extension,
        "durable-tool",
        "extension::dev.bitcode.durable-tool",
    ));
    durable.save(repo.path().join("project.aether")).unwrap();
    let root = repo.path().to_str().unwrap();
    let alice = repo.path().join("alice.aetherc");
    let bob = repo.path().join("bob.aetherc");
    let merged = repo.path().join("merged.aethercb");
    let graph_path = repo.path().join("merged.aether");

    let initialized = run_bitcode(&["collab", "init", root, "alice", alice.to_str().unwrap()]);
    assert!(initialized.contains("actor alice"), "{initialized}");
    let before_invite = std::fs::read(&alice).unwrap();
    let denied = run_bitcode_output(&[
        "collab",
        "fork",
        alice.to_str().unwrap(),
        "bob",
        bob.to_str().unwrap(),
    ]);
    assert!(!denied.status.success());
    assert_eq!(std::fs::read(&alice).unwrap(), before_invite);
    assert!(!bob.exists());
    run_bitcode(&[
        "collab",
        "fork",
        alice.to_str().unwrap(),
        "bob",
        bob.to_str().unwrap(),
        "--approve",
    ]);

    repo.write("src/lib.rs", "pub fn run() -> i64 { 2 }\n");
    let synchronized = run_bitcode(&["collab", "sync", root, bob.to_str().unwrap()]);
    assert!(
        synchronized.contains("nodes: +2 -0; edges: +0 -0"),
        "{synchronized}"
    );
    let mut bob_replica = aether_graph::GraphReplica::load(&bob).unwrap();
    let bob_dot = bob_replica
        .operations()
        .find(|(dot, _)| dot.actor.as_str() == "bob")
        .map(|(dot, _)| dot.clone())
        .unwrap();
    bob_replica
        .attest(
            &bob_dot,
            aether_graph::OperationAttestation::new([7; 32], vec![11; 64]).unwrap(),
        )
        .unwrap();
    bob_replica.save(&bob).unwrap();
    let merged_output = run_bitcode(&[
        "collab",
        "merge",
        alice.to_str().unwrap(),
        bob.to_str().unwrap(),
        merged.to_str().unwrap(),
    ]);
    assert!(merged_output.contains("2 inserted"), "{merged_output}");
    assert!(
        merged_output.contains("ignored 1 unverified operation attestation"),
        "{merged_output}"
    );
    run_bitcode(&[
        "collab",
        "materialize",
        merged.to_str().unwrap(),
        graph_path.to_str().unwrap(),
    ]);

    let graph = aether_graph::SemanticGraph::load(&graph_path).unwrap();
    assert!(graph
        .find_by_path("crate::lib::run")
        .unwrap()
        .source
        .contains("{ 2 }"));
    assert!(
        graph
            .find_by_path("extension::dev.bitcode.durable-tool")
            .is_some(),
        "graph-owned durable nodes must survive collaboration init and sync"
    );
    let status = run_bitcode(&["collab", "status", merged.to_str().unwrap()]);
    assert!(status.contains("actor: alice"), "{status}");
    assert!(status.contains("operations:"), "{status}");
    assert!(status.contains("nodes:"), "{status}");
    assert!(status.contains("active members:"), "{status}");
    assert!(status.contains("alice (local)"), "{status}");
    assert!(status.contains("bob"), "{status}");
    assert_eq!(
        aether_graph::GraphReplica::load(&merged)
            .unwrap()
            .attestation_count(),
        0
    );
}

#[test]
fn collaboration_membership_requires_approval_and_rolls_back_failed_invites() {
    let help = run_bitcode(&["--help"]);
    assert!(help.contains("fork, member, sync"), "{help}");
    assert!(help.contains("discover, join, join-peer"), "{help}");

    let repo = TempRepo::new("collaboration-membership");
    repo.write("src/lib.rs", "pub fn run() {}\n");
    let root = repo.path().to_str().unwrap();
    let alice = repo.path().join("alice.aetherc");
    let failed_fork = repo.path().join("missing").join("bob.aetherc");
    run_bitcode(&["collab", "init", root, "alice", alice.to_str().unwrap()]);

    let failed = run_bitcode_output(&[
        "collab",
        "fork",
        alice.to_str().unwrap(),
        "bob",
        failed_fork.to_str().unwrap(),
        "--approve",
    ]);
    assert!(!failed.status.success());
    let replica = aether_graph::GraphReplica::load(&alice).unwrap();
    assert!(!replica
        .is_member(&aether_graph::ActorId::new("bob").unwrap())
        .unwrap());

    let before_alias = std::fs::read(&alice).unwrap();
    let alias = run_bitcode_output(&[
        "collab",
        "fork",
        alice.to_str().unwrap(),
        "bob",
        alice.to_str().unwrap(),
        "--approve",
    ]);
    assert!(!alias.status.success());
    assert_eq!(std::fs::read(&alice).unwrap(), before_alias);

    let denied = run_bitcode_output(&["collab", "member", "add", alice.to_str().unwrap(), "bob"]);
    assert!(!denied.status.success());
    run_bitcode(&[
        "collab",
        "member",
        "add",
        alice.to_str().unwrap(),
        "bob",
        "--approve",
    ]);
    let mut replica = aether_graph::GraphReplica::load(&alice).unwrap();
    assert!(replica
        .is_member(&aether_graph::ActorId::new("bob").unwrap())
        .unwrap());
    assert!(replica.compact_acknowledged().is_err());

    run_bitcode(&[
        "collab",
        "member",
        "remove",
        alice.to_str().unwrap(),
        "bob",
        "--approve",
    ]);
    let replica = aether_graph::GraphReplica::load(&alice).unwrap();
    assert_eq!(
        replica.members().unwrap(),
        vec![aether_graph::ActorId::new("alice").unwrap()]
    );
}

#[test]
fn collaboration_cli_live_host_and_join_converge_authenticated_peers() {
    let repo = TempRepo::new("live-collaboration");
    repo.write("src/lib.rs", "pub fn run() -> i64 { 1 }\n");
    let root = repo.path().to_str().unwrap();
    let alice = repo.path().join("alice.aetherc");
    let bob = repo.path().join("bob.aetherc");
    let secret = repo.path().join("collaboration.secret");
    let ready = repo.path().join("host.ready");
    let discovery_directory = repo.path().join("peers");
    let alice_identity = repo.path().join("alice.identity");
    let alice_public = repo.path().join("alice.identity.pub");
    let alice_trust = repo.path().join("alice.trust");
    let bob_identity = repo.path().join("bob.identity");
    let bob_public = repo.path().join("bob.identity.pub");
    let bob_trust = repo.path().join("bob.trust");
    let secret_output = run_bitcode(&["collab", "secret", secret.to_str().unwrap()]);
    assert!(
        secret_output.contains("contents not displayed"),
        "{secret_output}"
    );

    run_bitcode(&["collab", "init", root, "alice", alice.to_str().unwrap()]);
    run_bitcode(&[
        "collab",
        "fork",
        alice.to_str().unwrap(),
        "bob",
        bob.to_str().unwrap(),
        "--approve",
    ]);
    repo.write("src/lib.rs", "pub fn run() -> i64 { 2 }\n");
    run_bitcode(&["collab", "sync", root, bob.to_str().unwrap()]);
    let alice_identity_output = run_bitcode(&[
        "collab",
        "identity",
        "generate",
        alice.to_str().unwrap(),
        alice_identity.to_str().unwrap(),
        alice_public.to_str().unwrap(),
    ]);
    let alice_fingerprint = generated_identity_fingerprint(&alice_identity_output);
    let bob_identity_output = run_bitcode(&[
        "collab",
        "identity",
        "generate",
        bob.to_str().unwrap(),
        bob_identity.to_str().unwrap(),
        bob_public.to_str().unwrap(),
    ]);
    let bob_fingerprint = generated_identity_fingerprint(&bob_identity_output);
    let shown = run_bitcode(&["collab", "identity", "show", alice_public.to_str().unwrap()]);
    assert!(shown.contains(&alice_fingerprint), "{shown}");
    let denied = run_bitcode_output(&[
        "collab",
        "identity",
        "trust",
        alice_trust.to_str().unwrap(),
        bob_public.to_str().unwrap(),
        "--approve",
        "wrong-fingerprint",
    ]);
    assert!(!denied.status.success());
    assert!(!alice_trust.exists());
    run_bitcode(&[
        "collab",
        "identity",
        "trust",
        alice_trust.to_str().unwrap(),
        bob_public.to_str().unwrap(),
        "--approve",
        &bob_fingerprint,
    ]);
    run_bitcode(&[
        "collab",
        "identity",
        "trust",
        bob_trust.to_str().unwrap(),
        alice_public.to_str().unwrap(),
        "--approve",
        &alice_fingerprint,
    ]);
    for (bundle, private_key) in [(&alice, &alice_identity), (&bob, &bob_identity)] {
        let attested = run_bitcode(&[
            "collab",
            "identity",
            "attest",
            bundle.to_str().unwrap(),
            private_key.to_str().unwrap(),
        ]);
        assert!(attested.contains("new proof(s) added"), "{attested}");
    }

    let mut host = Command::new(env!("CARGO_BIN_EXE_bitcode"))
        .args([
            "collab",
            "host",
            alice.to_str().unwrap(),
            "127.0.0.1:0",
            "--secret-file",
            secret.to_str().unwrap(),
            "--ready-file",
            ready.to_str().unwrap(),
            "--discovery-dir",
            discovery_directory.to_str().unwrap(),
            "--identity-file",
            alice_identity.to_str().unwrap(),
            "--trust-store",
            alice_trust.to_str().unwrap(),
            "--presence",
            "Alice is reviewing",
            "--once",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    if !ready.exists() {
        let _ = host.kill();
        panic!("live collaboration host did not become ready");
    }
    let address = std::fs::read_to_string(&ready).unwrap();
    assert!(address.starts_with("127.0.0.1:"), "{address}");
    let tickets = std::fs::read_dir(&discovery_directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(tickets.len(), 1);
    let ticket_bytes = std::fs::read(&tickets[0]).unwrap();
    assert!(
        !ticket_bytes
            .windows("Alice is reviewing".len())
            .any(|window| window == b"Alice is reviewing"),
        "ephemeral presence leaked into a discovery ticket"
    );
    for fingerprint in [&alice_fingerprint, &bob_fingerprint] {
        assert!(
            !ticket_bytes
                .windows(fingerprint.len())
                .any(|window| window == fingerprint.as_bytes()),
            "actor identity leaked into a discovery ticket"
        );
    }

    let discovered = run_bitcode(&[
        "collab",
        "discover",
        bob.to_str().unwrap(),
        discovery_directory.to_str().unwrap(),
        "--secret-file",
        secret.to_str().unwrap(),
    ]);
    assert!(
        discovered.contains("Discovered 1 authenticated local collaboration peer(s)"),
        "{discovered}"
    );
    assert!(discovered.contains("alice at 127.0.0.1:"), "{discovered}");

    let joined = run_bitcode(&[
        "collab",
        "join-peer",
        bob.to_str().unwrap(),
        "alice",
        discovery_directory.to_str().unwrap(),
        "--secret-file",
        secret.to_str().unwrap(),
        "--identity-file",
        bob_identity.to_str().unwrap(),
        "--trust-store",
        bob_trust.to_str().unwrap(),
        "--presence",
        "Bob is implementing",
    ]);
    assert!(
        joined.contains("Live synchronization with alice complete"),
        "{joined}"
    );
    assert!(
        joined.contains("peer presence: Alice is reviewing"),
        "{joined}"
    );
    assert!(
        joined.contains(&format!(
            "peer identity: Ed25519 SHA-256 {alice_fingerprint}"
        )),
        "{joined}"
    );
    let output = host.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "host failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("peer presence: Bob is implementing"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("peer identity: Ed25519 SHA-256 {bob_fingerprint}")),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    for (bundle, private_key, trust_store) in [
        (&alice, &alice_identity, &alice_trust),
        (&bob, &bob_identity, &bob_trust),
    ] {
        let verified = run_bitcode(&[
            "collab",
            "identity",
            "verify",
            bundle.to_str().unwrap(),
            private_key.to_str().unwrap(),
            trust_store.to_str().unwrap(),
        ]);
        assert!(
            verified.contains("Verified")
                && verified.contains("operation attestation(s) across 2 actor(s)"),
            "{verified}"
        );
    }
    assert!(
        std::fs::read_dir(&discovery_directory)
            .unwrap()
            .next()
            .is_none(),
        "host discovery lease was not removed"
    );
    let after = run_bitcode(&[
        "collab",
        "discover",
        bob.to_str().unwrap(),
        discovery_directory.to_str().unwrap(),
        "--secret-file",
        secret.to_str().unwrap(),
    ]);
    assert!(
        after.contains("Discovered 0 authenticated local collaboration peer(s)"),
        "{after}"
    );

    let bob_rotated_identity = repo.path().join("bob-rotated.identity");
    let bob_rotated_public = repo.path().join("bob-rotated.identity.pub");
    let rotated = run_bitcode(&[
        "collab",
        "identity",
        "generate",
        bob.to_str().unwrap(),
        bob_rotated_identity.to_str().unwrap(),
        bob_rotated_public.to_str().unwrap(),
    ]);
    let rotated_fingerprint = generated_identity_fingerprint(&rotated);
    let local_rotation = run_bitcode(&[
        "collab",
        "identity",
        "rotate-local",
        bob.to_str().unwrap(),
        bob_identity.to_str().unwrap(),
        bob_rotated_identity.to_str().unwrap(),
        "--from",
        &bob_fingerprint,
        "--approve",
        &rotated_fingerprint,
    ]);
    assert!(
        local_rotation.contains("Recorded identity rotation for bob"),
        "{local_rotation}"
    );
    let rotation = run_bitcode(&[
        "collab",
        "identity",
        "rotate",
        alice_trust.to_str().unwrap(),
        bob_rotated_public.to_str().unwrap(),
        "--from",
        &bob_fingerprint,
        "--approve",
        &rotated_fingerprint,
    ]);
    assert!(rotation.contains("Rotated bob"), "{rotation}");
    let trusted = run_bitcode(&["collab", "identity", "list", alice_trust.to_str().unwrap()]);
    assert!(trusted.contains(&rotated_fingerprint), "{trusted}");
    assert!(!trusted.contains(&bob_fingerprint), "{trusted}");
    let removed = run_bitcode(&[
        "collab",
        "identity",
        "remove",
        alice_trust.to_str().unwrap(),
        "bob",
        "--approve",
        &rotated_fingerprint,
    ]);
    assert!(removed.contains("Removed local trust for bob"), "{removed}");
    let trusted = run_bitcode(&["collab", "identity", "list", alice_trust.to_str().unwrap()]);
    assert!(trusted.contains("  none"), "{trusted}");

    let alice_replica = aether_graph::GraphReplica::load(&alice).unwrap();
    let bob_replica = aether_graph::GraphReplica::load(&bob).unwrap();
    assert_eq!(alice_replica.acknowledgements().count(), 1);
    assert_eq!(bob_replica.acknowledgements().count(), 1);
    assert_eq!(
        alice_replica.materialize().unwrap().to_ron().unwrap(),
        bob_replica.materialize().unwrap().to_ron().unwrap()
    );
    let before = alice_replica.operation_count();
    let compacted = run_bitcode(&["collab", "compact", alice.to_str().unwrap()]);
    assert!(compacted.contains("superseded operation"), "{compacted}");
    let alice_replica = aether_graph::GraphReplica::load(&alice).unwrap();
    assert!(alice_replica.operation_count() < before);
    assert_eq!(
        alice_replica.materialize().unwrap().to_ron().unwrap(),
        bob_replica.materialize().unwrap().to_ron().unwrap()
    );
    let status = run_bitcode(&["collab", "status", alice.to_str().unwrap()]);
    assert!(status.contains("compacted through:"), "{status}");
    assert!(
        status.contains("durable operation attestations:"),
        "{status}"
    );
    assert!(status.contains("bob:"), "{status}");
}

#[test]
fn collaboration_cli_reviews_and_applies_whole_file_projection() {
    let repo = TempRepo::new("collaboration-projection");
    repo.write("src/lib.rs", "pub fn value() -> i64 { 1 }\n");
    let root = repo.path().to_str().unwrap();
    let alice = repo.path().join("alice.aetherc");
    let bob = repo.path().join("bob.aetherc");
    run_bitcode(&["collab", "init", root, "alice", alice.to_str().unwrap()]);
    run_bitcode(&[
        "collab",
        "fork",
        alice.to_str().unwrap(),
        "bob",
        bob.to_str().unwrap(),
        "--approve",
    ]);

    repo.write("src/lib.rs", "pub fn value() -> i64 { 2 }\n");
    repo.write("src/new.rs", "pub fn added() {}\n");
    run_bitcode(&["collab", "sync", root, bob.to_str().unwrap()]);
    repo.write("src/lib.rs", "pub fn value() -> i64 { 1 }\n");
    repo.remove("src/new.rs");

    let review = run_bitcode(&["collab", "review", root, bob.to_str().unwrap()]);
    assert!(
        review.contains("Collaboration source-projection review"),
        "{review}"
    );
    assert!(review.contains("~ src/lib.rs"), "{review}");
    assert!(review.contains("+ src/new.rs"), "{review}");
    assert!(review.contains("conflicts: none"), "{review}");

    let denied = run_bitcode_output(&["collab", "apply", root, bob.to_str().unwrap()]);
    assert!(!denied.status.success());
    assert_eq!(
        std::fs::read_to_string(repo.path().join("src/lib.rs")).unwrap(),
        "pub fn value() -> i64 { 1 }\n"
    );
    assert!(!repo.path().join("src/new.rs").exists());

    let applied = run_bitcode(&["collab", "apply", root, bob.to_str().unwrap(), "--approve"]);
    assert!(
        applied.contains("Committed 2 reviewed source projection"),
        "{applied}"
    );
    assert_eq!(
        std::fs::read_to_string(repo.path().join("src/lib.rs")).unwrap(),
        "pub fn value() -> i64 { 2 }\n"
    );
    assert_eq!(
        std::fs::read_to_string(repo.path().join("src/new.rs")).unwrap(),
        "pub fn added() {}\n"
    );
    let graph = aether_graph::SemanticGraph::load(repo.path().join("project.aether")).unwrap();
    assert!(graph.find_by_path("crate::new::added").is_some());
}

#[test]
fn collaboration_apply_validation_failure_leaves_project_unchanged() {
    let repo = TempRepo::new("collaboration-projection-validation");
    repo.write("src/lib.rs", "pub fn value() -> i64 { 1 }\n");
    let root = repo.path().to_str().unwrap();
    let bundle = repo.path().join("remote.aetherc");
    run_bitcode(&["collab", "init", root, "remote", bundle.to_str().unwrap()]);
    repo.write("src/lib.rs", "pub fn value() -> i64 { 2 }\n");
    run_bitcode(&["collab", "sync", root, bundle.to_str().unwrap()]);
    repo.write("src/lib.rs", "pub fn value() -> i64 { 1 }\n");
    repo.write(
        "bitcode.toml",
        r#"
version = 1

[validation]
commands = [["sh", "-c", "printf collaboration-validation-broke >&2; exit 9"]]
"#,
    );

    let output = run_bitcode_output(&[
        "collab",
        "apply",
        root,
        bundle.to_str().unwrap(),
        "--approve",
    ]);
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("[failed] Configured check 1"), "{stdout}");
    assert!(
        stdout.contains("collaboration-validation-broke"),
        "{stdout}"
    );
    assert!(stderr.contains("project was not modified"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(repo.path().join("src/lib.rs")).unwrap(),
        "pub fn value() -> i64 { 1 }\n"
    );
    assert!(!repo.path().join("project.aether").exists());
}

#[test]
fn generated_extension_requires_approval_and_supports_lifecycle() {
    let repo = TempRepo::new("extension-generate");
    repo.write("src/lib.rs", "pub fn existing() {}\n");
    let root = repo.path().to_str().unwrap();

    let preview = run_bitcode(&["extension", root, "generate", "show authentication impact"]);

    assert!(preview.contains("Preview only"), "{preview}");
    assert!(preview.contains("read graph"), "{preview}");
    assert!(!repo.path().join("project.aether").exists());

    let installed = run_bitcode(&[
        "extension",
        root,
        "generate",
        "show authentication impact",
        "--approve",
    ]);
    assert!(installed.contains("Installed and enabled"), "{installed}");

    let listed = run_bitcode(&["extension", root, "list"]);
    assert!(
        listed.contains("dev.bitcode.generated.show-authentication-impact"),
        "{listed}"
    );
    assert!(listed.contains("Enabled"), "{listed}");

    run_bitcode(&[
        "extension",
        root,
        "disable",
        "dev.bitcode.generated.show-authentication-impact",
    ]);
    let listed = run_bitcode(&["extension", root, "list"]);
    assert!(listed.contains("Disabled"), "{listed}");

    run_bitcode(&[
        "extension",
        root,
        "remove",
        "dev.bitcode.generated.show-authentication-impact",
    ]);
    assert_eq!(
        run_bitcode(&["extension", root, "list"]).trim(),
        "No extensions installed."
    );
}

#[test]
fn marketplace_browse_adapt_and_approval_reuse_extension_security() {
    let repo = TempRepo::new("extension-marketplace");
    repo.write("src/lib.rs", "pub fn existing() {}\n");
    let root = repo.path().to_str().unwrap();

    let listed = run_bitcode(&["extension", root, "marketplace", "search", "impact"]);
    assert!(listed.contains("catalog SHA-256:"), "{listed}");
    assert!(listed.contains("org.bitcode.impact-navigator"), "{listed}");
    assert!(listed.contains("org.bitcode.test-focus"), "{listed}");
    assert!(!listed.contains("org.bitcode.rust-check\t"), "{listed}");

    let shown = run_bitcode(&[
        "extension",
        root,
        "marketplace",
        "show",
        "org.bitcode.impact-navigator",
    ]);
    assert!(
        shown.contains("189c188f7827271bd88c84a623305e286"),
        "{shown}"
    );
    assert!(
        shown.contains("Approved by org.bitcode.security"),
        "{shown}"
    );

    let preview = run_bitcode(&[
        "extension",
        root,
        "marketplace",
        "adapt",
        "org.bitcode.impact-navigator",
    ]);
    assert!(preview.contains("Capability delta"), "{preview}");
    assert!(preview.contains("unchanged"), "{preview}");
    assert!(preview.contains("Preview only"), "{preview}");
    assert!(!repo.path().join("project.aether").exists());

    let installed = run_bitcode(&[
        "extension",
        root,
        "marketplace",
        "adapt",
        "org.bitcode.impact-navigator",
        "--approve",
    ]);
    assert!(installed.contains("Installed and enabled"), "{installed}");
    let extensions = run_bitcode(&["extension", root, "list"]);
    assert!(
        extensions.contains("org.bitcode.impact-navigator"),
        "{extensions}"
    );
}

#[test]
fn extension_project_projections_are_restored_on_remove() {
    let repo = TempRepo::new("extension-projections");
    repo.write("src/lib.rs", "pub fn existing() {}\n");
    repo.write("generated/existing.md", "original\n");
    repo.write(
        "extension.json",
        r#"
{
  "version": 1,
  "id": "dev.bitcode.test.report",
  "name": "Test Report",
  "description": "Contributes a bounded report panel and files.",
  "intent": "show a test report",
  "capabilities": [
    {"kind": "write_project", "paths": ["generated/**"]},
    {"kind": "contribute_ui"}
  ],
  "contributions": [
    {
      "kind": "panel",
      "id": "report",
      "title": "Test Report",
      "location": "right",
      "view": {"kind": "markdown", "content": "Report ready"}
    }
  ],
  "projections": [
    {
      "path": "generated/existing.md",
      "contents": "replacement\n",
      "mode": "replace"
    },
    {
      "path": "generated/created.md",
      "contents": "created\n",
      "mode": "create"
    }
  ]
}
"#,
    );
    let root = repo.path().to_str().unwrap();
    let recipe = repo.path().join("extension.json");

    let installed = run_bitcode(&[
        "extension",
        root,
        "install",
        recipe.to_str().unwrap(),
        "--approve",
    ]);

    assert!(installed.contains("Installed and enabled"), "{installed}");
    assert_eq!(
        std::fs::read_to_string(repo.path().join("generated/existing.md")).unwrap(),
        "replacement\n"
    );
    assert_eq!(
        std::fs::read_to_string(repo.path().join("generated/created.md")).unwrap(),
        "created\n"
    );
    let graph = aether_graph::SemanticGraph::load(repo.path().join("project.aether")).unwrap();
    assert_eq!(
        graph.query_by_kind(aether_graph::NodeKind::Extension).len(),
        1
    );

    repo.write("generated/created.md", "user edit\n");
    let conflicted = run_bitcode_output(&["extension", root, "remove", "dev.bitcode.test.report"]);
    assert!(!conflicted.status.success());
    assert!(
        String::from_utf8_lossy(&conflicted.stderr).contains("changed on disk"),
        "{}",
        String::from_utf8_lossy(&conflicted.stderr)
    );
    let graph = aether_graph::SemanticGraph::load(repo.path().join("project.aether")).unwrap();
    assert_eq!(
        graph.query_by_kind(aether_graph::NodeKind::Extension).len(),
        1
    );
    repo.write("generated/created.md", "created\n");

    run_bitcode(&["extension", root, "remove", "dev.bitcode.test.report"]);

    assert_eq!(
        std::fs::read_to_string(repo.path().join("generated/existing.md")).unwrap(),
        "original\n"
    );
    assert!(!repo.path().join("generated/created.md").exists());
    let graph = aether_graph::SemanticGraph::load(repo.path().join("project.aether")).unwrap();
    assert!(graph
        .query_by_kind(aether_graph::NodeKind::Extension)
        .is_empty());
}

#[test]
fn concurrent_review_and_test_impact_produce_complete_output() {
    let repo = TempRepo::new("concurrent-analysis");
    repo.write(
        "src/lib.rs",
        "pub fn helper() -> i64 { 1 }\n\n#[test]\nfn test_helper() { assert!(helper() >= 1); }\n",
    );
    repo.commit_all("baseline");
    repo.write(
        "src/lib.rs",
        "pub fn helper() -> i64 { 2 }\n\n#[test]\nfn test_helper() { assert!(helper() >= 1); }\n",
    );

    let root = repo.path().to_str().unwrap().to_owned();
    // Uncontended runs define the complete expected output; the graph build
    // is deterministic and neither command mutates project state.
    let expected_review = run_bitcode(&["review", &root]);
    let expected_impact = run_bitcode(&["test-impact", &root]);

    let spawn = |command: &'static str, root: String| {
        std::thread::spawn(move || {
            Command::new(env!("CARGO_BIN_EXE_bitcode"))
                .args([command, &root])
                .output()
                .unwrap()
        })
    };

    for round in 0..5 {
        // Alternate the sharpest pairings: mixed commands and two index
        // readers of the same kind.
        let (first, second) = if round % 2 == 0 {
            ("review", "test-impact")
        } else {
            ("test-impact", "test-impact")
        };
        let a = spawn(first, root.clone());
        let b = spawn(second, root.clone());
        for (command, handle) in [(first, a), (second, b)] {
            let output = handle.join().unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success(),
                "round {round}: concurrent {command} failed\nstdout:\n{stdout}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let expected = if command == "review" {
                &expected_review
            } else {
                &expected_impact
            };
            assert_eq!(
                stdout.as_ref(),
                expected.as_str(),
                "round {round}: concurrent {command} output was incomplete or reordered"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn test_impact_run_kills_a_timed_out_process_tree() {
    let repo = TempRepo::new("run-timeout");
    repo.write(
        "src/lib.rs",
        "pub fn add(a: i64, b: i64) -> i64 { a + b }\n\n#[test]\nfn test_add() { assert_eq!(add(2, 3), 5); }\n",
    );
    repo.write(
        "bitcode.toml",
        r#"
version = 1

[tests]
rust = ["sh", "-c", "sleep 60 & echo $! > grandchild.pid; wait", "{test}"]
run_timeout_seconds = 1
"#,
    );

    let started = Instant::now();
    let output = run_bitcode_output(&[
        "test-impact",
        repo.path().to_str().unwrap(),
        "--run",
        "crate::lib::add",
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "timed-out run must fail");
    assert!(
        stderr.contains("timed out after 1s"),
        "stderr must classify the timeout:\n{stderr}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "the runner must not wait out the child"
    );

    // The configured command's own background child (a grandchild of
    // bitcode) must not survive the process-group kill.
    let grandchild = std::fs::read_to_string(repo.path().join("grandchild.pid"))
        .expect("runner wrote its grandchild pid before the kill");
    let grandchild = grandchild.trim().to_owned();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let alive = Command::new("kill")
            .args(["-0", &grandchild])
            .status()
            .unwrap()
            .success();
        if !alive {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "grandchild {grandchild} survived the process-tree kill"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(unix)]
#[test]
fn test_impact_run_kills_a_child_exceeding_the_output_budget() {
    let repo = TempRepo::new("run-output-cap");
    repo.write(
        "src/lib.rs",
        "pub fn add(a: i64, b: i64) -> i64 { a + b }\n\n#[test]\nfn test_add() { assert_eq!(add(2, 3), 5); }\n",
    );
    repo.write(
        "bitcode.toml",
        r#"
version = 1

[tests]
rust = ["sh", "-c", "yes overflowing-test-output", "{test}"]
run_max_output_bytes = 4096
"#,
    );

    let started = Instant::now();
    let output = run_bitcode_output(&[
        "test-impact",
        repo.path().to_str().unwrap(),
        "--run",
        "crate::lib::add",
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "overflowing run must fail");
    assert!(
        stderr.contains("produced more than 4096 bytes"),
        "stderr must classify the overflow:\n{stderr}"
    );
    assert!(started.elapsed() < Duration::from_secs(30));
    // The tee stops retaining once the cap is hit, so bitcode's own stdout
    // stays bounded instead of relaying the flood.
    assert!(output.stdout.len() < 64 * 1024);
}

#[cfg(unix)]
#[test]
fn analysis_classifies_a_hung_git_subprocess() {
    let repo = TempRepo::new("git-timeout");
    repo.write(
        "src/lib.rs",
        "pub fn add(a: i64, b: i64) -> i64 { a + b }\n",
    );
    repo.commit_all("baseline");

    let shims = repo.path().join("shims");
    std::fs::create_dir_all(&shims).unwrap();
    let shim = shims.join("git");
    std::fs::write(&shim, "#!/bin/sh\nsleep 60\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!(
        "{}:{}",
        shims.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let started = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_bitcode"))
        .args(["test-impact", repo.path().to_str().unwrap()])
        .env("PATH", path)
        .env("BITCODE_GIT_TIMEOUT_SECONDS", "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "hung git must fail the command");
    assert!(
        stderr.contains("timed out after 1s"),
        "stderr must classify the git timeout:\n{stderr}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "analysis must not wait out the hung git"
    );
}
