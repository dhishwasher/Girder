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

/// An unresolved call site. `callee` is the trailing identifier of the call
/// target and `qualifier` retains the receiver/path when one exists
/// (`catalog.search()` -> `catalog`). The project resolver uses that hint to
/// distinguish same-named methods without pretending to be a type checker.
#[derive(Debug, Clone)]
pub struct CallRef {
    pub caller: NodeId,
    pub callee: String,
    pub qualifier: Option<String>,
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
    let scope = Scope {
        path: &module,
        id: module_id,
        ty: None,
    };
    collect_defs(root, source, file, lang, &scope, &mut out);

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

/// Returns true if this function node is a test.
///
/// Rust: any preceding `attribute_item` sibling whose text contains "test"
/// (covers `#[test]`, `#[tokio::test]`, `#[rstest]`, etc.).
/// Python: pytest convention — name starts with `test_`.
fn is_test_fn(lang: Lang, node: TsNode, name: &str, source: &str) -> bool {
    match lang {
        Lang::Rust => {
            let mut sib = node.prev_named_sibling();
            while let Some(s) = sib {
                if s.kind() == "attribute_item" {
                    if node_text(s, source).contains("test") {
                        return true;
                    }
                    sib = s.prev_named_sibling();
                } else {
                    break;
                }
            }
            false
        }
        Lang::Python => name.starts_with("test_") || name == "test",
    }
}

/// tree-sitter node kinds that define a type, per language.
fn is_type_kind(lang: Lang, kind: &str) -> bool {
    match lang {
        Lang::Rust => matches!(kind, "struct_item" | "enum_item" | "trait_item"),
        Lang::Python => kind == "class_definition",
    }
}

/// The enclosing container while walking definitions. `path`/`id` name the
/// container a new def is attached to (a module at top level, a type inside a
/// class body or Rust `impl`). `ty` is the enclosing *type* path when we're
/// inside a class/impl, so a method's `self.field` accesses can be resolved to
/// field nodes and emitted as `DataFlow` edges.
struct Scope<'a> {
    path: &'a str,
    id: NodeId,
    ty: Option<&'a str>,
}

/// Recursively map definitions, carrying the [`Scope`] so methods belong to
/// their type (`crate::calc::Calculator::add`, not `crate::calc::add`) and field
/// accesses inside methods become `DataFlow` edges.
fn collect_defs(
    node: TsNode,
    source: &str,
    file: &str,
    lang: Lang,
    scope: &Scope,
    out: &mut BuildOutput,
) {
    let kind = node.kind();

    if is_function_kind(lang, kind) {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = node_text(name_node, source).to_string();
            let path = format!("{}::{name}", scope.path);
            let id = NodeId::from_path(&path);
            let mut n = Node::new(NodeKind::Function, &name, &path)
                .with_language(lang.name())
                .with_source(node_text(node, source));
            n.file = Some(file.to_string());
            n.span = span_of(node);
            if is_test_fn(lang, node, &name, source) {
                n.set_attr("is_test", "true");
            }
            out.nodes.push(n);
            out.edges
                .push((scope.id, id, Edge::new(EdgeKind::Contains)));
            // A method's `self.field` accesses flow data between method and field.
            if let Some(type_path) = scope.ty {
                extract_field_flows(node, source, lang, id, type_path, out);
            }
            recurse_children(node, source, file, lang, scope, out);
        }
        return;
    }

    if is_type_kind(lang, kind) {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = node_text(name_node, source).to_string();
            let path = format!("{}::{name}", scope.path);
            let id = NodeId::from_path(&path);
            let mut n = Node::new(NodeKind::Type, &name, &path)
                .with_language(lang.name())
                .with_source(node_text(node, source));
            n.file = Some(file.to_string());
            n.span = span_of(node);
            out.nodes.push(n);
            out.edges
                .push((scope.id, id, Edge::new(EdgeKind::Contains)));
            extract_fields(node, source, file, lang, &path, id, out);
            extract_supertypes(node, source, lang, id, out);
            // Methods inside the type body are scoped to the type.
            let inner = Scope {
                path: &path,
                id,
                ty: Some(&path),
            };
            recurse_children(node, source, file, lang, &inner, out);
        }
        return;
    }

    // Rust `impl Type { ... }` / `impl Trait for Type { ... }`: not a node, but a
    // scope container — its methods belong to the implemented type.
    if matches!(lang, Lang::Rust) && kind == "impl_item" {
        if let Some(type_node) = node.child_by_field_name("type") {
            let type_name = last_ident(node_text(type_node, source));
            let type_path = format!("{}::{type_name}", scope.path);
            let type_id = NodeId::from_path(&type_path);
            let inner = Scope {
                path: &type_path,
                id: type_id,
                ty: Some(&type_path),
            };
            recurse_children(node, source, file, lang, &inner, out);
            return;
        }
    }

    recurse_children(node, source, file, lang, scope, out);
}

fn recurse_children(
    node: TsNode,
    source: &str,
    file: &str,
    lang: Lang,
    scope: &Scope,
    out: &mut BuildOutput,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_defs(child, source, file, lang, scope, out);
    }
}

