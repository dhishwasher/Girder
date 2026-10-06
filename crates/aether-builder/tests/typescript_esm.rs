use aether_builder::{FileChange, GraphBuilder, Lang};
use aether_graph::{CallClass, SemanticGraph};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const REASON: &str = "proven-typescript-relative-esm-named-import";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "girder-esm-proof-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let path = to.join(entry.file_name());
        let kind = entry.file_type().unwrap();
        if kind.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(entry.path()).unwrap(), path).unwrap();
            #[cfg(not(unix))]
            panic!("symlink ingestion contract requires a symlink-capable test environment");
        } else if kind.is_dir() {
            copy(&entry.path(), &path);
        } else {
            std::fs::copy(entry.path(), path).unwrap();
        }
    }
}

fn sources(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        // Mirror the default CLI inventory, including the frozen case whose
        // existing mock target lives under excluded `target/`.
        if kind.is_symlink()
            || entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with('.'))
            || matches!(
                entry.file_name().to_str(),
                Some("node_modules" | "target" | "__pycache__" | "venv")
            )
        {
            continue;
        }
        if kind.is_dir() {
            sources(root, &entry.path(), out);
        } else {
            let file = entry
                .path()
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            if Lang::from_path(&file).is_some() {
                out.push((file, std::fs::read_to_string(entry.path()).unwrap()));
            }
        }
    }
}

fn build(root: &Path) -> (SemanticGraph, GraphBuilder) {
    let mut contents = Vec::new();
    sources(root, root, &mut contents);
    contents.sort();
    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();
    builder.set_source_root(root).unwrap();
    builder.load_files(
        &mut graph,
        contents
            .iter()
            .map(|(file, source)| (file.as_str(), source.as_str())),
    );
    (graph, builder)
}

fn marked(graph: &SemanticGraph, root: &Path, importer: &str) -> (CallClass, Vec<String>) {
    let source = std::fs::read_to_string(root.join(importer)).unwrap();
    let (_, tail) = source.split_once("/* claim */").unwrap();
    let offset = source.len() - tail.trim_start().len();
    let claims: Vec<_> = graph
        .nodes()
        .filter(|n| n.file.as_deref() == Some(importer))
        .filter_map(|n| graph.call_evidence(n.id).ok())
        .flat_map(|e| e.calls)
        .filter(|c| c.site.start_byte == offset)
        .collect();
    assert_eq!(
        claims.len(),
        1,
        "missing or duplicate evidence for {importer}"
    );
    let claim = &claims[0];
    let targets = claim
        .targets
        .iter()
        .map(|id| {
            let target = graph.get(*id).expect("certificate names a missing target");
            format!("{}:{}", target.file.as_deref().unwrap(), target.path)
        })
        .collect();
    if claim.class == CallClass::Must {
        assert_eq!(claim.reason, REASON);
    }
    (claim.class, targets)
}

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/typescript-esm-import-proof")
}

#[test]
fn frozen_73_cold_contracts() {
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(corpus().join("v1/manifest.json")).unwrap())
            .unwrap();
    let mut failures = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let root = Root::new();
        copy(&corpus().join("v1").join(id), &root.0);
        let (graph, _) = build(&root.0);
        let actual = marked(&graph, &root.0, case["importer"].as_str().unwrap());
        let expected = if case["expected_class"] == "must" {
            (
                CallClass::Must,
                vec![format!(
                    "{}:{}",
                    case["expected_target"]["file"].as_str().unwrap(),
                    case["expected_target"]["predicted_path"].as_str().unwrap()
                )],
            )
        } else {
            (CallClass::Unknown, vec![])
        };
        if actual != expected {
            failures.push(format!("{id}: expected {expected:?}, observed {actual:?}"));
        }
        for node in graph.nodes() {
            let Ok(evidence) = graph.call_evidence(node.id) else {
                continue;
            };
            if evidence.calls.iter().any(|c| c.reason == REASON) {
                for assumption in manifest["must_claim_assumptions"].as_array().unwrap() {
                    assert!(
                        evidence
                            .assumptions
                            .iter()
                            .any(|a| a == assumption.as_str().unwrap()),
                        "{id}: missing assumption"
                    );
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn frozen_incremental_sequence_matches_cold_graph_after_every_step() {
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(corpus().join("v1/manifest.json")).unwrap())
            .unwrap();
    for sequence in manifest["incremental_sequences"].as_array().unwrap() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let base = repo.join(sequence["base_dir"].as_str().unwrap());
        let root = Root::new();
        copy(&base, &root.0);
        let (mut graph, mut builder) = build(&root.0);
        let importer = sequence["importer"].as_str().unwrap();
        assert_eq!(marked(&graph, &root.0, importer).0, CallClass::Must);
        for (index, step) in sequence["steps"].as_array().unwrap().iter().enumerate() {
            let relative = step["path"].as_str().unwrap();
            let path = root.0.join(relative);
            match step["op"].as_str().unwrap() {
                "write" => {
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::copy(
                        base.parent()
                            .unwrap()
                            .join(step["source"].as_str().unwrap()),
                        &path,
                    )
                    .unwrap();
                }
                "delete-tree" => std::fs::remove_dir_all(&path).unwrap(),
                "delete" => std::fs::remove_file(&path).unwrap(),
                other => panic!("unmodeled operation {other}"),
            }
            // Inventory events include non-source paths. Deleted directories
            // remove every previously indexed descendant in the same batch.
            let mut current = Vec::new();
            sources(&root.0, &root.0, &mut current);
            let mut changes = Vec::new();
            for old in builder.source_files() {
                if !current.iter().any(|(file, _)| file == &old) {
                    changes.push(FileChange::delete(old));
                }
            }
            for (file, source) in current {
                if file == relative || !builder.source_files().contains(&file) {
                    changes.push(FileChange::replace(file, source));
                }
            }
            builder.update_files(&mut graph, &changes, &[]).unwrap();
            let (cold, _) = build(&root.0);
            let actual = marked(&graph, &root.0, importer);
            assert_eq!(
                actual,
                marked(&cold, &root.0, importer),
                "step {index}: {relative}"
            );
            let expected = if step["expected_class"] == "must" {
                CallClass::Must
            } else {
                CallClass::Unknown
            };
            assert_eq!(actual.0, expected, "step {index}: {relative}");
        }
    }
}

#[test]
fn source_only_and_modified_projection_loads_have_no_import_certificate() {
    let root = Root::new();
    copy(&corpus().join("v1/explicit-ts"), &root.0);
    let mut contents = Vec::new();
    sources(&root.0, &root.0, &mut contents);
    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();
    builder.load_files(
        &mut graph,
        contents.iter().map(|(f, s)| (f.as_str(), s.as_str())),
    );
    assert_eq!(marked(&graph, &root.0, "app.test.ts").0, CallClass::Unknown);
    builder.set_source_root(&root.0).unwrap();
    builder.resolve_calls(&mut graph);
    assert_eq!(marked(&graph, &root.0, "app.test.ts").0, CallClass::Must);
    builder.load_file(
        &mut graph,
        "app.ts",
        "export function target() { return 'projected'; }",
    );
    assert_eq!(marked(&graph, &root.0, "app.test.ts").0, CallClass::Unknown);
}
