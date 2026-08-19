use aether_graph::{NodeKind, SemanticGraph};

/// The two-line graph summary shared by `analyze`, `inspect`, and the swarm
/// commands. Returns the text rather than printing it, so a caller routing
/// through `--out`'s `Sink` (as `analyze` does) can capture it instead of it
/// leaking straight to stdout; plain-`println!` callers just print what
/// this returns, unchanged from before.
pub(crate) fn format_summary(graph: &SemanticGraph) -> String {
    let funcs = graph.query_by_kind(NodeKind::Function).len();
    let types = graph.query_by_kind(NodeKind::Type).len();
    let modules = graph.query_by_kind(NodeKind::Module).len();
    format!(
        "  graph: {} nodes, {} edges\n  {modules} modules · {funcs} functions · {types} types",
        graph.node_count(),
        graph.edge_count()
    )
}

pub(crate) fn print_summary(graph: &SemanticGraph) {
    println!("{}", format_summary(graph));
}