/// Emit `DataFlow` edges from a method to each `self.field` it touches, so impact
/// flows through fields (changing a field reaches the methods that use it). The
/// field node id is the enclosing type's path plus the accessed name; edges that
/// reference a field the type never declared are simply never applied.
fn extract_field_flows(
    fn_node: TsNode,
    source: &str,
    lang: Lang,
    fn_id: NodeId,
    type_path: &str,
    out: &mut BuildOutput,
) {
    use std::collections::HashSet;
    let mut seen: HashSet<String> = HashSet::new();
    let mut cursor = fn_node.walk();
    for d in descendants(fn_node, &mut cursor) {
        let field = match lang {
            // Python `self.total` -> attribute(object: identifier "self", attribute: identifier)
            Lang::Python if d.kind() == "attribute" => {
                let obj = d.child_by_field_name("object");
                let attr = d.child_by_field_name("attribute");
                match (obj, attr) {
                    (Some(o), Some(a)) if node_text(o, source) == "self" => {
                        Some(node_text(a, source).to_string())
                    }
                    _ => None,
                }
            }
            // Rust `self.total` -> field_expression(value: self, field: field_identifier)
            Lang::Rust if d.kind() == "field_expression" => {
                let val = d.child_by_field_name("value");
                let fld = d.child_by_field_name("field");
                match (val, fld) {
                    (Some(v), Some(f)) if node_text(v, source) == "self" => {
                        Some(node_text(f, source).to_string())
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(name) = field {
            if seen.insert(name.clone()) {
                let field_id = NodeId::from_path(&format!("{type_path}::{name}"));
                if field_id != fn_id {
                    out.edges
                        .push((fn_id, field_id, Edge::new(EdgeKind::DataFlow)));
                }
            }
        }
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
    // `current_type` tracks Python classes plus Rust traits/impls so method ids
    // match the type-scoped ids emitted by `collect_defs`.
    fn walk<'src>(
        node: TsNode,
        source: &'src str,
        lang: Lang,
        module: &'src str,
        current_type: Option<&'src str>,
        current_fn: Option<NodeId>,
        out: &mut BuildOutput,
    ) {
        let mut current = current_fn;
        let mut enclosing_type = current_type;

        if matches!(lang, Lang::Python) && node.kind() == "class_definition" {
            if let Some(name_node) = node.child_by_field_name("name") {
                enclosing_type = Some(node_text(name_node, source));
            }
        } else if matches!(lang, Lang::Rust) {
            if is_type_kind(lang, node.kind()) {
                if let Some(name_node) = node.child_by_field_name("name") {
                    enclosing_type = Some(node_text(name_node, source));
                }
            } else if node.kind() == "impl_item" {
                if let Some(type_node) = node.child_by_field_name("type") {
                    enclosing_type = Some(last_ident(node_text(type_node, source)));
                }
            }
        }

        if is_function_kind(lang, node.kind()) {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source);
                let path = match enclosing_type {
                    Some(ty) => format!("{module}::{ty}::{name}"),
                    None => format!("{module}::{name}"),
                };
                let caller = NodeId::from_path(&path);
                current = Some(caller);
            }
        }

        let call_kind = match lang {
            Lang::Rust => "call_expression",
            Lang::Python => "call",
        };
        if node.kind() == call_kind {
            if let (Some(caller), Some((callee, qualifier))) =
                (current, callee_target(node, source))
            {
                // Emit unresolved; the project resolver picks the concrete callee.
                out.calls.push(CallRef {
                    caller,
                    callee,
                    qualifier,
                });
            }
        }

        // Rust macro token trees are not parsed as normal call expressions.
        // Scan only the macro arguments, not the whole function: whole-function
        // fallback text loses receiver information and creates false Calls edges.
        if matches!(lang, Lang::Rust) && node.kind() == "macro_invocation" {
            if let Some(caller) = current {
                let mut cursor = node.walk();
                let mut children = node.children(&mut cursor);
                let tokens = children.find(|child| child.kind() == "token_tree");
                drop(children);
                if let Some(tokens) = tokens {
                    for callee in textual_call_names(node_text(tokens, source)) {
                        out.calls.push(CallRef {
                            caller,
                            callee,
                            qualifier: None,
                        });
                    }
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk(child, source, lang, module, enclosing_type, current, out);
        }
    }

    walk(node, source, lang, module, None, None, out);
}

/// Best-effort call target: trailing identifier plus its receiver/path.
fn callee_target(call: TsNode, source: &str) -> Option<(String, Option<String>)> {
    let func = call.child_by_field_name("function")?;
    let text = node_text(func, source);
    let split = text.rfind(['.', ':']);
    let (qualifier, last) = match split {
        Some(index) => {
            let separator_len = text[index..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(1);
            (
                Some(text[..index].trim_end_matches(':').trim().to_string()),
                text[index + separator_len..].trim(),
            )
        }
        None => (None, text.trim()),
    };
    if last.is_empty() {
        None
    } else {
        Some((last.to_string(), qualifier.filter(|q| !q.is_empty())))
    }
}

/// Conservative fallback for call-like identifiers inside syntax tree regions
/// that tree-sitter does not expose as normal call expressions, notably Rust
/// macro token trees such as `assert_eq!(add(1, 2), 3)`.
fn textual_call_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if !(ch == '_' || ch.is_ascii_alphabetic()) {
            continue;
        }
        let mut end = start + ch.len_utf8();
        while let Some(&(idx, next)) = chars.peek() {
            if next == '_' || next.is_ascii_alphanumeric() {
                end = idx + next.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        let name = &text[start..end];
        let rest = text[end..].trim_start();
        if rest.starts_with('(') && !is_call_noise(name) {
            names.push(name.to_string());
        }
    }
    names
}

fn is_call_noise(name: &str) -> bool {
    matches!(
        name,
        "fn" | "if" | "for" | "while" | "loop" | "match" | "return" | "Some" | "Ok" | "Err"
    )
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
