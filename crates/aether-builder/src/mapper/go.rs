//! Go extraction into Girder's language-neutral graph vocabulary.

use super::{node_text, span_of, BuildOutput, CallRef, InheritRef, RustImportRef};
use crate::parser::Lang;
use aether_graph::{Edge, EdgeKind, Node, NodeId, NodeKind};
use std::collections::{HashMap, HashSet};
use tree_sitter::{Node as TsNode, Tree};

// EXTENSION POINT: Generic declarations and type parameters are retained in
// Node.source but are not modeled as graph nodes or constraints; retrieval can
// find the declaration but cannot navigate a type-parameter relationship.
// EXTENSION POINT: Goroutines and channel sends/receives do not produce
// concurrency or dataflow edges; retrieval sees calls made by a go statement
// but cannot reconstruct happens-before or channel flow.
// EXTENSION POINT: Structurally inferred interface satisfaction is not modeled;
// retrieval follows only explicit struct/interface embedding, not implicit
// implementation relationships.
// EXTENSION POINT: cgo pseudo-imports and C selector dispatch are not resolved;
// retrieval cannot follow calls or types implemented behind import "C".
// EXTENSION POINT: Build tags and platform-conditional file selection are not
// evaluated; retrieval may include mutually exclusive declarations from the
// analyzed source tree.
// EXTENSION POINT: Method values, method expressions, and function-typed fields
// are not resolved as Calls edges; retrieval cannot follow deferred dispatch
// through a stored callable.
// EXTENSION POINT: Reflection-based dispatch is not modeled; retrieval cannot
// link calls selected through reflect or runtime name lookup.
// EXTENSION POINT: Vendored dependencies are not given module-replacement
// semantics; retrieval treats vendor trees as ordinary project directories and
// does not redirect imports to them.

const GO_IMPORT_PREFIX: &str = "go-import:";
const DOT_IMPORT_BINDING: &str = "__girder_go_dot_import";

#[derive(Clone)]
struct FunctionRange {
    start: usize,
    end: usize,
    id: NodeId,
    name: String,
    receiver_name: Option<String>,
    receiver_type: Option<String>,
    local_types: HashMap<String, String>,
    local_names: HashSet<String>,
}

#[derive(Default)]
struct TypeInfo {
    fields: HashMap<String, String>,
}

pub(super) fn extract(tree: &Tree, source: &str, file: &str, lang: Lang) -> BuildOutput {
    debug_assert_eq!(lang, Lang::Go);
    let module = module_path(file);
    let package_name = package_name(tree.root_node(), source).unwrap_or_default();
    let mut out = BuildOutput::default();

    let mut module_node = Node::new(NodeKind::Module, last_segment(&module), module.clone())
        .with_language(lang.name())
        .with_source(source);
    module_node.file = Some(file.to_string());
    module_node.span = span_of(tree.root_node());
    module_node.set_attr("source_module", &module);
    if !package_name.is_empty() {
        module_node.set_attr("package_name", package_name);
    }
    let module_id = module_node.id;
    out.nodes.push(module_node);

    let imports = collect_imports(tree.root_node(), source, &mut out);
    let mut types = HashMap::new();
    collect_types_and_constants(
        tree.root_node(),
        source,
        file,
        lang,
        &module,
        module_id,
        &imports,
        &mut types,
        &mut out,
    );
    let mut functions = Vec::new();
    collect_functions(
        tree.root_node(),
        source,
        file,
        lang,
        &module,
        module_id,
        &imports,
        &types,
        &mut functions,
        &mut out,
    );
    collect_calls(
        tree.root_node(),
        source,
        &imports,
        &functions,
        &types,
        &mut out,
    );
    out
}

fn module_path(file: &str) -> String {
    let normalized = file.replace('\\', "/");
    let directory = normalized
        .rsplit_once('/')
        .map(|(path, _)| path)
        .unwrap_or("");
    let parts = directory
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>();
    if parts.is_empty() {
        "crate".to_string()
    } else {
        format!("crate::{}", parts.join("::"))
    }
}

