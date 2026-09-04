use crate::project::config::ProjectConfig;
use crate::project::projection::{capture_agent_baseline, plan_authored_functions};
use crate::project::source::build_from_dir_with_config;
use crate::project::summary::print_summary;
use crate::project::validation::validate_candidate;
use aether_agents::{MsgKind, Orchestrator, SwarmContext};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub async fn swarm_plan(args: &[String]) -> std::io::Result<()> {
    let root = PathBuf::from(args.first().map(String::as_str).unwrap_or("."));
    let intent = args.get(1..).map(|rest| rest.join(" ")).unwrap_or_default();
    if intent.trim().is_empty() {
        eprintln!("usage: girder swarm-plan <dir> <intent...>");
        return Ok(());
    }

    println!("Loading {} ...", root.display());
    let config = ProjectConfig::load(&root)?;
    let (graph, _builder, files) = build_from_dir_with_config(&root, &config)?;
    println!("  {} file(s), {} nodes", files, graph.node_count());

    println!("\nPlanning: \"{intent}\"");
    let graph = Arc::new(Mutex::new(graph));
    let ctx = Arc::new(SwarmContext::new(
        aether_ai::default_router(),
        graph.clone(),
        &config.agents.output_module,
        &config.agents.output_file,
    ));
    let msgs = Orchestrator::new(ctx)
        .with_default_swarm()
        .plan_only(&intent)
        .await;

    let mut found_spec = false;
    for msg in &msgs {
        if let MsgKind::FeatureSpec {
            graph_context,
            fn_specs,
            ..
        } = &msg.kind
        {
            found_spec = true;
            println!("\nGraph context:");
            for line in graph_context.lines() {
                println!("  {line}");
            }
            println!("\nFunctions to build ({}):", fn_specs.len());
            for (i, spec) in fn_specs.iter().enumerate() {
                println!("  {}. fn {} — {}", i + 1, spec.name, spec.description);
            }
            println!(
                "\n(Run `girder forge {dir} {intent}` to build this feature.)",
                dir = root.display(),
                intent = intent
            );
        }
    }
    if !found_spec {
        println!("  (no structured spec produced — check intent or provider)");
    }
    Ok(())
}

/// `girder forge <dir> <intent...>` — load the project, dispatch the agent
/// swarm on a natural-language intent, persist the updated graph.
pub async fn forge(args: &[String]) -> std::io::Result<()> {
    let root = PathBuf::from(args.first().map(String::as_str).unwrap_or("."));
    let intent = args.get(1..).map(|rest| rest.join(" ")).unwrap_or_default();
    if intent.trim().is_empty() {
        eprintln!("usage: girder forge <dir> <intent...>");
        return Ok(());
    }

    println!("Loading {} ...", root.display());
    let config = ProjectConfig::load(&root)?;
    let baseline = capture_agent_baseline(&root, &config)?;
    let (graph, _builder, files) = build_from_dir_with_config(&root, &config)?;
    println!("  loaded {files} source file(s)");
    print_summary(&graph);

    println!("\nForging: \"{intent}\"");
    let graph = Arc::new(Mutex::new(graph));
    // Agent-authored code lands in a dedicated module/projection.
    let ctx = Arc::new(SwarmContext::new(
        aether_ai::default_router(),
        graph.clone(),
        &config.agents.output_module,
        &config.agents.output_file,
    ));
    let transcript = Orchestrator::new(ctx)
        .with_default_swarm()
        .run(&intent, Duration::from_secs(config.agents.timeout_seconds))
        .await;

    // Print the feature spec if the Planner produced one.
    for m in &transcript {
        if let MsgKind::FeatureSpec {
            graph_context,
            fn_specs,
            ..
        } = &m.kind
        {
            println!("\n  Graph context:");
            for line in graph_context.lines() {
                println!("    {line}");
            }
            println!("\n  Feature plan ({} function(s)):", fn_specs.len());
            for (i, spec) in fn_specs.iter().enumerate() {
                println!("  {}. fn {} — {}", i + 1, spec.name, spec.description);
            }
            println!();
        }
    }

    // Print per-function build progress.
    let mut tested: HashSet<String> = HashSet::new();
    for m in &transcript {
        match &m.kind {
            MsgKind::CodeReady { name, .. } => {
                println!("  [{}] wrote fn {name}", m.from.label());
            }
            MsgKind::TestsReady { for_fn, .. } => {
                let short = for_fn.rsplit("::").next().unwrap_or(for_fn);
                if tested.insert(short.to_string()) {
                    println!("  [{}] tested {for_fn}", m.from.label());
                }
            }
            MsgKind::FeatureComplete { module, built } => {
                println!("\n  Feature complete in module {module}:");
                for name in built {
                    let g = graph.lock().unwrap();
                    let path = format!("{module}::{name}");
                    if let Some(node) = g.find_by_path(&path) {
                        // Show call edges between new functions
                        let calls: Vec<String> = g
                            .neighbors(node.id, Some(aether_graph::EdgeKind::Calls))
                            .iter()
                            .filter_map(|n| g.get(n.id))
                            .filter(|n| n.path.starts_with(module.as_str()))
                            .map(|n| n.name.clone())
                            .collect();
                        if calls.is_empty() {
                            println!("    fn {name}");
                        } else {
                            println!("    fn {name}  →  calls [{}]", calls.join(", "));
                        }
                    }
                }
            }
            MsgKind::Note { text } => println!("  [{}] {text}", m.from.label()),
            _ => {}
        }
    }

    println!();
    let graph = graph.lock().unwrap();
    print_summary(&graph);
    let plan = plan_authored_functions(
        &root,
        &config,
        &graph,
        &config.agents.output_module,
        Some(&baseline),
    )
    .map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("source projection planning failed: {error}"),
        )
    })?;
    println!("\nValidating isolated candidate...");
    let cancel = Arc::new(AtomicBool::new(false));
    let validation = validate_candidate(&root, &config, plan.writes(), &cancel)?;
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

    let files = plan.commit(&root)?;
    match files.as_slice() {
        files if !files.is_empty() => {
            println!("  projected source file(s):");
            for file in files {
                println!("    {}", root.join(file).display());
            }
        }
        _ => {}
    }
    println!(
        "  committed source and graph -> {}",
        root.join(&config.graph.path).display()
    );
    Ok(())
}
