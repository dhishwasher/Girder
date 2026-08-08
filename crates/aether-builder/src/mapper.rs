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
    /// An explicit annotation exists but does not identify one safe owner.
    /// Preserve that refusal so later name-based fallback cannot invent an
    /// edge from the binding's spelling.
    Unresolved,
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
            Self::Unresolved => Self::Unresolved,
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
    /// Whether an unresolved qualifier may be matched to an owner by name.
    /// Python attribute chains rooted in an ordinary local disable this: the
    /// tail of `box.identity` is not evidence that the receiver is an
    /// `Identity` type.
    pub qualifier_owner_fallback: bool,
    /// Cargo target from an exact subprocess launch via
    /// `env!("CARGO_BIN_EXE_<target>")`. Its callee is the binary's top-level
    /// Rust `main`, not an ordinary same-module function call.
    pub process_entrypoint: Option<String>,
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
    struct WalkContext<'src> {
        source: &'src str,
        lang: Lang,
        module: &'src str,
    }

    #[derive(Clone, Copy)]
    struct PythonResolutionScope<'scope> {
        type_aliases: &'scope HashMap<String, String>,
        nullable_wrappers: &'scope HashMap<String, PythonNullableWrapper>,
        import_bindings: &'scope HashMap<String, String>,
    }

    fn resolved_receiver_hint(
        qualifier: Option<&str>,
        hints: &HashMap<String, ReceiverHint>,
    ) -> (Option<String>, Option<CallTargetRef>, bool) {
        match qualifier
            .and_then(direct_qualifier_binding)
            .and_then(|binding| hints.get(binding))
        {
            Some(ReceiverHint::Type(hint)) => (Some(hint.name.clone()), None, true),
            Some(ReceiverHint::ReturnOf(factory)) => (None, Some(factory.bounded_clone()), true),
            Some(ReceiverHint::Unresolved) => (None, None, false),
            None => (None, None, true),
        }
    }

    // Track the enclosing function as we descend so calls attach to a caller.
    // `current_type` tracks Python classes plus Rust traits/impls so method ids
    // match the type-scoped ids emitted by `collect_defs`.
    fn walk<'src>(
        node: TsNode,
        context: &WalkContext<'src>,
        scope: (Option<&'src str>, Option<NodeId>),
        type_hints: &HashMap<String, ReceiverHint>,
        python_scope: PythonResolutionScope<'_>,
        out: &mut BuildOutput,
    ) {
        let WalkContext {
            source,
            lang,
            module,
        } = context;
        let PythonResolutionScope {
            type_aliases,
            nullable_wrappers,
            import_bindings,
        } = python_scope;
        let lang = *lang;
        let (current_type, current_fn) = scope;
        let mut current = current_fn;
        let mut enclosing_type = current_type;
        let mut function_type_hints = None;
        let mut function_type_aliases = None;
        let mut function_nullable_wrappers = None;
        let mut function_import_bindings = None;

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
                match lang {
                    Lang::Rust => {
                        function_type_hints = Some(rust_function_type_hints(node, source));
                    }
                    Lang::Python => {
                        let aliases = python_scoped_type_aliases(node, type_aliases, source);
                        let wrappers =
                            python_scoped_nullable_wrapper_aliases(node, nullable_wrappers, source);
                        let imports = python_scoped_import_bindings(node, import_bindings, source);
                        function_type_hints = Some(python_function_type_hints(
                            node,
                            source,
                            type_aliases,
                            nullable_wrappers,
                        ));
                        function_type_aliases = Some(aliases);
                        function_nullable_wrappers = Some(wrappers);
                        function_import_bindings = Some(imports);
                    }
                }
            }
        }
        let active_type_hints = function_type_hints.as_ref().unwrap_or(type_hints);
        let active_type_aliases = function_type_aliases.as_ref().unwrap_or(type_aliases);
        let active_nullable_wrappers = function_nullable_wrappers
            .as_ref()
            .unwrap_or(nullable_wrappers);
        let active_import_bindings = function_import_bindings.as_ref().unwrap_or(import_bindings);
        let scoped_type_hints = if matches!(lang, Lang::Rust) {
            match node.kind() {
                "if_expression" | "while_expression" => {
                    rust_condition_type_hints(node, active_type_hints, source)
                }
                "match_arm" => rust_match_arm_type_hints(node, active_type_hints, source),
                _ => None,
            }
        } else {
            None
        };
        let narrowed_scope = match node.kind() {
            "if_expression" => node.child_by_field_name("consequence"),
            "while_expression" => node.child_by_field_name("body"),
            _ => None,
        };
        let narrows_all_children = node.kind() == "match_arm";

        let call_kind = match lang {
            Lang::Rust => "call_expression",
            Lang::Python => "call",
        };
        if node.kind() == call_kind {
            if let (Some(caller), Some((callee, qualifier))) =
                (current, callee_target(node, source))
            {
                let (receiver_type, receiver_factory, hint_allows_fallback) =
                    resolved_receiver_hint(qualifier.as_deref(), active_type_hints);
                let qualifier_owner_fallback = hint_allows_fallback
                    && qualifier.as_deref().is_none_or(|qualifier| {
                        python_qualifier_owner_fallback(lang, qualifier, active_import_bindings)
                    });
                // Emit unresolved; the project resolver picks the concrete callee.
                out.calls.push(CallRef {
                    caller,
                    callee,
                    qualifier,
                    receiver_type,
                    receiver_factory,
                    qualifier_owner_fallback,
                    process_entrypoint: None,
                });
                if let Some(target) = matches!(lang, Lang::Rust)
                    .then(|| rust_cargo_binary_target(node, source))
                    .flatten()
                {
                    out.calls.push(CallRef {
                        caller,
                        callee: "main".to_string(),
                        qualifier: None,
                        receiver_type: None,
                        receiver_factory: None,
                        qualifier_owner_fallback: true,
                        process_entrypoint: Some(target),
                    });
                }
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
                        let (receiver_type, receiver_factory, hint_allows_fallback) =
                            resolved_receiver_hint(qualifier.as_deref(), active_type_hints);
                        out.calls.push(CallRef {
                            caller,
                            callee,
                            qualifier,
                            receiver_type,
                            receiver_factory,
                            qualifier_owner_fallback: hint_allows_fallback,
                            process_entrypoint: None,
                        });
                    }
                }
            }
        }

        let mut following_type_hints = None;
        let mut following_type_aliases = None;
        let mut following_nullable_wrappers = None;
        let mut following_import_bindings = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let sequential_type_hints = following_type_hints.as_ref().unwrap_or(active_type_hints);
            let alternative_branch =
                matches!(lang, Lang::Python) && python_is_alternative_branch(node, child);
            let sequential_type_aliases = if alternative_branch {
                active_type_aliases
            } else {
                following_type_aliases
                    .as_ref()
                    .unwrap_or(active_type_aliases)
            };
            let sequential_nullable_wrappers = if alternative_branch {
                active_nullable_wrappers
            } else {
                following_nullable_wrappers
                    .as_ref()
                    .unwrap_or(active_nullable_wrappers)
            };
            let sequential_import_bindings = if alternative_branch {
                active_import_bindings
            } else {
                following_import_bindings
                    .as_ref()
                    .unwrap_or(active_import_bindings)
            };
            let child_type_hints = match (&scoped_type_hints, narrowed_scope) {
                (Some(hints), _) if narrows_all_children => hints,
                (Some(hints), Some(scope))
                    if child.start_byte() == scope.start_byte()
                        && child.end_byte() == scope.end_byte() =>
                {
                    hints
                }
                _ => sequential_type_hints,
            };
            walk(
                child,
                context,
                (enclosing_type, current),
                child_type_hints,
                PythonResolutionScope {
                    type_aliases: sequential_type_aliases,
                    nullable_wrappers: sequential_nullable_wrappers,
                    import_bindings: sequential_import_bindings,
                },
                out,
            );
            let following = match lang {
                Lang::Rust => match node.kind() {
                    "block" => rust_let_else_type_hints(child, sequential_type_hints, source),
                    "let_chain" => {
                        rust_let_condition_type_hints(child, sequential_type_hints, source)
                    }
                    _ => None,
                },
                Lang::Python if node.kind() == "block" => python_assignment_type_hints(
                    child,
                    sequential_type_hints,
                    sequential_type_aliases,
                    sequential_nullable_wrappers,
                    source,
                ),
                Lang::Python => None,
            };
            if let Some(hints) = following {
                following_type_hints = Some(hints);
            }
            if matches!(lang, Lang::Python) {
                let mut aliases = sequential_type_aliases.clone();
                update_python_type_aliases(child, source, &mut aliases);
                following_type_aliases = Some(aliases);

                let mut wrappers = sequential_nullable_wrappers.clone();
                update_python_nullable_wrappers(child, source, &mut wrappers);
                following_nullable_wrappers = Some(wrappers);

                let mut imports = sequential_import_bindings.clone();
                update_python_import_bindings(child, source, &mut imports);
                following_import_bindings = Some(imports);
            }
        }
    }

    let python_aliases = HashMap::new();
    let python_nullable_wrappers = HashMap::new();
    let python_import_bindings = HashMap::new();
    let context = WalkContext {
        source,
        lang,
        module,
    };
    walk(
        node,
        &context,
        (None, None),
        &HashMap::new(),
        PythonResolutionScope {
            type_aliases: &python_aliases,
            nullable_wrappers: &python_nullable_wrappers,
            import_bindings: &python_import_bindings,
        },
        out,
    );
}

