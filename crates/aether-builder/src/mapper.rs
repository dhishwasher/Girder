//! Map a tree-sitter syntax tree into semantic-graph nodes and edges.
//!
//! This is deliberately a *pragmatic* extractor, not a full type checker: it
//! recovers modules, functions, types, fields, and call *references*. Call sites
//! are emitted unresolved (`caller id` + `callee name`); the project-wide
//! resolver in `sync` turns them into `Calls` edges, which is what enables
//! **cross-file** call graphs (a callee defined in another module still links).

use crate::parser::Lang;
use aether_graph::{Edge, EdgeKind, Node, NodeId, NodeKind, Span};
use std::collections::HashMap;
use tree_sitter::{Node as TsNode, Tree};

/// A callable used to infer the type of a local binding from its return type.
///
/// `let plan = make_plan()?; plan.commit()` records `make_plan` here so the
/// project-wide resolver can inspect its signature even when it lives in
/// another file.
#[derive(Debug, Clone)]
pub struct CallTargetRef {
    pub callee: String,
    pub qualifier: Option<String>,
    pub receiver_type: Option<String>,
    /// Factory that produced this factory call's receiver, when the receiver
    /// itself is not statically annotated.
    pub receiver_factory: Option<Box<CallTargetRef>>,
    /// Single nested result source for generic pass-through wrappers such as
    /// `collaboration_result(GraphReplica::load(...))?`.
    pub fallback_type: Option<String>,
    pub fallback_factory: Option<Box<CallTargetRef>>,
}

#[derive(Debug, Clone)]
enum ReceiverHint {
    Type(RustTypeHint),
    ReturnOf(CallTargetRef),
}

#[derive(Debug, Clone)]
struct RustTypeHint {
    name: String,
    generic_arguments: Vec<RustTypeHint>,
}

impl RustTypeHint {
    fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            generic_arguments: Vec::new(),
        }
    }
}

const MAX_RECEIVER_HINT_NODES: usize = 16;

impl CallTargetRef {
    fn bounded_clone(&self) -> Self {
        fn clone_with_budget(source: &CallTargetRef, budget: &mut usize) -> Option<CallTargetRef> {
            if *budget == 0 {
                return None;
            }
            *budget -= 1;
            Some(CallTargetRef {
                callee: source.callee.clone(),
                qualifier: source.qualifier.clone(),
                receiver_type: source.receiver_type.clone(),
                receiver_factory: source
                    .receiver_factory
                    .as_deref()
                    .and_then(|factory| clone_with_budget(factory, budget))
                    .map(Box::new),
                fallback_type: source.fallback_type.clone(),
                fallback_factory: source
                    .fallback_factory
                    .as_deref()
                    .and_then(|factory| clone_with_budget(factory, budget))
                    .map(Box::new),
            })
        }

        let mut budget = MAX_RECEIVER_HINT_NODES;
        clone_with_budget(self, &mut budget).expect("receiver hint budget includes its root")
    }
}

impl ReceiverHint {
    fn bounded_clone(&self) -> Self {
        match self {
            Self::Type(name) => Self::Type(name.clone()),
            Self::ReturnOf(factory) => Self::ReturnOf(factory.bounded_clone()),
        }
    }
}

/// An unresolved call site. `callee` is the trailing identifier of the call
/// target and `qualifier` retains the receiver/path when one exists
/// (`catalog.search()` -> `catalog`). The project resolver uses those hints to
/// distinguish same-named methods without pretending to be a full type checker.
#[derive(Debug, Clone)]
pub struct CallRef {
    pub caller: NodeId,
    pub callee: String,
    pub qualifier: Option<String>,
    pub receiver_type: Option<String>,
    pub receiver_factory: Option<CallTargetRef>,
}

/// An unresolved inheritance: type `sub` inherits/implements something named
/// `base` (a superclass in Python, a trait in Rust). Resolved project-wide like
/// calls so `Inherits` edges link across files.
#[derive(Debug, Clone)]
pub struct InheritRef {
    pub sub: NodeId,
    pub base: String,
}

