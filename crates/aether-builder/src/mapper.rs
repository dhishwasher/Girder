//! Map a tree-sitter syntax tree into semantic-graph nodes and edges.
//!
//! This is deliberately a *pragmatic* extractor, not a full type checker: it
//! recovers modules, functions, types, fields, and intra-file call edges — more
//! than enough to demonstrate semantic navigation, impact analysis, and agent
//! edits. Cross-file resolution is an EXTENSION POINT (see `resolve` notes).

use crate::parser::Lang;
use aether_graph::{Edge, EdgeKind, Node, NodeId, NodeKind, Span};
use std::collections::HashMap;
use tree_sitter::{Node as TsNode, Tree};

/// Everything extracted from a single file: the nodes to upsert and the edges
/// to add between them (by stable id).
#[derive(Debug, Default)]
pub struct BuildOutput {
    pub nodes: Vec<Node>,
    pub edges: Vec<(NodeId, NodeId, Edge)>,
}

impl BuildOutput {
    /// Ids of all nodes produced — used by the incremental sync to figure out
    /// which previously-known nodes for a file have disappeared.
    pub fn node_ids(&self) -> Vec<NodeId> {
        self.nodes.iter().map(|n| n.id).collect()
    }
}

/// Derive a module path from a file path: `src/math.rs` -> `crate::math`.
pub fn module_path_for(file: &str) -> String {
    let stem = file
        .rsplit('/')
        .next()
        .and_then(|f| f.split('.').next())
        .unwrap_or("root");
    format!("crate::{stem}")
}

/// Extract a [`BuildOutput`] from a parsed tree.
pub fn extract(tree: &Tree, source: &str, file: &str, lang: Lang) -> BuildOutput {
    let module = module_path_for(file);
    let mut out = BuildOutput::default();

    // The module node itself.
    let module_node = Node::new(NodeKind::Module, last_segment(&module), module.clone())
        .with_language(lang.name());
    let module_id = module_node.id;
    out.nodes.push(Node {
        file: Some(file.to_string()),
        ..module_node
    });

    // Pass 1: collect definitions and a name -> id map for call resolution.
    let mut name_to_id: HashMap<String, NodeId> = HashMap::new();
    let root = tree.root_node();
    collect_defs(
        root,
        source,
        file,
        lang,
        &module,
        module_id,
        &mut out,
        &mut name_to_id,
    );

    // Pass 2: resolve intra-file calls into Calls edges.
    collect_calls(root, source, lang, &module, &name_to_id, &mut out);

    out
}

fn last_segment(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
}

fn node_text<'a>(node: TsNode, source: &'a str) -> &'a str {
    &source[node.start_byte()..node.end_byte()]
}

fn span_of(node: TsNode) -> Span {
    let start = node.start_position();
    Span {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        start_row: start.row,
        start_col: start.column,
    }
}

/// tree-sitter node kinds that define a function, per language.
fn is_function_kind(lang: Lang, kind: &str) -> bool {
    match lang {
        Lang::Rust => kind == "function_item",
        Lang::Python => kind == "function_definition",
    }
}

