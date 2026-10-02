//! Restricted lexical proofs, separate from name-based call resolution.

use super::{node_text, BuildOutput};
use aether_graph::{NodeId, NodeKind};
use std::collections::HashMap;
use tree_sitter::Node as TsNode;

/// Call only after the shared parse, identity, transformation, and module gates.
pub(super) fn proven_functions(
    root: TsNode<'_>,
    syntax: &[TsNode<'_>],
    source: &str,
    out: &BuildOutput,
) -> HashMap<String, NodeId> {
    let mut proven = HashMap::new();
    if syntax.iter().any(|n| {
        n.kind() == "with_statement"
            || (n.kind().contains("identifier")
                && (node_text(*n, source) == "eval" || node_text(*n, source).contains('\\')))
    }) {
        // Direct eval and escaped spellings can hide writes to an otherwise
        // unique lexical binding. Refuse until those forms have their own proof.
        return proven;
    }

    // Index once: large compiler files contain thousands of nested declarations.
    let mut uses = HashMap::<&str, Vec<TsNode<'_>>>::new();
    for n in syntax.iter().filter(|n| n.kind().contains("identifier")) {
        uses.entry(node_text(*n, source)).or_default().push(*n);
    }
    let mut targets = HashMap::<(usize, usize), Vec<NodeId>>::new();
    for n in out.nodes.iter().filter(|n| n.kind == NodeKind::Function) {
        targets
            .entry((n.span.start_byte, n.span.end_byte))
            .or_default()
            .push(n.id);
    }

    for declaration in syntax.iter().filter(|n| n.kind() == "function_declaration") {
        if declaration.child_by_field_name("body").is_none() {
            continue;
        }
        let Some(name_node) = declaration.child_by_field_name("name") else {
            continue;
        };
        let Some(scope) = binding_scope(*declaration, root, source) else {
            continue;
        };
        let Some(nodes) = targets.get(&(declaration.start_byte(), declaration.end_byte())) else {
            continue;
        };
        let [target] = nodes.as_slice() else {
            continue;
        };
        let name = node_text(name_node, source);
        let clean = uses.get(name).is_some_and(|occurrences| {
            occurrences.iter().all(|n| {
                n.id() == name_node.id()
                    || (n.kind() == "identifier"
                        && n.parent().is_some_and(|p| {
                            p.kind() == "call_expression"
                                && p.child_by_field_name("function")
                                    .is_some_and(|callee| callee.id() == n.id())
                        })
                        && within_scope(*n, scope))
            })
        });
        if clean {
            proven.insert(name.to_string(), *target);
        }
    }
    proven
}

fn binding_scope<'tree>(
    declaration: TsNode<'tree>,
    root: TsNode<'tree>,
    source: &str,
) -> Option<TsNode<'tree>> {
    let parent = declaration.parent()?;
    if parent.id() == root.id()
        || (parent.kind() == "export_statement"
            && parent.parent().is_some_and(|p| p.id() == root.id()))
    {
        return Some(root);
    }
    if parent.kind() != "statement_block" {
        return None;
    }
    let owner = parent.parent()?;
    if !matches!(
        owner.kind(),
        "function_declaration" | "function_expression" | "arrow_function" | "method_definition"
    ) || owner.child_by_field_name("body")?.id() != parent.id()
    {
        return None;
    }
    if owner.kind() == "method_definition"
        && (owner
            .child_by_field_name("name")
            .is_some_and(|name| node_text(name, source) == "constructor")
            || (0..owner.child_count()).any(|i| owner.child(i).is_some_and(|c| c.kind() == "*")))
    {
        return None;
    }
    Some(parent)
}

fn within_scope(mut node: TsNode<'_>, scope: TsNode<'_>) -> bool {
    loop {
        if node.id() == scope.id() {
            return true;
        }
        let Some(parent) = node.parent() else {
            return false;
        };
        node = parent;
    }
}