fn python_if_branches(root: TsNode) -> Option<(Vec<TsNode>, bool)> {
    if root.kind() != "if_statement" {
        return None;
    }
    let consequence = root.child_by_field_name("consequence")?;
    let mut branches = vec![consequence];
    let mut cursor = root.walk();
    branches.extend(root.children_by_field_name("alternative", &mut cursor));
    let has_else = branches
        .iter()
        .skip(1)
        .any(|branch| branch.kind() == "else_clause");
    Some((branches, has_else))
}

fn python_is_alternative_branch(parent: TsNode, child: TsNode) -> bool {
    if parent.kind() != "if_statement" {
        return false;
    }
    let mut cursor = parent.walk();
    let is_alternative = parent
        .children_by_field_name("alternative", &mut cursor)
        .any(|alternative| {
            child.start_byte() == alternative.start_byte()
                && child.end_byte() == alternative.end_byte()
        });
    is_alternative
}

fn python_is_type_checking_guard(
    statement: TsNode,
    source: &str,
    wrappers: &HashMap<String, PythonNullableWrapper>,
) -> bool {
    statement
        .child_by_field_name("condition")
        .map(|condition| node_text(condition, source).trim())
        .is_some_and(|binding| wrappers.get(binding) == Some(&PythonNullableWrapper::TypeChecking))
}

