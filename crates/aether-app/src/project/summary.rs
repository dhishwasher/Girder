use aether_graph::{NodeKind, SemanticGraph};

pub(crate) fn print_summary(graph: &SemanticGraph) {
    println!(
        "  graph: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );
    let funcs = graph.query_by_kind(NodeKind::Function).len();
    let types = graph.query_by_kind(NodeKind::Type).len();
    let modules = graph.query_by_kind(NodeKind::Module).len();
    println!("  {modules} modules · {funcs} functions · {types} types");
}
