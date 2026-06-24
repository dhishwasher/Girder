//! Map a tree-sitter syntax tree into semantic-graph nodes and edges.
//!
//! This is deliberately a *pragmatic* extractor, not a full type checker: it
//! recovers modules, functions, types, fields, and call *references*. Call sites
//! are emitted unresolved (`caller id` + `callee name`); the project-wide
//! resolver in `sync` turns them into `Calls` edges, which is what enables
//! **cross-file** call graphs (a callee defined in another module still links).

use crate::parser::Lang;
use aether_graph::{Edge, EdgeKind, Node, NodeId, NodeKind, Span};
use tree_sitter::{Node as TsNode, Tree};

/// An unresolved call site: `caller` invokes something named `callee` (the
/// trailing identifier of the call target). Resolved later against the whole
/// graph so cross-module calls link correctly.
#[derive(Debug, Clone)]
pub struct CallRef {
    pub caller: NodeId,
    pub callee: String,
}

/// An unresolved inheritance: type `sub` inherits/implements something named
/// `base` (a superclass in Python, a trait in Rust). Resolved project-wide like
/// calls so `Inherits` edges link across files.
#[derive(Debug, Clone)]
pub struct InheritRef {
    pub sub: NodeId,
    pub base: String,
}

/// Everything extracted from a single file: nodes to upsert, non-call edges to
/// add (Contains), and unresolved call/inheritance references for the
/// project-wide resolver.
#[derive(Debug, Default)]
pub struct BuildOutput {
    pub nodes: Vec<Node>,
    pub edges: Vec<(NodeId, NodeId, Edge)>,
    pub calls: Vec<CallRef>,
    pub inherits: Vec<InheritRef>,
}

impl BuildOutput {
    /// Ids of all nodes produced — used by the incremental sync to figure out
    /// which previously-known nodes for a file have disappeared.
    pub fn node_ids(&self) -> Vec<NodeId> {
        self.nodes.iter().map(|n| n.id).collect()
    }
}

/// Derive a module path from a (relative) file path, directory-aware so files
/// in different folders don't collide:
///   `src/math.rs`       -> `crate::math`
///   `src/net/client.rs` -> `crate::net::client`
///   `app/main.py`       -> `crate::app::main`
/// A leading `src/` (or `./`) is dropped; remaining path segments become `::`.
pub fn module_path_for(file: &str) -> String {
    let no_ext = file.rsplit_once('.').map(|(head, _)| head).unwrap_or(file);
    let mut parts: Vec<&str> = no_ext
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.first() == Some(&"src") {
        parts.remove(0);
    }
    if parts.is_empty() {
        return "crate".to_string();
    }
    format!("crate::{}", parts.join("::"))
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

    // Pass 1: collect definitions (functions, types, fields) + Contains edges.
    let root = tree.root_node();
    collect_defs(root, source, file, lang, &module, module_id, &mut out);

    // Pass 2: record every call site as an unresolved reference. Resolution to
    // a concrete callee happens project-wide in `sync`, enabling cross-file links.
    collect_calls(root, source, lang, &module, &mut out);

    // Pass 3: Rust `impl Trait for Type` blocks -> Type Inherits Trait.
    if matches!(lang, Lang::Rust) {
        collect_impls(root, source, &module, &mut out);
    }

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

/// Recursively map definitions, carrying the **enclosing scope** so that methods
/// belong to their type rather than the module: `scope_path`/`scope_id` is the
/// current container (a module at top level, a type inside a class body or Rust
/// `impl` block). A Python method `Calculator.add` becomes
/// `crate::calc::Calculator::add`, `Contains`-ed by the class — not the module.
fn collect_defs(
    node: TsNode,
    source: &str,
    file: &str,
    lang: Lang,
    scope_path: &str,
    scope_id: NodeId,
    out: &mut BuildOutput,
) {
    let kind = node.kind();

    if is_function_kind(lang, kind) {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = node_text(name_node, source).to_string();
            let path = format!("{scope_path}::{name}");
            let id = NodeId::from_path(&path);
            let mut n = Node::new(NodeKind::Function, &name, &path)
                .with_language(lang.name())
                .with_source(node_text(node, source));
            n.file = Some(file.to_string());
            n.span = span_of(node);
            out.nodes.push(n);
            out.edges
                .push((scope_id, id, Edge::new(EdgeKind::Contains)));
            // Recurse into the body with the same scope (nested functions).
            recurse_children(node, source, file, lang, scope_path, scope_id, out);
        }
        return;
    }

    if is_type_kind(lang, kind) {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = node_text(name_node, source).to_string();
            let path = format!("{scope_path}::{name}");
            let id = NodeId::from_path(&path);
            let mut n = Node::new(NodeKind::Type, &name, &path)
                .with_language(lang.name())
                .with_source(node_text(node, source));
            n.file = Some(file.to_string());
            n.span = span_of(node);
            out.nodes.push(n);
            out.edges
                .push((scope_id, id, Edge::new(EdgeKind::Contains)));
            extract_fields(node, source, file, lang, &path, id, out);
            extract_supertypes(node, source, lang, id, out);
            // Methods inside the type body are scoped to the type.
            recurse_children(node, source, file, lang, &path, id, out);
        }
        return;
    }

    // Rust `impl Type { ... }` / `impl Trait for Type { ... }`: not a node, but a
    // scope container — its methods belong to the implemented type.
    if matches!(lang, Lang::Rust) && kind == "impl_item" {
        if let Some(type_node) = node.child_by_field_name("type") {
            let type_name = last_ident(node_text(type_node, source));
            let type_path = format!("{scope_path}::{type_name}");
            let type_id = NodeId::from_path(&type_path);
            recurse_children(node, source, file, lang, &type_path, type_id, out);
            return;
        }
    }

    recurse_children(node, source, file, lang, scope_path, scope_id, out);
}