fn python_scoped_type_aliases(
    function: TsNode,
    inherited: &HashMap<String, String>,
    source: &str,
) -> HashMap<String, String> {
    let mut aliases = inherited.clone();
    if let Some(parameters) = function.child_by_field_name("parameters") {
        let mut cursor = parameters.walk();
        for parameter in parameters.named_children(&mut cursor) {
            if let Some(name) = python_parameter_name(parameter) {
                aliases.remove(node_text(name, source));
            }
        }
    }
    aliases
}

fn update_python_type_aliases(root: TsNode, source: &str, aliases: &mut HashMap<String, String>) {
    if let Some((branches, has_else)) = python_if_branches(root) {
        let mut branch_aliases = branches
            .into_iter()
            .map(|branch| {
                let mut branch_aliases = aliases.clone();
                update_python_type_aliases(branch, source, &mut branch_aliases);
                branch_aliases
            })
            .collect::<Vec<_>>();
        if !has_else {
            branch_aliases.push(aliases.clone());
        }
        let mut joined = branch_aliases.remove(0);
        joined.retain(|name, target| {
            branch_aliases
                .iter()
                .all(|branch| branch.get(name) == Some(target))
        });
        *aliases = joined;
        return;
    }
    match root.kind() {
        "import_statement" | "import_from_statement" => {
            let mut children = root.walk();
            for import in root
                .named_children(&mut children)
                .filter(|child| child.kind() == "aliased_import")
            {
                let (Some(name), Some(alias)) = (
                    import.child_by_field_name("name"),
                    import.child_by_field_name("alias"),
                ) else {
                    continue;
                };
                let target = node_text(name, source)
                    .rsplit('.')
                    .next()
                    .unwrap_or_default();
                if !target.is_empty() {
                    aliases.insert(node_text(alias, source).to_string(), target.to_string());
                }
            }
        }
        "class_definition" | "function_definition" => {
            if let Some(name) = root.child_by_field_name("name") {
                aliases.remove(node_text(name, source));
            }
        }
        "expression_statement" => {
            if let Some(left) = root
                .named_child(0)
                .filter(|child| child.kind() == "assignment")
                .and_then(|assignment| assignment.child_by_field_name("left"))
                .filter(|left| left.kind() == "identifier")
            {
                aliases.remove(node_text(left, source));
            }
        }
        _ => {
            let mut cursor = root.walk();
            for child in root.named_children(&mut cursor) {
                update_python_type_aliases(child, source, aliases);
            }
        }
    }
}

fn python_scoped_import_bindings(
    function: TsNode,
    inherited: &HashMap<String, String>,
    source: &str,
) -> HashMap<String, String> {
    let mut bindings = inherited.clone();
    if let Some(parameters) = function.child_by_field_name("parameters") {
        let mut cursor = parameters.walk();
        for parameter in parameters.named_children(&mut cursor) {
            if let Some(name) = python_parameter_name(parameter) {
                bindings.remove(node_text(name, source));
            }
        }
    }
    bindings
}

