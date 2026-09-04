use crate::project::config::ProjectConfig;
use crate::project::output_sink::{out, Sink};
use crate::project::projection::project_rename;
use crate::project::source::{build_from_dir, build_from_dir_with_config, save_graph};
use crate::project::summary::{format_summary, print_summary};
use aether_graph::SemanticGraph;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::ErrorKind;
use std::path::PathBuf;
use std::time::Instant;

const ANALYZE_USAGE: &str = "usage: girder analyze <dir> [--json] [--out <path>]";

pub fn analyze(args: &[String]) -> std::io::Result<()> {
    let Some(root) = args.first() else {
        return Err(invalid_input(ANALYZE_USAGE));
    };
    let rest = args.get(1..).unwrap_or_default();
    let mut json = false;
    let mut out_path: Option<PathBuf> = None;
    let mut index = 0;
    while index < rest.len() {
        match rest[index].as_str() {
            "--json" => {
                json = true;
                index += 1;
            }
            "--out" => {
                let Some(value) = rest.get(index + 1) else {
                    return Err(invalid_input(ANALYZE_USAGE));
                };
                out_path = Some(PathBuf::from(value));
                index += 2;
            }
            _ => return Err(invalid_input(ANALYZE_USAGE)),
        }
    }
    let root = PathBuf::from(root);
    let mut sink = if out_path.is_some() {
        Sink::Buffer(String::new())
    } else {
        Sink::Stdout
    };
    if !json {
        out!(sink, "Analyzing {} ...", root.display());
    }
    let config = ProjectConfig::load(&root)?;
    let build_started = Instant::now();
    let (mut graph, _builder, source_files) = build_from_dir_with_config(&root, &config)?;
    let build_ms = elapsed_millis(build_started);
    if !json {
        out!(sink, "  loaded {source_files} source file(s)");
        out!(sink, "{}", format_summary(&graph));
    }

    // Report inheritance relationships (Python class bases, Rust trait impls).
    let inherits: Vec<_> = graph
        .edges()
        .into_iter()
        .filter(|(_, _, k)| *k == aether_graph::EdgeKind::Inherits)
        .collect();
    if !json && !inherits.is_empty() {
        let mut inherits = inherits;
        inherits.sort_by_key(|(a, b, _)| {
            let left = graph.get(*a).map(|n| n.path.clone()).unwrap_or_default();
            let right = graph.get(*b).map(|n| n.path.clone()).unwrap_or_default();
            (left, right)
        });
        out!(sink, "  inheritance: {} relationship(s):", inherits.len());
        for (a, b, _) in &inherits {
            if let (Some(na), Some(nb)) = (graph.get(*a), graph.get(*b)) {
                out!(sink, "    {}  ⊳  {}", na.path, nb.path);
            }
        }
    }

    // Derive semantic-similarity edges and report likely duplicate functions.
    // SemanticSimilar edges are stored both ways; print each unordered pair once.
    let similarity_started = Instant::now();
    let linked = graph.compute_similarity_edges(0.6);
    let similarity_ms = elapsed_millis(similarity_started);
    if !json && linked > 0 {
        out!(
            sink,
            "  similarity: {linked} likely-duplicate function pair(s):"
        );
        let mut similar: Vec<_> = graph
            .edges()
            .into_iter()
            .filter(|(a, b, kind)| *kind == aether_graph::EdgeKind::SemanticSimilar && *a < *b)
            .collect();
        similar.sort_by_key(|(a, b, _)| {
            let left = graph.get(*a).map(|n| n.path.clone()).unwrap_or_default();
            let right = graph.get(*b).map(|n| n.path.clone()).unwrap_or_default();
            (left, right)
        });
        for (a, b, _) in similar {
            if let (Some(na), Some(nb)) = (graph.get(a), graph.get(b)) {
                out!(sink, "    {}  ~  {}", na.path, nb.path);
            }
        }
    }

    let save_started = Instant::now();
    let graph_path = save_graph(&root, &config, &graph)?;
    let save_ms = elapsed_millis(save_started);
    if json {
        let rendered = serde_json::to_string(&AnalyzeJson {
            schema_version: 1,
            source_files,
            nodes: graph.node_count(),
            edges: graph.edge_count(),
            similarity_pairs: linked,
            build_ms,
            similarity_ms,
            save_ms,
            graph_path: graph_path.to_string_lossy().into_owned(),
        })
        .map_err(|error| std::io::Error::other(format!("could not render JSON: {error}")))?;
        out!(sink, "{rendered}");
    } else {
        out!(sink, "  saved semantic graph -> {}", graph_path.display());
    }

    if let Sink::Buffer(_) = sink {
        println!(
            "analyze: {} node(s), {} edge(s), {linked} duplicate pair(s); graph saved -> {}; report -> {}",
            graph.node_count(),
            graph.edge_count(),
            graph_path.display(),
            out_path.as_ref().unwrap().display()
        );
    }
    sink.finish(out_path.as_deref())?;
    Ok(())
}

