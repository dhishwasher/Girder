//! Keep the semantic graph in sync with source edits.
//!
//! The [`GraphBuilder`] owns one [`IncrementalParser`] per open file and the set
//! of node ids each file currently contributes. On every edit it incrementally
//! reparses, re-extracts, and *diffs* the result into the graph: new/changed
//! nodes are upserted, vanished nodes are removed, and edges are rebuilt for the
//! file. This is the machinery behind bidirectional editor⇄graph sync.

use crate::mapper::{extract, module_path_for, BuildOutput, CallRef, CallTargetRef, InheritRef};
use crate::parser::{IncrementalParser, Lang};
use aether_graph::{Edge, EdgeKind, NodeId, NodeKind, SemanticGraph};
use std::collections::{HashMap, HashSet};
use tree_sitter::{InputEdit, Point};

/// Per-file parsing state.
struct FileState {
    parser: IncrementalParser,
    source: String,
    /// Node ids this file currently contributes to the graph.
    owned: HashSet<NodeId>,
    /// Unresolved call references found in this file, for the project resolver.
    calls: Vec<CallRef>,
    /// Unresolved inheritance references found in this file.
    inherits: Vec<InheritRef>,
}

/// The module path that owns a node, derived from its full path:
/// `crate::math::add` -> `crate::math`.
fn module_of(path: &str) -> String {
    match path.rfind("::") {
        Some(i) => path[..i].to_string(),
        None => path.to_string(),
    }
}

fn source_module(file: Option<&str>, path: &str) -> String {
    file.map(module_path_for).unwrap_or_else(|| module_of(path))
}

fn normalized_symbol(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn qualifier_tail(qualifier: &str) -> &str {
    qualifier
        .rsplit(['.', ':'])
        .find(|part| !part.is_empty())
        .unwrap_or(qualifier)
        .trim()
}

fn qualifier_matches_owner(qualifier: &str, owner: &str) -> bool {
    let hint = normalized_symbol(qualifier_tail(qualifier));
    if hint.len() < 3 || matches!(hint.as_str(), "self" | "cls") {
        return false;
    }
    let owner = normalized_symbol(owner.rsplit("::").next().unwrap_or(owner));
    owner == hint || owner.ends_with(&hint)
}

struct FunctionCandidate {
    source_module: String,
    owner: String,
    id: NodeId,
    return_type: Option<String>,
}

fn only_candidate<'a>(
    mut candidates: impl Iterator<Item = &'a FunctionCandidate>,
) -> Option<&'a FunctionCandidate> {
    let first = candidates.next()?;
    candidates.next().is_none().then_some(first)
}

fn select_candidate<'a>(
    candidates: &'a [FunctionCandidate],
    caller_source_module: &str,
    caller_owner: &str,
    qualifier: Option<&str>,
    receiver_type: Option<&str>,
) -> Option<&'a FunctionCandidate> {
    if let Some(qualifier) = qualifier {
        if matches!(qualifier_tail(qualifier), "self" | "Self" | "cls") {
            only_candidate(
                candidates
                    .iter()
                    .filter(|candidate| candidate.owner == caller_owner),
            )
        } else {
            let receiver_hint = receiver_type.unwrap_or(qualifier);
            only_candidate(
                candidates
                    .iter()
                    .filter(|candidate| qualifier_matches_owner(receiver_hint, &candidate.owner)),
            )
        }
    } else {
        only_candidate(candidates.iter().filter(|candidate| {
            candidate.source_module == caller_source_module
                && candidate.owner == candidate.source_module
        }))
        .or_else(|| {
            only_candidate(
                candidates
                    .iter()
                    .filter(|candidate| candidate.source_module == caller_source_module),
            )
        })
        .or_else(|| (candidates.len() == 1).then(|| &candidates[0]))
    }
}

fn factory_return_type<'a>(
    factory: &CallTargetRef,
    by_name: &'a HashMap<String, Vec<FunctionCandidate>>,
    caller_source_module: &str,
    caller_owner: &str,
) -> Option<&'a str> {
    let candidates = by_name.get(&factory.callee)?;
    select_candidate(
        candidates,
        caller_source_module,
        caller_owner,
        factory.qualifier.as_deref(),
        factory.receiver_type.as_deref(),
    )?
    .return_type
    .as_deref()
}

