use crate::project::source::build_from_dir;
use std::path::PathBuf;

pub fn query(args: &[String]) -> std::io::Result<()> {
    use aether_graph::parse_query;

    let root = PathBuf::from(args.first().map(String::as_str).unwrap_or("."));
    let question_words: Vec<&str> = args
        .get(1..)
        .unwrap_or(&[])
        .iter()
        .map(String::as_str)
        .collect();

    println!("Loading {} ...", root.display());
    let (graph, _builder, files) = build_from_dir(&root)?;
    println!(
        "  {} file(s), {} nodes, {} edges\n",
        files,
        graph.node_count(),
        graph.edge_count()
    );

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
        let q = parse_query(&question);
        let result = graph.answer_query(&q);
        println!("{}", result.display());
    }

    Ok(())
}
