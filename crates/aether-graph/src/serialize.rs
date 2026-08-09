//! Serialization to the compact `.aether` project format.
//!
//! Because the graph *is* the project, an `.aether` file is the whole project:
//! every node carries its own source projection, so files on disk are optional
//! caches. Two encodings are offered:
//!   * **RON** (`.aether`) — human-diffable, plays nicely with git/code review.
//!   * **bincode** (`.aetherb`) — compact/fast for large projects and autosave.

use crate::{GraphError, SemanticGraph};
use petgraph::stable_graph::StableDiGraph;
use petgraph::visit::{EdgeRef, IntoEdgeReferences};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

/// Wire format. We serialize the raw petgraph (which derives Serialize via the
/// `serde-1` feature) plus a small header for forward-compat versioning.
#[derive(Serialize, Deserialize)]
struct AetherFile {
    magic: String,
    version: u32,
    graph: StableDiGraph<crate::Node, crate::Edge>,
}

const MAGIC: &str = "BITCODE";
const LEGACY_MAGIC: &str = "AETHERFORGE";
const VERSION: u32 = 1;

impl SemanticGraph {
    /// Encode to pretty RON text.
    pub fn to_ron(&self) -> Result<String, GraphError> {
        let file = AetherFile {
            magic: MAGIC.to_string(),
            version: VERSION,
            graph: canonical_graph(self)?,
        };
        ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default())
            .map_err(|e| GraphError::Serialize(e.to_string()))
    }

    /// Decode from RON text, rebuilding the id index.
    pub fn from_ron(text: &str) -> Result<Self, GraphError> {
        let file: AetherFile =
            ron::from_str(text).map_err(|e| GraphError::Deserialize(e.to_string()))?;
        validate_header(&file)?;
        Ok(SemanticGraph::from_raw(file.graph))
    }

    /// Encode to compact bincode bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, GraphError> {
        let file = AetherFile {
            magic: MAGIC.to_string(),
            version: VERSION,
            graph: canonical_graph(self)?,
        };
        bincode::serialize(&file).map_err(|e| GraphError::Serialize(e.to_string()))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, GraphError> {
        let file: AetherFile =
            bincode::deserialize(bytes).map_err(|e| GraphError::Deserialize(e.to_string()))?;
        validate_header(&file)?;
        Ok(SemanticGraph::from_raw(file.graph))
    }

    /// Save to a path, picking RON or bincode by extension (`.aetherb` = binary).
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), GraphError> {
        let path = path.as_ref();
        if path.extension().and_then(|e| e.to_str()) == Some("aetherb") {
            std::fs::write(path, self.to_bytes()?)?;
        } else {
            std::fs::write(path, self.to_ron()?)?;
        }
        Ok(())
    }

    /// Load from a path, picking the decoder by extension.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, GraphError> {
        let path = path.as_ref();
        if path.extension().and_then(|e| e.to_str()) == Some("aetherb") {
            SemanticGraph::from_bytes(&std::fs::read(path)?)
        } else {
            SemanticGraph::from_ron(&std::fs::read_to_string(path)?)
        }
    }
}

/// Rebuild the graph in semantic order before persistence. `StableDiGraph`'s
/// wire representation preserves internal slot and insertion order, which is
/// intentionally irrelevant to graph meaning and can vary after incremental
/// edits, randomized map traversal, or parallel derived-edge computation.
fn canonical_graph(
    graph: &SemanticGraph,
) -> Result<StableDiGraph<crate::Node, crate::Edge>, GraphError> {
    let mut nodes = graph
        .raw()
        .node_indices()
        .map(|index| {
            let mut node = graph.raw()[index].clone();
            node.attributes.sort_by(|left, right| left.0.cmp(&right.0));
            (index, node)
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        left.1
            .path
            .cmp(&right.1.path)
            .then_with(|| left.1.id.cmp(&right.1.id))
    });

    let mut ids = BTreeSet::new();
    let mut canonical = StableDiGraph::new();
    let mut canonical_indices = HashMap::new();
    let mut canonical_order = HashMap::new();
    for (order, (original, node)) in nodes.into_iter().enumerate() {
        if !ids.insert(node.id) {
            return Err(GraphError::Serialize(format!(
                "duplicate node id in semantic graph: {:?}",
                node.id
            )));
        }
        if node
            .attributes
            .windows(2)
            .any(|pair| pair[0].0 == pair[1].0)
        {
            return Err(GraphError::Serialize(format!(
                "duplicate attribute key on semantic node {}",
                node.path
            )));
        }
        let index = canonical.add_node(node);
        canonical_indices.insert(original, index);
        canonical_order.insert(original, order);
    }

    let mut edges = graph
        .raw()
        .edge_references()
        .map(|edge| (edge.source(), edge.target(), edge.weight().clone()))
        .collect::<Vec<_>>();
    edges.sort_by_key(|(source, target, edge)| {
        (
            canonical_order[source],
            canonical_order[target],
            edge.kind,
            edge.weight.to_bits(),
        )
    });
    let mut relationships = BTreeSet::new();
    for (source, target, edge) in edges {
        let source_order = canonical_order[&source];
        let target_order = canonical_order[&target];
        if !relationships.insert((source_order, target_order, edge.kind)) {
            return Err(GraphError::Serialize(format!(
                "duplicate {:?} edge in semantic graph",
                edge.kind
            )));
        }
        canonical.add_edge(canonical_indices[&source], canonical_indices[&target], edge);
    }
    Ok(canonical)
}

