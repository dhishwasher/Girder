//! # aether-graph
//!
//! The **living semantic knowledge graph** that is AetherForge's single source
//! of truth. Nodes are code concepts (functions, types, modules, …); edges are
//! semantic relationships (calls, inherits, dataflow, impact, …). Source text is
//! a *projection* derived from nodes — never the other way around.
//!
//! The graph is intentionally UI- and parser-agnostic: `aether-builder` fills it
//! from tree-sitter, agents mutate it, the debugger references its nodes, and the
//! app renders projections of it. Everything else in the workspace depends on
//! this crate.

mod edge;
mod impact;
mod node;
mod query;
mod serialize;
mod similarity;

pub use edge::{Edge, EdgeKind};
pub use impact::ImpactReport;
pub use node::{Node, NodeId, NodeKind, Span};
pub use similarity::{jaccard, tokenize};

use petgraph::stable_graph::{NodeIndex, StableDiGraph};
use std::collections::HashMap;

/// Errors surfaced by graph operations.
#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error("node not found: {0:?}")]
    NodeNotFound(NodeId),
    #[error("serialization failed: {0}")]
    Serialize(String),
    #[error("deserialization failed: {0}")]
    Deserialize(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// The semantic graph.
///
/// Backed by a `petgraph::StableDiGraph` (stable indices survive removals) plus
/// an id→index map so callers reference nodes by stable [`NodeId`] without caring
/// about petgraph internals.
#[derive(Debug, Default, Clone)]
pub struct SemanticGraph {
    graph: StableDiGraph<Node, Edge>,
    index: HashMap<NodeId, NodeIndex>,
}

impl SemanticGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a node, or update it in place if its id already exists.
    /// Returns the node's id for convenient chaining.
    pub fn upsert_node(&mut self, node: Node) -> NodeId {
        let id = node.id;
        if let Some(&idx) = self.index.get(&id) {
            self.graph[idx] = node;
        } else {
            let idx = self.graph.add_node(node);
            self.index.insert(id, idx);
        }
        id
    }

    /// Add a directed edge `from -> to`. Both nodes must already exist.
    pub fn add_edge(&mut self, from: NodeId, to: NodeId, edge: Edge) -> Result<(), GraphError> {
        let a = *self
            .index
            .get(&from)
            .ok_or(GraphError::NodeNotFound(from))?;
        let b = *self.index.get(&to).ok_or(GraphError::NodeNotFound(to))?;
        self.graph.add_edge(a, b, edge);
        Ok(())
    }

    /// Remove a node and all its incident edges.
    pub fn remove_node(&mut self, id: NodeId) -> Option<Node> {
        let idx = self.index.remove(&id)?;
        self.graph.remove_node(idx)
    }

    /// Remove every edge of a given kind. Used by the project-wide call resolver
    /// to rebuild `Calls` edges from scratch after any source change.
    pub fn clear_edges_of_kind(&mut self, kind: EdgeKind) {
        let to_remove: Vec<_> = self
            .graph
            .edge_indices()
            .filter(|&e| self.graph[e].kind == kind)
            .collect();
        for e in to_remove {
            self.graph.remove_edge(e);
        }
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.index.get(&id).map(|&idx| &self.graph[idx])
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        let idx = *self.index.get(&id)?;
        Some(&mut self.graph[idx])
    }

    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.index.contains_key(&id)
    }

    /// Iterate over every node in the graph.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.graph.node_weights()
    }

    // ---- internal access for sibling modules (query / impact / serialize) ----

    pub(crate) fn raw(&self) -> &StableDiGraph<Node, Edge> {
        &self.graph
    }

    pub(crate) fn index_of(&self, id: NodeId) -> Option<NodeIndex> {
        self.index.get(&id).copied()
    }

    pub(crate) fn id_at(&self, idx: NodeIndex) -> NodeId {
        self.graph[idx].id
    }

    /// Rebuild the id→index map after a bulk deserialize.
    pub(crate) fn reindex(&mut self) {
        self.index = self
            .graph
            .node_indices()
            .map(|idx| (self.graph[idx].id, idx))
            .collect();
    }

    /// Construct directly from a petgraph (used by serialization).
    pub(crate) fn from_raw(graph: StableDiGraph<Node, Edge>) -> Self {
        let mut g = SemanticGraph {
            graph,
            index: HashMap::new(),
        };
        g.reindex();
        g
    }
}
