//! Go same-package direct-call proof (Stage 3, Go): the 21 frozen fixtures and a
//! cold-versus-incremental equality sequence. Policy:
//! docs/observations/stage3-go-audit/policy.md.
use aether_builder::{GraphBuilder, Lang};
use aether_graph::{CallClass, SemanticGraph};
use serde_json::Value;
use std::path::{Path, PathBuf};

const REASON: &str = "proven-go-same-package-call";

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/go-direct-call-proof/v1")
}

fn sources(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with('.'))
        {
            continue;
        }
        if entry.file_type().unwrap().is_dir() {
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

fn build(files: &[(String, String)]) -> (SemanticGraph, GraphBuilder) {
    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();
    let mut sorted = files.to_vec();
    sorted.sort();
    builder.load_files(
        &mut graph,
        sorted.iter().map(|(f, s)| (f.as_str(), s.as_str())),
    );
    (graph, builder)
}

/// End byte of the call that follows the `/* claim */` marker.
fn marked_call_end(source: &str, marker: &str) -> usize {
    let after = source.find(marker).unwrap() + marker.len();
    let rest = &source[after..];
    let open = rest.find('(').unwrap();
    let mut depth = 0usize;
    for (i, c) in rest[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return after + open + i + 1;
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced call after marker");
}

fn claim_at(graph: &SemanticGraph, file: &str, end: usize) -> Vec<aether_graph::CallClaim> {
    graph
        .nodes()
        .filter(|n| n.file.as_deref() == Some(file))
        .filter_map(|n| graph.call_evidence(n.id).ok())
        .flat_map(|e| e.calls)
        .filter(|c| !c.coverage_gap && c.site.end_byte == end)
        .collect()
}

#[test]
fn frozen_21_fixture_contracts() {
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(corpus().join("manifest.json")).unwrap())
            .unwrap();
    let mut failures = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(case["path"].as_str().unwrap());
        let mut files = Vec::new();
        sources(&root, &root, &mut files);
        let (graph, _) = build(&files);
        let importer = case["importer"].as_str().unwrap();
        let source = std::fs::read_to_string(root.join(importer)).unwrap();
        let end = marked_call_end(&source, "/* claim */");
        let claims = claim_at(&graph, importer, end);
        if claims.len() != 1 {
            failures.push(format!(
                "{id}: expected one marked claim, got {}",
                claims.len()
            ));
            continue;
        }
        let claim = &claims[0];
        if case["expected"] == "unknown" {
            if claim.class != CallClass::Unknown || !claim.targets.is_empty() {
                failures.push(format!(
                    "{id}: expected Unknown, got {:?} {:?}",
                    claim.class, claim.reason
                ));
            }
            continue;
        }
        let (file, symbol) = case["expected_target"]
            .as_str()
            .unwrap()
            .split_once("::")
            .unwrap();
        // A method with the same name is deeper (`crate::T::Target`); the
        // top-level function has the shallowest path.
        let named: Vec<_> = graph
            .nodes()
            .filter(|n| {
                n.file.as_deref() == Some(file) && n.path.rsplit("::").next() == Some(symbol)
            })
            .collect();
        let shallowest = named.iter().map(|n| n.path.matches("::").count()).min();
        let expected: Vec<_> = named
            .iter()
            .filter(|n| Some(n.path.matches("::").count()) == shallowest)
            .map(|n| n.id)
            .collect();
        if claim.class != CallClass::Must
            || claim.reason != REASON
            || claim.targets != expected
            || expected.len() != 1
        {
            failures.push(format!(
                "{id}: expected Must at {file}::{symbol}, got {:?} {:?} {:?}",
                claim.class, claim.reason, claim.targets
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Canonical comparison of every node's call evidence.
fn snapshot(graph: &SemanticGraph) -> Vec<String> {
    let mut out = Vec::new();
    for node in graph.nodes() {
        if node.language != "go" {
            continue;
        }
        let mut rows = Vec::new();
        if let Ok(evidence) = graph.call_evidence(node.id) {
            let mut assumptions = evidence.assumptions.clone();
            assumptions.sort();
            for c in &evidence.calls {
                rows.push(format!(
                    "{}-{}:{:?}:{:?}:{}:{}",
                    c.site.start_byte,
                    c.site.end_byte,
                    c.class,
                    c.targets,
                    c.reason,
                    c.coverage_gap
                ));
            }
            rows.sort();
            rows.push(format!("assumptions={assumptions:?}"));
        }
        out.push(format!(
            "{}|{:?}|{}|{:?}",
            node.path, node.file, node.kind as u8, rows
        ));
    }
    out.sort();
    out
}

#[test]
fn cold_and_incremental_go_graphs_agree_through_package_edits() {
    let caller = "package p\n\nfunc Caller() int { return /* c */ Target() }\n";
    let target = "package p\n\nfunc Target() int { return 1 }\n";
    let constrained = "//go:build linux\n\npackage p\n\nfunc Target() int { return 1 }\n";
    let duplicate = "package p\n\nfunc Target() int { return 2 }\n";
    let filler = "package p\n\nfunc Other() int { return 3 }\n";
    let mut files: Vec<(String, String)> = vec![
        ("p/a.go".into(), caller.into()),
        ("p/b.go".into(), target.into()),
    ];
    let (mut graph, mut builder) = build(&files);
    let end = marked_call_end(caller, "/* c */");
    let class = |graph: &SemanticGraph| claim_at(graph, "p/a.go", end)[0].class;
    assert_eq!(class(&graph), CallClass::Must);
    assert_eq!(snapshot(&graph), snapshot(&build(&files).0));
    let steps: Vec<(&str, &str)> = vec![
        ("p/b.go", constrained),
        ("p/b.go", target),
        ("p/c.go", duplicate),
        ("p/c.go", filler),
        ("p/b.go", constrained),
        ("p/b.go", target),
    ];
    let expected = [
        CallClass::Unknown,
        CallClass::Must,
        CallClass::Unknown,
        CallClass::Must,
        CallClass::Unknown,
        CallClass::Must,
    ];
    for ((file, source), want) in steps.into_iter().zip(expected) {
        builder.update_file(&mut graph, file, source);
        match files.iter_mut().find(|(f, _)| f == file) {
            Some(slot) => slot.1 = source.into(),
            None => files.push((file.into(), source.into())),
        }
        assert_eq!(class(&graph), want, "after editing {file}");
        assert_eq!(
            snapshot(&graph),
            snapshot(&build(&files).0),
            "incremental differs from cold after editing {file}"
        );
    }
}

/// A call in a package-level initializer has no enclosing function, so it stays
/// Unknown even when its claim sits on the surviving module node (single-file
/// package, or the file loaded last). Exercises the enclosing-function rule.
#[test]
fn package_level_initializer_call_is_never_must() {
    let tail = "package p\n\nfunc Target() int { return 42 }\n\nvar Value = /* claim */ Target()\n";
    for files in [
        vec![("p/app.go".to_string(), tail.to_string())],
        vec![
            (
                "p/a.go".to_string(),
                "package p\n\nfunc Other() int { return 1 }\n".to_string(),
            ),
            ("p/b.go".to_string(), tail.to_string()),
        ],
    ] {
        let importer = &files.last().unwrap().0.clone();
        let (graph, _) = build(&files);
        let claims = claim_at(&graph, importer, marked_call_end(tail, "/* claim */"));
        assert_eq!(claims.len(), 1, "{files:?}");
        assert_eq!(claims[0].class, CallClass::Unknown, "{files:?}");
        assert!(claims[0].targets.is_empty());
    }
}

/// A generic call with a qualified type argument can defeat the grammar. Code
/// the grammar cannot parse must still produce an explicit Unknown claim at the
/// call, never an omitted one.
#[test]
fn call_with_qualified_type_argument_gets_a_claim() {
    let source = "package p\n\nfunc F(v any) {\n\tm, ok := reflect.TypeAssert[encoding.TextMarshaler](v)\n\t_, _ = m, ok\n}\n";
    let files = vec![("p/a.go".to_string(), source.to_string())];
    let (graph, _) = build(&files);
    let end = source.find("(v)").unwrap() + 3;
    let claims = claim_at(&graph, "p/a.go", end);
    assert_eq!(claims.len(), 1, "no claim for the call ending at {end}");
    assert_eq!(claims[0].class, CallClass::Unknown);
}

/// Policy G1-a: a call that is the direct operand of `go` or `defer` stays
/// Unknown (same-file and cross-file); calls nested inside such a statement
/// (an argument, or the body of a deferred literal) remain eligible.
#[test]
fn go_and_defer_statement_calls_stay_unknown_but_nested_calls_stay_eligible() {
    let a = "package p\n\nfunc F() {}\nfunc G() int { return 1 }\nfunc F2(int) {}\n\nfunc Run() {\n\tdefer /* d1 */ F()\n\tgo /* g1 */ H()\n\tdefer /* n1 */ F2(/* n2 */ G())\n\tdefer func() { /* n3 */ G() }()\n}\n";
    let b = "package p\n\nfunc H() {}\n";
    let files = vec![
        ("p/a.go".to_string(), a.to_string()),
        ("p/b.go".to_string(), b.to_string()),
    ];
    let (graph, _) = build(&files);
    let class = |marker: &str| {
        let claims = claim_at(&graph, "p/a.go", marked_call_end(a, marker));
        assert_eq!(claims.len(), 1, "{marker}");
        claims[0].class
    };
    assert_eq!(class("/* d1 */"), CallClass::Unknown, "same-file defer");
    assert_eq!(class("/* g1 */"), CallClass::Unknown, "cross-file go");
    assert_eq!(class("/* n1 */"), CallClass::Unknown, "outer deferred call");
    assert_eq!(
        class("/* n2 */"),
        CallClass::Must,
        "argument of a deferred call"
    );
    assert_eq!(
        class("/* n3 */"),
        CallClass::Must,
        "call inside a deferred literal"
    );
}
