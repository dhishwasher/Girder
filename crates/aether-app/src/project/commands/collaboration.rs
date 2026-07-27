use crate::project::collaboration_discovery;
use crate::project::collaboration_identity::{self, TrustChange};
use crate::project::collaboration_projection::{
    load_collaboration_projection, CollaborationFileChangeKind, CollaborationProjectionPlan,
};
use crate::project::collaboration_transport;
use crate::project::config::ProjectConfig;
use crate::project::source::load_reconciled_graph;
use crate::project::validation::validate_candidate;
use aether_graph::{ActorId, GraphError, GraphReplica};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const USAGE: &str = "\
usage:
  bitcode collab init <dir> <actor> <bundle>
  bitcode collab status <bundle>
  bitcode collab fork <bundle> <actor> <out> --approve
  bitcode collab member add|remove <bundle> <actor> --approve
  bitcode collab sync <dir> <bundle> [out]
  bitcode collab merge <bundle> <peer> <out>
  bitcode collab compact <bundle> [out]
  bitcode collab review <dir> <bundle>
  bitcode collab apply <dir> <bundle> --approve
  bitcode collab materialize <bundle> <graph.aether>
  bitcode collab secret <path>
  bitcode collab identity generate <bundle> <private-key> <public-key>
  bitcode collab identity show <public-key>
  bitcode collab identity list <trust-store>
  bitcode collab identity attest <bundle> <private-key>
  bitcode collab identity verify <bundle> <private-key> <trust-store>
  bitcode collab identity trust <trust-store> <public-key> --approve <fingerprint>
  bitcode collab identity rotate <trust-store> <public-key> --from <old-fingerprint> --approve <new-fingerprint>
  bitcode collab identity rotate-local <bundle> <old-private-key> <new-private-key> --from <old-fingerprint> --approve <new-fingerprint>
  bitcode collab identity remove <trust-store> <actor> --approve <fingerprint>
  bitcode collab host <bundle> <127.0.0.1:port> --secret-file <path> [--identity-file <path> --trust-store <path>] [--discovery-dir <path>] [--presence <status>] [--once] [--ready-file <path>]
  bitcode collab discover <bundle> <directory> --secret-file <path>
  bitcode collab join <bundle> <127.0.0.1:port> --secret-file <path> [--identity-file <path> --trust-store <path>] [--presence <status>] [out]
  bitcode collab join-peer <bundle> <actor> <directory> --secret-file <path> [--identity-file <path> --trust-store <path>] [--presence <status>] [out]";

