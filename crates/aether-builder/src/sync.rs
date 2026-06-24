//! Keep the semantic graph in sync with source edits.
//!
//! The [`GraphBuilder`] owns one [`IncrementalParser`] per open file and the set
//! of node ids each file currently contributes. On every edit it incrementally
//! reparses, re-extracts, and *diffs* the result into the graph: new/changed
//! nodes are upserted, vanished nodes are removed, and edges are rebuilt for the
//! file. This is the machinery behind bidirectional editor⇄graph sync.

use crate::mapper::{extract, BuildOutput};
use crate::parser::{IncrementalParser, Lang};
use aether_graph::{NodeId, SemanticGraph};
use std::collections::{HashMap, HashSet};
use tree_sitter::{InputEdit, Point};

/// Per-file parsing state.
struct FileState {
    parser: IncrementalParser,
    source: String,
    /// Node ids this file currently contributes to the graph.
    owned: HashSet<NodeId>,
}

/// Incrementally maps source files into a [`SemanticGraph`].
#[derive(Default)]
pub struct GraphBuilder {
    files: HashMap<String, FileState>,
}

impl GraphBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Initial load of a file. Full parse + extract + insert.
    pub fn load_file(&mut self, graph: &mut SemanticGraph, file: &str, source: &str) {
        let Some(lang) = Lang::from_path(file) else {
            return;
        };
        let mut parser = IncrementalParser::new(lang);
        let tree = parser.parse(source);
        let out = extract(&tree, source, file, lang);
        let owned = self.apply(graph, file, &out, &HashSet::new());
        self.files.insert(
            file.to_string(),
            FileState {
                parser,
                source: source.to_string(),
                owned,
            },
        );
    }

    /// Re-sync a file after its full text changed (e.g. the editor buffer).
    /// Uses tree-sitter incremental reparse seeded with a coarse whole-buffer
    /// edit, then diffs the freshly-extracted nodes against what the file owned.
    pub fn update_file(&mut self, graph: &mut SemanticGraph, file: &str, new_source: &str) {
        let Some(lang) = Lang::from_path(file) else {
            return;
        };
        let prev_owned = self
            .files
            .get(file)
            .map(|s| s.owned.clone())
            .unwrap_or_default();

        let entry = self.files.entry(file.to_string()).or_insert_with(|| FileState {
            parser: IncrementalParser::new(lang),
            source: String::new(),
            owned: HashSet::new(),
        });

        // Inform tree-sitter where the edit happened so it reparses incrementally.
        let edit = whole_buffer_edit(&entry.source, new_source);
        entry.parser.apply_edit(&edit);
        let tree = entry.parser.reparse(new_source);
        entry.source = new_source.to_string();

        let out = extract(&tree, new_source, file, lang);
        let new_owned = self.apply(graph, file, &out, &prev_owned);
        if let Some(state) = self.files.get_mut(file) {
            state.owned = new_owned;
        }
    }

    /// Upsert all nodes/edges from `out`, then remove any previously-owned node
    /// that is no longer present. Returns the new owned-id set.
    fn apply(
        &self,
        graph: &mut SemanticGraph,
        _file: &str,
        out: &BuildOutput,
        prev_owned: &HashSet<NodeId>,
    ) -> HashSet<NodeId> {
        let new_owned: HashSet<NodeId> = out.node_ids().into_iter().collect();

        for node in &out.nodes {
            graph.upsert_node(node.clone());
        }
        for (from, to, edge) in &out.edges {
            // Both endpoints exist because defs are emitted before call edges,
            // and intra-file calls only reference ids we just inserted.
            let _ = graph.add_edge(*from, *to, edge.clone());
        }

        // Remove nodes that this file used to own but no longer does (deletions).
        for stale in prev_owned.difference(&new_owned) {
            graph.remove_node(*stale);
        }
        new_owned
    }

    /// The current text projection of a file, if loaded.
    pub fn source_of(&self, file: &str) -> Option<&str> {
        self.files.get(file).map(|s| s.source.as_str())
    }
}

/// Build a conservative [`InputEdit`] describing "the whole buffer changed".
///
/// A production editor would derive a minimal edit from the keystroke; for the
/// prototype we hand tree-sitter the changed byte range from the start of the
/// first difference, which still lets it reuse the unchanged prefix's subtree.
fn whole_buffer_edit(old: &str, new: &str) -> InputEdit {
    let common = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    let start_point = byte_to_point(old, common);
    InputEdit {
        start_byte: common,
        old_end_byte: old.len(),
        new_end_byte: new.len(),
        start_position: start_point,
        old_end_position: byte_to_point(old, old.len()),
        new_end_position: byte_to_point(new, new.len()),
    }
}

fn byte_to_point(text: &str, byte: usize) -> Point {
    let mut row = 0;
    let mut col = 0;
    for (i, c) in text.char_indices() {
        if i >= byte {
            break;
        }
        if c == '\n' {
            row += 1;
            col = 0;
        } else {
            col += c.len_utf8();
        }
    }
    Point::new(row, col)
}
