//! Graph-semantic refactoring operations.
//!
//! The flagship example is **rename**, which is *not* a text search/replace:
//! it re-identifies the node, remaps every incident edge to the new id, and then
//! rewrites the call sites by **following `Calls` edges** — only the functions
//! the graph knows call this one get their projected source touched. That makes
//! rename impact-aware and language-agnostic (it operates on the graph, not on
//! syntax), exactly the property a file-based editor can't offer.

use crate::{Edge, GraphError, Node, NodeId, SemanticGraph};
use petgraph::visit::EdgeRef;
use petgraph::Direction;

/// What a rename touched — surfaced to the CLI/agents so the change is auditable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameOutcome {
    pub old_path: String,
    pub new_path: String,
    pub new_id: NodeId,
    /// Paths of caller nodes whose source was rewritten (reached via `Calls`).
    pub updated_callers: Vec<String>,
}

/// Replace whole-identifier occurrences of `old` with `new` in `text`.
///
/// Boundary-aware so renaming `add` does not corrupt `address` or `padding`:
/// a match only counts when neither neighbor is an identifier character.
fn replace_identifier(text: &str, old: &str, new: &str) -> String {
    if old.is_empty() {
        return text.to_string();
    }
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i..].starts_with(old) {
            let before_ok = i == 0 || !text[..i].chars().next_back().is_some_and(is_ident);
            let after = i + old.len();
            let after_ok =
                after >= text.len() || !text[after..].chars().next().is_some_and(is_ident);
            if before_ok && after_ok {
                out.push_str(new);
                i = after;
                continue;
            }
        }
        // Advance one full UTF-8 char.
        let ch_len = text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        out.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// Derive a node's new path by swapping the trailing `::` segment for `new_name`.
fn repath(path: &str, new_name: &str) -> String {
    match path.rfind("::") {
        Some(i) => format!("{}::{new_name}", &path[..i]),
        None => new_name.to_string(),
    }
}

