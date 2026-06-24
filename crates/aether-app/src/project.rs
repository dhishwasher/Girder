//! Real project loading, the CLI subcommands, and `.aether` persistence.
//!
//! Turns AetherForge from a fixed demo into a usable tool: point it at a
//! directory and it builds the semantic graph from every `.rs`/`.py` file,
//! resolves calls project-wide, runs the agent swarm on an intent, and persists
//! the graph as an `.aether` file (the graph *is* the project).

use aether_agents::{MsgKind, Orchestrator, SwarmContext};
use aether_builder::GraphBuilder;
use aether_graph::{NodeKind, SemanticGraph};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Directories we never descend into.
const SKIP_DIRS: &[&str] = &["target", ".git", "node_modules", ".aether-cache"];

/// Recursively collect source files we can parse, returning paths *relative* to
/// `root` (so module names reflect directory structure).
fn collect_sources(root: &Path) -> std::io::Result<Vec<(PathBuf, String)>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push(path);
                }
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("rs") | Some("py")
            ) {
                if let Ok(rel) = path.strip_prefix(root) {
                    out.push((path.clone(), rel.to_string_lossy().replace('\\', "/")));
                }
            }
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(out)
}

/// Build a graph from a directory of source files. Returns the graph and the
/// number of files loaded.
fn build_from_dir(root: &Path) -> std::io::Result<(SemanticGraph, GraphBuilder, usize)> {
    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();
    let sources = collect_sources(root)?;
    for (abs, rel) in &sources {
        match std::fs::read_to_string(abs) {
            Ok(text) => builder.load_file(&mut graph, rel, &text),
            Err(e) => eprintln!("  ! skipped {rel}: {e}"),
        }
    }
    Ok((graph, builder, sources.len()))
}

fn default_aether_path(root: &Path) -> PathBuf {
    root.join("project.aether")
}

fn print_summary(graph: &SemanticGraph) {
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

/// `aetherforge analyze <dir>` — build the graph, derive similarity, print a
/// summary + likely duplicates, save `.aether`.
pub fn analyze(args: &[String]) -> std::io::Result<()> {
    let root = PathBuf::from(args.first().map(String::as_str).unwrap_or("."));
    println!("Analyzing {} ...", root.display());
    let (mut graph, _builder, files) = build_from_dir(&root)?;
    println!("  loaded {files} source file(s)");
    print_summary(&graph);

    // Derive semantic-similarity edges and report likely duplicate functions.
    let linked = graph.compute_similarity_edges(0.6);
    if linked > 0 {
        println!("  similarity: {linked} likely-duplicate function pair(s):");
        for (a, b, kind) in graph.edges() {
            if kind == aether_graph::EdgeKind::SemanticSimilar {
                if let (Some(na), Some(nb)) = (graph.get(a), graph.get(b)) {
                    println!("    {}  ~  {}", na.path, nb.path);
                }
            }
        }
    }

    let out = default_aether_path(&root);
    match graph.save(&out) {
        Ok(()) => println!("  saved semantic graph -> {}", out.display()),
        Err(e) => eprintln!("  ! could not save {}: {e}", out.display()),
    }
    Ok(())
}

/// `aetherforge search <dir> <query...>` — concept search over the codebase.
pub fn search(args: &[String]) -> std::io::Result<()> {
    let root = PathBuf::from(args.first().map(String::as_str).unwrap_or("."));
    let query = args.get(1..).map(|rest| rest.join(" ")).unwrap_or_default();
    if query.trim().is_empty() {
        eprintln!("usage: aetherforge search <dir> <query...>");
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

/// `aetherforge forge <dir> <intent...>` — load the project, dispatch the agent
/// swarm on a natural-language intent, persist the updated graph.
pub async fn forge(args: &[String]) -> std::io::Result<()> {
    let root = PathBuf::from(args.first().map(String::as_str).unwrap_or("."));
    let intent = args.get(1..).map(|rest| rest.join(" ")).unwrap_or_default();
    if intent.trim().is_empty() {
        eprintln!("usage: aetherforge forge <dir> <intent...>");
        return Ok(());
    }

    println!("Loading {} ...", root.display());
    let (graph, _builder, files) = build_from_dir(&root)?;
    println!("  loaded {files} source file(s)");
    print_summary(&graph);

    println!("\nForging: \"{intent}\"");
    let graph = Arc::new(Mutex::new(graph));
    // Agent-authored code lands in a dedicated module/projection.
    let ctx = Arc::new(SwarmContext::new(
        aether_ai::default_router(),
        graph.clone(),
        "crate::forge",
        "src/forge.rs",
    ));
    let transcript = Orchestrator::new(ctx)
        .with_default_swarm()
        .run(&intent, Duration::from_secs(10))
        .await;

    for m in &transcript {
        match &m.kind {
            MsgKind::CodeReady { name, .. } => println!("  [{}] wrote fn {name}", m.from.label()),
            MsgKind::TestsReady { for_fn, .. } => {
                println!("  [{}] tested {for_fn}", m.from.label())
            }
            MsgKind::Note { text } => println!("  [{}] {text}", m.from.label()),
            _ => {}
        }
    }

    let graph = graph.lock().unwrap();
    print_summary(&graph);
    let out = default_aether_path(&root);
    match graph.save(&out) {
        Ok(()) => println!("  saved updated graph -> {}", out.display()),
        Err(e) => eprintln!("  ! could not save {}: {e}", out.display()),
    }
    Ok(())
}

/// `aetherforge inspect <file.aether> [path]` — load a saved graph and print it,
/// optionally focusing on one node's neighbors + impact.
pub fn inspect(args: &[String]) -> std::io::Result<()> {
    let Some(file) = args.first() else {
        eprintln!("usage: aetherforge inspect <file.aether> [node::path]");
        return Ok(());
    };
    let graph = match SemanticGraph::load(file) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("could not load {file}: {e}");
            return Ok(());
        }
    };
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
            None => println!("  no node with path '{path}'"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_a_directory_and_resolves_across_files() {
        // Build a throwaway project on disk with two files that call across.
        let dir = std::env::temp_dir().join(format!("aetherforge-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/util")).unwrap();
        std::fs::write(
            dir.join("src/util/math.rs"),
            "fn double(x: i64) -> i64 { x + x }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("src/app.rs"),
            "fn run() -> i64 { double(21) }\n",
        )
        .unwrap();

        let (graph, _builder, files) = build_from_dir(&dir).unwrap();
        assert_eq!(files, 2);
        // Directory-aware module naming.
        assert!(graph.find_by_path("crate::util::math::double").is_some());
        assert!(graph.find_by_path("crate::app::run").is_some());
        // Cross-file (and cross-directory) call resolved.
        let run = aether_graph::NodeId::from_path("crate::app::run");
        let double = aether_graph::NodeId::from_path("crate::util::math::double");
        let calls: Vec<_> = graph
            .neighbors(run, Some(aether_graph::EdgeKind::Calls))
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert!(calls.contains(&double));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
