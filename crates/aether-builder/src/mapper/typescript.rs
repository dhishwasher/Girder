//! TypeScript/TSX extraction into Girder's language-neutral graph vocabulary.

use super::{node_text, span_of, BuildOutput, CallRef, InheritRef, RustImportRef};
use crate::parser::Lang;
use aether_graph::{Edge, EdgeKind, Node, NodeId, NodeKind};
use std::collections::HashMap;
use tree_sitter::{Node as TsNode, Tree};

#[derive(Clone)]
struct ImportBinding {
    target: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Module,
    Type,
    Function,
}

#[derive(Clone)]
struct Scope {
    path: String,
    id: NodeId,
    kind: ScopeKind,
}

#[derive(Clone, Copy)]
struct FunctionRange {
    start: usize,
    end: usize,
    id: NodeId,
}

pub(super) fn extract(tree: &Tree, source: &str, file: &str, lang: Lang) -> BuildOutput {
    debug_assert!(lang.is_typescript());
    let module = module_path(file);
    let mut out = BuildOutput::default();

    let mut module_node = Node::new(NodeKind::Module, last_segment(&module), module.clone())
        .with_language(lang.name())
        .with_source(source);
    module_node.file = Some(file.to_string());
    module_node.span = span_of(tree.root_node());
    module_node.set_attr("source_projection", "file-v1");
    let module_id = module_node.id;
    out.nodes.push(module_node);

    let mut imports = HashMap::new();
    collect_imports(tree.root_node(), source, file, &mut imports, &mut out);
    collect_exports(tree.root_node(), source, file, &module, &imports, &mut out);

    let scope = Scope {
        path: module,
        id: module_id,
        kind: ScopeKind::Module,
    };
    let mut functions = Vec::new();
    collect_definitions(
        tree.root_node(),
        source,
        file,
        lang,
        &scope,
        &imports,
        &mut functions,
        &mut out,
    );
    collect_calls(tree.root_node(), source, &imports, &functions, &mut out);
    out
}

fn module_path(file: &str) -> String {
    let normalized = file.replace('\\', "/");
    let without_extension = [".d.ts", ".d.mts", ".d.cts", ".tsx", ".mts", ".cts", ".ts"]
        .iter()
        .find_map(|extension| normalized.strip_suffix(extension))
        .unwrap_or(&normalized);
    let mut parts = without_extension
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>();
    if parts.first() == Some(&"src") {
        parts.remove(0);
    }
    if parts.is_empty() {
        "crate".to_string()
    } else {
        format!("crate::{}", parts.join("::"))
    }
}

fn import_module_path(file: &str, specifier: &str) -> Option<String> {
    if !specifier.starts_with('.') {
        return None;
    }
    let normalized = file.replace('\\', "/");
    let mut parts = normalized
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .map(str::to_string)
        .collect::<Vec<_>>();
    parts.pop();
    for part in specifier.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other.to_string()),
        }
    }
    let joined = parts.join("/");
    Some(module_path(&joined))
}

fn collect_imports(
    node: TsNode,
    source: &str,
    file: &str,
    bindings: &mut HashMap<String, ImportBinding>,
    out: &mut BuildOutput,
) {
    if node.kind() == "import_statement" {
        collect_import_statement(node, source, file, bindings, out);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_imports(child, source, file, bindings, out);
    }
}

fn collect_import_statement(
    statement: TsNode,
    source: &str,
    file: &str,
    bindings: &mut HashMap<String, ImportBinding>,
    out: &mut BuildOutput,
) {
    let Some(source_node) = statement.child_by_field_name("source") else {
        // EXTENSION POINT: CommonJS `import x = require(...)` and `require(...)`
        // bindings are not modeled. Retrieval cannot follow those bindings to
        // definitions or use them to disambiguate call edges.
        return;
    };
    let Some(target_module) = import_module_path(file, string_value(source_node, source)) else {
        // Bare package imports intentionally stay unresolved: the analyzed
        // project has no graph nodes for code outside its pinned source tree.
        return;
    };
    let Some(clause) = named_child(statement, "import_clause") else {
        return;
    };

    let mut cursor = clause.walk();
    for child in clause.named_children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let local = node_text(child, source);
                push_import(
                    local,
                    format!("{target_module}::default"),
                    false,
                    bindings,
                    out,
                );
            }
            "namespace_import" => {
                if let Some(identifier) = first_named_child(child) {
                    let local = node_text(identifier, source);
                    push_import(local, target_module.clone(), false, bindings, out);
                }
            }
            "named_imports" => {
                let mut named_cursor = child.walk();
                for specifier in child.named_children(&mut named_cursor) {
                    if specifier.kind() != "import_specifier" {
                        continue;
                    }
                    let Some(name) = specifier.child_by_field_name("name") else {
                        continue;
                    };
                    let imported = string_value(name, source);
                    let local = specifier
                        .child_by_field_name("alias")
                        .map(|alias| string_value(alias, source))
                        .unwrap_or(imported);
                    push_import(
                        local,
                        format!("{target_module}::{imported}"),
                        false,
                        bindings,
                        out,
                    );
                }
            }
            "import_require_clause" => {
                // EXTENSION POINT: CommonJS require aliases are deliberately
                // unhandled; calls through them remain unresolved in retrieval.
            }
            _ => {}
        }
    }
}