fn update_python_import_bindings(
    root: TsNode,
    source: &str,
    bindings: &mut HashMap<String, String>,
) {
    if let Some((branches, has_else)) = python_if_branches(root) {
        let mut branch_bindings = branches
            .into_iter()
            .map(|branch| {
                let mut branch_bindings = bindings.clone();
                update_python_import_bindings(branch, source, &mut branch_bindings);
                branch_bindings
            })
            .collect::<Vec<_>>();
        if !has_else {
            branch_bindings.push(bindings.clone());
        }
        let mut joined = branch_bindings.remove(0);
        joined.retain(|name, target| {
            branch_bindings
                .iter()
                .all(|branch| branch.get(name) == Some(target))
        });
        *bindings = joined;
        return;
    }
    match root.kind() {
        "import_statement" | "import_from_statement" => {
            let module = root.child_by_field_name("module_name");
            let mut imports = root.walk();
            for import in root.named_children(&mut imports) {
                if module.is_some_and(|module| {
                    import.start_byte() == module.start_byte()
                        && import.end_byte() == module.end_byte()
                }) {
                    continue;
                }
                let imported = if import.kind() == "aliased_import" {
                    import
                        .child_by_field_name("name")
                        .map(|name| node_text(name, source))
                } else if import.kind() == "dotted_name" {
                    Some(node_text(import, source))
                } else {
                    None
                };
                let binding = if import.kind() == "aliased_import" {
                    import
                        .child_by_field_name("alias")
                        .map(|alias| node_text(alias, source))
                } else {
                    imported.map(|imported| {
                        if root.kind() == "import_statement" {
                            imported.split('.').next().unwrap_or(imported)
                        } else {
                            imported.rsplit('.').next().unwrap_or(imported)
                        }
                    })
                };
                if let (Some(imported), Some(binding)) = (imported, binding) {
                    let module_prefix = module.map(|module| node_text(module, source));
                    let target = module_prefix
                        .map(|module| format!("{module}.{imported}"))
                        .unwrap_or_else(|| imported.to_string());
                    bindings.insert(binding.to_string(), target);
                }
            }
        }
        "class_definition" | "function_definition" => {
            if let Some(name) = root.child_by_field_name("name") {
                bindings.remove(node_text(name, source));
            }
        }
        "expression_statement" => {
            if let Some(left) = root
                .named_child(0)
                .filter(|child| child.kind() == "assignment")
                .and_then(|assignment| assignment.child_by_field_name("left"))
                .filter(|left| left.kind() == "identifier")
            {
                bindings.remove(node_text(left, source));
            }
        }
        _ => {
            let mut cursor = root.walk();
            for child in root.named_children(&mut cursor) {
                update_python_import_bindings(child, source, bindings);
            }
        }
    }
}

fn python_qualifier_owner_fallback(
    lang: Lang,
    qualifier: &str,
    import_bindings: &HashMap<String, String>,
) -> bool {
    if !matches!(lang, Lang::Python) || !qualifier.contains('.') {
        return true;
    }
    let qualifier = qualifier.trim();
    let root = qualifier.split('.').next().unwrap_or(qualifier);
    let tail = qualifier.rsplit('.').next().unwrap_or(qualifier);
    import_bindings.contains_key(root)
        && (qualifier == root || tail.starts_with(char::is_uppercase))
}

fn python_scoped_nullable_wrapper_aliases(
    function: TsNode,
    inherited: &HashMap<String, PythonNullableWrapper>,
    source: &str,
) -> HashMap<String, PythonNullableWrapper> {
    let mut wrappers = inherited.clone();
    if let Some(parameters) = function.child_by_field_name("parameters") {
        let mut cursor = parameters.walk();
        for parameter in parameters.named_children(&mut cursor) {
            if let Some(name) = python_parameter_name(parameter) {
                remove_python_wrapper_binding(&mut wrappers, node_text(name, source));
            }
        }
    }
    wrappers
}

fn python_parameter_name(parameter: TsNode) -> Option<TsNode> {
    if parameter.kind() == "identifier" {
        return Some(parameter);
    }
    parameter.child_by_field_name("name").or_else(|| {
        let mut children = parameter.walk();
        let name = parameter
            .named_children(&mut children)
            .find(|child| child.kind() == "identifier");
        name
    })
}

