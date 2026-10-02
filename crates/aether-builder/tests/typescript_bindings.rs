use aether_builder::GraphBuilder;
use aether_graph::{CallClass, NodeKind, SemanticGraph};
use serde_json::Value;
use std::path::Path;

#[test]
fn frozen_typescript_binding_contract() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/typescript-binding-corpus/v1");
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).unwrap())
            .unwrap();
    let marker = manifest["marker"].as_str().unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 22);
    let mut failures = Vec::new();
    for case in cases {
        let file = case["file"].as_str().unwrap();
        let source = std::fs::read_to_string(root.join(file)).unwrap();
        let marker_end = source.find(marker).unwrap() + marker.len();
        let offset =
            marker_end + source[marker_end..].len() - source[marker_end..].trim_start().len();
        let mut graph = SemanticGraph::new();
        GraphBuilder::new().load_file(&mut graph, file, &source);
        let claims: Vec<_> = graph
            .nodes()
            .filter_map(|node| graph.call_evidence(node.id).ok())
            .flat_map(|evidence| evidence.calls)
            .filter(|claim| claim.site.start_byte == offset)
            .collect();
        let expected = if case["expected_class"] == "must" {
            CallClass::Must
        } else {
            CallClass::Unknown
        };
        if claims.len() != 1 || claims[0].class != expected {
            failures.push(format!("{file}: expected one {expected:?}, got {claims:?}"));
            continue;
        }
        if expected == CallClass::Must {
            let declaration = source
                .find(case["target_marker"].as_str().unwrap())
                .unwrap();
            let targets: Vec<_> = graph
                .nodes()
                .filter(|n| n.kind == NodeKind::Function && n.span.start_byte == declaration)
                .map(|n| n.id)
                .collect();
            if targets.len() != 1 || claims[0].targets != targets {
                failures.push(format!("{file}: wrong declaration target: {:?}", claims[0]));
            }
        } else if !claims[0].targets.is_empty() {
            failures.push(format!("{file}: Unknown contains guessed targets"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
