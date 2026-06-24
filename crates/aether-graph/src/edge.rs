//! Edge types for the semantic graph.

use serde::{Deserialize, Serialize};

/// The semantic relationship an edge encodes. Direction is `source -> target`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EdgeKind {
    /// `source` calls `target` (function-call graph).
    Calls,
    /// `source` inherits from / implements `target`.
    Inherits,
    /// A value flows from `source` to `target` (dataflow).
    DataFlow,
    /// `source` syntactically/semantically contains `target` (module->fn, type->field).
    Contains,
    /// `source` and `target` are semantically similar (embedding cosine, agent-inferred).
    SemanticSimilar,
    /// Changing `source` is predicted to impact `target`. Derived/maintained by
    /// impact analysis and the Optimizer/Refactorer agents.
    Impacts,
}

impl EdgeKind {
    /// Whether this edge should be traversed by default impact analysis.
    /// Impact propagates through call/dataflow/explicit-impact edges, not
    /// through mere containment or similarity.
    pub fn propagates_impact(self) -> bool {
        matches!(self, EdgeKind::Calls | EdgeKind::DataFlow | EdgeKind::Impacts)
    }
}

/// Edge payload stored on each graph edge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub kind: EdgeKind,
    /// Confidence in this relationship in `[0, 1]`. Statically-derived edges are
    /// `1.0`; agent-inferred edges (similarity, predicted impact) may be lower.
    pub weight: f32,
}

impl Edge {
    pub fn new(kind: EdgeKind) -> Self {
        Edge { kind, weight: 1.0 }
    }

    pub fn with_weight(kind: EdgeKind, weight: f32) -> Self {
        Edge { kind, weight }
    }
}