fn update_python_nullable_wrappers(
    root: TsNode,
    source: &str,
    wrappers: &mut HashMap<String, PythonNullableWrapper>,
) {
    if let Some((branches, has_else)) = python_if_branches(root) {
        if python_is_type_checking_guard(root, source, wrappers) {
            update_python_nullable_wrappers(branches[0], source, wrappers);
            return;
        }
        let mut branch_wrappers = branches
            .into_iter()
            .map(|branch| {
                let mut branch_wrappers = wrappers.clone();
                update_python_nullable_wrappers(branch, source, &mut branch_wrappers);
                branch_wrappers
            })
            .collect::<Vec<_>>();
        if !has_else {
            branch_wrappers.push(wrappers.clone());
        }
        let mut joined = branch_wrappers.remove(0);
        joined.retain(|name, wrapper| {
            branch_wrappers
                .iter()
                .all(|branch| branch.get(name) == Some(wrapper))
        });
        *wrappers = joined;
        return;
    }
    match root.kind() {
        "import_from_statement" => {
            let Some(module) = root.child_by_field_name("module_name") else {
                return;
            };
            let trusted_module =
                matches!(node_text(module, source), "typing" | "typing_extensions");
            let mut imports = root.walk();
            for import in root.named_children(&mut imports) {
                if import.start_byte() == module.start_byte()
                    && import.end_byte() == module.end_byte()
                {
                    continue;
                }
                let (imported, binding) = if import.kind() == "aliased_import" {
                    let (Some(name), Some(alias)) = (
                        import.child_by_field_name("name"),
                        import.child_by_field_name("alias"),
                    ) else {
                        continue;
                    };
                    (node_text(name, source), node_text(alias, source))
                } else if import.kind() == "dotted_name" {
                    let imported = node_text(import, source);
                    (imported, imported.rsplit('.').next().unwrap_or(imported))
                } else {
                    continue;
                };
                remove_python_wrapper_binding(wrappers, binding);
                if trusted_module {
                    insert_python_nullable_wrapper(wrappers, binding, imported);
                }
            }
        }
        "import_statement" => {
            let mut imports = root.walk();
            for import in root.named_children(&mut imports) {
                let (imported, binding) = if import.kind() == "aliased_import" {
                    let (Some(name), Some(alias)) = (
                        import.child_by_field_name("name"),
                        import.child_by_field_name("alias"),
                    ) else {
                        continue;
                    };
                    (node_text(name, source), node_text(alias, source))
                } else if import.kind() == "dotted_name" {
                    let imported = node_text(import, source);
                    (imported, imported.split('.').next().unwrap_or(imported))
                } else {
                    continue;
                };
                remove_python_wrapper_binding(wrappers, binding);
                if matches!(imported, "typing" | "typing_extensions") {
                    wrappers.insert(
                        format!("{binding}.Optional"),
                        PythonNullableWrapper::Optional,
                    );
                    wrappers.insert(format!("{binding}.Union"), PythonNullableWrapper::Union);
                    wrappers.insert(
                        format!("{binding}.TYPE_CHECKING"),
                        PythonNullableWrapper::TypeChecking,
                    );
                }
            }
        }
        "class_definition" | "function_definition" => {
            if let Some(name) = root.child_by_field_name("name") {
                remove_python_wrapper_binding(wrappers, node_text(name, source));
            }
        }
        "expression_statement" => {
            if let Some(left) = root
                .named_child(0)
                .filter(|child| child.kind() == "assignment")
                .and_then(|assignment| assignment.child_by_field_name("left"))
                .filter(|left| left.kind() == "identifier")
            {
                remove_python_wrapper_binding(wrappers, node_text(left, source));
            }
        }
        _ => {
            let mut cursor = root.walk();
            for child in root.named_children(&mut cursor) {
                update_python_nullable_wrappers(child, source, wrappers);
            }
        }
    }
}

fn insert_python_nullable_wrapper(
    wrappers: &mut HashMap<String, PythonNullableWrapper>,
    binding: &str,
    imported: &str,
) {
    let wrapper = match imported.rsplit('.').next().unwrap_or(imported) {
        "Optional" => PythonNullableWrapper::Optional,
        "Union" => PythonNullableWrapper::Union,
        "TYPE_CHECKING" => PythonNullableWrapper::TypeChecking,
        _ => return,
    };
    wrappers.insert(binding.to_string(), wrapper);
}

fn remove_python_wrapper_binding(
    wrappers: &mut HashMap<String, PythonNullableWrapper>,
    binding: &str,
) {
    wrappers.retain(|name, _| {
        name != binding
            && name
                .strip_prefix(binding)
                .is_none_or(|suffix| !suffix.starts_with('.'))
    });
}

fn python_function_type_hints(
    function: TsNode,
    source: &str,
    aliases: &HashMap<String, String>,
    nullable_wrappers: &HashMap<String, PythonNullableWrapper>,
) -> HashMap<String, ReceiverHint> {
    let mut hints = HashMap::new();
    let Some(parameters) = function.child_by_field_name("parameters") else {
        return hints;
    };
    let mut cursor = parameters.walk();
    for parameter in parameters.named_children(&mut cursor) {
        if !matches!(
            parameter.kind(),
            "typed_parameter" | "typed_default_parameter"
        ) {
            continue;
        }
        let Some(type_node) = parameter.child_by_field_name("type") else {
            continue;
        };
        let name_node = python_parameter_name(parameter);
        let Some(name_node) = name_node else {
            continue;
        };
        let hint = python_direct_type_name(type_node, source, aliases, nullable_wrappers)
            .map(|type_name| ReceiverHint::Type(RustTypeHint::named(type_name)))
            .unwrap_or(ReceiverHint::Unresolved);
        hints.insert(node_text(name_node, source).to_string(), hint);
    }
    hints
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PythonNullableWrapper {
    Optional,
    Union,
    TypeChecking,
}

fn python_direct_type_name(
    type_node: TsNode,
    source: &str,
    aliases: &HashMap<String, String>,
    nullable_wrappers: &HashMap<String, PythonNullableWrapper>,
) -> Option<String> {
    python_unambiguous_type_name(node_text(type_node, source), aliases, nullable_wrappers)
}

fn python_unambiguous_type_name(
    annotation: &str,
    aliases: &HashMap<String, String>,
    nullable_wrappers: &HashMap<String, PythonNullableWrapper>,
) -> Option<String> {
    let annotation = annotation.trim();
    let unquoted = annotation
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            annotation
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(annotation);
    if unquoted != annotation {
        return python_unambiguous_type_name(unquoted, aliases, nullable_wrappers);
    }

    let annotation = strip_python_outer_parentheses(unquoted);
    let union_parts = python_top_level_parts(annotation, '|');
    if union_parts.len() > 1 {
        return python_union_type_name(&union_parts, aliases, nullable_wrappers);
    }

    if let Some(open) = python_top_level_subscript(annotation) {
        let wrapper = annotation[..open].trim();
        python_qualified_type_name(wrapper)?;
        let arguments = annotation[open + 1..annotation.len() - 1].trim();
        return match nullable_wrappers.get(wrapper) {
            Some(PythonNullableWrapper::Optional) => {
                python_unambiguous_type_name(arguments, aliases, nullable_wrappers)
            }
            Some(PythonNullableWrapper::Union) => python_union_type_name(
                &python_top_level_parts(arguments, ','),
                aliases,
                nullable_wrappers,
            ),
            _ => None,
        };
    }

    if python_annotation_is_none(annotation) {
        return None;
    }
    let type_name = python_qualified_type_name(annotation)?;
    Some(aliases.get(&type_name).cloned().unwrap_or(type_name))
}

fn python_union_type_name(
    parts: &[&str],
    aliases: &HashMap<String, String>,
    nullable_wrappers: &HashMap<String, PythonNullableWrapper>,
) -> Option<String> {
    let mut selected = None;
    for part in parts {
        let part = strip_python_outer_parentheses(part.trim());
        if python_annotation_is_none(part) {
            continue;
        }
        let candidate = python_unambiguous_type_name(part, aliases, nullable_wrappers)?;
        match selected.as_deref() {
            None => selected = Some(candidate),
            Some(existing) if existing == candidate => {}
            Some(_) => return None,
        }
    }
    selected
}

fn python_annotation_is_none(annotation: &str) -> bool {
    let annotation = strip_python_outer_parentheses(annotation.trim());
    let unquoted = annotation
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            annotation
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(annotation);
    strip_python_outer_parentheses(unquoted.trim()) == "None"
}

fn python_qualified_type_name(annotation: &str) -> Option<String> {
    let mut segments = annotation.split('.');
    let first = segments.next()?;
    if first.is_empty()
        || !first
            .chars()
            .all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        return None;
    }
    let mut type_name = first;
    for segment in segments {
        if segment.is_empty()
            || !segment
                .chars()
                .all(|character| character == '_' || character.is_ascii_alphanumeric())
        {
            return None;
        }
        type_name = segment;
    }
    Some(type_name.to_string())
}