fn package_name<'a>(root: TsNode<'a>, source: &'a str) -> Option<&'a str> {
    let mut cursor = root.walk();
    let clause = root
        .named_children(&mut cursor)
        .find(|child| child.kind() == "package_clause")?;
    let mut clause_cursor = clause.walk();
    let identifier = clause
        .named_children(&mut clause_cursor)
        .find(|child| child.kind() == "package_identifier");
    identifier.map(|identifier| node_text(identifier, source).trim())
}

fn collect_imports(
    root: TsNode,
    source: &str,
    out: &mut BuildOutput,
) -> HashMap<String, Vec<String>> {
    let mut imports: HashMap<String, Vec<String>> = HashMap::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "import_spec" {
            let Some(path_node) = node.child_by_field_name("path") else {
                continue;
            };
            let import_path = unquote(node_text(path_node, source).trim());
            if import_path.is_empty() {
                continue;
            }
            let explicit_name = node
                .child_by_field_name("name")
                .map(|name| node_text(name, source).trim());
            let local = match explicit_name {
                Some(".") => DOT_IMPORT_BINDING,
                Some(name) => name,
                None => import_path.rsplit('/').next().unwrap_or(import_path),
            };
            let target = format!("{GO_IMPORT_PREFIX}{import_path}");
            imports
                .entry(local.to_string())
                .or_default()
                .push(target.clone());
            out.rust_imports.push(RustImportRef {
                local: local.to_string(),
                target,
                is_reexport: false,
                exact: true,
            });
            continue;
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    for targets in imports.values_mut() {
        targets.sort();
        targets.dedup();
    }
    imports
}

#[allow(clippy::too_many_arguments)]
fn collect_types_and_constants(
    root: TsNode,
    source: &str,
    file: &str,
    lang: Lang,
    module: &str,
    module_id: NodeId,
    imports: &HashMap<String, Vec<String>>,
    types: &mut HashMap<String, TypeInfo>,
    out: &mut BuildOutput,
) {
    let mut cursor = root.walk();
    for declaration in root.named_children(&mut cursor) {
        match declaration.kind() {
            "type_declaration" => {
                let mut declaration_cursor = declaration.walk();
                for spec in declaration.named_children(&mut declaration_cursor) {
                    if matches!(spec.kind(), "type_spec" | "type_alias") {
                        add_type(
                            spec, source, file, lang, module, module_id, imports, types, out,
                        );
                    }
                }
            }
            "const_declaration" => {
                let mut declaration_cursor = declaration.walk();
                for spec in declaration.named_children(&mut declaration_cursor) {
                    if spec.kind() == "const_spec" {
                        add_constants(spec, source, file, lang, module, module_id, out);
                    }
                }
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn add_type(
    spec: TsNode,
    source: &str,
    file: &str,
    lang: Lang,
    module: &str,
    module_id: NodeId,
    imports: &HashMap<String, Vec<String>>,
    types: &mut HashMap<String, TypeInfo>,
    out: &mut BuildOutput,
) {
    let Some(name_node) = spec.child_by_field_name("name") else {
        return;
    };
    let name = node_text(name_node, source).trim();
    let path = format!("{module}::{name}");
    let id = NodeId::from_path(&path);
    let mut graph_node = Node::new(NodeKind::Type, name, &path)
        .with_language(lang.name())
        .with_source(node_text(spec, source));
    graph_node.file = Some(file.to_string());
    graph_node.span = span_of(spec);
    graph_node.set_attr("source_module", module);
    graph_node.set_attr("go_declaration_kind", "type");
    if spec.child_by_field_name("type_parameters").is_some() {
        graph_node.set_attr("type_parameters", "unmodeled");
    }
    out.nodes.push(graph_node);
    out.edges
        .push((module_id, id, Edge::new(EdgeKind::Contains)));

    let mut info = TypeInfo::default();
    if let Some(type_node) = spec.child_by_field_name("type") {
        match type_node.kind() {
            "struct_type" => collect_struct_members(
                type_node, source, file, lang, module, id, imports, &mut info, out,
            ),
            "interface_type" => collect_interface_members(
                type_node, source, file, lang, module, id, imports, &mut info, out,
            ),
            _ => {}
        }
    }
    types.insert(name.to_string(), info);
}

fn add_constants(
    spec: TsNode,
    source: &str,
    file: &str,
    lang: Lang,
    module: &str,
    module_id: NodeId,
    out: &mut BuildOutput,
) {
    let mut cursor = spec.walk();
    for name_node in spec.children_by_field_name("name", &mut cursor) {
        if name_node.kind() != "identifier" {
            continue;
        }
        let name = node_text(name_node, source).trim();
        let path = format!("{module}::{name}");
        let id = NodeId::from_path(&path);
        let mut graph_node = Node::new(NodeKind::Field, name, &path)
            .with_language(lang.name())
            .with_source(node_text(spec, source));
        graph_node.file = Some(file.to_string());
        graph_node.span = span_of(spec);
        graph_node.set_attr("source_module", module);
        graph_node.set_attr("go_declaration_kind", "const");
        out.nodes.push(graph_node);
        out.edges
            .push((module_id, id, Edge::new(EdgeKind::Contains)));
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_struct_members(
    struct_type: TsNode,
    source: &str,
    file: &str,
    lang: Lang,
    module: &str,
    type_id: NodeId,
    imports: &HashMap<String, Vec<String>>,
    info: &mut TypeInfo,
    out: &mut BuildOutput,
) {
    let Some(fields) = named_child(struct_type, "field_declaration_list") else {
        return;
    };
    let mut cursor = fields.walk();
    for field in fields.named_children(&mut cursor) {
        if field.kind() != "field_declaration" {
            continue;
        }
        let Some(type_node) = field.child_by_field_name("type") else {
            continue;
        };
        let type_path = resolve_type_path(node_text(type_node, source), module, imports);
        let mut name_cursor = field.walk();
        let names = field
            .children_by_field_name("name", &mut name_cursor)
            .collect::<Vec<_>>();
        if names.is_empty() {
            if let Some(base) = type_path.clone() {
                let embedded_name = base.rsplit("::").next().unwrap_or(&base).to_string();
                info.fields.insert(embedded_name, base.clone());
                out.inherits.push(InheritRef { sub: type_id, base });
            }
            continue;
        }
        for name_node in names {
            let name = node_text(name_node, source).trim();
            if let Some(type_path) = type_path.clone() {
                info.fields.insert(name.to_string(), type_path);
            }
            add_field(field, name, source, file, lang, module, type_id, out);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_interface_members(
    interface_type: TsNode,
    source: &str,
    file: &str,
    lang: Lang,
    module: &str,
    type_id: NodeId,
    imports: &HashMap<String, Vec<String>>,
    info: &mut TypeInfo,
    out: &mut BuildOutput,
) {
    let mut cursor = interface_type.walk();
    for member in interface_type.named_children(&mut cursor) {
        match member.kind() {
            "method_elem" => {
                let Some(name_node) = member.child_by_field_name("name") else {
                    continue;
                };
                let name = node_text(name_node, source).trim();
                let owner_path = out
                    .nodes
                    .iter()
                    .find(|node| node.id == type_id)
                    .map(|node| node.path.clone())
                    .unwrap_or_else(|| module.to_string());
                add_function_node(
                    member,
                    name,
                    source,
                    file,
                    lang,
                    module,
                    &owner_path,
                    type_id,
                    false,
                    out,
                );
            }
            "type_elem" => {
                let embedded = node_text(member, source).trim();
                if let Some(base) = resolve_type_path(embedded, module, imports) {
                    let embedded_name = base.rsplit("::").next().unwrap_or(&base).to_string();
                    info.fields.insert(embedded_name, base.clone());
                    out.inherits.push(InheritRef { sub: type_id, base });
                }
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn add_field(
    syntax: TsNode,
    name: &str,
    source: &str,
    file: &str,
    lang: Lang,
    module: &str,
    type_id: NodeId,
    out: &mut BuildOutput,
) {
    let type_path = out
        .nodes
        .iter()
        .find(|node| node.id == type_id)
        .map(|node| node.path.as_str())
        .unwrap_or(module);
    let path = format!("{type_path}::{name}");
    let id = NodeId::from_path(&path);
    let mut graph_node = Node::new(NodeKind::Field, name, &path)
        .with_language(lang.name())
        .with_source(node_text(syntax, source));
    graph_node.file = Some(file.to_string());
    graph_node.span = span_of(syntax);
    graph_node.set_attr("source_module", module);
    out.nodes.push(graph_node);
    out.edges.push((type_id, id, Edge::new(EdgeKind::Contains)));
}

#[allow(clippy::too_many_arguments)]
fn collect_functions(
    root: TsNode,
    source: &str,
    file: &str,
    lang: Lang,
    module: &str,
    module_id: NodeId,
    imports: &HashMap<String, Vec<String>>,
    types: &HashMap<String, TypeInfo>,
    functions: &mut Vec<FunctionRange>,
    out: &mut BuildOutput,
) {
    let mut cursor = root.walk();
    for declaration in root.named_children(&mut cursor) {
        if !matches!(
            declaration.kind(),
            "function_declaration" | "method_declaration"
        ) {
            continue;
        }
        let Some(name_node) = declaration.child_by_field_name("name") else {
            continue;
        };
        let name = node_text(name_node, source).trim();
        let receiver = declaration
            .child_by_field_name("receiver")
            .and_then(|receiver| receiver_binding(receiver, source, module, imports));
        let (owner_path, owner_id, receiver_name, receiver_type) = match receiver {
            Some((binding, receiver_type)) => {
                let owner_path = receiver_type
                    .strip_prefix("go-type:")
                    .unwrap_or(&receiver_type)
                    .to_string();
                let owner_id = NodeId::from_path(&owner_path);
                (
                    owner_path,
                    owner_id,
                    binding.unwrap_or_default(),
                    Some(receiver_type),
                )
            }
            None => (module.to_string(), module_id, String::new(), None),
        };
        let path = format!("{owner_path}::{name}");
        let id = NodeId::from_path(&path);
        let is_test = receiver_type.is_none() && is_go_test(file, name);
        add_function_node(
            declaration,
            name,
            source,
            file,
            lang,
            module,
            &owner_path,
            owner_id,
            is_test,
            out,
        );
        let mut local_types = HashMap::new();
        let mut local_names = HashSet::new();
        if !receiver_name.is_empty() {
            local_names.insert(receiver_name.clone());
            if let Some(receiver_type) = receiver_type.clone() {
                local_types.insert(receiver_name.clone(), receiver_type);
            }
        }
        collect_local_bindings(
            declaration,
            source,
            module,
            imports,
            &mut local_names,
            &mut local_types,
        );
        if let Some(receiver_type) = receiver_type.as_deref() {
            if let Some(type_name) = receiver_type.rsplit("::").next() {
                if let Some(type_info) = types.get(type_name) {
                    for (field, field_type) in &type_info.fields {
                        local_types
                            .entry(format!("{receiver_name}.{field}"))
                            .or_insert_with(|| field_type.clone());
                    }
                }
            }
        }
        functions.push(FunctionRange {
            start: declaration.start_byte(),
            end: declaration.end_byte(),
            id,
            name: name.to_string(),
            receiver_name: (!receiver_name.is_empty()).then_some(receiver_name),
            receiver_type,
            local_types,
            local_names,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn add_function_node(
    syntax: TsNode,
    name: &str,
    source: &str,
    file: &str,
    lang: Lang,
    module: &str,
    owner_path: &str,
    owner_id: NodeId,
    is_test: bool,
    out: &mut BuildOutput,
) {
    let path = format!("{owner_path}::{name}");
    let id = NodeId::from_path(&path);
    let mut graph_node = Node::new(NodeKind::Function, name, &path)
        .with_language(lang.name())
        .with_source(node_text(syntax, source));
    graph_node.file = Some(file.to_string());
    graph_node.span = span_of(syntax);
    graph_node.set_attr("source_module", module);
    if is_test {
        graph_node.set_attr("is_test", "true");
    }
    if syntax.child_by_field_name("type_parameters").is_some() {
        graph_node.set_attr("type_parameters", "unmodeled");
    }
    if let Some(result) = syntax.child_by_field_name("result") {
        graph_node.set_attr("return_type", node_text(result, source).trim());
    }
    out.nodes.push(graph_node);
    out.edges
        .push((owner_id, id, Edge::new(EdgeKind::Contains)));
}

fn receiver_binding(
    receiver: TsNode,
    source: &str,
    module: &str,
    imports: &HashMap<String, Vec<String>>,
) -> Option<(Option<String>, String)> {
    let parameter = named_child(receiver, "parameter_declaration")?;
    let ty = parameter.child_by_field_name("type")?;
    let receiver_type = resolve_type_path(node_text(ty, source), module, imports)?;
    let name = parameter
        .child_by_field_name("name")
        .map(|name| node_text(name, source).trim().to_string());
    Some((name, receiver_type))
}

fn collect_local_bindings(
    syntax: TsNode,
    source: &str,
    module: &str,
    imports: &HashMap<String, Vec<String>>,
    names: &mut HashSet<String>,
    types: &mut HashMap<String, String>,
) {
    let mut stack = vec![syntax];
    while let Some(node) = stack.pop() {
        match node.kind() {
            "parameter_declaration" | "variadic_parameter_declaration" | "var_spec" => {
                let explicit_type = node
                    .child_by_field_name("type")
                    .and_then(|ty| resolve_type_path(node_text(ty, source), module, imports));
                let mut cursor = node.walk();
                for name_node in node.children_by_field_name("name", &mut cursor) {
                    let name = node_text(name_node, source).trim().to_string();
                    names.insert(name.clone());
                    if let Some(explicit_type) = explicit_type.clone() {
                        types.insert(name, explicit_type);
                    }
                }
            }
            "short_var_declaration" => {
                if let Some(left) = node.child_by_field_name("left") {
                    let mut cursor = left.walk();
                    for child in left.named_children(&mut cursor) {
                        if child.kind() == "identifier" {
                            names.insert(node_text(child, source).trim().to_string());
                        }
                    }
                }
            }
            _ => {}
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
}

fn collect_calls(
    root: TsNode,
    source: &str,
    imports: &HashMap<String, Vec<String>>,
    functions: &[FunctionRange],
    types: &HashMap<String, TypeInfo>,
    out: &mut BuildOutput,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "call_expression" {
            collect_call(node, source, imports, functions, types, out);
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
}

fn collect_call(
    call: TsNode,
    source: &str,
    imports: &HashMap<String, Vec<String>>,
    functions: &[FunctionRange],
    _types: &HashMap<String, TypeInfo>,
    out: &mut BuildOutput,
) {
    let Some(function_range) = functions
        .iter()
        .filter(|function| function.start <= call.start_byte() && function.end >= call.end_byte())
        .min_by_key(|function| function.end - function.start)
    else {
        return;
    };
    let Some(function) = call.child_by_field_name("function") else {
        return;
    };

    let (callee, qualifier, receiver_type, qualifier_owner_fallback, shadowed_by_local) =
        match function.kind() {
            "identifier" => {
                let callee = node_text(function, source).trim().to_string();
                let shadowed = function_range.local_names.contains(&callee);
                let dot_import = imports
                    .get(DOT_IMPORT_BINDING)
                    .is_some_and(|targets| targets.len() == 1)
                    && !functions.iter().any(|candidate| {
                        candidate.receiver_type.is_none() && candidate.name == callee
                    });
                let qualifier = (!shadowed && dot_import).then(|| DOT_IMPORT_BINDING.to_string());
                (callee, qualifier, None, false, shadowed)
            }
            "selector_expression" => {
                let Some(field) = function.child_by_field_name("field") else {
                    return;
                };
                let Some(operand) = function.child_by_field_name("operand") else {
                    return;
                };
                let callee = node_text(field, source).trim().to_string();
                let operand_text = node_text(operand, source).trim().to_string();
                if imports.contains_key(&operand_text) {
                    (callee, Some(operand_text), None, true, false)
                } else if function_range.receiver_name.as_deref() == Some(operand_text.as_str()) {
                    (
                        callee,
                        Some("self".to_string()),
                        function_range.receiver_type.clone(),
                        true,
                        false,
                    )
                } else {
                    let receiver_type = function_range.local_types.get(&operand_text).cloned();
                    (
                        callee,
                        Some(operand_text),
                        receiver_type.clone(),
                        receiver_type.is_some(),
                        false,
                    )
                }
            }
            _ => return,
        };

    out.calls.push(CallRef {
        caller: function_range.id,
        callee,
        qualifier,
        receiver_type,
        receiver_factory: None,
        qualifier_owner_fallback,
        process_entrypoint: None,
        route: None,
        route_guard: None,
        first_string_argument: None,
        shadowed_by_local,
    });
}

fn resolve_type_path(
    raw: &str,
    module: &str,
    imports: &HashMap<String, Vec<String>>,
) -> Option<String> {
    let mut ty = raw.trim();
    while let Some(stripped) = ty.strip_prefix('*') {
        ty = stripped.trim();
    }
    if ty.is_empty()
        || ty.starts_with("[]")
        || ty.starts_with("map[")
        || ty.starts_with("chan ")
        || ty.starts_with("func(")
        || ty.starts_with("interface {")
        || ty.contains(['[', ']', '|', '~'])
    {
        return None;
    }
    if let Some((qualifier, name)) = ty.split_once('.') {
        let targets = imports.get(qualifier)?;
        let [target] = targets.as_slice() else {
            return None;
        };
        return Some(format!("go-type:{target}::{name}"));
    }
    ty.chars()
        .all(|character| character == '_' || character.is_alphanumeric())
        .then(|| format!("{module}::{ty}"))
}

fn is_go_test(file: &str, name: &str) -> bool {
    if !file.replace('\\', "/").ends_with("_test.go") {
        return false;
    }
    ["Test", "Benchmark", "Fuzz", "Example"]
        .iter()
        .any(|prefix| {
            name.strip_prefix(prefix).is_some_and(|suffix| {
                suffix
                    .chars()
                    .next()
                    .is_some_and(|character| !character.is_lowercase())
            })
        })
}

fn named_child<'tree>(node: TsNode<'tree>, kind: &str) -> Option<TsNode<'tree>> {
    let mut cursor = node.walk();
    let child = node
        .named_children(&mut cursor)
        .find(|child| child.kind() == kind);
    child
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('`')
                .and_then(|value| value.strip_suffix('`'))
        })
        .unwrap_or(value)
}

fn last_segment(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::IncrementalParser;

    #[test]
    fn retains_aliased_dot_and_blank_imports_without_rewriting_their_paths() {
        let source = r#"
package sample
import (
    alias "example.com/project/library"
    . "example.com/project/dotted"
    _ "example.com/project/hooks"
)
func run() { alias.Target(); DotTarget() }
"#;
        let mut parser = IncrementalParser::new(Lang::Go);
        let tree = parser.parse(source);
        let output = extract(&tree, source, "sample.go", Lang::Go);
        let imports = output
            .rust_imports
            .iter()
            .map(|import| (import.local.as_str(), import.target.as_str(), import.exact))
            .collect::<Vec<_>>();
        assert!(imports.contains(&("alias", "go-import:example.com/project/library", true)));
        assert!(imports.contains(&(
            DOT_IMPORT_BINDING,
            "go-import:example.com/project/dotted",
            true
        )));
        assert!(imports.contains(&("_", "go-import:example.com/project/hooks", true)));
    }
}
