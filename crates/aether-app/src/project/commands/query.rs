use crate::project::source::build_from_dir;
use std::path::{Path, PathBuf};

pub fn query(args: &[String]) -> std::io::Result<()> {
    use aether_graph::parse_query;

    let root = PathBuf::from(args.first().map(String::as_str).unwrap_or("."));
    let question_words: Vec<&str> = args
        .get(1..)
        .unwrap_or(&[])
        .iter()
        .map(String::as_str)
        .collect();

    let (graph, _builder, files) = build_from_dir(&root)?;
    let mut sink = crate::project::output_sink::Sink::Stdout;
    query_header(&root, &graph, files, &mut sink);

    if question_words.is_empty() {
        // Interactive REPL.
        println!("Girder Knowledge Query REPL");
        println!("  Type a question about your codebase, or 'exit' to quit.\n");
        use std::io::BufRead;
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let line = line?;
            let input = line.trim();
            if input.is_empty() {
                continue;
            }
            if input == "exit" || input == "quit" {
                break;
            }
            let q = parse_query(input);
            let result = graph.answer_query(&q);
            println!("{}", result.display());
        }
    } else {
        // Single-shot mode.
        let question = question_words.join(" ");
        let rendered = query_answer(&graph, &question);
        println!("{rendered}");
    }

    Ok(())
}

pub(super) fn query_header(
    root: &Path,
    graph: &aether_graph::SemanticGraph,
    files: usize,
    sink: &mut crate::project::output_sink::Sink,
) {
    use crate::project::output_sink::out;
    out!(sink, "Loading {} ...", root.display());
    out!(
        sink,
        "  {} file(s), {} nodes, {} edges\n",
        files,
        graph.node_count(),
        graph.edge_count()
    );
}

pub(super) fn query_answer(graph: &aether_graph::SemanticGraph, question: &str) -> String {
    graph
        .answer_query(&aether_graph::parse_query(question))
        .display()
}
