use aether_builder::{FileChange, FullRebuildReason, GraphBuilder, UpdateReport};
use aether_graph::{NodeId, SemanticGraph};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub fn corpus() -> Value {
    serde_json::from_str(include_str!(
        "../../../../docs/incremental-mutation-corpus.json"
    ))
    .unwrap()
}

pub fn sources(fixture: &Value) -> BTreeMap<String, String> {
    serde_json::from_value(fixture["files"].clone()).unwrap()
}

pub fn cold(files: &BTreeMap<String, String>) -> (SemanticGraph, GraphBuilder) {
    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();
    builder.load_files(
        &mut graph,
        files.iter().map(|(p, s)| (p.as_str(), s.as_str())),
    );
    (graph, builder)
}

pub fn changes(step: &Value, files: &mut BTreeMap<String, String>) -> Vec<FileChange> {
    step["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|change| {
            let path = change["path"].as_str().unwrap();
            let normalized = aether_builder::sync::normalize_source_path(path).unwrap();
            let source = change["source"].as_str().map(str::to_owned);
            if let Some(source) = &source {
                files.insert(normalized, source.clone());
            } else {
                files.remove(&normalized);
            }
            FileChange {
                path: path.into(),
                source,
            }
        })
        .collect()
}

pub fn reasons(step: &Value) -> Vec<FullRebuildReason> {
    step["structural_reasons"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|reason| match reason.as_str().unwrap() {
            "configuration_changed" => FullRebuildReason::ConfigurationChanged,
            "ownership_changed" => FullRebuildReason::OwnershipChanged,
            "uncertain_event_mapping" => FullRebuildReason::UncertainEventMapping,
            "cache_inconsistent" => FullRebuildReason::CacheInconsistent,
            "dirty_set_exceeds_half" => FullRebuildReason::DirtySetExceedsHalf,
            other => panic!("unknown frozen reason: {other}"),
        })
        .collect()
}

pub fn canonical(graph: &SemanticGraph) -> Value {
    let mut nodes: Vec<_> = graph.nodes().cloned().collect();
    nodes.sort_by_key(|node| node.id);
    let mut edges = graph.edge_records();
    edges.sort_by_key(|(from, to, edge)| (*from, *to, edge.kind));
    json!({"nodes": nodes, "edges": edges})
}

pub fn assert_edges(graph: &SemanticGraph, checks: &Value) {
    for check in checks.as_array().into_iter().flatten() {
        let expected = (
            NodeId::from_path(check["from"].as_str().unwrap()),
            NodeId::from_path(check["to"].as_str().unwrap()),
            serde_json::from_value(check["kind"].clone()).unwrap(),
        );
        assert_eq!(
            graph.edges().contains(&expected),
            check["present"].as_bool().unwrap(),
            "{check}"
        );
    }
}

pub fn report_json(report: &UpdateReport) -> Value {
    json!({
        "dirty_files":report.dirty_files,"parsed_files":report.parsed_files,
        "reused_files":report.reused_files,"invalidation_scope":report.invalidation_scope,
        "invalidated_source_nodes":report.invalidated_source_nodes,
        "invalidated_incident_edges":report.invalidated_incident_edges,
        "full_rebuild_reasons":report.full_rebuild_reasons.iter().map(|r|r.as_str()).collect::<Vec<_>>(),
        "collect_seconds":report.collect_seconds,"parse_seconds":report.parse_seconds,
        "reconstruct_seconds":report.reconstruct_seconds,"resolve_seconds":report.resolve_seconds,
        "reconcile_seconds":report.reconcile_seconds,"elapsed_seconds":report.elapsed_seconds,
        "full_rebuild_seconds":report.full_rebuild_seconds,
    })
}
