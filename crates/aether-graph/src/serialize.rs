//! Serialization to the compact `.aether` project format.
//!
//! Because the graph *is* the project, an `.aether` file is the whole project:
//! every node carries its own source projection, so files on disk are optional
//! caches. Two encodings are offered:
//!   * **RON** (`.aether`) — human-diffable, plays nicely with git/code review.
//!   * **bincode** (`.aetherb`) — compact/fast for large projects and autosave.

use crate::{GraphError, SemanticGraph};
use petgraph::stable_graph::StableDiGraph;
use serde::{Deserialize, Serialize};
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
            graph: self.raw().clone(),
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
            graph: self.raw().clone(),
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