impl SemanticGraph {
    /// Rename a node and propagate the change across the graph.
    ///
    /// - Re-ids the node (its id is derived from its path) and moves all incident
    ///   edges to the new id, so the call graph, impact set, and any agent/debugger
    ///   references stay attached.
    /// - Rewrites the renamed node's own projected source and the source of every
    ///   **caller reached via a `Calls` edge** — semantic, not textual, scope.
    ///
    /// Returns a [`RenameOutcome`] describing what changed, or an error if the
    /// node is absent or the target name already exists.
    pub fn rename_node(&mut self, id: NodeId, new_name: &str) -> Result<RenameOutcome, GraphError> {
        let node = self.get(id).ok_or(GraphError::NodeNotFound(id))?.clone();
        let old_name = node.name.clone();
        let old_path = node.path.clone();
        let new_path = repath(&old_path, new_name);
        let new_id = NodeId::from_path(&new_path);

        if old_name == new_name {
            return Ok(RenameOutcome {
                old_path,
                new_path,
                new_id: id,
                updated_callers: Vec::new(),
            });
        }
        if new_id != id && self.contains(new_id) {
            return Err(GraphError::RenameConflict(new_id));
        }

        // 1. Snapshot every incident edge before we disturb the node. Self-loops
        //    (recursion) are captured once, on the outgoing pass.
        let idx = self.index_of(id).ok_or(GraphError::NodeNotFound(id))?;
        let mut incident: Vec<(NodeId, NodeId, Edge)> = Vec::new();
        for e in self.raw().edges_directed(idx, Direction::Outgoing) {
            incident.push((id, self.id_at(e.target()), e.weight().clone()));
        }
        for e in self.raw().edges_directed(idx, Direction::Incoming) {
            let src = self.id_at(e.source());
            if src != id {
                incident.push((src, id, e.weight().clone()));
            }
        }

        // 2. Rewrite call sites by following Calls edges — only real callers.
        let caller_ids: Vec<NodeId> = self
            .callers(id)
            .into_iter()
            .map(|n| n.id)
            .filter(|c| *c != id)
            .collect();
        let mut updated_callers = Vec::new();
        for caller in &caller_ids {
            if let Some(c) = self.get_mut(*caller) {
                let rewritten = replace_identifier(&c.source, &old_name, new_name);
                if rewritten != c.source {
                    c.source = rewritten;
                    updated_callers.push(c.path.clone());
                }
            }
        }

        // 3. Build the renamed node (new id/name/path + rewritten own source).
        let mut renamed = Node {
            id: new_id,
            name: new_name.to_string(),
            path: new_path.clone(),
            source: replace_identifier(&node.source, &old_name, new_name),
            ..node
        };
        renamed.set_attr("renamed_from", &old_path);

        // 4. Swap old -> new, then re-attach edges (remapping the old endpoint).
        self.upsert_node(renamed);
        if new_id != id {
            self.remove_node(id);
        }
        let remap = |n: NodeId| if n == id { new_id } else { n };
        for (from, to, edge) in incident {
            let _ = self.add_edge(remap(from), remap(to), edge);
        }

        updated_callers.sort();
        Ok(RenameOutcome {
            old_path,
            new_path,
            new_id,
            updated_callers,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EdgeKind, NodeKind};

    fn fn_node(name: &str, path: &str, src: &str) -> Node {
        Node::new(NodeKind::Function, name, path).with_source(src)
    }

    #[test]
    fn replace_identifier_respects_boundaries() {
        assert_eq!(replace_identifier("add(a, b)", "add", "plus"), "plus(a, b)");
        // Must NOT touch substrings.
        assert_eq!(
            replace_identifier("address = add(x)", "add", "plus"),
            "address = plus(x)"
        );
        assert_eq!(replace_identifier("padding", "add", "plus"), "padding");
    }

    #[test]
    fn rename_follows_calls_and_remaps_edges() {
        let mut g = SemanticGraph::new();
        let add = g.upsert_node(fn_node("add", "crate::math::add", "fn add(a, b) { a + b }"));
        let sum = g.upsert_node(fn_node(
            "sum_list",
            "crate::math::sum_list",
            "fn sum_list(xs) { let mut t = 0; for x in xs { t = add(t, x); } t }",
        ));
        // A function that does NOT call add but mentions the substring "add".
        let unrelated = g.upsert_node(fn_node(
            "store",
            "crate::math::store",
            "fn store(address) { address }",
        ));
        g.add_edge(sum, add, Edge::new(EdgeKind::Calls)).unwrap();

        let outcome = g.rename_node(add, "plus").unwrap();
        let new_id = NodeId::from_path("crate::math::plus");
        assert_eq!(outcome.new_id, new_id);
        assert_eq!(
            outcome.updated_callers,
            vec!["crate::math::sum_list".to_string()]
        );

        // Old id is gone; new node carries the rewritten definition.
        assert!(g.get(add).is_none());
        let plus = g.get(new_id).unwrap();
        assert!(plus.source.contains("fn plus"));
        assert_eq!(plus.attr("renamed_from"), Some("crate::math::add"));

        // The caller's source was rewritten because it's connected by a Calls edge.
        assert!(g.get(sum).unwrap().source.contains("plus(t, x)"));
        // The unrelated function (no Calls edge) is untouched — boundary-safe too.
        assert_eq!(
            g.get(unrelated).unwrap().source,
            "fn store(address) { address }"
        );

        // The Calls edge now points sum_list -> plus (edge remapped, not dropped).
        let callers: Vec<_> = g.callers(new_id).into_iter().map(|n| n.id).collect();
        assert!(callers.contains(&sum));
    }

    #[test]
    fn rename_conflict_is_rejected() {
        let mut g = SemanticGraph::new();
        let a = g.upsert_node(fn_node("a", "crate::m::a", "fn a() {}"));
        g.upsert_node(fn_node("b", "crate::m::b", "fn b() {}"));
        assert!(matches!(
            g.rename_node(a, "b"),
            Err(GraphError::RenameConflict(_))
        ));
    }
}