/// tree-sitter node kinds that define a type, per language.
fn is_type_kind(lang: Lang, kind: &str) -> bool {
    match lang {
        Lang::Rust => matches!(kind, "struct_item" | "enum_item" | "trait_item"),
        Lang::Python => kind == "class_definition",
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_defs(
    node: TsNode,
    source: &str,
    file: &str,
    lang: Lang,
    module: &str,
    module_id: NodeId,
    out: &mut BuildOutput,
    name_to_id: &mut HashMap<String, NodeId>,
) {
    let kind = node.kind();

    if is_function_kind(lang, kind) {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = node_text(name_node, source).to_string();
            let path = format!("{module}::{name}");
            let id = NodeId::from_path(&path);
            let mut n = Node::new(NodeKind::Function, &name, &path)
                .with_language(lang.name())
                .with_source(node_text(node, source));
            n.file = Some(file.to_string());
            n.span = span_of(node);
            out.nodes.push(n);
            out.edges
                .push((module_id, id, Edge::new(EdgeKind::Contains)));
            name_to_id.insert(name, id);
        }
    } else if is_type_kind(lang, kind) {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = node_text(name_node, source).to_string();
            let path = format!("{module}::{name}");
            let id = NodeId::from_path(&path);
            let mut n = Node::new(NodeKind::Type, &name, &path)
                .with_language(lang.name())
                .with_source(node_text(node, source));
            n.file = Some(file.to_string());
            n.span = span_of(node);
            out.nodes.push(n);
            out.edges
                .push((module_id, id, Edge::new(EdgeKind::Contains)));
            name_to_id.insert(name, id);
            extract_fields(node, source, file, lang, &path, id, out);
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_defs(child, source, file, lang, module, module_id, out, name_to_id);
    }
}

/// Pull fields out of a Rust struct / Python class body as `Field` nodes.
fn extract_fields(
    type_node: TsNode,
    source: &str,
    file: &str,
    lang: Lang,
    type_path: &str,
    type_id: NodeId,
    out: &mut BuildOutput,
) {
    let mut cursor = type_node.walk();
    for descendant in descendants(type_node, &mut cursor) {
        let is_field = match lang {
            Lang::Rust => descendant.kind() == "field_declaration",
            // For Python we treat assignments in the class body as fields.
            Lang::Python => descendant.kind() == "assignment",
        };
        if !is_field {
            continue;
        }
        let field_name = descendant
            .child_by_field_name("name")
            .or_else(|| descendant.child_by_field_name("left"))
            .map(|n| node_text(n, source).to_string());
        if let Some(fname) = field_name {
            let path = format!("{type_path}::{fname}");
            let mut n = Node::new(NodeKind::Field, &fname, &path).with_language(lang.name());
            n.file = Some(file.to_string());
            n.span = span_of(descendant);
            let id = n.id;
            out.nodes.push(n);
            out.edges.push((type_id, id, Edge::new(EdgeKind::Contains)));
        }
    }
}

fn collect_calls(
    node: TsNode,
    source: &str,
    lang: Lang,
    module: &str,
    name_to_id: &HashMap<String, NodeId>,
    out: &mut BuildOutput,
) {
    // Track the enclosing function as we descend so calls attach to a caller.
    fn walk(
        node: TsNode,
        source: &str,
        lang: Lang,
        module: &str,
        current_fn: Option<NodeId>,
        name_to_id: &HashMap<String, NodeId>,
        out: &mut BuildOutput,
    ) {
        let mut current = current_fn;
        if is_function_kind(lang, node.kind()) {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source);
                current = Some(NodeId::from_path(&format!("{module}::{name}")));
            }
        }

        let call_kind = match lang {
            Lang::Rust => "call_expression",
            Lang::Python => "call",
        };
        if node.kind() == call_kind {
            if let (Some(caller), Some(callee)) = (current, callee_name(node, source, lang)) {
                if let Some(&callee_id) = name_to_id.get(&callee) {
                    if callee_id != caller {
                        out.edges
                            .push((caller, callee_id, Edge::new(EdgeKind::Calls)));
                    }
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk(child, source, lang, module, current, name_to_id, out);
        }
    }

    walk(node, source, lang, module, None, name_to_id, out);
}

/// Best-effort callee name: the trailing identifier of the call target.
fn callee_name(call: TsNode, source: &str, _lang: Lang) -> Option<String> {
    let func = call.child_by_field_name("function")?;
    // For `a.b.c()` / `path::to::f()` take the last identifier-ish leaf.
    let text = node_text(func, source);
    let last = text
        .rsplit(|c| c == '.' || c == ':')
        .next()
        .unwrap_or(text)
        .trim();
    if last.is_empty() {
        None
    } else {
        Some(last.to_string())
    }
}

/// Iterative pre-order descendant collection (avoids borrow gymnastics).
fn descendants<'a>(node: TsNode<'a>, cursor: &mut tree_sitter::TreeCursor<'a>) -> Vec<TsNode<'a>> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        for child in n.children(cursor) {
            out.push(child);
            stack.push(child);
        }
    }
    out
}
