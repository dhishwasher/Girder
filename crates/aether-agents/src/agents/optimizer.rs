//! Optimizer.
//!
//! In the swarm it records a complexity estimate on new code (EXTENSION POINT
//! for a real cost model). Its other half consumes the **debugger's hot-path
//! profile** (`Timeline::hot_functions`) to rank what to optimize and annotate
//! the graph — the "debugger feeds the Optimizer real hot-path data" loop.

use super::{Agent, AgentResult};
use crate::bus::{MsgKind, Role, SwarmMessage};
use crate::orchestrator::SwarmContext;
use aether_graph::{NodeId, NodeKind, SemanticGraph};
use async_trait::async_trait;

/// A ranked optimization target derived from execution counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotTarget {
    pub function: String,
    pub calls: usize,
    pub recommendation: String,
}

pub struct OptimizerAgent;

impl OptimizerAgent {
    /// Rank a hot-path profile (`function -> call count`) into recommendations,
    /// busiest first. The hottest path gets a stronger suggestion.
    pub fn rank_hot_paths(profile: &[(String, usize)]) -> Vec<HotTarget> {
        let mut sorted = profile.to_vec();
        sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        sorted
            .into_iter()
            .enumerate()
            .map(|(rank, (function, calls))| {
                let recommendation = if rank == 0 {
                    format!("hottest path ({calls}×) — prime candidate for memoization or inlining")
                } else {
                    format!("called {calls}×")
                };
                HotTarget {
                    function,
                    calls,
                    recommendation,
                }
            })
            .collect()
    }

    /// Annotate graph function nodes from a hot-path profile: every matched
    /// function gets a `hot_calls` attribute, and the hottest get
    /// `optimize_priority = hot`. Returns the paths annotated. This is the
    /// Optimizer mutating the graph from real trace data.
    pub fn annotate_graph(graph: &mut SemanticGraph, profile: &[(String, usize)]) -> Vec<String> {
        let max = profile.iter().map(|(_, c)| *c).max().unwrap_or(0);
        let counts: std::collections::HashMap<&str, usize> =
            profile.iter().map(|(n, c)| (n.as_str(), *c)).collect();
        let targets: Vec<(NodeId, usize)> = graph
            .query_by_kind(NodeKind::Function)
            .into_iter()
            .filter_map(|n| counts.get(n.name.as_str()).map(|c| (n.id, *c)))
            .collect();

        let mut annotated = Vec::new();
        for (id, count) in targets {
            if let Some(node) = graph.get_mut(id) {
                node.set_attr("hot_calls", count.to_string());
                if max > 0 && count == max {
                    node.set_attr("optimize_priority", "hot");
                }
                annotated.push(node.path.clone());
            }
        }
        annotated.sort();
        annotated
    }
}

#[async_trait]
impl Agent for OptimizerAgent {
    fn role(&self) -> Role {
        Role::Optimizer
    }

    async fn handle(&self, msg: &SwarmMessage, ctx: &SwarmContext) -> AgentResult {
        let MsgKind::CodeReady { module, name, .. } = &msg.kind else {
            return AgentResult::Idle;
        };
        let path = format!("{module}::{name}");
        {
            let mut graph = ctx.graph.lock().unwrap();
            if let Some(node) = graph.get_mut(NodeId::from_path(&path)) {
                // EXTENSION POINT: derive complexity from the trace / AST.
                node.set_attr("complexity_estimate", "O(1)");
            }
        }
        AgentResult::one(SwarmMessage::new(
            Role::Optimizer,
            MsgKind::Note {
                text: format!("analyzed {name}: complexity=O(1)"),
            },
        ))
    }
}