fn return_type_matches_owner(return_type: &str, owner: &str) -> bool {
    let owner = normalized_symbol(owner.rsplit("::").next().unwrap_or(owner));
    return_type
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .map(normalized_symbol)
        .filter(|token| token.len() >= 3)
        .any(|token| owner == token)
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
                calls: out.calls.clone(),
                inherits: out.inherits.clone(),
            },
        );
        self.resolve_calls(graph);
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

        let entry = self
            .files
            .entry(file.to_string())
            .or_insert_with(|| FileState {
                parser: IncrementalParser::new(lang),
                source: String::new(),
                owned: HashSet::new(),
                calls: Vec::new(),
                inherits: Vec::new(),
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
            state.calls = out.calls.clone();
            state.inherits = out.inherits.clone();
        }
        self.resolve_calls(graph);
    }

    /// Project-wide call resolution. Rebuilds **all** `Calls` edges from the
    /// accumulated unresolved references against a whole-graph symbol index, so a
    /// call links to its callee even when the callee lives in another file. When
    /// a name is ambiguous, a same-module definition wins; otherwise a unique
    /// global match is used, and truly ambiguous names are left unlinked.
    pub fn resolve_calls(&self, graph: &mut SemanticGraph) {
        let source_owned: HashSet<NodeId> = self
            .files
            .values()
            .flat_map(|state| state.owned.iter().copied())
            .collect();
        let graph_owned_calls: Vec<_> = graph
            .edge_records()
            .into_iter()
            .filter(|(from, to, edge)| {
                edge.kind == EdgeKind::Calls
                    && (!source_owned.contains(from) || !source_owned.contains(to))
            })
            .collect();
        graph.clear_edges_of_kind(EdgeKind::Calls);

        // name -> [(source module, lexical owner, function id)]
        let mut by_name: HashMap<String, Vec<FunctionCandidate>> = HashMap::new();
        for n in graph.query_by_kind(NodeKind::Function) {
            by_name
                .entry(n.name.clone())
                .or_default()
                .push(FunctionCandidate {
                    source_module: source_module(n.file.as_deref(), &n.path),
                    owner: module_of(&n.path),
                    id: n.id,
                    return_type: n.attr("return_type").map(str::to_string),
                });
        }

        let mut added: HashSet<(NodeId, NodeId)> = HashSet::new();
        for state in self.files.values() {
            for call in &state.calls {
                let (caller_source_module, caller_owner) = match graph.get(call.caller) {
                    Some(node) => (
                        source_module(node.file.as_deref(), &node.path),
                        module_of(&node.path),
                    ),
                    None => continue,
                };
                let Some(candidates) = by_name.get(&call.callee) else {
                    continue;
                };
                let factory_return = call.receiver_factory.as_ref().and_then(|factory| {
                    factory_return_type(factory, &by_name, &caller_source_module, &caller_owner)
                });
                let chosen = if let Some(return_type) = factory_return {
                    only_candidate(candidates.iter().filter(|candidate| {
                        return_type_matches_owner(return_type, &candidate.owner)
                    }))
                } else {
                    select_candidate(
                        candidates,
                        &caller_source_module,
                        &caller_owner,
                        call.qualifier.as_deref(),
                        call.receiver_type.as_deref(),
                    )
                };
                if let Some(candidate) = chosen {
                    if candidate.id != call.caller && added.insert((call.caller, candidate.id)) {
                        let _ =
                            graph.add_edge(call.caller, candidate.id, Edge::new(EdgeKind::Calls));
                    }
                }
            }
        }
        for (from, to, edge) in graph_owned_calls {
            if graph.contains(from) && graph.contains(to) {
                let _ = graph.add_edge(from, to, edge);
            }
        }

        self.resolve_inherits(graph);
    }

    /// Project-wide inheritance resolution. Rebuilds **all** `Inherits` edges
    /// from accumulated references against a whole-graph *type* index, so a
    /// Python subclass or Rust trait impl links to its base even across files.
    /// Same-module definitions win ties; otherwise a unique global match is used.
    fn resolve_inherits(&self, graph: &mut SemanticGraph) {
        let source_owned: HashSet<NodeId> = self
            .files
            .values()
            .flat_map(|state| state.owned.iter().copied())
            .collect();
        let graph_owned_inherits: Vec<_> = graph
            .edge_records()
            .into_iter()
            .filter(|(from, to, edge)| {
                edge.kind == EdgeKind::Inherits
                    && (!source_owned.contains(from) || !source_owned.contains(to))
            })
            .collect();
        graph.clear_edges_of_kind(EdgeKind::Inherits);

        // name -> [(owning module, type id)]
        let mut by_type: HashMap<String, Vec<(String, NodeId)>> = HashMap::new();
        for n in graph.query_by_kind(NodeKind::Type) {
            by_type
                .entry(n.name.clone())
                .or_default()
                .push((module_of(&n.path), n.id));
        }

        let mut added: HashSet<(NodeId, NodeId)> = HashSet::new();
        for state in self.files.values() {
            for inh in &state.inherits {
                let sub_module = match graph.get(inh.sub) {
                    Some(node) => module_of(&node.path),
                    None => continue,
                };
                let Some(candidates) = by_type.get(&inh.base) else {
                    continue;
                };
                let chosen = candidates.iter().find(|(m, _)| *m == sub_module).or(
                    if candidates.len() == 1 {
                        candidates.first()
                    } else {
                        None
                    },
                );
                if let Some((_, base_id)) = chosen {
                    if *base_id != inh.sub && added.insert((inh.sub, *base_id)) {
                        let _ = graph.add_edge(inh.sub, *base_id, Edge::new(EdgeKind::Inherits));
                    }
                }
            }
        }
        for (from, to, edge) in graph_owned_inherits {
            if graph.contains(from) && graph.contains(to) {
                let _ = graph.add_edge(from, to, edge);
            }
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

        let stale_projection_edges: Vec<_> = graph
            .edges()
            .into_iter()
            .filter(|(from, _, kind)| {
                prev_owned.contains(from) && matches!(kind, EdgeKind::Contains | EdgeKind::DataFlow)
            })
            .collect();
        for (from, to, kind) in stale_projection_edges {
            graph.remove_edge(from, to, kind);
        }

        for node in &out.nodes {
            graph.upsert_projection_node(node.clone());
        }
        for (from, to, edge) in &out.edges {
            // These are Contains edges (module->fn/type, type->field); both
            // endpoints were just upserted. Calls are resolved project-wide later.
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