#[derive(Serialize)]
struct AnalyzeJson {
    schema_version: u32,
    source_files: usize,
    nodes: usize,
    edges: usize,
    similarity_pairs: usize,
    build_ms: u64,
    similarity_ms: u64,
    save_ms: u64,
    graph_path: String,
}

fn elapsed_millis(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

/// `girder search <dir> <query...>` — concept search over the codebase.
pub fn search(args: &[String]) -> std::io::Result<()> {
    let root = PathBuf::from(args.first().map(String::as_str).unwrap_or("."));
    let query = args.get(1..).map(|rest| rest.join(" ")).unwrap_or_default();
    if query.trim().is_empty() {
        eprintln!("usage: girder search <dir> <query...>");
        return Ok(());
    }
    let (graph, _builder, _files) = build_from_dir(&root)?;
    println!("Searching {} for \"{query}\" ...", root.display());
    let hits = graph.semantic_search(&query, 10);
    if hits.is_empty() {
        println!("  no matches");
    }
    for (id, score) in hits {
        if let Some(n) = graph.get(id) {
            println!("  {:.2}  {}", score, n.path);
        }
    }
    Ok(())
}

/// `girder plan <dir> <intent...>`
///
/// Runs only the graph-aware Planner against the project, printing the full
/// feature specification (what functions would be built, why, and what context
/// the Planner used) — without running the Coder or touching the graph.
pub fn inspect(args: &[String]) -> std::io::Result<()> {
    let Some(file) = args.first() else {
        return Err(invalid_input(
            "usage: girder inspect <file.aether> [node::path|--json]",
        ));
    };
    if args.len() > 2 {
        return Err(invalid_input(
            "usage: girder inspect <file.aether> [node::path|--json]",
        ));
    }
    let graph = SemanticGraph::load(file)
        .map_err(|error| std::io::Error::other(format!("could not load {file}: {error}")))?;
    if args.get(1).is_some_and(|argument| argument == "--json") {
        return print_graph_json(&graph);
    }
    println!("Loaded {file}");
    print_summary(&graph);

    if let Some(path) = args.get(1) {
        match graph.find_by_path(path) {
            Some(node) => {
                println!("\n{} ({:?})", node.path, node.kind);
                let impact = graph.impact_of(node.id);
                println!("  impacts {} node(s) if changed:", impact.affected.len());
                for (id, dist) in impact.ranked() {
                    if let Some(n) = graph.get(id) {
                        println!("    {} (distance {dist})", n.path);
                    }
                }
            }
            None => {
                return Err(std::io::Error::new(
                    ErrorKind::NotFound,
                    format!("no node with path '{path}'"),
                ));
            }
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct GraphExport {
    schema_version: u32,
    nodes: Vec<GraphExportNode>,
    edges: Vec<GraphExportEdge>,
}

#[derive(Serialize)]
struct GraphExportNode {
    id: String,
    path: String,
    kind: String,
    name: String,
    language: String,
    file: Option<String>,
    span: GraphExportSpan,
    source_sha256: String,
    attributes: Vec<(String, String)>,
}

#[derive(Serialize)]
struct GraphExportSpan {
    start_byte: usize,
    end_byte: usize,
    start_row: usize,
    start_col: usize,
}

#[derive(Serialize)]
struct GraphExportEdge {
    source: String,
    target: String,
    kind: String,
    weight_bits: u32,
}

fn print_graph_json(graph: &SemanticGraph) -> std::io::Result<()> {
    let mut nodes = graph
        .nodes()
        .map(|node| {
            let mut attributes = node.attributes.clone();
            attributes.sort();
            GraphExportNode {
                id: format!("{:016x}", node.id.0),
                path: node.path.clone(),
                kind: format!("{:?}", node.kind),
                name: node.name.clone(),
                language: node.language.clone(),
                file: node.file.clone(),
                span: GraphExportSpan {
                    start_byte: node.span.start_byte,
                    end_byte: node.span.end_byte,
                    start_row: node.span.start_row,
                    start_col: node.span.start_col,
                },
                source_sha256: hex_digest(&Sha256::digest(node.source.as_bytes())),
                attributes,
            }
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| (&left.path, &left.id).cmp(&(&right.path, &right.id)));

    let mut edges = graph
        .edge_records()
        .into_iter()
        .map(|(source, target, edge)| GraphExportEdge {
            source: graph
                .get(source)
                .expect("graph edge source must exist")
                .path
                .clone(),
            target: graph
                .get(target)
                .expect("graph edge target must exist")
                .path
                .clone(),
            kind: format!("{:?}", edge.kind),
            weight_bits: edge.weight.to_bits(),
        })
        .collect::<Vec<_>>();
    edges.sort_by(|left, right| {
        (&left.source, &left.target, &left.kind, left.weight_bits).cmp(&(
            &right.source,
            &right.target,
            &right.kind,
            right.weight_bits,
        ))
    });

    print_json(&GraphExport {
        schema_version: 1,
        nodes,
        edges,
    })
}

fn print_json(value: &impl Serialize) -> std::io::Result<()> {
    let rendered = serde_json::to_string(value)
        .map_err(|error| std::io::Error::other(format!("could not render JSON: {error}")))?;
    println!("{rendered}");
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("writing to String cannot fail");
    }
    out
}

fn invalid_input(message: &str) -> std::io::Error {
    std::io::Error::new(ErrorKind::InvalidInput, message)
}

/// `girder refactor <dir> rename <node::path> <new_name>` — semantic rename
/// across the graph (follows `Calls` edges, not text search), then persist.
pub fn refactor(args: &[String]) -> std::io::Result<()> {
    let root = PathBuf::from(args.first().map(String::as_str).unwrap_or("."));
    let op = args.get(1).map(String::as_str);
    let target = args.get(2);
    let new_name = args.get(3);
    let (Some("rename"), Some(target), Some(new_name)) = (op, target, new_name) else {
        eprintln!("usage: girder refactor <dir> rename <node::path> <new_name>");
        return Ok(());
    };

    let (mut graph, _builder, _files) = build_from_dir(&root)?;
    let Some(node) = graph.find_by_path(target) else {
        eprintln!("no node with path '{target}'");
        return Ok(());
    };
    let id = node.id;
    let before = graph.clone();
    match graph.rename_node(id, new_name) {
        Ok(outcome) => {
            println!("Renamed {} -> {}", outcome.old_path, outcome.new_path);
            if outcome.updated_callers.is_empty() {
                println!("  no callers needed updating");
            } else {
                println!("  rewrote {} call site(s):", outcome.updated_callers.len());
                for caller in &outcome.updated_callers {
                    println!("    {caller}");
                }
            }
            let config = ProjectConfig::load(&root)?;
            let projected = project_rename(&root, &config, &before, &graph, &outcome)?;
            if !projected.is_empty() {
                println!("  projected source file(s):");
                for file in projected {
                    println!("    {}", root.join(file).display());
                }
            }
            println!(
                "  committed source and graph -> {}",
                root.join(&config.graph.path).display()
            );
        }
        Err(e) => eprintln!("rename failed: {e}"),
    }
    Ok(())
}