pub fn collaboration(args: &[String]) -> std::io::Result<()> {
    match args.first().map(String::as_str) {
        Some("init") => init(&args[1..]),
        Some("status") => status(&args[1..]),
        Some("fork") => fork(&args[1..]),
        Some("member") => member(&args[1..]),
        Some("sync") => sync(&args[1..]),
        Some("merge") => merge(&args[1..]),
        Some("compact") => compact(&args[1..]),
        Some("review") => review_projection(&args[1..]),
        Some("apply") => apply_projection(&args[1..]),
        Some("materialize") => materialize(&args[1..]),
        Some("secret") => secret(&args[1..]),
        Some("identity") => identity(&args[1..]),
        Some("host") => host(&args[1..]),
        Some("discover") => discover(&args[1..]),
        Some("join") => join(&args[1..]),
        Some("join-peer") => join_peer(&args[1..]),
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

fn identity(args: &[String]) -> std::io::Result<()> {
    match args.first().map(String::as_str) {
        Some("generate") => {
            let [_, bundle, private_key, public_key] = args else {
                return Err(invalid_input(
                    "identity generate requires <bundle> <private-key> <public-key>",
                ));
            };
            let summary = collaboration_identity::generate_identity(
                Path::new(bundle),
                Path::new(private_key),
                Path::new(public_key),
            )?;
            println!(
                "Generated Ed25519 identity for {}:\n  private: {}\n  public: {}\n  SHA-256 fingerprint: {}",
                summary.actor, private_key, public_key, summary.fingerprint
            );
            println!("Verify this fingerprint out of band before a peer trusts the public file.");
            Ok(())
        }
        Some("show") => {
            let [_, public_key] = args else {
                return Err(invalid_input("identity show requires <public-key>"));
            };
            let summary = collaboration_identity::inspect_public_identity(Path::new(public_key))?;
            println!(
                "Public identity {}: Ed25519 SHA-256 {}",
                summary.actor, summary.fingerprint
            );
            Ok(())
        }
        Some("list") => {
            let [_, trust_store] = args else {
                return Err(invalid_input("identity list requires <trust-store>"));
            };
            let identities = collaboration_identity::trusted_identities(Path::new(trust_store))?;
            println!("Pinned actor identities in {trust_store}:");
            if identities.is_empty() {
                println!("  none");
            } else {
                for identity in identities {
                    println!(
                        "  {}: Ed25519 SHA-256 {}",
                        identity.actor, identity.fingerprint
                    );
                }
            }
            Ok(())
        }
        Some("attest") => {
            let [_, bundle, private_key] = args else {
                return Err(invalid_input(
                    "identity attest requires <bundle> <private-key>",
                ));
            };
            let summary = collaboration_identity::attest_local_bundle(
                Path::new(bundle),
                Path::new(private_key),
            )?;
            println!(
                "Attested {} local operation(s) in {bundle}; {} new proof(s) added",
                summary.operations, summary.attestations_added
            );
            Ok(())
        }
        Some("verify") => {
            let [_, bundle, private_key, trust_store] = args else {
                return Err(invalid_input(
                    "identity verify requires <bundle> <private-key> <trust-store>",
                ));
            };
            let summary = collaboration_identity::verify_bundle_provenance(
                Path::new(bundle),
                Path::new(private_key),
                Path::new(trust_store),
            )?;
            println!(
                "Verified {} durable operation attestation(s) across {} actor(s) in {bundle}",
                summary.operations, summary.actors
            );
            Ok(())
        }
        Some("trust") => {
            let [_, trust_store, public_key, approval, fingerprint] = args else {
                return Err(invalid_input(
                    "identity trust requires <trust-store> <public-key> --approve <fingerprint>",
                ));
            };
            if approval != "--approve" {
                return Err(invalid_input(
                    "identity trust requires --approve followed by the exact reviewed fingerprint",
                ));
            }
            print_trust_change(collaboration_identity::trust_identity(
                Path::new(trust_store),
                Path::new(public_key),
                fingerprint,
            )?);
            Ok(())
        }
        Some("rotate") => {
            let [_, trust_store, public_key, from, old, approval, new] = args else {
                return Err(invalid_input(
                    "identity rotate requires <trust-store> <public-key> --from <old-fingerprint> --approve <new-fingerprint>",
                ));
            };
            if from != "--from" || approval != "--approve" {
                return Err(invalid_input(
                    "identity rotate requires --from <old-fingerprint> --approve <new-fingerprint>",
                ));
            }
            print_trust_change(collaboration_identity::rotate_identity(
                Path::new(trust_store),
                Path::new(public_key),
                old,
                new,
            )?);
            Ok(())
        }
        Some("rotate-local") => {
            let [_, bundle, old_private, new_private, from, old, approval, new] = args else {
                return Err(invalid_input(
                    "identity rotate-local requires <bundle> <old-private-key> <new-private-key> --from <old-fingerprint> --approve <new-fingerprint>",
                ));
            };
            if from != "--from" || approval != "--approve" {
                return Err(invalid_input(
                    "identity rotate-local requires --from <old-fingerprint> --approve <new-fingerprint>",
                ));
            }
            let rotation = collaboration_identity::rotate_local_identity(
                Path::new(bundle),
                Path::new(old_private),
                Path::new(new_private),
                old,
                new,
            )?;
            println!(
                "Recorded identity rotation for {} at operation {} from Ed25519 SHA-256 {} to {}",
                rotation.actor,
                rotation.counter,
                rotation.previous_fingerprint,
                rotation.fingerprint
            );
            println!(
                "Peers must pin the successor public identity before the next strict session."
            );
            Ok(())
        }
        Some("remove") => {
            let [_, trust_store, actor, approval, fingerprint] = args else {
                return Err(invalid_input(
                    "identity remove requires <trust-store> <actor> --approve <fingerprint>",
                ));
            };
            if approval != "--approve" {
                return Err(invalid_input(
                    "identity remove requires --approve followed by the exact current fingerprint",
                ));
            }
            print_trust_change(collaboration_identity::remove_trusted_identity(
                Path::new(trust_store),
                actor,
                fingerprint,
            )?);
            Ok(())
        }
        _ => {
            eprintln!("{USAGE}");
            Ok(())
        }
    }
}

fn print_trust_change(change: TrustChange) {
    match change {
        TrustChange::Added(identity) => println!(
            "Pinned {} to Ed25519 SHA-256 {}",
            identity.actor, identity.fingerprint
        ),
        TrustChange::AlreadyTrusted(identity) => println!(
            "{} is already pinned to Ed25519 SHA-256 {}",
            identity.actor, identity.fingerprint
        ),
        TrustChange::Rotated {
            actor,
            previous_fingerprint,
            fingerprint,
        } => {
            println!("Rotated {actor} from Ed25519 SHA-256 {previous_fingerprint} to {fingerprint}")
        }
        TrustChange::Removed(identity) => println!(
            "Removed local trust for {} at Ed25519 SHA-256 {}",
            identity.actor, identity.fingerprint
        ),
    }
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
    let mut discovery_directory = None;
    let mut identity_file = None;
    let mut trust_store = None;
    let mut presence = None;
    let mut once = false;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--secret-file" => {
                secret_file = Some(PathBuf::from(args.get(index + 1).ok_or_else(|| {
                    invalid_input("collab host requires a value after --secret-file")
                })?));
                index += 2;
            }
            "--ready-file" => {
                ready_file = Some(PathBuf::from(args.get(index + 1).ok_or_else(|| {
                    invalid_input("collab host requires a value after --ready-file")
                })?));
                index += 2;
            }
            "--discovery-dir" => {
                discovery_directory =
                    Some(PathBuf::from(args.get(index + 1).ok_or_else(|| {
                        invalid_input("collab host requires a value after --discovery-dir")
                    })?));
                index += 2;
            }
            "--identity-file" => {
                identity_file = Some(PathBuf::from(args.get(index + 1).ok_or_else(|| {
                    invalid_input("collab host requires a value after --identity-file")
                })?));
                index += 2;
            }
            "--trust-store" => {
                trust_store = Some(PathBuf::from(args.get(index + 1).ok_or_else(|| {
                    invalid_input("collab host requires a value after --trust-store")
                })?));
                index += 2;
            }
            "--presence" => {
                presence = Some(
                    args.get(index + 1)
                        .ok_or_else(|| {
                            invalid_input("collab host requires a value after --presence")
                        })?
                        .clone(),
                );
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
        collaboration_transport::ServeOptions {
            once,
            ready_file: ready_file.as_deref(),
            presence: presence.as_deref(),
            discovery_directory: discovery_directory.as_deref(),
            identity_file: identity_file.as_deref(),
            trust_store: trust_store.as_deref(),
        },
    )
}

fn discover(args: &[String]) -> std::io::Result<()> {
    let [bundle, directory, rest @ ..] = args else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let mut secret_file = None;
    let mut index = 0;
    while index < rest.len() {
        match rest[index].as_str() {
            "--secret-file" => {
                secret_file = Some(PathBuf::from(rest.get(index + 1).ok_or_else(|| {
                    invalid_input("collab discover requires a value after --secret-file")
                })?));
                index += 2;
            }
            option => {
                return Err(invalid_input(format!(
                    "unknown collab discover option '{option}'"
                )));
            }
        }
    }
    let secret_file = secret_file
        .ok_or_else(|| invalid_input("collab discover requires --secret-file <path>"))?;
    let scan =
        collaboration_discovery::discover(Path::new(bundle), Path::new(directory), &secret_file)?;
    println!(
        "Discovered {} authenticated local collaboration peer(s):",
        scan.peers.len()
    );
    for peer in scan.peers {
        println!(
            "  {} at {} (pid {})",
            peer.actor, peer.address, peer.process_id
        );
    }
    if scan.ignored_entries > 0 {
        println!(
            "  ignored {} invalid, stale, unrelated, or unauthorized entr{}",
            scan.ignored_entries,
            if scan.ignored_entries == 1 {
                "y"
            } else {
                "ies"
            }
        );
    }
    Ok(())
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
    let mut presence = None;
    let mut identity_file = None;
    let mut trust_store = None;
    let mut out = None;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--secret-file" => {
                secret_file = Some(PathBuf::from(args.get(index + 1).ok_or_else(|| {
                    invalid_input("collab join requires a value after --secret-file")
                })?));
                index += 2;
            }
            "--presence" => {
                presence = Some(
                    args.get(index + 1)
                        .ok_or_else(|| {
                            invalid_input("collab join requires a value after --presence")
                        })?
                        .clone(),
                );
                index += 2;
            }
            "--identity-file" => {
                identity_file = Some(PathBuf::from(args.get(index + 1).ok_or_else(|| {
                    invalid_input("collab join requires a value after --identity-file")
                })?));
                index += 2;
            }
            "--trust-store" => {
                trust_store = Some(PathBuf::from(args.get(index + 1).ok_or_else(|| {
                    invalid_input("collab join requires a value after --trust-store")
                })?));
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
    let report = collaboration_transport::join(
        Path::new(bundle),
        address,
        &secret_file,
        out.as_deref(),
        presence.as_deref(),
        identity_file.as_deref(),
        trust_store.as_deref(),
    )?;
    print_live_sync_report(&report);
    Ok(())
}

fn join_peer(args: &[String]) -> std::io::Result<()> {
    let [bundle, actor, directory, rest @ ..] = args else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let mut secret_file = None;
    let mut presence = None;
    let mut identity_file = None;
    let mut trust_store = None;
    let mut out = None;
    let mut index = 0;
    while index < rest.len() {
        match rest[index].as_str() {
            "--secret-file" => {
                secret_file = Some(PathBuf::from(rest.get(index + 1).ok_or_else(|| {
                    invalid_input("collab join-peer requires a value after --secret-file")
                })?));
                index += 2;
            }
            "--presence" => {
                presence = Some(
                    rest.get(index + 1)
                        .ok_or_else(|| {
                            invalid_input("collab join-peer requires a value after --presence")
                        })?
                        .clone(),
                );
                index += 2;
            }
            "--identity-file" => {
                identity_file = Some(PathBuf::from(rest.get(index + 1).ok_or_else(|| {
                    invalid_input("collab join-peer requires a value after --identity-file")
                })?));
                index += 2;
            }
            "--trust-store" => {
                trust_store = Some(PathBuf::from(rest.get(index + 1).ok_or_else(|| {
                    invalid_input("collab join-peer requires a value after --trust-store")
                })?));
                index += 2;
            }
            option if option.starts_with('-') => {
                return Err(invalid_input(format!(
                    "unknown collab join-peer option '{option}'"
                )));
            }
            path if out.is_none() => {
                out = Some(PathBuf::from(path));
                index += 1;
            }
            path => {
                return Err(invalid_input(format!(
                    "unexpected collab join-peer argument '{path}'"
                )));
            }
        }
    }
    let secret_file = secret_file
        .ok_or_else(|| invalid_input("collab join-peer requires --secret-file <path>"))?;
    let report = collaboration_discovery::join_peer(
        Path::new(bundle),
        actor,
        Path::new(directory),
        &secret_file,
        collaboration_discovery::JoinPeerOptions {
            out: out.as_deref(),
            presence: presence.as_deref(),
            identity_file: identity_file.as_deref(),
            trust_store: trust_store.as_deref(),
        },
    )?;
    print_live_sync_report(&report);
    Ok(())
}

fn print_live_sync_report(report: &collaboration_transport::LiveSyncReport) {
    println!(
        "Live synchronization with {} complete: sent {}, received {}, inserted {}",
        report.peer, report.sent_operations, report.received_operations, report.inserted_operations
    );
    println!(
        "  converged graph: {} nodes, {} edges",
        report.node_count, report.edge_count
    );
    match report.peer_presence() {
        Some(status) => println!("  peer presence: {status}"),
        None => println!("  peer presence: online (no status shared)"),
    }
    match report.peer_identity_fingerprint() {
        Some(fingerprint) => println!("  peer identity: Ed25519 SHA-256 {fingerprint}"),
        None => println!("  peer identity: group-secret only (legacy mode)"),
    }
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
    let floor = version_string(replica.history_floor());
    println!("Collaboration bundle: {bundle}");
    println!("  actor: {}", replica.actor());
    println!("  operations: {}", replica.operation_count());
    println!(
        "  durable operation attestations: {} / {}",
        replica.attestation_count(),
        replica
            .operations()
            .filter(|(dot, _)| !dot.actor.is_bootstrap())
            .count()
    );
    println!("  version: {versions}");
    println!(
        "  compacted through: {}",
        if floor.is_empty() { "none" } else { &floor }
    );
    println!("  active members:");
    for member in collaboration_result(replica.members())? {
        if &member == replica.actor() {
            println!("    {member} (local)");
        } else if replica.acknowledgements().any(|(peer, _)| peer == &member) {
            println!("    {member} (durably acknowledged)");
        } else {
            println!("    {member} (awaiting durable acknowledgement)");
        }
    }
    println!("  durable peer acknowledgements:");
    if replica.acknowledgements().count() == 0 {
        println!("    none");
    } else {
        for (peer, version) in replica.acknowledgements() {
            let version = version_string(version);
            println!(
                "    {peer}: {}",
                if version.is_empty() {
                    "empty"
                } else {
                    &version
                }
            );
        }
    }
    println!("  nodes: {}", graph.node_count());
    println!("  edges: {}", graph.edge_count());
    Ok(())
}

fn fork(args: &[String]) -> std::io::Result<()> {
    let [bundle, actor, out, approval] = args else {
        return Err(invalid_input(
            "collab fork registers a durable member and requires the exact --approve flag",
        ));
    };
    if approval != "--approve" {
        return Err(invalid_input(
            "collab fork registers a durable member and requires the exact --approve flag",
        ));
    }
    let bundle_path = Path::new(bundle);
    let out_path = Path::new(out);
    if paths_alias(bundle_path, out_path) {
        return Err(invalid_input(
            "collab fork output must differ from the source bundle",
        ));
    }
    let mut replica = collaboration_result(GraphReplica::load(bundle))?;
    let original = replica.clone();
    let forked = collaboration_result(replica.fork(collaboration_result(ActorId::new(actor))?))?;
    collaboration_result(replica.save(bundle))?;
    if let Err(error) = forked.save(out) {
        return match original.save(bundle) {
            Ok(()) => Err(std::io::Error::other(format!(
                "failed to save forked bundle; source membership was rolled back: {error}"
            ))),
            Err(rollback) => Err(std::io::Error::other(format!(
                "failed to save forked bundle ({error}) and failed to roll back source membership ({rollback}); actor '{actor}' may remain registered"
            ))),
        };
    }
    println!(
        "Registered member {} in {bundle} and forked -> {out} ({} operations)",
        forked.actor(),
        forked.operation_count()
    );
    Ok(())
}

fn member(args: &[String]) -> std::io::Result<()> {
    let [operation, bundle, actor, approval] = args else {
        return Err(invalid_input(
            "collab member add/remove requires the exact --approve flag",
        ));
    };
    if approval != "--approve" {
        return Err(invalid_input(
            "collab member add/remove requires the exact --approve flag",
        ));
    }
    let actor = collaboration_result(ActorId::new(actor))?;
    let mut replica = collaboration_result(GraphReplica::load(bundle))?;
    match operation.as_str() {
        "add" => {
            collaboration_result(replica.add_member(actor.clone()))?;
        }
        "remove" => {
            collaboration_result(replica.remove_member(&actor))?;
        }
        _ => {
            return Err(invalid_input(
                "collab member operation must be 'add' or 'remove'",
            ));
        }
    }
    collaboration_result(replica.save(bundle))?;
    let members = collaboration_result(replica.members())?
        .into_iter()
        .map(|member| member.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "{} member {actor} in {bundle}; active roster: {members}",
        if operation == "add" {
            "Added"
        } else {
            "Removed"
        }
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
    let mut delta = collaboration_result(peer_replica.delta_since(replica.version()))?;
    let unverified_attestations = delta.attestations.len();
    delta.attestations.clear();
    let report = collaboration_result(replica.apply_delta(&delta))?;
    let graph = collaboration_result(replica.materialize())?;
    collaboration_result(replica.save(out))?;
    println!(
        "Merged {peer} into {bundle} -> {out}: {} inserted, {} already present",
        report.inserted, report.already_present
    );
    if unverified_attestations != 0 {
        println!(
            "  ignored {unverified_attestations} unverified operation attestation(s); use an identity-authenticated live session to import provenance"
        );
    }
    println!(
        "  converged graph: {} nodes, {} edges, {} operations",
        graph.node_count(),
        graph.edge_count(),
        replica.operation_count()
    );
    Ok(())
}

fn compact(args: &[String]) -> std::io::Result<()> {
    let ([bundle] | [bundle, _]) = args else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let out = args.get(1).map(String::as_str).unwrap_or(bundle);
    let mut replica = collaboration_result(GraphReplica::load(bundle))?;
    let report = collaboration_result(replica.compact_acknowledged())?;
    collaboration_result(replica.save(out))?;
    println!(
        "Compacted {bundle} -> {out}: removed {} superseded operation(s), {} -> {} retained",
        report.removed_operations, report.operations_before, report.operations_after
    );
    let floor = version_string(&report.history_floor);
    println!(
        "  history floor: {}",
        if floor.is_empty() { "none" } else { &floor }
    );
    println!("  stale peers older than this floor require a current bundle");
    Ok(())
}

fn review_projection(args: &[String]) -> std::io::Result<()> {
    let [root, bundle] = args else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let root = Path::new(root);
    let plan = collaboration_projection_plan(root, Path::new(bundle))?;
    print_projection_review(&plan);
    Ok(())
}

fn apply_projection(args: &[String]) -> std::io::Result<()> {
    let Some(root) = args.first() else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let Some(bundle) = args.get(1) else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    if args.get(2).map(String::as_str) != Some("--approve") || args.len() != 3 {
        return Err(invalid_input(
            "collab apply is write-capable and requires the exact --approve flag",
        ));
    }
    let root = Path::new(root);
    let plan = collaboration_projection_plan(root, Path::new(bundle))?;
    print_projection_review(&plan);
    if !plan.can_apply() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "collaboration projection has unresolved conflicts; project was not modified",
        ));
    }

    println!("Validating isolated collaboration candidate...");
    let validation = validate_candidate(
        root,
        &ProjectConfig::load(root)?,
        plan.writes(),
        &Arc::new(AtomicBool::new(false)),
    )?;
    for step in &validation.steps {
        let command = step
            .command
            .as_deref()
            .map(|command| format!(": {command}"))
            .unwrap_or_default();
        println!(
            "  [{}] {}{} ({:.2}s)",
            step.status.label(),
            step.label,
            command,
            step.duration.as_secs_f32()
        );
        if step.status != crate::project::validation::ValidationStatus::Passed
            && !step.output.trim().is_empty()
        {
            for line in step.output.lines() {
                println!("    {line}");
            }
        }
    }
    if !validation.passed() {
        return Err(std::io::Error::other(format!(
            "{}; project was not modified",
            validation.summary()
        )));
    }
    let projected = plan.commit(root)?;
    println!(
        "Committed {} reviewed source projection(s) and the semantic graph",
        projected.len()
    );
    Ok(())
}

