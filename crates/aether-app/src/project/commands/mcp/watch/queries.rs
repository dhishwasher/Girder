//! Shared read-only command handlers. Only Git baseline extraction may read
//! historical sources; the working-tree graph always comes from the generation.
use super::Generation;
use crate::project::commands::{context_cmd, graph, names, orient, query, review, test_impact};
use crate::project::output_sink::{out, Sink};
use serde_json::Value;
use std::io;
use std::path::Path;

pub(super) fn execute(root: &Path, generation: &Generation, argv: &[String]) -> io::Result<String> {
    let source = &generation.graph;
    let args = &argv[1..];
    let json_text = |value: Value| -> io::Result<String> {
        serde_json::to_string_pretty(&value)
            .map(|s| s + "\n")
            .map_err(io::Error::other)
    };
    let mut sink = Sink::Buffer(String::new());
    match argv[0].as_str() {
        "context" => return json_text(context_cmd::source_from_graph(source, args)?),
        "names" => {
            return json_text(
                serde_json::to_value(names::names_from_graph(source, &args[1], args)?)
                    .map_err(io::Error::other)?,
            )
        }
        "orient" => return json_text(orient::output_from_graph(source, args)?),
        "search" => graph::search_into(root, source, &args[1], &mut sink),
        "query" => {
            query::query_header(root, source, generation.files, &mut sink);
            out!(sink, "{}", query::query_answer(source, &args[1]));
        }
        "test-impact" => {
            let explicit: Vec<_> = args[1..]
                .iter()
                .filter(|a| *a != "--quiet")
                .map(String::as_str)
                .collect();
            return test_impact::quiet_from_graph(root, source, &generation.config, &explicit);
        }
        "review" => {
            let since = args
                .windows(2)
                .find(|w| w[0] == "--since")
                .map(|w| w[1].as_str())
                .unwrap_or("HEAD");
            review::review_into(
                root,
                source,
                generation.files,
                &generation.config,
                since,
                args.iter().any(|a| a == "--quiet"),
                &mut sink,
            )?;
        }
        _ => return Err(io::Error::other("unsupported cached graph command")),
    }
    match sink {
        Sink::Buffer(text) => Ok(text),
        Sink::Stdout => unreachable!(),
    }
}
