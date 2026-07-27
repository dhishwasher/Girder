//! Reconcile a durable semantic graph with fresh source projections.

use crate::{is_parser_owned_attribute, Node, NodeId, NodeKind, SemanticGraph};
use std::collections::HashSet;

/// What survived while a persisted graph was reconciled with files on disk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    pub metadata_nodes: usize,
    pub graph_owned_nodes: usize,
    pub graph_owned_edges: usize,
    pub source_changes: usize,
}

impl SemanticGraph {
    /// Fold fresh source projections into a previously persisted graph.
    ///
    /// Source-derived structure and source text come from `source`; the durable
    /// graph contributes agent metadata, graph-native nodes, and inferred edges.
    /// Stale parser-owned nodes are intentionally dropped.
    pub fn reconcile_persisted(
        source: SemanticGraph,
        persisted: &SemanticGraph,
    ) -> (SemanticGraph, ReconcileReport) {
        let source_ids: HashSet<NodeId> = source.nodes().map(|node| node.id).collect();
        let mut report = ReconcileReport::default();
        let mut reconciled = SemanticGraph::new();

        for mut node in source.nodes().cloned() {
            if let Some(previous) = persisted.get(node.id) {
                if previous.source != node.source {
                    report.source_changes += 1;
                }
                let before = node.attributes.len();
                merge_graph_metadata(&mut node, previous);
                if node.attributes.len() > before {
                    report.metadata_nodes += 1;
                }
            }
            reconciled.upsert_node(node);
        }

        for node in persisted.nodes() {
            if !source_ids.contains(&node.id) && is_graph_owned(node) {
                reconciled.upsert_node(node.clone());
                report.graph_owned_nodes += 1;
            }
        }

        for (from, to, edge) in source.edge_records() {
            let _ = reconciled.add_edge(from, to, edge);
        }

        for (from, to, edge) in persisted.edge_records() {
            if !reconciled.contains(from) || !reconciled.contains(to) {
                continue;
            }
            let both_source_owned = source_ids.contains(&from) && source_ids.contains(&to);
            if edge.kind.is_projection_derived() && both_source_owned {
                continue;
            }
            let _ = reconciled.add_edge(from, to, edge);
            report.graph_owned_edges += 1;
        }

        (reconciled, report)
    }
}

fn is_graph_owned(node: &Node) -> bool {
    node.attr("authored_by").is_some()
        || node.file.is_none()
        || matches!(
            node.kind,
            NodeKind::Concept
                | NodeKind::Dependency
                | NodeKind::Extension
                | NodeKind::ExtensionContribution
        )
}

fn merge_graph_metadata(projected: &mut Node, persisted: &Node) {
    for (key, value) in &persisted.attributes {
        if !is_parser_owned_attribute(key)
            && !projected
                .attributes
                .iter()
                .any(|(current, _)| current == key)
        {
            projected.attributes.push((key.clone(), value.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, EdgeKind, Span};

    fn projected(path: &str, source: &str) -> Node {
        let mut node = Node::new(NodeKind::Function, path.rsplit("::").next().unwrap(), path)
            .with_language("rust")
            .with_source(source);
        node.file = Some("src/lib.rs".into());
        node.span = Span {
            start_byte: 0,
            end_byte: source.len(),
            start_row: 0,
            start_col: 0,
        };
        node
    }

    #[test]
    fn source_wins_while_agent_metadata_survives() {
        let path = "crate::lib::run";
        let mut persisted = SemanticGraph::new();
        let mut old = projected(path, "fn run() { old() }");
        old.set_attr("summary", "important entry point");
        old.set_attr("is_test", "true");
        old.set_attr("return_type", "OldResult");
        persisted.upsert_node(old);

        let mut source = SemanticGraph::new();
        source.upsert_node(projected(path, "fn run() { fresh() }"));

        let (reconciled, report) = SemanticGraph::reconcile_persisted(source, &persisted);
        let run = reconciled.find_by_path(path).unwrap();

        assert_eq!(run.source, "fn run() { fresh() }");
        assert_eq!(run.attr("summary"), Some("important entry point"));
        assert_eq!(run.attr("is_test"), None);
        assert_eq!(run.attr("return_type"), None);
        assert_eq!(report.metadata_nodes, 1);
        assert_eq!(report.source_changes, 1);
    }

    #[test]
    fn graph_owned_nodes_and_edges_survive() {
        let mut persisted = SemanticGraph::new();
        let module = persisted.upsert_node(Node::new(NodeKind::Module, "forge", "crate::forge"));
        let mut generated = projected("crate::forge::generated", "fn generated() {}");
        generated.file = Some("src/forge.rs".into());
        generated.set_attr("authored_by", "Coder");
        let generated = persisted.upsert_node(generated);
        persisted
            .add_edge(module, generated, Edge::new(EdgeKind::Contains))
            .unwrap();

        let (reconciled, report) =
            SemanticGraph::reconcile_persisted(SemanticGraph::new(), &persisted);

        assert!(reconciled.contains(module));
        assert!(reconciled.contains(generated));
        assert_eq!(report.graph_owned_nodes, 2);
        assert_eq!(report.graph_owned_edges, 1);
    }

    #[test]
    fn stale_source_nodes_are_dropped() {
        let mut persisted = SemanticGraph::new();
        persisted.upsert_node(projected("crate::lib::removed", "fn removed() {}"));

        let (reconciled, report) =
            SemanticGraph::reconcile_persisted(SemanticGraph::new(), &persisted);

        assert!(reconciled.find_by_path("crate::lib::removed").is_none());
        assert_eq!(report.graph_owned_nodes, 0);
    }
}