fn python_top_level_parts(value: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in value.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if ch == delimiter && depth == 0 => {
                parts.push(value[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(value[start..].trim());
    parts
}

fn python_top_level_subscript(value: &str) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in value.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '[' if value.ends_with(']') => return Some(index),
            _ => {}
        }
    }
    None
}

fn strip_python_outer_parentheses(mut value: &str) -> &str {
    loop {
        if !value.starts_with('(') || !value.ends_with(')') {
            return value;
        }
        let mut depth = 0_u32;
        let mut quote = None;
        let mut escaped = false;
        let mut closes_before_end = false;
        for (index, ch) in value.char_indices() {
            if let Some(active_quote) = quote {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == active_quote {
                    quote = None;
                }
                continue;
            }
            match ch {
                '\'' | '"' => quote = Some(ch),
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 && index + ch.len_utf8() < value.len() {
                        closes_before_end = true;
                        break;
                    }
                }
                _ => {}
            }
        }
        if closes_before_end || depth != 0 {
            return value;
        }
        value = value[1..value.len() - 1].trim();
    }
}

fn python_assignment_type_hints(
    statement: TsNode,
    hints: &HashMap<String, ReceiverHint>,
    aliases: &HashMap<String, String>,
    nullable_wrappers: &HashMap<String, PythonNullableWrapper>,
    source: &str,
) -> Option<HashMap<String, ReceiverHint>> {
    let assignment = if statement.kind() == "assignment" {
        statement
    } else if statement.kind() == "expression_statement" {
        let child = statement.named_child(0)?;
        (child.kind() == "assignment").then_some(child)?
    } else {
        return None;
    };
    let left = assignment.child_by_field_name("left")?;
    if left.kind() != "identifier" {
        return None;
    }
    let binding = node_text(left, source).to_string();
    let annotation_node = assignment.child_by_field_name("type");
    let annotation = annotation_node.and_then(|type_node| {
        python_direct_type_name(type_node, source, aliases, nullable_wrappers)
    });
    let constructor = assignment
        .child_by_field_name("right")
        .filter(|right| right.kind() == "call")
        .and_then(|right| callee_target(right, source))
        .map(|(callee, _)| callee)
        .filter(|callee| callee.starts_with(char::is_uppercase));

    let mut updated = hints.clone();
    if annotation_node.is_some() && annotation.is_none() {
        updated.insert(binding, ReceiverHint::Unresolved);
    } else if let Some(type_name) = annotation.or(constructor) {
        let type_name = aliases.get(&type_name).unwrap_or(&type_name);
        updated.insert(binding, ReceiverHint::Type(RustTypeHint::named(type_name)));
    } else {
        updated.remove(&binding);
    }
    Some(updated)
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
    match condition.kind() {
        "let_condition" => rust_let_condition_type_hints(condition, hints, source),
        "let_chain" => {
            let mut narrowed = hints.clone();
            let mut found = false;
            let mut cursor = condition.walk();
            for operand in condition.named_children(&mut cursor) {
                if let Some(next) = rust_let_condition_type_hints(operand, &narrowed, source) {
                    narrowed = next;
                    found = true;
                }
            }
            found.then_some(narrowed)
        }
        _ => None,
    }
}