fn validate_header(file: &AetherFile) -> Result<(), GraphError> {
    if file.magic != MAGIC && file.magic != LEGACY_MAGIC {
        return Err(GraphError::Deserialize("bad magic".into()));
    }
    if file.version != VERSION {
        return Err(GraphError::Deserialize(format!(
            "unsupported graph version {}; expected {VERSION}",
            file.version
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{Edge, EdgeKind, Node, NodeKind, SemanticGraph};

    /// Build a tiny graph: module `math` containing `add`, which is called by
    /// `sum_list`, whose result flows into `main`.
    fn fixture() -> SemanticGraph {
        let mut g = SemanticGraph::new();
        let math = g.upsert_node(Node::new(NodeKind::Module, "math", "crate::math"));
        let add = g.upsert_node(
            Node::new(NodeKind::Function, "add", "crate::math::add")
                .with_source("fn add(a: i64, b: i64) -> i64 { a + b }")
                .with_language("rust"),
        );
        let sum = g.upsert_node(Node::new(
            NodeKind::Function,
            "sum_list",
            "crate::math::sum_list",
        ));
        let main = g.upsert_node(Node::new(NodeKind::Function, "main", "crate::main"));

        g.add_edge(math, add, Edge::new(EdgeKind::Contains))
            .unwrap();
        g.add_edge(sum, add, Edge::new(EdgeKind::Calls)).unwrap();
        g.add_edge(main, sum, Edge::new(EdgeKind::Calls)).unwrap();
        g.add_edge(sum, main, Edge::new(EdgeKind::DataFlow))
            .unwrap();
        g
    }

    fn reordered_fixture(reverse: bool) -> SemanticGraph {
        let mut add = Node::new(NodeKind::Function, "add", "crate::math::add")
            .with_source("fn add(a: i64, b: i64) -> i64 { a + b }")
            .with_language("rust");
        if reverse {
            add.set_attr("summary", "adds two integers");
            add.set_attr("risk", "low");
        } else {
            add.set_attr("risk", "low");
            add.set_attr("summary", "adds two integers");
        }
        let mut nodes = vec![
            Node::new(NodeKind::Module, "math", "crate::math"),
            add,
            Node::new(NodeKind::Function, "sum_list", "crate::math::sum_list"),
            Node::new(NodeKind::Function, "main", "crate::main"),
        ];
        if reverse {
            nodes.reverse();
        }

        let mut graph = SemanticGraph::new();
        for node in nodes {
            graph.upsert_node(node);
        }
        let math = crate::NodeId::from_path("crate::math");
        let add = crate::NodeId::from_path("crate::math::add");
        let sum = crate::NodeId::from_path("crate::math::sum_list");
        let main = crate::NodeId::from_path("crate::main");
        let mut edges = vec![
            (math, add, Edge::new(EdgeKind::Contains)),
            (sum, add, Edge::new(EdgeKind::Calls)),
            (main, sum, Edge::new(EdgeKind::Calls)),
            (sum, main, Edge::new(EdgeKind::DataFlow)),
        ];
        if reverse {
            edges.reverse();
        }
        for (source, target, edge) in edges {
            graph.add_edge(source, target, edge).unwrap();
        }
        graph
    }

    #[test]
    fn ron_round_trip_preserves_graph() {
        let g = fixture();
        let text = g.to_ron().unwrap();
        let back = SemanticGraph::from_ron(&text).unwrap();
        assert_eq!(back.node_count(), g.node_count());
        assert_eq!(back.edge_count(), g.edge_count());
        // Stable ids survive the round trip, so lookups still resolve.
        let add = back.find_by_path("crate::math::add").unwrap();
        assert_eq!(add.source, "fn add(a: i64, b: i64) -> i64 { a + b }");
    }

    #[test]
    fn bincode_round_trip_preserves_graph() {
        let g = fixture();
        let bytes = g.to_bytes().unwrap();
        let back = SemanticGraph::from_bytes(&bytes).unwrap();
        assert_eq!(back.node_count(), g.node_count());
        assert_eq!(back.edge_count(), g.edge_count());
    }

    #[test]
    fn serialization_is_canonical_across_insertion_and_attribute_order() {
        let forward = reordered_fixture(false);
        let reverse = reordered_fixture(true);
        assert_eq!(forward.to_ron().unwrap(), reverse.to_ron().unwrap());
        assert_eq!(forward.to_bytes().unwrap(), reverse.to_bytes().unwrap());
    }

    #[test]
    fn serialization_compacts_graph_mutation_history() {
        let pristine = reordered_fixture(false);
        let mut mutated = reordered_fixture(false);
        let temporary = mutated.upsert_node(Node::new(
            NodeKind::Function,
            "temporary",
            "crate::temporary",
        ));
        let add = crate::NodeId::from_path("crate::math::add");
        mutated
            .add_edge(temporary, add, Edge::new(EdgeKind::Impacts))
            .unwrap();
        mutated.remove_node(temporary).unwrap();
        assert_eq!(pristine.to_ron().unwrap(), mutated.to_ron().unwrap());
        assert_eq!(pristine.to_bytes().unwrap(), mutated.to_bytes().unwrap());
    }

    #[test]
    fn serialization_rejects_duplicate_attribute_keys_and_typed_edges() {
        let mut duplicate_attributes = fixture();
        duplicate_attributes
            .get_mut(crate::NodeId::from_path("crate::math::add"))
            .unwrap()
            .attributes
            .extend([
                ("risk".to_string(), "low".to_string()),
                ("risk".to_string(), "high".to_string()),
            ]);
        assert!(duplicate_attributes.to_ron().is_err());
        assert!(duplicate_attributes.to_bytes().is_err());

        let mut raw = petgraph::stable_graph::StableDiGraph::new();
        let source = raw.add_node(Node::new(NodeKind::Function, "source", "crate::source"));
        let target = raw.add_node(Node::new(NodeKind::Function, "target", "crate::target"));
        raw.add_edge(source, target, Edge::new(EdgeKind::Calls));
        raw.add_edge(source, target, Edge::new(EdgeKind::Calls));
        let duplicate_edges = SemanticGraph::from_raw(raw);
        assert!(duplicate_edges.to_ron().is_err());
        assert!(duplicate_edges.to_bytes().is_err());
    }

    #[test]
    fn rejects_unknown_format_version() {
        let text = fixture().to_ron().unwrap();
        let incompatible = text.replacen("version: 1", "version: 999", 1);
        let error = SemanticGraph::from_ron(&incompatible).unwrap_err();
        assert!(error.to_string().contains("unsupported graph version"));
    }

    #[test]
    fn accepts_legacy_magic_and_writes_bit_code_magic() {
        let text = fixture().to_ron().unwrap();
        assert!(text.contains("BITCODE"));
        let legacy = text.replacen("BITCODE", "AETHERFORGE", 1);
        assert!(SemanticGraph::from_ron(&legacy).is_ok());
    }

    #[test]
    fn impact_propagates_backwards_through_calls() {
        let g = fixture();
        let add = crate::NodeId::from_path("crate::math::add");
        let report = g.impact_of(add);
        // Changing `add` impacts `sum_list` (1 hop) and `main` (2 hops).
        let sum = crate::NodeId::from_path("crate::math::sum_list");
        let main = crate::NodeId::from_path("crate::main");
        assert_eq!(report.affected.get(&sum), Some(&1));
        assert_eq!(report.affected.get(&main), Some(&2));
        // The module that merely *contains* `add` is NOT impacted by a body change.
        let math = crate::NodeId::from_path("crate::math");
        assert!(!report.affected.contains_key(&math));
    }

    #[test]
    fn impact_ranked_is_nearest_first() {
        let g = fixture();
        let report = g.impact_of(crate::NodeId::from_path("crate::math::add"));
        let ranked = report.ranked();
        assert_eq!(ranked.first().map(|(_, d)| *d), Some(1));
    }
}
