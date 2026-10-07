//! Application contracts extending the unchanged 73-case/19-step ESM policy.
use super::*;
use aether_graph::{CallClass, NodeKind};

/// ESM fixtures must not inherit the shared Rust seed files: `app.rs` and
/// `app.ts` share module path `app`, which policy R-GRAPH-2 refuses
/// (`mixed-language-module-collision`).
fn esm_fixture() -> Fixture {
    let fixture = Fixture::new();
    for name in ["lib.rs", "math.rs", "app.rs"] {
        std::fs::remove_file(fixture.0.join(name)).unwrap();
    }
    fixture
}

fn write_esm(root: &Path) {
    std::fs::write(
        root.join("app.ts"),
        "export function target() { return 1; }",
    )
    .unwrap();
    std::fs::write(
        root.join("app.test.ts"),
        "import { target } from './app.ts'; export function run() { return /* claim */ target(); }",
    )
    .unwrap();
}

fn marked(graph: &SemanticGraph, root: &Path, importer: &str) -> aether_graph::CallClaim {
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
    assert_eq!(claims.len(), 1);
    for target in &claims[0].targets {
        assert!(graph.contains(*target));
    }
    claims[0].clone()
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &to.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
fn esm_snapshot_tracks_non_source_proof_inputs() {
    for name in [
        "package.json",
        "nested/package.json",
        "target/package.json",
        ".npmrc",
        ".mocharc.json",
        "nested/jest.config.json",
        "__mocks__",
    ] {
        let root = esm_fixture();
        write_esm(&root.0);
        let config = ProjectConfig::default();
        let before = Snapshot::capture(&root.0, &config).unwrap();
        let path = root.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        if name == "__mocks__" {
            std::fs::create_dir(&path).unwrap();
        } else {
            std::fs::write(&path, "{}").unwrap();
        }
        let after = Snapshot::capture(&root.0, &config).unwrap();
        assert!(
            !before.sources_equal(&after),
            "untracked proof input: {name}"
        );
        assert!(after.dirty_since(&before).iter().any(|p| p == name));
    }
}

#[test]
fn esm_watch_non_source_creation_edit_and_removal_match_cold() {
    let root = esm_fixture();
    write_esm(&root.0);
    let server = Server::start(&root.0).unwrap();
    assert_eq!(
        marked(&equals_cold(&server).graph, &root.0, "app.test.ts").class,
        CallClass::Must
    );
    for name in [
        "package.json",
        "nested/package.json",
        "target/package.json",
        ".npmrc",
        ".mocharc.json",
        "nested/jest.config.json",
        "__mocks__",
    ] {
        let path = root.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        if name == "__mocks__" {
            std::fs::create_dir(&path).unwrap();
        } else {
            std::fs::write(
                &path,
                r#"{"scripts":{"test":"node --import ./hooks.mjs app.test.ts"}}"#,
            )
            .unwrap();
        }
        let generation = equals_cold(&server);
        assert_eq!(
            marked(&generation.graph, &root.0, "app.test.ts").class,
            CallClass::Unknown,
            "{name}"
        );
        if name.ends_with("package.json") {
            std::fs::write(&path, "{}").unwrap();
            assert_eq!(
                marked(&equals_cold(&server).graph, &root.0, "app.test.ts").class,
                CallClass::Must,
                "{name}"
            );
        }
        if path.is_dir() {
            std::fs::remove_dir_all(&path).unwrap();
        } else {
            std::fs::remove_file(&path).unwrap();
        }
        assert_eq!(
            marked(&equals_cold(&server).graph, &root.0, "app.test.ts").class,
            CallClass::Must,
            "{name}"
        );
    }
}

#[test]
fn esm_watch_replays_frozen_19_steps_and_serves_mcp() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(
            repo.join("fixtures/typescript-esm-import-proof/v1/manifest.json"),
        )
        .unwrap(),
    )
    .unwrap();
    for sequence in manifest["incremental_sequences"].as_array().unwrap() {
        let root = esm_fixture();
        let base = repo.join(sequence["base_dir"].as_str().unwrap());
        copy(&base, &root.0);
        let importer = sequence["importer"].as_str().unwrap();
        let server = Server::start(&root.0).unwrap();
        assert_eq!(
            marked(&equals_cold(&server).graph, &root.0, importer).class,
            CallClass::Must
        );
        for (index, step) in sequence["steps"].as_array().unwrap().iter().enumerate() {
            let path = root.0.join(step["path"].as_str().unwrap());
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
                op => panic!("unmodeled operation {op}"),
            }
            let generation = equals_cold(&server); // compares the complete graph and evidence
            let expected = if step["expected_class"] == "must" {
                CallClass::Must
            } else {
                CallClass::Unknown
            };
            assert_eq!(
                marked(&generation.graph, &root.0, importer).class,
                expected,
                "step {index}"
            );
            let module = generation
                .graph
                .nodes()
                .find(|n| n.kind == NodeKind::Module && n.file.as_deref() == Some(importer))
                .unwrap();
            let request = json!({"jsonrpc":"2.0","id":index,"method":"tools/call","params":{"name":"orient","arguments":{"nodes":[module.path]}}});
            let mut output = Vec::new();
            server
                .respond(
                    &request.to_string(),
                    &std::env::current_exe().unwrap(),
                    &mut output,
                )
                .unwrap();
            let response: Value = serde_json::from_slice(&output).unwrap();
            assert!(response.get("error").is_none(), "{response}");
            assert_ne!(response["result"]["isError"], true, "{response}");
            let result: Value =
                serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert!(result["nodes"].is_array(), "{result}");
        }
    }
}

