//! Node types for the semantic graph.
//!
//! A node is *the* representation of a code concept. Text in a file is only a
//! projection of one or more nodes. A function lives in the graph first; its
//! source text is rendered on demand.

use serde::{Deserialize, Serialize};

/// Deterministic identifier for a semantic path.
///
/// IDs are stable across reparses, body edits, and serialization as long as the
/// semantic path stays the same. A semantic rename changes the path, produces a
/// new `NodeId`, and must remap edges/references as part of the rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId(pub u64);

impl NodeId {
    /// Deterministic id derived from a fully-qualified semantic path.
    /// FNV-1a keeps this dependency-free and stable across runs/machines.
    pub fn from_path(path: &str) -> Self {
        const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut hash = FNV_OFFSET;
        for byte in path.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        NodeId(hash)
    }
}

/// What kind of code concept a node represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeKind {
    /// A file/module namespace.
    Module,
    /// A function or method.
    Function,
    /// A struct/enum/class/interface — anything that defines a type.
    Type,
    /// A field/member/property of a type.
    Field,
    /// An abstract concept extracted by an agent (e.g. "authentication"),
    /// not tied to a single syntactic construct. Enables semantic navigation.
    Concept,
    /// An external dependency / crate / package.
    Dependency,
    /// A declarative, permission-bound Girder extension recipe.
    Extension,
    /// A panel, command, or other contribution owned by an extension.
    ExtensionContribution,
}

/// A position in a source projection, used to render text and map edits back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Span {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_row: usize,
    pub start_col: usize,
}

/// A node in the semantic graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    /// Short display name, e.g. `add`.
    pub name: String,
    /// Fully-qualified semantic path, e.g. `crate::math::add`.
    pub path: String,
    /// The language this node was projected from (e.g. "rust", "python").
    pub language: String,
    /// Source file the current projection lives in, if any.
    pub file: Option<String>,
    /// Byte/row span of this node within `file`'s current text projection.
    pub span: Span,
    /// The rendered source text for this node (the projection). For a function
    /// this is its body; for a module it may be empty. The graph owns this;
    /// the editor borrows it.
    pub source: String,
    /// Free-form metadata agents attach (summaries, complexity, risk, etc.).
    pub attributes: Vec<(String, String)>,
}

impl Node {
    pub fn new(kind: NodeKind, name: impl Into<String>, path: impl Into<String>) -> Self {
        let path = path.into();
        Node {
            id: NodeId::from_path(&path),
            kind,
            name: name.into(),
            path,
            language: String::new(),
            file: None,
            span: Span::default(),
            source: String::new(),
            attributes: Vec::new(),
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = language.into();
        self
    }

    /// Read an attribute value by key, if present.
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Insert or replace an attribute. Agents use this to annotate nodes
    /// (e.g. the Documenter writes `summary`, the SecurityAuditor writes `risk`).
    pub fn set_attr(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let key = key.into();
        let value = value.into();
        if let Some(slot) = self.attributes.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            self.attributes.push((key, value));
        }
    }
}
