use aether_builder::{FileChange, GraphBuilder};
use aether_graph::{EdgeKind, NodeId, SemanticGraph};
use std::collections::BTreeMap;

#[test]
fn incremental_three_file_facade_export_rename_matches_cold() {
    let mut files = BTreeMap::from([
        ("src/lib.rs", "mod math;\nmod app;\npub use math::add;\n"),
        (
            "src/math.rs",
            "pub fn add() -> i32 { 1 }\npub fn other() -> i32 { 2 }\n",
        ),
        ("src/app.rs", "pub fn run() -> i32 { crate::add() }\n"),
    ]);
    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();
    builder.load_files(
        &mut graph,
        files.iter().map(|(path, source)| (*path, *source)),
    );
    let old_edge = (
        NodeId::from_path("crate::app::run"),
        NodeId::from_path("crate::math::add"),
        EdgeKind::Calls,
    );
    assert!(
        graph.edges().contains(&old_edge),
        "fixture must exercise its facade call"
    );

    let renamed = "pub fn plus() -> i32 { 1 }\npub fn other() -> i32 { 2 }\n";
    files.insert("src/math.rs", renamed);
    let report = builder
        .update_files(
            &mut graph,
            &[FileChange::replace("src/math.rs", renamed)],
            &[],
        )
        .unwrap();
    assert_eq!(report.parsed_files, ["src/math.rs"]);
    assert_eq!(report.reused_files, ["src/app.rs", "src/lib.rs"]);
    assert!(report.full_rebuild_reasons.is_empty());
    assert!(!graph.edges().contains(&old_edge));

    let mut cold = SemanticGraph::new();
    GraphBuilder::new().load_files(
        &mut cold,
        files.iter().map(|(path, source)| (*path, *source)),
    );
    let mut actual_nodes: Vec<_> = graph.nodes().cloned().collect();
    let mut expected_nodes: Vec<_> = cold.nodes().cloned().collect();
    actual_nodes.sort_by_key(|node| node.id);
    expected_nodes.sort_by_key(|node| node.id);
    assert_eq!(actual_nodes, expected_nodes, "complete node records");
    let mut actual_edges = graph.edge_records();
    let mut expected_edges = cold.edge_records();
    actual_edges.sort_by_key(|(from, to, edge)| (*from, *to, edge.kind));
    expected_edges.sort_by_key(|(from, to, edge)| (*from, *to, edge.kind));
    assert_eq!(actual_edges, expected_edges, "complete edge records");
}