fn collaboration_projection_plan(
    root: &Path,
    bundle: &Path,
) -> std::io::Result<CollaborationProjectionPlan> {
    load_collaboration_projection(root, bundle)
}

fn print_projection_review(plan: &CollaborationProjectionPlan) {
    println!("Collaboration source-projection review");
    println!(
        "  semantic nodes: +{} ~{} -{}; edges: +{} -{}",
        plan.semantic.added.len(),
        plan.semantic.modified.len(),
        plan.semantic.removed.len(),
        plan.semantic.added_edges.len(),
        plan.semantic.removed_edges.len()
    );
    println!("  source files ({}):", plan.files.len());
    for change in &plan.files {
        let marker = match change.kind {
            CollaborationFileChangeKind::Added => "+",
            CollaborationFileChangeKind::Modified => "~",
            CollaborationFileChangeKind::Removed => "-",
        };
        println!("    {marker} {}", change.path);
    }
    if plan.files.is_empty() {
        println!("    none");
    }
    if plan.conflicts.is_empty() {
        println!("  conflicts: none");
        if let Some(digest) = plan.approval_digest() {
            println!("  approval SHA-256: {digest}");
        }
    } else {
        println!("  conflicts ({}):", plan.conflicts.len());
        for conflict in &plan.conflicts {
            println!("    ! {conflict}");
        }
    }
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

fn paths_alias(left: &Path, right: &Path) -> bool {
    left == right
        || match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
            (Ok(left), Ok(right)) => left == right,
            _ => false,
        }
}

fn collaboration_result<T>(result: Result<T, GraphError>) -> std::io::Result<T> {
    result.map_err(std::io::Error::other)
}

fn version_string(version: &aether_graph::VersionVector) -> String {
    version
        .actors()
        .map(|(actor, counter)| format!("{actor}:{counter}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn invalid_input(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into())
}