/// A module-scope Rust name imported into a file, retained so project-wide call
/// resolution can follow renamed imports and public re-export chains without
/// guessing by trailing identifier alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustImportRef {
    pub local: String,
    pub target: String,
    pub is_reexport: bool,
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
    pub rust_imports: Vec<RustImportRef>,
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
    let mut module_node = Node::new(NodeKind::Module, last_segment(&module), module.clone())
        .with_language(lang.name())
        .with_source(source);
    module_node.file = Some(file.to_string());
    module_node.span = span_of(tree.root_node());
    module_node.set_attr("source_projection", "file-v1");
    let module_id = module_node.id;
    out.nodes.push(module_node);

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

    // Pass 3: Rust imports/re-exports retain exact symbol identities for the
    // project resolver, and `impl Trait for Type` blocks become Inherits refs.
    if matches!(lang, Lang::Rust) {
        collect_rust_imports(root, source, &mut out);
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
            if let Some(return_type) = node.child_by_field_name("return_type") {
                n.set_attr("return_type", node_text(return_type, source).trim());
            }
            if matches!(lang, Lang::Rust) {
                if let Some(type_parameters) = node.child_by_field_name("type_parameters") {
                    n.set_attr("type_parameters", node_text(type_parameters, source).trim());
                }
                if let Some(parameters) = node.child_by_field_name("parameters") {
                    let mut cursor = parameters.walk();
                    let first_parameter_type = parameters
                        .named_children(&mut cursor)
                        .find(|parameter| parameter.kind() == "parameter")
                        .and_then(|parameter| parameter.child_by_field_name("type"));
                    if let Some(first_parameter_type) = first_parameter_type {
                        n.set_attr(
                            "first_parameter_type",
                            node_text(first_parameter_type, source).trim(),
                        );
                    }
                }
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

fn collect_rust_imports(root: TsNode, source: &str, out: &mut BuildOutput) {
    fn combine_path(prefix: &str, suffix: &str) -> String {
        let suffix = suffix.trim();
        if suffix == "self" {
            return prefix.to_string();
        }
        if prefix.is_empty()
            || suffix == "crate"
            || suffix.starts_with("crate::")
            || suffix == "super"
            || suffix.starts_with("super::")
        {
            suffix.to_string()
        } else {
            format!("{prefix}::{suffix}")
        }
    }

    fn collect_clause(
        node: TsNode,
        source: &str,
        prefix: &str,
        is_reexport: bool,
        out: &mut BuildOutput,
    ) {
        match node.kind() {
            "scoped_use_list" => {
                let path = node
                    .child_by_field_name("path")
                    .map(|path| combine_path(prefix, node_text(path, source)))
                    .unwrap_or_else(|| prefix.to_string());
                if let Some(list) = node.child_by_field_name("list") {
                    collect_clause(list, source, &path, is_reexport, out);
                }
            }
            "use_list" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    collect_clause(child, source, prefix, is_reexport, out);
                }
            }
            "use_as_clause" => {
                let (Some(path), Some(alias)) = (
                    node.child_by_field_name("path"),
                    node.child_by_field_name("alias"),
                ) else {
                    return;
                };
                let local = node_text(alias, source).trim();
                if local == "_" {
                    return;
                }
                let target = combine_path(prefix, node_text(path, source));
                if !target.is_empty() {
                    out.rust_imports.push(RustImportRef {
                        local: local.to_string(),
                        target,
                        is_reexport,
                    });
                }
            }
            "use_wildcard" => {}
            _ => {
                let target = combine_path(prefix, node_text(node, source));
                let local = last_ident(&target);
                if !local.is_empty() && local != "*" {
                    out.rust_imports.push(RustImportRef {
                        local: local.to_string(),
                        target,
                        is_reexport,
                    });
                }
            }
        }
    }

    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        if node.kind() == "use_declaration" {
            let mut use_cursor = node.walk();
            let is_reexport = node
                .named_children(&mut use_cursor)
                .any(|child| child.kind() == "visibility_modifier");
            if let Some(argument) = node.child_by_field_name("argument") {
                collect_clause(argument, source, "", is_reexport, out);
            }
        }
    }
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
    fn resolved_receiver_hint(
        qualifier: Option<&str>,
        hints: &HashMap<String, ReceiverHint>,
    ) -> (Option<String>, Option<CallTargetRef>) {
        match qualifier
            .map(qualifier_binding)
            .and_then(|binding| hints.get(binding))
        {
            Some(ReceiverHint::Type(hint)) => (Some(hint.name.clone()), None),
            Some(ReceiverHint::ReturnOf(factory)) => (None, Some(factory.bounded_clone())),
            None => (None, None),
        }
    }

    // Track the enclosing function as we descend so calls attach to a caller.
    // `current_type` tracks Python classes plus Rust traits/impls so method ids
    // match the type-scoped ids emitted by `collect_defs`.
    fn walk<'src>(
        node: TsNode,
        source: &'src str,
        lang: Lang,
        module: &'src str,
        scope: (Option<&'src str>, Option<NodeId>),
        type_hints: &HashMap<String, ReceiverHint>,
        out: &mut BuildOutput,
    ) {
        let (current_type, current_fn) = scope;
        let mut current = current_fn;
        let mut enclosing_type = current_type;
        let mut function_type_hints = None;

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
                if matches!(lang, Lang::Rust) {
                    function_type_hints = Some(rust_function_type_hints(node, source));
                }
            }
        }
        let active_type_hints = function_type_hints.as_ref().unwrap_or(type_hints);
        let scoped_type_hints = if matches!(lang, Lang::Rust)
            && matches!(node.kind(), "if_expression" | "while_expression")
        {
            rust_condition_type_hints(node, active_type_hints, source)
        } else {
            None
        };
        let narrowed_scope = match node.kind() {
            "if_expression" => node.child_by_field_name("consequence"),
            "while_expression" => node.child_by_field_name("body"),
            _ => None,
        };

        let call_kind = match lang {
            Lang::Rust => "call_expression",
            Lang::Python => "call",
        };
        if node.kind() == call_kind {
            if let (Some(caller), Some((callee, qualifier))) =
                (current, callee_target(node, source))
            {
                let (receiver_type, receiver_factory) =
                    resolved_receiver_hint(qualifier.as_deref(), active_type_hints);
                // Emit unresolved; the project resolver picks the concrete callee.
                out.calls.push(CallRef {
                    caller,
                    callee,
                    qualifier,
                    receiver_type,
                    receiver_factory,
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
                    for (callee, qualifier) in textual_call_refs(node_text(tokens, source)) {
                        let (receiver_type, receiver_factory) =
                            resolved_receiver_hint(qualifier.as_deref(), active_type_hints);
                        out.calls.push(CallRef {
                            caller,
                            callee,
                            qualifier,
                            receiver_type,
                            receiver_factory,
                        });
                    }
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let child_type_hints = match (&scoped_type_hints, narrowed_scope) {
                (Some(hints), Some(scope))
                    if child.start_byte() == scope.start_byte()
                        && child.end_byte() == scope.end_byte() =>
                {
                    hints
                }
                _ => active_type_hints,
            };
            walk(
                child,
                source,
                lang,
                module,
                (enclosing_type, current),
                child_type_hints,
                out,
            );
        }
    }

    walk(
        node,
        source,
        lang,
        module,
        (None, None),
        &HashMap::new(),
        out,
    );
}

fn rust_function_type_hints(function: TsNode, source: &str) -> HashMap<String, ReceiverHint> {
    let mut hints = HashMap::new();
    if let Some(parameters) = function.child_by_field_name("parameters") {
        let mut cursor = parameters.walk();
        for parameter in parameters.named_children(&mut cursor) {
            if parameter.kind() != "parameter" {
                continue;
            }
            if let (Some(pattern), Some(type_node)) = (
                parameter.child_by_field_name("pattern"),
                parameter.child_by_field_name("type"),
            ) {
                record_type_hint(&mut hints, pattern, type_node, source);
            }
        }
    }

    let mut stack = function
        .child_by_field_name("body")
        .into_iter()
        .collect::<Vec<_>>();
    let mut declarations = Vec::new();
    while let Some(node) = stack.pop() {
        if node.kind() == "let_declaration" {
            declarations.push(node);
        } else if node != function && is_function_kind(Lang::Rust, node.kind()) {
            continue;
        }
        let mut children = node.walk();
        stack.extend(node.named_children(&mut children));
    }
    declarations.sort_by_key(|node| node.start_byte());
    for declaration in declarations {
        let Some(pattern) = declaration.child_by_field_name("pattern") else {
            continue;
        };
        if let Some(type_node) = declaration.child_by_field_name("type") {
            record_type_hint(&mut hints, pattern, type_node, source);
            continue;
        }
        let Some(value) = declaration.child_by_field_name("value") else {
            continue;
        };
        if let Some(hint) = infer_rust_receiver_hint(value, &hints, source) {
            record_named_hint(&mut hints, pattern, hint, source);
        }
    }
    hints
}

fn rust_condition_type_hints(
    expression: TsNode,
    hints: &HashMap<String, ReceiverHint>,
    source: &str,
) -> Option<HashMap<String, ReceiverHint>> {
    let condition = expression.child_by_field_name("condition")?;
    if condition.kind() != "let_condition" {
        return None;
    }
    let pattern = condition.child_by_field_name("pattern")?;
    let value = condition.child_by_field_name("value")?;
    let (binding, hint) = rust_narrowed_pattern_hint(pattern, value, hints, source)?;
    let mut narrowed = hints.clone();
    narrowed.insert(binding, hint);
    Some(narrowed)
}

fn rust_narrowed_pattern_hint(
    pattern: TsNode,
    value: TsNode,
    hints: &HashMap<String, ReceiverHint>,
    source: &str,
) -> Option<(String, ReceiverHint)> {
    if pattern.kind() != "tuple_struct_pattern" {
        return None;
    }
    let variant_node = pattern.child_by_field_name("type")?;
    let variant = last_ident(node_text(variant_node, source));
    let (wrapper, argument_index) = match variant {
        "Some" => ("Option", 0),
        "Ok" => ("Result", 0),
        "Err" => ("Result", 1),
        _ => return None,
    };

    let source_hint = rust_expression_receiver_hint(value, hints, source)?;
    let ReceiverHint::Type(source_type) = source_hint else {
        return None;
    };
    if source_type.name != wrapper {
        return None;
    }
    let narrowed_type = source_type.generic_arguments.get(argument_index)?.clone();

    let mut cursor = pattern.walk();
    let mut bindings = pattern.named_children(&mut cursor).filter(|child| {
        child.start_byte() != variant_node.start_byte()
            || child.end_byte() != variant_node.end_byte()
    });
    let binding_node = bindings.next()?;
    if bindings.next().is_some() {
        return None;
    }
    let binding = node_text(binding_node, source)
        .trim()
        .trim_start_matches("mut ")
        .trim_start_matches("ref ")
        .to_string();
    if binding.is_empty()
        || !binding
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    {
        return None;
    }
    Some((binding, ReceiverHint::Type(narrowed_type)))
}

fn rust_expression_receiver_hint(
    value: TsNode,
    hints: &HashMap<String, ReceiverHint>,
    source: &str,
) -> Option<ReceiverHint> {
    let value = unwrap_rust_expression(value);
    if matches!(value.kind(), "identifier" | "self") {
        return hints
            .get(node_text(value, source))
            .map(ReceiverHint::bounded_clone);
    }
    infer_rust_receiver_hint(value, hints, source)
}

fn infer_rust_receiver_hint(
    value: TsNode,
    hints: &HashMap<String, ReceiverHint>,
    source: &str,
) -> Option<ReceiverHint> {
    let value = unwrap_rust_expression(value);
    if value.kind() != "call_expression" {
        return None;
    }
    let (callee, qualifier) = callee_target(value, source)?;

    if callee == "clone" {
        return qualifier
            .as_deref()
            .map(qualifier_binding)
            .and_then(|binding| hints.get(binding))
            .map(ReceiverHint::bounded_clone);
    }

    if matches!(
        callee.as_str(),
        "unwrap" | "expect" | "map_err" | "inspect" | "inspect_err" | "context" | "with_context"
    ) {
        let function = value.child_by_field_name("function")?;
        if function.kind() == "field_expression" {
            let receiver = function.child_by_field_name("value")?;
            if let Some(hint) = infer_rust_receiver_hint(receiver, hints, source) {
                return Some(hint);
            }
        }
    }

    if callee.starts_with(char::is_uppercase) {
        return Some(ReceiverHint::Type(RustTypeHint::named(callee)));
    }

    let (receiver_type, receiver_factory) = qualifier
        .as_deref()
        .map(qualifier_binding)
        .map(|binding| {
            if binding.starts_with(char::is_uppercase) {
                (Some(binding.to_string()), None)
            } else {
                match hints.get(binding) {
                    Some(ReceiverHint::Type(hint)) => (Some(hint.name.clone()), None),
                    Some(ReceiverHint::ReturnOf(factory)) => {
                        (None, Some(Box::new(factory.bounded_clone())))
                    }
                    None => (None, None),
                }
            }
        })
        .unwrap_or((None, None));
    if let Some(type_name) = qualifier
        .as_deref()
        .map(qualifier_binding)
        .filter(|binding| binding.starts_with(char::is_uppercase))
    {
        return Some(ReceiverHint::Type(RustTypeHint::named(type_name)));
    }

    let fallback = qualifier
        .is_none()
        .then(|| single_rust_argument_hint(value, hints, source))
        .flatten();
    let (fallback_type, fallback_factory) = match fallback {
        Some(ReceiverHint::Type(hint)) => (Some(hint.name), None),
        Some(ReceiverHint::ReturnOf(factory)) => (None, Some(Box::new(factory))),
        None => (None, None),
    };
    Some(ReceiverHint::ReturnOf(CallTargetRef {
        callee,
        qualifier,
        receiver_type,
        receiver_factory,
        fallback_type,
        fallback_factory,
    }))
}

fn single_rust_argument_hint(
    call: TsNode,
    hints: &HashMap<String, ReceiverHint>,
    source: &str,
) -> Option<ReceiverHint> {
    let arguments = call.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    let mut arguments = arguments.named_children(&mut cursor);
    let argument = arguments.next()?;
    if arguments.next().is_some() {
        return None;
    }
    infer_rust_receiver_hint(argument, hints, source)
}

fn record_type_hint(
    hints: &mut HashMap<String, ReceiverHint>,
    pattern: TsNode,
    type_node: TsNode,
    source: &str,
) {
    if let Some(type_hint) = rust_type_hint(type_node, source) {
        record_named_hint(hints, pattern, ReceiverHint::Type(type_hint), source);
    }
}

fn rust_type_hint(type_node: TsNode, source: &str) -> Option<RustTypeHint> {
    fn with_budget(type_node: TsNode, source: &str, budget: &mut usize) -> Option<RustTypeHint> {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        match type_node.kind() {
            "reference_type" | "pointer_type" | "parenthesized_type" => {
                let inner = type_node
                    .child_by_field_name("type")
                    .or_else(|| type_node.named_child(0))?;
                with_budget(inner, source, budget)
            }
            "generic_type" => {
                let name_node = type_node.child_by_field_name("type")?;
                let arguments_node = type_node.child_by_field_name("type_arguments")?;
                let mut cursor = arguments_node.walk();
                let generic_arguments = arguments_node
                    .named_children(&mut cursor)
                    .filter_map(|argument| with_budget(argument, source, budget))
                    .collect();
                Some(RustTypeHint {
                    name: last_ident(node_text(name_node, source)).to_string(),
                    generic_arguments,
                })
            }
            "type_identifier" | "primitive_type" | "scoped_type_identifier" => Some(
                RustTypeHint::named(last_ident(node_text(type_node, source))),
            ),
            _ => None,
        }
    }
    let mut budget = MAX_RECEIVER_HINT_NODES;
    with_budget(type_node, source, &mut budget)
}

fn record_named_hint(
    hints: &mut HashMap<String, ReceiverHint>,
    pattern: TsNode,
    hint: ReceiverHint,
    source: &str,
) {
    let binding = node_text(pattern, source).trim().trim_start_matches("mut ");
    if binding
        .chars()
        .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    {
        hints.insert(binding.to_string(), hint);
    }
}

fn unwrap_rust_expression(mut node: TsNode) -> TsNode {
    while matches!(
        node.kind(),
        "try_expression" | "reference_expression" | "parenthesized_expression"
    ) {
        let Some(inner) = node.named_child(0) else {
            break;
        };
        node = inner;
    }
    node
}

fn qualifier_binding(qualifier: &str) -> &str {
    qualifier
        .rsplit(['.', ':'])
        .find(|part| !part.is_empty())
        .unwrap_or(qualifier)
        .trim()
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
fn textual_call_refs(text: &str) -> Vec<(String, Option<String>)> {
    let mut calls = Vec::new();
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
            calls.push((name.to_string(), textual_qualifier(text, start)));
        }
    }
    calls
}