fn rust_let_condition_type_hints(
    condition: TsNode,
    hints: &HashMap<String, ReceiverHint>,
    source: &str,
) -> Option<HashMap<String, ReceiverHint>> {
    if condition.kind() != "let_condition" {
        return None;
    }
    let pattern = condition.child_by_field_name("pattern")?;
    let value = condition.child_by_field_name("value")?;
    rust_narrowed_type_hints(pattern, value, hints, source)
}

fn rust_match_arm_type_hints(
    arm: TsNode,
    hints: &HashMap<String, ReceiverHint>,
    source: &str,
) -> Option<HashMap<String, ReceiverHint>> {
    let match_pattern = arm.child_by_field_name("pattern")?;
    let condition = match_pattern.child_by_field_name("condition");
    let mut cursor = match_pattern.walk();
    let pattern = match_pattern
        .named_children(&mut cursor)
        .find(|child| condition.is_none_or(|guard| child.id() != guard.id()))?;
    let match_expression = arm.parent()?.parent()?;
    if match_expression.kind() != "match_expression" {
        return None;
    }
    let value = match_expression.child_by_field_name("value")?;
    rust_narrowed_type_hints(pattern, value, hints, source)
}

fn rust_let_else_type_hints(
    declaration: TsNode,
    hints: &HashMap<String, ReceiverHint>,
    source: &str,
) -> Option<HashMap<String, ReceiverHint>> {
    if declaration.kind() != "let_declaration"
        || declaration.child_by_field_name("alternative").is_none()
    {
        return None;
    }
    let pattern = declaration.child_by_field_name("pattern")?;
    let value = declaration.child_by_field_name("value")?;
    rust_narrowed_type_hints(pattern, value, hints, source)
}

