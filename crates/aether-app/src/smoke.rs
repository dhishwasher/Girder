//! Headless end-to-end demo.
//!
//! Runs the whole AetherForge pipeline without a window so it works in CI and
//! on this display-less box, printing each stage to stdout:
//!   1. Build the semantic graph from source (graph is the source of truth).
//!   2. Run the parallel agent swarm on a natural-language intent; watch it
//!      mutate the graph.
//!   3. Predictive impact analysis over the graph.
//!   4. Time-travel debugger: record a buggy run, branch a what-if fix, and ask
//!      the AI layer for a root cause.

use aether_agents::{MsgKind, Orchestrator, SwarmContext};
use aether_builder::GraphBuilder;
use aether_graph::{NodeId, NodeKind, SemanticGraph};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A self-contained sample so the smoke test runs from any working directory.
/// (The on-disk `sample-project/` mirrors this for the GUI demo.)
const SAMPLE_RS: &str = r#"
struct Point { x: i64, y: i64 }

fn add(a: i64, b: i64) -> i64 {
    a + b
}

fn sum_list(xs: &[i64]) -> i64 {
    let mut total = 0;
    for x in xs {
        total = add(total, *x);
    }
    total
}

fn main() {
    let r = sum_list(&[1, 2, 3]);
    println!("{}", r);
}
"#;

/// A second module that calls into `math` — exercises cross-file resolution.
const APP_RS: &str = r#"
fn run() -> i64 {
    sum_list(&[10, 20, 30])
}
"#;

pub async fn run() {
    println!("\n=== AetherForge IDE — headless pipeline demo ===\n");

    // 1. Build the graph from source — two files, to show CROSS-FILE linking.
    let graph = Arc::new(Mutex::new(SemanticGraph::new()));
    {
        let mut g = graph.lock().unwrap();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut g, "src/math.rs", SAMPLE_RS);
        builder.load_file(&mut g, "src/app.rs", APP_RS);
        println!(
            "[1] Built semantic graph from src/math.rs + src/app.rs: {} nodes, {} edges",
            g.node_count(),
            g.edge_count()
        );
        for f in g.query_by_kind(NodeKind::Function) {
            println!("      fn {}  ({})", f.name, f.path);
        }
        // `run` lives in src/app.rs but calls `sum_list` defined in src/math.rs.
        let run = NodeId::from_path("crate::app::run");
        let callees: Vec<String> = g
            .neighbors(run, Some(aether_graph::EdgeKind::Calls))
            .into_iter()
            .filter_map(|n| g.get(n.id).map(|node| node.path.clone()))
            .collect();
        println!("      cross-file: crate::app::run -> {callees:?}");
    }

    // 2. Run the agent swarm on a natural-language intent.
    println!("\n[2] Agent swarm intent: \"Add a multiply function to the math module\"");
    let router = aether_ai::default_router();
    let ctx = Arc::new(SwarmContext::new(
        router,
        graph.clone(),
        "crate::math",
        "src/math.rs",
    ));
    let orchestrator = Orchestrator::new(ctx).with_default_swarm();
    let transcript = orchestrator
        .run(
            "Add a multiply function to the math module",
            Duration::from_secs(5),
        )
        .await;
    for msg in &transcript {
        match &msg.kind {
            MsgKind::Intent(t) => println!("      [Conductor] intent: {t}"),
            MsgKind::PlanReady { steps } => {
                println!("      [{}] plan: {} steps", msg.from.label(), steps.len())
            }
            MsgKind::CodeReady { name, .. } => {
                println!("      [{}] wrote fn {name} into the graph", msg.from.label())
            }
            MsgKind::TestsReady { for_fn, .. } => {
                println!("      [{}] generated tests for {for_fn}", msg.from.label())
            }
            MsgKind::Note { text } => println!("      [{}] {text}", msg.from.label()),
        }
    }

    {
        let g = graph.lock().unwrap();
        if let Some(mult) = g.find_by_path("crate::math::multiply") {
            println!("\n      New graph node `crate::math::multiply`:");
            println!("        source : {}", mult.source.replace('\n', " "));
            println!("        summary: {}", mult.attr("summary").unwrap_or("-"));
            println!("        risk   : {}", mult.attr("risk").unwrap_or("-"));
            println!(
                "        test   : {}",
                mult.attr("test").unwrap_or("-").replace('\n', " ")
            );
        }
    }

    // 3. Predictive impact analysis.
    {
        let g = graph.lock().unwrap();
        let add = NodeId::from_path("crate::math::add");
        let report = g.impact_of(add);
        println!("\n[3] Impact of changing `add`: {} affected node(s)", report.affected.len());
        for (id, dist) in report.ranked() {
            if let Some(n) = g.get(id) {
                println!("      {} (distance {dist})", n.path);
            }
        }
    }

    // 4. Time-travel debugger.
    println!("\n[4] Time-travel debugger on a deliberately buggy program:");
    let mut timeline = aether_debugger::Timeline::record(aether_debugger::buggy_demo_program());
    for step in &timeline.branch(0).unwrap().trace.steps {
        println!("      step {}: {}", step.seq, step.description);
    }
    let buggy = timeline.branch(0).unwrap().trace.last_value("scaled");
    println!("      => buggy result: scaled = {:?} (expected 24)", buggy);

    let fixed = timeline.fork_what_if(0, 2, "area", 12, "what-if: area = w * h");
    let fixed_scaled = timeline.branch(fixed).unwrap().trace.last_value("scaled");
    println!(
        "      branched what-if (area=12) -> scaled = {:?}  [downstream recomputed]",
        fixed_scaled
    );
    println!(
        "      first divergence between branches at step {:?}",
        timeline.first_divergence(0, fixed)
    );

    let router = aether_ai::default_router();
    if let Some(rc) = timeline
        .ai_root_cause(&router, 0, "area is 7 but expected 12")
        .await
    {
        println!("      AI root-cause: {}", rc.lines().next().unwrap_or(""));
    }

    println!("\n=== demo complete ===\n");
}