fn recurse_children(
    node: TsNode,
    source: &str,
    file: &str,
    lang: Lang,
    scope_path: &str,
    scope_id: NodeId,
    out: &mut BuildOutput,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_defs(child, source, file, lang, scope_path, scope_id, out);
    }
}

/// Record unresolved `Inherits` references for a type definition: Python class
/// bases (`class Foo(Bar):`) and Rust supertraits (`trait Sub: Super`). Rust
/// `impl Trait for Type` is handled separately in [`collect_impls`] because the
/// subtype there is itself a name reference, not the node we're defining.
fn extract_supertypes(
    type_node: TsNode,
    source: &str,
    lang: Lang,
    sub: NodeId,
    out: &mut BuildOutput,
) {
    let mut cursor = type_node.walk();
    match lang {
        Lang::Python => {
            // class_definition has a `superclasses` argument_list of bases.
            if let Some(supers) = type_node.child_by_field_name("superclasses") {
                for child in supers.children(&mut supers.walk()) {
                    if child.kind() == "identifier" {
                        out.inherits.push(InheritRef {
                            sub,
                            base: node_text(child, source).to_string(),
                        });
                    }
                }
            }
        }
        Lang::Rust => {
            // `trait Sub: Super + Other` — the trait_bounds list sits after `:`.
            for child in type_node.children(&mut cursor) {
                if child.kind() == "trait_bounds" {
                    let mut inner = child.walk();
                    for b in child.children(&mut inner) {
                        if matches!(b.kind(), "type_identifier" | "scoped_type_identifier") {
                            out.inherits.push(InheritRef {
                                sub,
                                base: last_ident(node_text(b, source)).to_string(),
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Trailing identifier of a possibly-qualified type path (`a::b::Trait` -> `Trait`).
fn last_ident(text: &str) -> &str {
    text.rsplit("::").next().unwrap_or(text).trim()
}

/// Walk Rust `impl Trait for Type` blocks, recording `Type Inherits Trait`.
/// Both ends are name references resolved project-wide by the sync resolver.
fn collect_impls(root: TsNode, source: &str, module: &str, out: &mut BuildOutput) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "impl_item" {
            if let (Some(trait_node), Some(type_node)) = (
                node.child_by_field_name("trait"),
                node.child_by_field_name("type"),
            ) {
                let trait_name = last_ident(node_text(trait_node, source));
                let type_name = last_ident(node_text(type_node, source));
                // The implementing type's node id is its in-module path.
                let sub = NodeId::from_path(&format!("{module}::{type_name}"));
                out.inherits.push(InheritRef {
                    sub,
                    base: trait_name.to_string(),
                });
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
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

fn collect_calls(node: TsNode, source: &str, lang: Lang, module: &str, out: &mut BuildOutput) {
    // Track the enclosing function as we descend so calls attach to a caller.
    fn walk(
        node: TsNode,
        source: &str,
        lang: Lang,
        module: &str,
        current_fn: Option<NodeId>,
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
                // Emit unresolved; the project resolver picks the concrete callee.
                out.calls.push(CallRef { caller, callee });
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk(child, source, lang, module, current, out);
        }
    }

    walk(node, source, lang, module, None, out);
}

/// Best-effort callee name: the trailing identifier of the call target.
fn callee_name(call: TsNode, source: &str, _lang: Lang) -> Option<String> {
    let func = call.child_by_field_name("function")?;
    // For `a.b.c()` / `path::to::f()` take the last identifier-ish leaf.
    let text = node_text(func, source);
    let last = text.rsplit(['.', ':']).next().unwrap_or(text).trim();
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