fn rust_narrowed_type_hints(
    pattern: TsNode,
    value: TsNode,
    hints: &HashMap<String, ReceiverHint>,
    source: &str,
) -> Option<HashMap<String, ReceiverHint>> {
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
                    Some(ReceiverHint::Unresolved) => (None, None),
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
        Some(ReceiverHint::Unresolved) => (None, None),
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

fn direct_qualifier_binding(qualifier: &str) -> Option<&str> {
    let qualifier = qualifier.trim();
    (!qualifier.is_empty()
        && qualifier
            .chars()
            .all(|character| character == '_' || character.is_ascii_alphanumeric()))
    .then_some(qualifier)
}

/// Recognize an exact Cargo integration-test launch:
/// `Command::new(env!("CARGO_BIN_EXE_<target>"))`.
///
/// The Cargo-provided environment variable is a compile-time, executable path,
/// so this is stronger evidence than an arbitrary command string. Resolution
/// requires either an exact conventional `src/bin` target or one unambiguous
/// conventional Rust entrypoint in the project.
fn rust_cargo_binary_target(call: TsNode, source: &str) -> Option<String> {
    let (callee, qualifier) = callee_target(call, source)?;
    if callee != "new"
        || qualifier
            .as_deref()
            .map(qualifier_binding)
            .is_none_or(|tail| tail != "Command")
    {
        return None;
    }
    let arguments = call.child_by_field_name("arguments")?;
    let mut argument_cursor = arguments.walk();
    let mut arguments = arguments.named_children(&mut argument_cursor);
    let argument = arguments.next()?;
    if arguments.next().is_some() || argument.kind() != "macro_invocation" {
        return None;
    }
    let macro_name = argument.child_by_field_name("macro")?;
    if last_ident(node_text(macro_name, source)) != "env" {
        return None;
    }
    let mut macro_cursor = argument.walk();
    let tokens = argument
        .named_children(&mut macro_cursor)
        .find(|child| child.kind() == "token_tree")?;
    let mut token_cursor = tokens.walk();
    let mut token_arguments = tokens.named_children(&mut token_cursor);
    let variable = token_arguments.next()?;
    if token_arguments.next().is_some() || variable.kind() != "string_literal" {
        return None;
    }
    node_text(variable, source)
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .and_then(|value| value.strip_prefix("CARGO_BIN_EXE_"))
        .filter(|target| {
            !target.is_empty()
                && target.chars().all(|character| {
                    character == '_' || character == '-' || character.is_ascii_alphanumeric()
                })
        })
        .map(str::to_string)
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
    let code = mask_rust_macro_non_code(text);
    let text = code.as_str();
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

fn mask_rust_macro_non_code(text: &str) -> String {
    fn token_boundary(bytes: &[u8], start: usize) -> bool {
        start == 0
            || !matches!(
                bytes[start - 1],
                b'_' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9'
            )
    }

    fn raw_string_end(bytes: &[u8], start: usize) -> Option<usize> {
        if !token_boundary(bytes, start) {
            return None;
        }
        let mut cursor = start;
        match bytes.get(cursor..) {
            Some([b'b' | b'c', b'r', ..]) => cursor += 2,
            Some([b'r', ..]) => cursor += 1,
            _ => return None,
        }
        let hashes_start = cursor;
        while bytes.get(cursor) == Some(&b'#') {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'"') {
            return None;
        }
        let hash_count = cursor - hashes_start;
        cursor += 1;
        while cursor < bytes.len() {
            if bytes[cursor] == b'"'
                && bytes
                    .get(cursor + 1..cursor + 1 + hash_count)
                    .is_some_and(|suffix| suffix.iter().all(|byte| *byte == b'#'))
            {
                return Some(cursor + 1 + hash_count);
            }
            cursor += 1;
        }
        Some(bytes.len())
    }

    fn quoted_string_end(bytes: &[u8], start: usize) -> usize {
        let mut cursor = start + 1;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'\\' => cursor = (cursor + 2).min(bytes.len()),
                b'"' => return cursor + 1,
                _ => cursor += 1,
            }
        }
        bytes.len()
    }

    fn char_literal_end(text: &str, start: usize) -> Option<usize> {
        let tail = text.get(start + 1..)?;
        let mut chars = tail.char_indices();
        let (offset, first) = chars.next()?;
        debug_assert_eq!(offset, 0);
        if matches!(first, '\n' | '\r' | '\'') {
            return None;
        }
        if first != '\\' {
            let closing = start + 1 + first.len_utf8();
            return (text.as_bytes().get(closing) == Some(&b'\'')).then_some(closing + 1);
        }

        let (escape_offset, escape) = chars.next()?;
        let mut closing = start + 1 + escape_offset + escape.len_utf8();
        match escape {
            'x' => {
                closing = closing.checked_add(2)?;
            }
            'u' if text.as_bytes().get(closing) == Some(&b'{') => {
                closing += 1;
                let end = text.get(closing..)?.find('}')?;
                closing += end + 1;
            }
            '\n' | '\r' => return None,
            _ => {}
        }
        (text.as_bytes().get(closing) == Some(&b'\'')).then_some(closing + 1)
    }

    fn mask_range(masked: &mut [u8], start: usize, end: usize) {
        for byte in &mut masked[start..end] {
            if !matches!(*byte, b'\n' | b'\r') {
                *byte = b' ';
            }
        }
    }

    let bytes = text.as_bytes();
    let mut masked = bytes.to_vec();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes.get(cursor..cursor + 2) == Some(b"//") {
            let end = bytes[cursor + 2..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|offset| cursor + 2 + offset)
                .unwrap_or(bytes.len());
            mask_range(&mut masked, cursor, end);
            cursor = end;
            continue;
        }
        if bytes.get(cursor..cursor + 2) == Some(b"/*") {
            let mut end = cursor + 2;
            let mut depth = 1_u32;
            while end < bytes.len() && depth > 0 {
                if bytes.get(end..end + 2) == Some(b"/*") {
                    depth += 1;
                    end += 2;
                } else if bytes.get(end..end + 2) == Some(b"*/") {
                    depth -= 1;
                    end += 2;
                } else {
                    end += 1;
                }
            }
            mask_range(&mut masked, cursor, end);
            cursor = end;
            continue;
        }
        if let Some(end) = raw_string_end(bytes, cursor) {
            mask_range(&mut masked, cursor, end);
            cursor = end;
            continue;
        }
        if bytes[cursor] == b'"' {
            let end = quoted_string_end(bytes, cursor);
            mask_range(&mut masked, cursor, end);
            cursor = end;
            continue;
        }
        if bytes[cursor] == b'\'' {
            if let Some(end) = char_literal_end(text, cursor) {
                mask_range(&mut masked, cursor, end);
                cursor = end;
                continue;
            }
        }
        cursor += 1;
    }
    String::from_utf8(masked).expect("masking valid UTF-8 with ASCII spaces preserves UTF-8")
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
    fn macro_call_scan_ignores_literals_comments_and_lifetimes() {
        let token_tree = r####"{
            actual();
            receiver.real();
            let normal = "hidden_normal()";
            let bytes = b"hidden_bytes()";
            let raw = r#"hidden_raw()"#;
            let raw_hashes = r##"hidden_raw_hashes()"##;
            let raw_bytes = br##"hidden_raw_bytes()"##;
            let c_string = c"hidden_c_string()";
            let raw_c_string = cr#"hidden_raw_c_string()"#;
            let character = 'x';
            let escaped = '\n';
            let unicode = '🦀';
            let byte_character = b'y';
            let lifetime: &'static str = "";
            // hidden_line_comment()
            /* hidden_block_comment() /* hidden_nested_comment() */ */
            another(/* hidden_argument_comment() */);
        }"####;

        assert_eq!(
            textual_call_refs(token_tree),
            vec![
                ("actual".to_string(), None),
                ("real".to_string(), Some("receiver".to_string())),
                ("another".to_string(), None),
            ]
        );
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