fn push_import(
    local: &str,
    target: String,
    is_reexport: bool,
    bindings: &mut HashMap<String, ImportBinding>,
    out: &mut BuildOutput,
) {
    bindings.insert(
        local.to_string(),
        ImportBinding {
            target: target.clone(),
        },
    );
    out.rust_imports.push(RustImportRef {
        local: local.to_string(),
        target,
        is_reexport,
        exact: true,
    });
}

fn collect_exports(
    node: TsNode,
    source: &str,
    file: &str,
    module: &str,
    imports: &HashMap<String, ImportBinding>,
    out: &mut BuildOutput,
) {
    if node.kind() == "export_statement" {
        collect_export_statement(node, source, file, module, imports, out);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_exports(child, source, file, module, imports, out);
    }
}

fn collect_export_statement(
    statement: TsNode,
    source: &str,
    file: &str,
    module: &str,
    imports: &HashMap<String, ImportBinding>,
    out: &mut BuildOutput,
) {
    let target_module = statement
        .child_by_field_name("source")
        .and_then(|source_node| import_module_path(file, string_value(source_node, source)));

    if let Some(clause) = named_child(statement, "export_clause") {
        let mut cursor = clause.walk();
        for specifier in clause.named_children(&mut cursor) {
            if specifier.kind() != "export_specifier" {
                continue;
            }
            let Some(name_node) = specifier.child_by_field_name("name") else {
                continue;
            };
            let name = string_value(name_node, source);
            let local = specifier
                .child_by_field_name("alias")
                .map(|alias| string_value(alias, source))
                .unwrap_or(name);
            let target = target_module
                .as_ref()
                .map(|target_module| format!("{target_module}::{name}"))
                .or_else(|| imports.get(name).map(|binding| binding.target.clone()))
                .unwrap_or_else(|| format!("{module}::{name}"));
            out.rust_imports.push(RustImportRef {
                local: local.to_string(),
                target,
                is_reexport: true,
                exact: true,
            });
        }
        return;
    }

    if let Some(target_module) = target_module {
        if let Some(namespace) = named_child(statement, "namespace_export") {
            if let Some(name) = first_named_child(namespace) {
                out.rust_imports.push(RustImportRef {
                    local: node_text(name, source).to_string(),
                    target: target_module,
                    is_reexport: true,
                    exact: true,
                });
            }
        } else if node_text(statement, source).contains('*') {
            out.rust_imports.push(RustImportRef {
                local: "*".to_string(),
                target: target_module,
                is_reexport: true,
                exact: true,
            });
        }
        return;
    }

    if node_text(statement, source)
        .trim_start()
        .starts_with("export default")
    {
        let target = statement
            .child_by_field_name("declaration")
            .and_then(|declaration| declaration.child_by_field_name("name"))
            .map(|name| format!("{module}::{}", node_text(name, source)))
            .or_else(|| {
                statement
                    .child_by_field_name("value")
                    .or_else(|| first_identifier_child(statement))
                    .map(|value| node_text(value, source))
                    .map(|name| {
                        imports
                            .get(name)
                            .map(|binding| binding.target.clone())
                            .unwrap_or_else(|| format!("{module}::{name}"))
                    })
            });
        if let Some(target) = target {
            out.rust_imports.push(RustImportRef {
                local: "default".to_string(),
                target,
                is_reexport: true,
                exact: true,
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_definitions(
    node: TsNode,
    source: &str,
    file: &str,
    lang: Lang,
    scope: &Scope,
    imports: &HashMap<String, ImportBinding>,
    functions: &mut Vec<FunctionRange>,
    out: &mut BuildOutput,
) {
    match node.kind() {
        "decorator" => {
            // EXTENSION POINT: decorators are not semantic graph nodes and
            // decorator calls are not attributed. Retrieval still returns the
            // decorated declaration, but cannot answer decorator relationships.
            return;
        }
        "ambient_declaration" | "internal_module" => {
            // EXTENSION POINT: `declare global`, quoted module augmentation,
            // and namespace bodies are not lowered. Ordinary top-level aliases
            // and interfaces in `.d.ts` files do not use this path and remain
            // fully retrievable. Augmented or ambient-only members may be absent.
            return;
        }
        "function_declaration" | "generator_function_declaration" => {
            if let Some(name) = node.child_by_field_name("name") {
                add_function(
                    node,
                    node_text(name, source),
                    source,
                    file,
                    lang,
                    scope,
                    false,
                    functions,
                    out,
                );
                let inner = function_scope(scope, node_text(name, source));
                recurse_definitions(node, source, file, lang, &inner, imports, functions, out);
            }
            return;
        }
        "method_definition" => {
            if let Some(name) = node.child_by_field_name("name") {
                let name = node_text(name, source);
                add_function(node, name, source, file, lang, scope, false, functions, out);
                let inner = function_scope(scope, name);
                recurse_definitions(node, source, file, lang, &inner, imports, functions, out);
            }
            return;
        }
        "class_declaration"
        | "abstract_class_declaration"
        | "interface_declaration"
        | "type_alias_declaration"
        | "enum_declaration" => {
            if let Some(name) = node.child_by_field_name("name") {
                let name = node_text(name, source);
                let path = format!("{}::{name}", scope.path);
                let id = NodeId::from_path(&path);
                let mut graph_node = Node::new(NodeKind::Type, name, &path)
                    .with_language(lang.name())
                    .with_source(node_text(node, source));
                graph_node.file = Some(file.to_string());
                graph_node.span = span_of(node);
                // EXTENSION POINT: generic/type-parameter declarations are
                // preserved in Node.source but are not separate nodes or edges.
                // Retrieval finds the owning declaration, not an individual
                // type parameter, its constraint, or its instantiations.
                out.nodes.push(graph_node);
                out.edges
                    .push((scope.id, id, Edge::new(EdgeKind::Contains)));
                collect_heritage(node, source, id, imports, out);
                if node.kind() != "type_alias_declaration" {
                    let inner = Scope {
                        path,
                        id,
                        kind: ScopeKind::Type,
                    };
                    recurse_definitions(node, source, file, lang, &inner, imports, functions, out);
                }
            }
            return;
        }
        "variable_declarator" => {
            let value = node.child_by_field_name("value");
            let name = node.child_by_field_name("name");
            if let (Some(value), Some(name)) = (value, name) {
                if name.kind() == "identifier"
                    && matches!(value.kind(), "arrow_function" | "function_expression")
                {
                    let name = node_text(name, source);
                    add_function(node, name, source, file, lang, scope, false, functions, out);
                    let inner = function_scope(scope, name);
                    recurse_definitions(value, source, file, lang, &inner, imports, functions, out);
                    return;
                }
            }
        }
        "public_field_definition" => {
            if let Some(name) = node.child_by_field_name("name") {
                let name = node_text(name, source);
                if node
                    .child_by_field_name("value")
                    .is_some_and(|value| value.kind() == "arrow_function")
                {
                    add_function(node, name, source, file, lang, scope, false, functions, out);
                } else if scope.kind == ScopeKind::Type {
                    add_field(node, name, source, file, lang, scope, out);
                }
                return;
            }
        }
        "property_signature" => {
            if scope.kind == ScopeKind::Type {
                if let Some(name) = node.child_by_field_name("name") {
                    add_field(
                        node,
                        node_text(name, source),
                        source,
                        file,
                        lang,
                        scope,
                        out,
                    );
                }
            }
            return;
        }
        "call_expression" if scope.kind != ScopeKind::Function => {
            if let Some((framework, title, callback)) = test_registration(node, source) {
                if framework == "describe" {
                    recurse_definitions(
                        callback, source, file, lang, scope, imports, functions, out,
                    );
                } else {
                    add_function(node, title, source, file, lang, scope, true, functions, out);
                    let inner = function_scope(scope, title);
                    recurse_definitions(
                        callback, source, file, lang, &inner, imports, functions, out,
                    );
                }
                return;
            }
        }
        _ => {}
    }
    recurse_definitions(node, source, file, lang, scope, imports, functions, out);
}

#[allow(clippy::too_many_arguments)]
fn recurse_definitions(
    node: TsNode,
    source: &str,
    file: &str,
    lang: Lang,
    scope: &Scope,
    imports: &HashMap<String, ImportBinding>,
    functions: &mut Vec<FunctionRange>,
    out: &mut BuildOutput,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_definitions(child, source, file, lang, scope, imports, functions, out);
    }
}

#[allow(clippy::too_many_arguments)]
fn add_function(
    syntax: TsNode,
    name: &str,
    source: &str,
    file: &str,
    lang: Lang,
    scope: &Scope,
    is_test: bool,
    functions: &mut Vec<FunctionRange>,
    out: &mut BuildOutput,
) {
    let path_name = name.replace("::", ":");
    let path = format!("{}::{path_name}", scope.path);
    let id = NodeId::from_path(&path);
    let mut node = Node::new(NodeKind::Function, name, &path)
        .with_language(lang.name())
        .with_source(node_text(syntax, source));
    node.file = Some(file.to_string());
    node.span = span_of(syntax);
    if is_test {
        node.set_attr("is_test", "true");
    }
    if let Some(return_type) = syntax.child_by_field_name("return_type") {
        node.set_attr("return_type", node_text(return_type, source).trim());
    }
    out.nodes.push(node);
    out.edges
        .push((scope.id, id, Edge::new(EdgeKind::Contains)));
    functions.push(FunctionRange {
        start: syntax.start_byte(),
        end: syntax.end_byte(),
        id,
    });
}

fn add_field(
    syntax: TsNode,
    name: &str,
    source: &str,
    file: &str,
    lang: Lang,
    scope: &Scope,
    out: &mut BuildOutput,
) {
    let path = format!("{}::{name}", scope.path);
    let id = NodeId::from_path(&path);
    let mut node = Node::new(NodeKind::Field, name, &path)
        .with_language(lang.name())
        .with_source(node_text(syntax, source));
    node.file = Some(file.to_string());
    node.span = span_of(syntax);
    out.nodes.push(node);
    out.edges
        .push((scope.id, id, Edge::new(EdgeKind::Contains)));
}

fn function_scope(scope: &Scope, name: &str) -> Scope {
    let path_name = name.replace("::", ":");
    let path = format!("{}::{path_name}", scope.path);
    Scope {
        id: NodeId::from_path(&path),
        path,
        kind: ScopeKind::Function,
    }
}

fn collect_heritage(
    declaration: TsNode,
    source: &str,
    sub: NodeId,
    imports: &HashMap<String, ImportBinding>,
    out: &mut BuildOutput,
) {
    let mut stack = vec![declaration];
    while let Some(node) = stack.pop() {
        if node != declaration
            && matches!(
                node.kind(),
                "class_declaration"
                    | "abstract_class_declaration"
                    | "interface_declaration"
                    | "type_alias_declaration"
            )
        {
            continue;
        }
        if matches!(
            node.kind(),
            "extends_clause" | "implements_clause" | "extends_type_clause"
        ) {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "type_arguments" {
                    continue;
                }
                let raw = node_text(child, source);
                if let Some(base) = resolve_type_target(raw, imports) {
                    out.inherits.push(InheritRef { sub, base });
                }
            }
            continue;
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
}

fn resolve_type_target(raw: &str, imports: &HashMap<String, ImportBinding>) -> Option<String> {
    let without_generics = raw.split('<').next()?.trim();
    let mut parts = without_generics.split('.');
    let first = parts.next()?.trim();
    if first.is_empty() {
        return None;
    }
    if let Some(binding) = imports.get(first) {
        let suffix = parts.collect::<Vec<_>>().join("::");
        return if suffix.is_empty() {
            Some(binding.target.clone())
        } else {
            Some(format!("{}::{suffix}", binding.target))
        };
    }
    Some(
        without_generics
            .rsplit('.')
            .next()
            .unwrap_or(without_generics)
            .trim()
            .to_string(),
    )
}

fn test_registration<'a>(
    node: TsNode<'a>,
    source: &'a str,
) -> Option<(&'a str, &'a str, TsNode<'a>)> {
    let callee = node.child_by_field_name("function")?;
    let framework = match callee.kind() {
        "identifier" => node_text(callee, source),
        "member_expression" => callee
            .child_by_field_name("property")
            .map(|property| node_text(property, source))?,
        _ => return None,
    };
    if !matches!(framework, "describe" | "it" | "test") {
        return None;
    }
    let arguments = node.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    let args = arguments.named_children(&mut cursor).collect::<Vec<_>>();
    let title_node = args.first().copied().filter(|arg| arg.kind() == "string")?;
    let callback = args.iter().copied().find(|arg| {
        matches!(
            arg.kind(),
            "arrow_function" | "function_expression" | "function"
        )
    })?;
    Some((framework, string_value(title_node, source), callback))
}

fn collect_calls(
    node: TsNode,
    source: &str,
    imports: &HashMap<String, ImportBinding>,
    functions: &[FunctionRange],
    out: &mut BuildOutput,
) {
    if node.kind() == "decorator" {
        return;
    }
    if node.kind() == "call_expression" {
        collect_call(node, source, imports, functions, out);
    }
    // EXTENSION POINT: JSX component elements are not interpreted as Calls
    // edges. A TSX function that returns JSX is still a normal Function node,
    // but retrieval cannot navigate from it to the rendered component.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_calls(child, source, imports, functions, out);
    }
}

fn collect_call(
    call: TsNode,
    source: &str,
    imports: &HashMap<String, ImportBinding>,
    functions: &[FunctionRange],
    out: &mut BuildOutput,
) {
    let Some(caller) = functions
        .iter()
        .filter(|function| function.start <= call.start_byte() && function.end >= call.end_byte())
        .min_by_key(|function| function.end - function.start)
        .map(|function| function.id)
    else {
        return;
    };
    let Some(function) = call.child_by_field_name("function") else {
        return;
    };
    let (callee, qualifier, qualifier_owner_fallback) = match function.kind() {
        "identifier" => {
            let callee = node_text(function, source);
            if matches!(callee, "require" | "import") {
                // EXTENSION POINT: CommonJS `require()` and dynamic `import()`
                // do not produce call/import relationships. Retrieval cannot
                // follow modules loaded only through these runtime forms.
                return;
            }
            (callee.to_string(), None, false)
        }
        "member_expression" => {
            let Some(property) = function.child_by_field_name("property") else {
                return;
            };
            let Some(object) = function.child_by_field_name("object") else {
                return;
            };
            let qualifier = node_text(object, source).trim();
            if qualifier.contains(['(', '[', '?']) {
                return;
            }
            let fallback =
                qualifier == "this" || qualifier == "super" || imports.contains_key(qualifier);
            let qualifier = match qualifier {
                "this" | "super" => "self",
                other => other,
            };
            (
                string_value(property, source).to_string(),
                Some(qualifier.to_string()),
                fallback,
            )
        }
        _ => return,
    };
    out.calls.push(CallRef {
        caller,
        callee,
        qualifier,
        receiver_type: None,
        receiver_factory: None,
        qualifier_owner_fallback,
        process_entrypoint: None,
        route: None,
        route_guard: None,
        first_string_argument: None,
        shadowed_by_local: false,
    });
}

fn named_child<'a>(node: TsNode<'a>, kind: &str) -> Option<TsNode<'a>> {
    let mut cursor = node.walk();
    let found = node
        .named_children(&mut cursor)
        .find(|child| child.kind() == kind);
    found
}

fn first_named_child(node: TsNode) -> Option<TsNode> {
    let mut cursor = node.walk();
    let first = node.named_children(&mut cursor).next();
    first
}

fn first_identifier_child(node: TsNode) -> Option<TsNode> {
    let mut cursor = node.walk();
    let found = node
        .named_children(&mut cursor)
        .find(|child| child.kind() == "identifier");
    found
}

fn string_value<'a>(node: TsNode, source: &'a str) -> &'a str {
    let text = node_text(node, source);
    if text.len() >= 2
        && matches!(text.as_bytes().first(), Some(b'\'' | b'"'))
        && text.as_bytes().first() == text.as_bytes().last()
    {
        &text[1..text.len() - 1]
    } else {
        text
    }
}

fn last_segment(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
}
