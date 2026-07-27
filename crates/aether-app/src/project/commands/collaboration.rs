use crate::project::source::build_from_dir;
use aether_graph::{ActorId, GraphError, GraphReplica};
use std::path::{Path, PathBuf};

const USAGE: &str = "\
usage:
  bitcode collab init <dir> <actor> <bundle>
  bitcode collab status <bundle>
  bitcode collab fork <bundle> <actor> <out>
  bitcode collab sync <dir> <bundle> [out]
  bitcode collab merge <bundle> <peer> <out>
  bitcode collab materialize <bundle> <graph.aether>";

pub fn collaboration(args: &[String]) -> std::io::Result<()> {
    match args.first().map(String::as_str) {
        Some("init") => init(&args[1..]),
        Some("status") => status(&args[1..]),
        Some("fork") => fork(&args[1..]),
        Some("sync") => sync(&args[1..]),
        Some("merge") => merge(&args[1..]),
        Some("materialize") => materialize(&args[1..]),
        _ => {
            eprintln!("{USAGE}");
            Ok(())
        }
    }
}

fn init(args: &[String]) -> std::io::Result<()> {
    let [root, actor, bundle] = args else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let root = PathBuf::from(root);
    let (graph, _builder, files) = build_from_dir(&root)?;
    let replica = GraphReplica::from_graph(collaboration_result(ActorId::new(actor))?, &graph);
    collaboration_result(replica.save(bundle))?;
    println!(
        "Initialized {} for actor {} from {files} source file(s): {} nodes, {} edges, {} operations",
        bundle,
        replica.actor(),
        graph.node_count(),
        graph.edge_count(),
        replica.operation_count()
    );
    Ok(())
}

fn status(args: &[String]) -> std::io::Result<()> {
    let [bundle] = args else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let replica = collaboration_result(GraphReplica::load(bundle))?;
    let graph = collaboration_result(replica.materialize())?;
    let versions = replica
        .version()
        .actors()
        .map(|(actor, counter)| format!("{actor}:{counter}"))
        .collect::<Vec<_>>()
        .join(", ");
    println!("Collaboration bundle: {bundle}");
    println!("  actor: {}", replica.actor());
    println!("  operations: {}", replica.operation_count());
    println!("  version: {versions}");
    println!("  nodes: {}", graph.node_count());
    println!("  edges: {}", graph.edge_count());
    Ok(())
}

fn fork(args: &[String]) -> std::io::Result<()> {
    let [bundle, actor, out] = args else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let replica = collaboration_result(GraphReplica::load(bundle))?;
    let forked = collaboration_result(replica.fork(collaboration_result(ActorId::new(actor))?))?;
    collaboration_result(forked.save(out))?;
    println!(
        "Forked {bundle} -> {out} for actor {} ({} operations)",
        forked.actor(),
        forked.operation_count()
    );
    Ok(())
}

fn sync(args: &[String]) -> std::io::Result<()> {
    let ([root, bundle] | [root, bundle, _]) = args else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let out = args.get(2).map(String::as_str).unwrap_or(bundle);
    let (graph, _builder, files) = build_from_dir(Path::new(root))?;
    let mut replica = collaboration_result(GraphReplica::load(bundle))?;
    let report = collaboration_result(replica.sync_graph(&graph))?;
    collaboration_result(replica.save(out))?;
    println!(
        "Synchronized {files} source file(s) into {out}: {} operation(s)",
        report.operation_count()
    );
    println!(
        "  nodes: +{} -{}; edges: +{} -{}",
        report.nodes_upserted, report.nodes_removed, report.edges_upserted, report.edges_removed
    );
    Ok(())
}

fn merge(args: &[String]) -> std::io::Result<()> {
    let [bundle, peer, out] = args else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let mut replica = collaboration_result(GraphReplica::load(bundle))?;
    let peer_replica = collaboration_result(GraphReplica::load(peer))?;
    let report = collaboration_result(replica.merge(&peer_replica))?;
    let graph = collaboration_result(replica.materialize())?;
    collaboration_result(replica.save(out))?;
    println!(
        "Merged {peer} into {bundle} -> {out}: {} inserted, {} already present",
        report.inserted, report.already_present
    );
    println!(
        "  converged graph: {} nodes, {} edges, {} operations",
        graph.node_count(),
        graph.edge_count(),
        replica.operation_count()
    );
    Ok(())
}

fn materialize(args: &[String]) -> std::io::Result<()> {
    let [bundle, out] = args else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let replica = collaboration_result(GraphReplica::load(bundle))?;
    let graph = collaboration_result(replica.materialize())?;
    collaboration_result(graph.save(out))?;
    println!(
        "Materialized {bundle} -> {out}: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );
    Ok(())
}

fn collaboration_result<T>(result: Result<T, GraphError>) -> std::io::Result<T> {
    result.map_err(std::io::Error::other)
}
