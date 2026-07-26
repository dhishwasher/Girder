use crate::project::config::ProjectConfig;
use crate::project::projection::project_rename;
use crate::project::source::{build_from_dir, build_from_dir_with_config, save_graph};
use crate::project::summary::print_summary;
use aether_graph::SemanticGraph;
use std::path::PathBuf;

pub fn analyze(args: &[String]) -> std::io::Result<()> {
    let root = PathBuf::from(args.first().map(String::as_str).unwrap_or("."));
    println!("Analyzing {} ...", root.display());
    let config = ProjectConfig::load(&root)?;
    let (mut graph, _builder, files) = build_from_dir_with_config(&root, &config)?;
    println!("  loaded {files} source file(s)");
    print_summary(&graph);

    // Report inheritance relationships (Python class bases, Rust trait impls).
    let inherits: Vec<_> = graph
        .edges()
        .into_iter()
        .filter(|(_, _, k)| *k == aether_graph::EdgeKind::Inherits)
        .collect();
    if !inherits.is_empty() {
        let mut inherits = inherits;
        inherits.sort_by_key(|(a, b, _)| {
            let left = graph.get(*a).map(|n| n.path.clone()).unwrap_or_default();
            let right = graph.get(*b).map(|n| n.path.clone()).unwrap_or_default();
            (left, right)
        });
        println!("  inheritance: {} relationship(s):", inherits.len());
        for (a, b, _) in &inherits {
            if let (Some(na), Some(nb)) = (graph.get(*a), graph.get(*b)) {
                println!("    {}  ⊳  {}", na.path, nb.path);
            }
        }
    }

    // Derive semantic-similarity edges and report likely duplicate functions.
    // SemanticSimilar edges are stored both ways; print each unordered pair once.
    let linked = graph.compute_similarity_edges(0.6);
    if linked > 0 {
        println!("  similarity: {linked} likely-duplicate function pair(s):");
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
                println!("    {}  ~  {}", na.path, nb.path);
            }
        }
    }

    let out = save_graph(&root, &config, &graph)?;
    println!("  saved semantic graph -> {}", out.display());
    Ok(())
}

/// `bitcode search <dir> <query...>` — concept search over the codebase.
pub fn search(args: &[String]) -> std::io::Result<()> {
    let root = PathBuf::from(args.first().map(String::as_str).unwrap_or("."));
    let query = args.get(1..).map(|rest| rest.join(" ")).unwrap_or_default();
    if query.trim().is_empty() {
        eprintln!("usage: bitcode search <dir> <query...>");
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

/// `bitcode plan <dir> <intent...>`
///
/// Runs only the graph-aware Planner against the project, printing the full
/// feature specification (what functions would be built, why, and what context
/// the Planner used) — without running the Coder or touching the graph.
pub fn inspect(args: &[String]) -> std::io::Result<()> {
    let Some(file) = args.first() else {
        eprintln!("usage: bitcode inspect <file.aether> [node::path]");
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

/// `bitcode refactor <dir> rename <node::path> <new_name>` — semantic rename
/// across the graph (follows `Calls` edges, not text search), then persist.
pub fn refactor(args: &[String]) -> std::io::Result<()> {
    let root = PathBuf::from(args.first().map(String::as_str).unwrap_or("."));
    let op = args.get(1).map(String::as_str);
    let target = args.get(2);
    let new_name = args.get(3);
    let (Some("rename"), Some(target), Some(new_name)) = (op, target, new_name) else {
        eprintln!("usage: bitcode refactor <dir> rename <node::path> <new_name>");
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
