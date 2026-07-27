use crate::project::collaboration_transport;
use crate::project::config::ProjectConfig;
use crate::project::source::load_reconciled_graph;
use aether_graph::{ActorId, GraphError, GraphReplica};
use std::path::{Path, PathBuf};

const USAGE: &str = "\
usage:
  bitcode collab init <dir> <actor> <bundle>
  bitcode collab status <bundle>
  bitcode collab fork <bundle> <actor> <out>
  bitcode collab sync <dir> <bundle> [out]
  bitcode collab merge <bundle> <peer> <out>
  bitcode collab materialize <bundle> <graph.aether>
  bitcode collab secret <path>
  bitcode collab host <bundle> <127.0.0.1:port> --secret-file <path> [--once] [--ready-file <path>]
  bitcode collab join <bundle> <127.0.0.1:port> --secret-file <path> [out]";

pub fn collaboration(args: &[String]) -> std::io::Result<()> {
    match args.first().map(String::as_str) {
        Some("init") => init(&args[1..]),
        Some("status") => status(&args[1..]),
        Some("fork") => fork(&args[1..]),
        Some("sync") => sync(&args[1..]),
        Some("merge") => merge(&args[1..]),
        Some("materialize") => materialize(&args[1..]),
        Some("secret") => secret(&args[1..]),
        Some("host") => host(&args[1..]),
        Some("join") => join(&args[1..]),
        _ => {
            eprintln!("{USAGE}");
            Ok(())
        }
    }
}

fn secret(args: &[String]) -> std::io::Result<()> {
    let [path] = args else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    collaboration_transport::generate_secret(Path::new(path))?;
    println!(
        "Created private collaboration secret {} (contents not displayed)",
        path
    );
    Ok(())
}

fn host(args: &[String]) -> std::io::Result<()> {
    let Some(bundle) = args.first() else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let Some(bind) = args.get(1) else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let mut secret_file = None;
    let mut ready_file = None;
    let mut once = false;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--secret-file" => {
                secret_file = args.get(index + 1).map(PathBuf::from);
                index += 2;
            }
            "--ready-file" => {
                ready_file = args.get(index + 1).map(PathBuf::from);
                index += 2;
            }
            "--once" => {
                once = true;
                index += 1;
            }
            option => {
                return Err(invalid_input(format!(
                    "unknown collab host option '{option}'"
                )));
            }
        }
    }
    let secret_file =
        secret_file.ok_or_else(|| invalid_input("collab host requires --secret-file <path>"))?;
    collaboration_transport::serve(
        Path::new(bundle),
        bind,
        &secret_file,
        once,
        ready_file.as_deref(),
    )
}

fn join(args: &[String]) -> std::io::Result<()> {
    let Some(bundle) = args.first() else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let Some(address) = args.get(1) else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let mut secret_file = None;
    let mut out = None;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--secret-file" => {
                secret_file = args.get(index + 1).map(PathBuf::from);
                index += 2;
            }
            option if option.starts_with('-') => {
                return Err(invalid_input(format!(
                    "unknown collab join option '{option}'"
                )));
            }
            path if out.is_none() => {
                out = Some(PathBuf::from(path));
                index += 1;
            }
            path => {
                return Err(invalid_input(format!(
                    "unexpected collab join argument '{path}'"
                )));
            }
        }
    }
    let secret_file =
        secret_file.ok_or_else(|| invalid_input("collab join requires --secret-file <path>"))?;
    let report =
        collaboration_transport::join(Path::new(bundle), address, &secret_file, out.as_deref())?;
    println!(
        "Live synchronization with {} complete: sent {}, received {}, inserted {}",
        report.peer, report.sent_operations, report.received_operations, report.inserted_operations
    );
    println!(
        "  converged graph: {} nodes, {} edges",
        report.node_count, report.edge_count
    );
    Ok(())
}

fn init(args: &[String]) -> std::io::Result<()> {
    let [root, actor, bundle] = args else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let root = PathBuf::from(root);
    let config = ProjectConfig::load(&root)?;
    let (graph, _baseline, files) = load_reconciled_graph(&root, &config)?;
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
    let config = ProjectConfig::load(Path::new(root))?;
    let (graph, _baseline, files) = load_reconciled_graph(Path::new(root), &config)?;
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

fn invalid_input(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into())
}