#[test]
fn esm_watch_subdirectory_target_deletion_and_recreate_match_cold() {
    let root = esm_fixture();
    let write_target = |root: &std::path::Path| {
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::write(
            root.join("lib/util.ts"),
            "export function target() { return 1; }",
        )
        .unwrap();
    };
    write_target(&root.0);
    std::fs::write(
        root.0.join("app.test.ts"),
        "import { target } from './lib/util.ts'; export function run() { return /* claim */ target(); }",
    )
    .unwrap();
    let server = Server::start(&root.0).unwrap();
    assert_eq!(
        marked(&equals_cold(&server).graph, &root.0, "app.test.ts").class,
        CallClass::Must
    );
    std::fs::remove_dir_all(root.0.join("lib")).unwrap();
    assert_eq!(
        marked(&equals_cold(&server).graph, &root.0, "app.test.ts").class,
        CallClass::Unknown
    );
    write_target(&root.0);
    assert_eq!(
        marked(&equals_cold(&server).graph, &root.0, "app.test.ts").class,
        CallClass::Must
    );
}

#[test]
#[cfg(unix)]
fn esm_watch_symlinked_target_file_stays_unknown() {
    let root = esm_fixture();
    std::fs::create_dir_all(root.0.join("real")).unwrap();
    std::fs::write(
        root.0.join("real/app.ts"),
        "export function target() { return 1; }",
    )
    .unwrap();
    std::fs::write(
        root.0.join("app.test.ts"),
        "import { target } from './app.ts'; export function run() { return /* claim */ target(); }",
    )
    .unwrap();
    std::os::unix::fs::symlink("real/app.ts", root.0.join("app.ts")).unwrap();
    let server = Server::start(&root.0).unwrap();
    assert_eq!(
        marked(&equals_cold(&server).graph, &root.0, "app.test.ts").class,
        CallClass::Unknown
    );
}

#[test]
fn esm_watch_follow_symlinks_keeps_plain_target_unknown() {
    let root = esm_fixture();
    write_esm(&root.0);
    std::fs::write(
        root.0.join("girder.toml"),
        "[source]\nroots = [\".\"]\nfollow_symlinks = true\n",
    )
    .unwrap();
    let server = Server::start(&root.0).unwrap();
    assert_eq!(
        marked(&equals_cold(&server).graph, &root.0, "app.test.ts").class,
        CallClass::Unknown
    );
}

#[test]
fn esm_stale_environment_candidate_is_not_publishable() {
    let root = esm_fixture();
    write_esm(&root.0);
    let project = CachedProject::open_with_exclusions(&root.0, &exclusions()).unwrap();
    let used = project.builder.typescript_environment().unwrap().clone();
    std::fs::write(root.0.join("package.json"), "{}").unwrap();
    let snapshot = Snapshot::capture(&root.0, &project.config).unwrap();
    assert_ne!(used, snapshot.environment);
    for path in project.builder.source_files() {
        assert!(
            snapshot.matches_bytes(&path, project.builder.source_of(&path).map(str::as_bytes)),
            "{path}"
        );
    }
    for (path, bytes) in project.configuration_inputs() {
        assert!(snapshot.matches_bytes(path, bytes.as_deref()), "{path}");
    }
    assert_eq!(snapshot.graph, project.persisted_bytes);
    assert!(!snapshot.matches_candidate(&project));
}

#[test]
#[cfg(unix)]
fn esm_watch_symlinked_directory_target_stays_unknown() {
    let root = esm_fixture();
    std::fs::create_dir_all(root.0.join("real_lib")).unwrap();
    std::fs::write(
        root.0.join("real_lib/util.ts"),
        "export function target() { return 1; }",
    )
    .unwrap();
    std::fs::write(
        root.0.join("app.test.ts"),
        "import { target } from './lib/util.ts'; export function run() { return /* claim */ target(); }",
    )
    .unwrap();
    std::os::unix::fs::symlink("real_lib", root.0.join("lib")).unwrap();
    let server = Server::start(&root.0).unwrap();
    assert_eq!(
        marked(&equals_cold(&server).graph, &root.0, "app.test.ts").class,
        CallClass::Unknown
    );
}