fn textual_qualifier(text: &str, callee_start: usize) -> Option<String> {
    let before = text[..callee_start].trim_end();
    let stem = if let Some(stem) = before.strip_suffix('.') {
        stem.trim_end()
    } else {
        let stem = before.strip_suffix("::")?;
        stem.trim_end()
    };
    let end = stem.len();
    let start = stem
        .char_indices()
        .rev()
        .take_while(|(_, ch)| *ch == '_' || ch.is_ascii_alphanumeric())
        .map(|(index, _)| index)
        .last()
        .unwrap_or(end);
    let identifier = &stem[start..end];
    Some(if identifier.is_empty() {
        // Retain the fact that this was a qualified expression so the resolver
        // never applies its unqualified unique-name fallback.
        "<expression>".to_string()
    } else {
        identifier.to_string()
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::IncrementalParser;

    fn receiver_nodes(target: &CallTargetRef) -> usize {
        1 + target
            .receiver_factory
            .as_deref()
            .map(receiver_nodes)
            .unwrap_or(0)
            + target
                .fallback_factory
                .as_deref()
                .map(receiver_nodes)
                .unwrap_or(0)
    }

    fn rust_type_nodes(hint: &RustTypeHint) -> usize {
        1 + hint
            .generic_arguments
            .iter()
            .map(rust_type_nodes)
            .sum::<usize>()
    }

    #[test]
    fn receiver_hint_clone_budget_bounds_adversarial_factory_chains() {
        let mut target: CallTargetRef = CallTargetRef {
            callee: "root".into(),
            qualifier: None,
            receiver_type: None,
            receiver_factory: None,
            fallback_type: Some("GraphReplica".into()),
            fallback_factory: None,
        };
        for index in 0..(MAX_RECEIVER_HINT_NODES * 4) {
            target = CallTargetRef {
                callee: format!("factory_{index}"),
                qualifier: Some(format!("receiver_{index}")),
                receiver_type: None,
                receiver_factory: Some(Box::new(target)),
                fallback_type: None,
                fallback_factory: None,
            };
        }

        let cloned = target.bounded_clone();
        assert_eq!(receiver_nodes(&cloned), MAX_RECEIVER_HINT_NODES);

        let hint: ReceiverHint = ReceiverHint::ReturnOf(target);
        let ReceiverHint::ReturnOf(cloned) = hint.bounded_clone() else {
            panic!("factory hint changed variant");
        };
        assert_eq!(receiver_nodes(&cloned), MAX_RECEIVER_HINT_NODES);
    }

    #[test]
    fn rust_type_hint_budget_bounds_adversarial_generic_nesting() {
        let depth = MAX_RECEIVER_HINT_NODES * 4;
        let nested = format!(
            "{}SessionIdentity{}",
            "Option<".repeat(depth),
            ">".repeat(depth)
        );
        let source = format!("fn inspect(value: {nested}) {{}}\n");
        let mut parser = IncrementalParser::new(Lang::Rust);
        let tree = parser.parse(&source);
        let mut cursor = tree.root_node().walk();
        let function = descendants(tree.root_node(), &mut cursor)
            .into_iter()
            .find(|node| node.kind() == "function_item")
            .unwrap();
        let parameters = function.child_by_field_name("parameters").unwrap();
        let mut cursor = parameters.walk();
        let parameter = parameters
            .named_children(&mut cursor)
            .find(|node| node.kind() == "parameter")
            .unwrap();
        let hint = rust_type_hint(parameter.child_by_field_name("type").unwrap(), &source).unwrap();

        assert_eq!(rust_type_nodes(&hint), MAX_RECEIVER_HINT_NODES);
    }

    #[test]
    fn rust_imports_preserve_nested_aliases_and_reexport_visibility() {
        let source = r#"
pub(crate) use crate::alpha::{
    beta as gamma,
    nested::{delta, epsilon as zeta},
    self,
    *,
};
use super::theta;
use crate::hidden as _;
fn local_only() {
    use crate::local as function_scoped;
}
"#;
        let mut parser = IncrementalParser::new(Lang::Rust);
        let tree = parser.parse(source);
        let mut imports = extract(&tree, source, "src/imports.rs", Lang::Rust).rust_imports;
        imports.sort_by(|left, right| left.local.cmp(&right.local));

        assert_eq!(
            imports,
            vec![
                RustImportRef {
                    local: "alpha".into(),
                    target: "crate::alpha".into(),
                    is_reexport: true,
                },
                RustImportRef {
                    local: "delta".into(),
                    target: "crate::alpha::nested::delta".into(),
                    is_reexport: true,
                },
                RustImportRef {
                    local: "gamma".into(),
                    target: "crate::alpha::beta".into(),
                    is_reexport: true,
                },
                RustImportRef {
                    local: "theta".into(),
                    target: "super::theta".into(),
                    is_reexport: false,
                },
                RustImportRef {
                    local: "zeta".into(),
                    target: "crate::alpha::nested::epsilon".into(),
                    is_reexport: true,
                },
            ]
        );
    }
}
