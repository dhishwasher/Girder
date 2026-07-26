//! Coder: generates all functions in a feature and wires their call graph.
//!
//! When it receives a [`FeatureSpec`] (the new intent-first path), it iterates
//! every [`FnSpec`] in order (leaves first), calls the AI for each body,
//! upserts each function into the semantic graph, then does a lightweight
//! call-wiring pass: if function A's source text contains function B's name,
//! a `Calls(A → B)` edge is added so the impact panel and semantic search
//! immediately reflect the new architecture.
//!
//! The legacy [`PlanReady`] path (single-function generation) is kept for
//! backward compatibility with external code that drives the Coder directly.

use super::{rust_fn_name, Agent, AgentResult};
use crate::bus::{MsgKind, Role, SwarmMessage};
use crate::orchestrator::SwarmContext;
use aether_ai::{Prompt, TaskClass};
use aether_graph::{Edge, EdgeKind, Node, NodeId, NodeKind};
use async_trait::async_trait;

pub struct CoderAgent;

#[async_trait]
impl Agent for CoderAgent {
    fn role(&self) -> Role {
        Role::Coder
    }

    async fn handle(&self, msg: &SwarmMessage, ctx: &SwarmContext) -> AgentResult {
        match &msg.kind {
            MsgKind::FeatureSpec {
                intent, fn_specs, ..
            } => handle_feature_spec(ctx, intent, fn_specs).await,

            MsgKind::PlanReady { steps } => handle_plan_ready(ctx, steps).await,

            _ => AgentResult::Idle,
        }
    }
}

// ── FeatureSpec path: multi-function generation ────────────────────────────

async fn handle_feature_spec(
    ctx: &SwarmContext,
    intent: &str,
    fn_specs: &[crate::bus::FnSpec],
) -> AgentResult {
    if fn_specs.is_empty() {
        return AgentResult::Idle;
    }

    let module = ctx.target_module.clone();
    // Track (NodeId, source, name) for call wiring after all functions are built.
    let mut built: Vec<(NodeId, String, String)> = Vec::new();
    let mut out_msgs: Vec<SwarmMessage> = Vec::new();

    for spec in fn_specs {
        // Ask the AI to write the function body.
        let prompt = Prompt::new(
            TaskClass::Codegen,
            "You are the Coder. Emit a single Rust function for the specification.",
            format!("fn {} — {}", spec.name, spec.description),
        );
        let Ok(completion) = ctx.router.complete(prompt).await else {
            continue;
        };
        let source = completion.text;

        // Derive the canonical function name: prefer what we planned, fall
        // back to what the AI actually emitted (they should match).
        let name = if source.contains(&format!("fn {}", spec.name)) {
            spec.name.clone()
        } else {
            rust_fn_name(&source).unwrap_or_else(|| spec.name.clone())
        };

        let path = format!("{module}::{name}");

        // Graph mutation — no await held across this block.
        let fn_id = {
            let mut graph = ctx.graph.lock().unwrap();

            // Ensure the module node exists.
            let module_id = NodeId::from_path(&module);
            if !graph.contains(module_id) {
                let seg = module.rsplit("::").next().unwrap_or(&module).to_string();
                graph.upsert_node(Node::new(NodeKind::Module, seg, &module).with_language("rust"));
            }

            // Upsert the function and add a Contains edge.
            let module_id = NodeId::from_path(&module);
            let mut fn_node = Node::new(NodeKind::Function, &name, &path)
                .with_language("rust")
                .with_source(&source);
            fn_node.file = Some(ctx.target_file.clone());
            fn_node.set_attr("authored_by", "Coder");
            fn_node.set_attr("intent", intent);
            fn_node.set_attr("spec", &spec.description);
            let id = graph.upsert_node(fn_node);
            let _ = graph.add_edge(module_id, id, Edge::new(EdgeKind::Contains));
            graph.materialize_impact_edges(id);
            id
        };

        built.push((fn_id, source.clone(), name.clone()));
        out_msgs.push(SwarmMessage::new(
            Role::Coder,
            MsgKind::CodeReady {
                module: module.clone(),
                name,
                source,
            },
        ));
    }

    // ── Call wiring pass ─────────────────────────────────────────────────────
    // For every pair of newly built functions, if the caller's source text
    // contains the callee's name, add a Calls edge. This gives the semantic
    // graph an immediate architecture snapshot — no tree-sitter parse needed.
    if built.len() > 1 {
        let mut graph = ctx.graph.lock().unwrap();
        for i in 0..built.len() {
            for j in 0..built.len() {
                if i == j {
                    continue;
                }
                let (caller_id, caller_src, _) = &built[i];
                let (callee_id, _, callee_name) = &built[j];
                // Simple substring check: caller calls callee if callee's name
                // appears in caller's source as an identifier (surrounded by
                // non-alphanumeric chars or at a word boundary).
                let needle = format!("{callee_name}(");
                if caller_src.contains(&needle) {
                    let _ = graph.add_edge(*caller_id, *callee_id, Edge::new(EdgeKind::Calls));
                }
            }
        }
    }

    // Emit FeatureComplete so the CLI/GUI can summarise what was built.
    let built_names: Vec<String> = built.into_iter().map(|(_, _, n)| n).collect();
    if !built_names.is_empty() {
        out_msgs.push(SwarmMessage::new(
            Role::Coder,
            MsgKind::FeatureComplete {
                module,
                built: built_names,
            },
        ));
    }

    if out_msgs.is_empty() {
        AgentResult::Idle
    } else {
        AgentResult::Emit(out_msgs)
    }
}

// ── PlanReady path: legacy single-function generation ─────────────────────

async fn handle_plan_ready(ctx: &SwarmContext, steps: &[String]) -> AgentResult {
    let brief = steps.join(" ");
    let prompt = Prompt::new(
        TaskClass::Codegen,
        "You are the Coder. Emit a single Rust function implementing the plan.",
        brief,
    );
    let Ok(completion) = ctx.router.complete(prompt).await else {
        return AgentResult::Idle;
    };
    let source = completion.text;
    let Some(name) = rust_fn_name(&source) else {
        return AgentResult::Idle;
    };

    let module = ctx.target_module.clone();
    let path = format!("{module}::{name}");

    {
        let mut graph = ctx.graph.lock().unwrap();

        let module_id = NodeId::from_path(&module);
        if !graph.contains(module_id) {
            let seg = module.rsplit("::").next().unwrap_or(&module).to_string();
            graph.upsert_node(Node::new(NodeKind::Module, seg, &module).with_language("rust"));
        }

        let mut fn_node = Node::new(NodeKind::Function, &name, &path)
            .with_language("rust")
            .with_source(&source);
        fn_node.file = Some(ctx.target_file.clone());
        fn_node.set_attr("authored_by", "Coder");
        let fn_id = graph.upsert_node(fn_node);
        let module_id = NodeId::from_path(&module);
        let _ = graph.add_edge(module_id, fn_id, Edge::new(EdgeKind::Contains));
        graph.materialize_impact_edges(fn_id);
    }

    AgentResult::one(SwarmMessage::new(
        Role::Coder,
        MsgKind::CodeReady {
            module,
            name,
            source,
        },
    ))
}
