//! Independent syntax inventory for call evidence. The legacy name resolver is
//! useful for navigation, but its chosen candidate is not proof of a binding.

use super::{node_text, span_of, BuildOutput};
use crate::parser::Lang;
use aether_graph::{CallClaim, CallClass, CallEvidence, NodeId, NodeKind};
use std::collections::{HashMap, HashSet};
use tree_sitter::{Node as TsNode, Tree};

pub(super) fn annotate(tree: &Tree, source: &str, lang: Lang, out: &mut BuildOutput) {
    let root = tree.root_node();
    let Some(module) = out
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::Module && n.source == source)
    else {
        return;
    };
    let mut syntax = Vec::new();
    walk(root, &mut syntax);
    let callable =
        |n: &TsNode<'_>| matches!(n.kind(), "call_expression" | "call" | "new_expression");
    let mut claims: HashMap<usize, Vec<CallClaim>> = HashMap::new();
    let mut seen = HashSet::new();
    let duplicate_paths = out.nodes.iter().any(|n| !seen.insert(n.id));
    let transformed_scope = syntax.iter().any(|n| {
        matches!(n.kind(), "decorator" | "decorated_definition")
            || (matches!(n.kind(), "attribute_item" | "inner_attribute_item")
                && !matches!(node_text(*n, source).trim(), "#[test]" | "#[tokio::test]"))
            || (n.kind() == "macro_invocation" && owner(*n, out, module) == module)
    });
    // Direct-call proof is deliberately restricted to top-level declarations.
    // Do not certify nested functions, methods, imports, aliases or factories
    // until their binding/dispatch rules are implemented and measured.
    let mut top = HashMap::<String, Vec<(NodeId, usize)>>::new();
    for n in &syntax {
        if !matches!(
            n.kind(),
            "function_item" | "function_definition" | "function_declaration"
        ) {
            continue;
        }
        let Some(parent) = n.parent() else { continue };
        let top_level = parent.id() == root.id()
            || (parent.kind() == "export_statement"
                && parent.parent().is_some_and(|p| p.id() == root.id()));
        if !top_level || n.child_by_field_name("body").is_none() {
            continue;
        }
        if lang == Lang::Rust
            && node_text(*n, source)
                .split('{')
                .next()
                .unwrap_or("")
                .contains("extern")
        {
            continue;
        }
        let Some(name) = n.child_by_field_name("name") else {
            continue;
        };
        let Some(node) = out.nodes.iter().find(|node| {
            node.kind == NodeKind::Function
                && node.span.start_byte == n.start_byte()
                && node.span.end_byte == n.end_byte()
        }) else {
            continue;
        };
        top.entry(node_text(name, source).to_string())
            .or_default()
            .push((node.id, name.id()));
    }
    let ts_module = !lang.is_typescript()
        || syntax
            .iter()
            .any(|n| matches!(n.kind(), "export_statement" | "import_statement"));
    let mut proven = HashMap::new();
    if !root.has_error() && !duplicate_paths && !transformed_scope && ts_module {
        for (name, candidates) in top {
            if candidates.len() != 1 {
                continue;
            }
            let (target, declaration) = candidates[0];
            // Every occurrence must be either the declaration or a direct call.
            // This rejects parameter/local/import shadowing, assignments,
            // escaping function values, and unmodeled name uses conservatively.
            let clean = syntax
                .iter()
                .filter(|n| {
                    matches!(
                        n.kind(),
                        "identifier"
                            | "field_identifier"
                            | "property_identifier"
                            | "type_identifier"
                    ) && node_text(**n, source) == name
                })
                .all(|n| {
                    n.id() == declaration
                        || n.parent().is_some_and(|p| {
                            callable(&p)
                                && p.kind() != "new_expression"
                                && p.child_by_field_name("function")
                                    .is_some_and(|f| f.id() == n.id())
                        })
                });
            if clean {
                proven.insert(name, target);
            }
        }
    }
    let macro_owners: HashSet<usize> = syntax
        .iter()
        .filter(|n| n.kind() == "macro_invocation")
        .map(|n| owner(*n, out, module))
        .collect();
    for n in &syntax {
        if callable(n) {
            let index = owner(*n, out, module);
            let function = n.child_by_field_name("function");
            let target = function
                .filter(|f| f.kind() == "identifier")
                .filter(|_| n.kind() != "new_expression" && !macro_owners.contains(&index))
                .and_then(|f| proven.get(node_text(f, source)))
                .copied();
            let reason = if target.is_some() {
                "proven-top-level-lexical-binding"
            } else {
                unresolved_reason(*n, source, lang)
            };
            claims.entry(index).or_default().push(CallClaim {
                site: span_of(*n),
                class: if target.is_some() {
                    CallClass::Must
                } else {
                    CallClass::Unknown
                },
                targets: target.into_iter().collect(),
                reason: reason.into(),
                coverage_gap: false,
            });
        } else if matches!(
            n.kind(),
            "macro_invocation" | "decorator" | "attribute_item" | "inner_attribute_item"
        ) {
            claims
                .entry(owner(*n, out, module))
                .or_default()
                .push(CallClaim {
                    site: span_of(*n),
                    class: CallClass::Unknown,
                    targets: vec![],
                    reason: "unexpanded-macro-or-decorator".into(),
                    coverage_gap: true,
                });
        }
    }
    let mut gaps = Vec::new();
    if root.has_error() {
        gaps.push("parse-error");
    }
    if duplicate_paths {
        gaps.push("duplicate-semantic-path");
    }
    // These are explicit coverage boundaries, never invented missing-call counts.
    // Attribute access, operators, iteration and destruction can invoke code
    // without a syntactic call expression. Their dispatch is not yet certified.
    if lang == Lang::Python || lang.is_typescript() {
        gaps.push("implicit-runtime-dispatch-not-certified");
    }
    if lang == Lang::Rust
        && syntax.iter().any(|n| {
            matches!(
                n.kind(),
                "let_declaration"
                    | "binary_expression"
                    | "unary_expression"
                    | "try_expression"
                    | "index_expression"
                    | "for_expression"
                    | "impl_item"
            )
        })
    {
        gaps.push("implicit-drop-or-operator-dispatch-not-certified");
    }
    if lang == Lang::Go
        && source
            .lines()
            .any(|line| line.trim_start().starts_with("//go:"))
    {
        gaps.push("go-build-or-compiler-directive-not-certified");
    }
    for reason in gaps {
        claims.entry(module).or_default().push(CallClaim {
            site: span_of(root),
            class: CallClass::Unknown,
            targets: vec![],
            reason: reason.into(),
            coverage_gap: true,
        });
    }
    for (index, node) in out
        .nodes
        .iter_mut()
        .enumerate()
        .filter(|(_, node)| node.kind == NodeKind::Function || node.kind == NodeKind::Module)
    {
        let mut assumptions = vec!["indexed-source-snapshot".into()];
        if lang == Lang::Python || lang.is_typescript() {
            assumptions.push("no-runtime-rebinding-or-monkey-patching".into());
        }
        let evidence =
            CallEvidence::new(node, claims.remove(&index).unwrap_or_default(), assumptions);
        // All fields are strings/integers/enums supported by RON. Should encoding
        // nevertheless fail, missing evidence is surfaced as Unknown by queries.
        let _ = evidence.attach(node);
    }
}

fn walk<'tree>(node: TsNode<'tree>, nodes: &mut Vec<TsNode<'tree>>) {
    nodes.push(node);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk(child, nodes);
    }
}

fn owner(node: TsNode<'_>, out: &BuildOutput, module: usize) -> usize {
    out.nodes
        .iter()
        .enumerate()
        .filter(|(_, candidate)| {
            candidate.kind == NodeKind::Function
                && candidate.span.start_byte <= node.start_byte()
                && candidate.span.end_byte >= node.end_byte()
        })
        .min_by_key(|(_, candidate)| candidate.span.end_byte - candidate.span.start_byte)
        .map_or(module, |(index, _)| index)
}

fn unresolved_reason(node: TsNode<'_>, source: &str, lang: Lang) -> &'static str {
    let Some(function) = node.child_by_field_name("function") else {
        return "unresolved-call-syntax";
    };
    let text = node_text(function, source);
    match lang {
        Lang::Python if matches!(text, "getattr" | "setattr" | "eval" | "exec" | "__import__") => {
            "python-reflection"
        }
        Lang::Python => "python-binding-or-dispatch-unproven",
        Lang::TypeScript | Lang::Tsx => "typescript-binding-or-structural-dispatch-unproven",
        Lang::Go if text.starts_with("reflect.") || text.starts_with("C.") => {
            "go-reflection-or-ffi"
        }
        Lang::Go => "go-binding-or-interface-dispatch-unproven",
        Lang::Rust => "rust-binding-or-dispatch-unproven",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GraphBuilder;
    use aether_graph::SemanticGraph;

    fn graph(file: &str, source: &str) -> SemanticGraph {
        let mut graph = SemanticGraph::new();
        GraphBuilder::new().load_file(&mut graph, file, source);
        graph
    }

    #[test]
    fn direct_local_bindings_have_evidence_in_all_four_languages() {
        for (file, source) in [
            ("src/lib.rs", "fn target() {} fn caller() { target(); }"),
            (
                "app.py",
                "def target():\n    pass\ndef caller():\n    target()\n",
            ),
            (
                "app.ts",
                "export function target() {} export function caller() { target(); }",
            ),
            (
                "app.go",
                "package app\nfunc target() {}\nfunc caller() { target() }\n",
            ),
        ] {
            let g = graph(file, source);
            let caller = g.nodes().find(|n| n.name == "caller").unwrap();
            let claims = g.call_evidence(caller.id).unwrap();
            assert_eq!(claims.calls.len(), 1, "{file}: {claims:?}");
            assert_eq!(claims.calls[0].class, CallClass::Must, "{file}: {claims:?}");
        }
    }

    #[test]
    fn a_unique_name_does_not_certify_a_shadowed_call() {
        for (file, source) in [
            (
                "src/lib.rs",
                "fn target() {} fn caller(target: fn()) { target(); }",
            ),
            (
                "app.py",
                "def target():\n    pass\ndef caller(target):\n    target()\n",
            ),
            (
                "app.ts",
                "export function target() {} function caller(target: () => void) { target(); }",
            ),
            (
                "app.go",
                "package app\nfunc target() {}\nfunc caller(target func()) { target() }\n",
            ),
        ] {
            let g = graph(file, source);
            let caller = g.nodes().find(|n| n.name == "caller").unwrap();
            assert!(
                g.call_evidence(caller.id)
                    .unwrap()
                    .calls
                    .iter()
                    .all(|c| c.class == CallClass::Unknown),
                "{file}"
            );
        }
    }

    #[test]
    fn reflection_macro_and_parse_gaps_are_recorded_without_targets() {
        for (file, source, reason) in [
            (
                "app.py",
                "def caller(x):\n    getattr(x, 'run')()\n",
                "python-reflection",
            ),
            (
                "src/lib.rs",
                "fn caller() { opaque!(); }",
                "unexpanded-macro-or-decorator",
            ),
            ("app.ts", "function caller( {", "parse-error"),
        ] {
            let g = graph(file, source);
            let claims: Vec<_> = g
                .nodes()
                .filter_map(|n| g.call_evidence(n.id).ok())
                .flat_map(|e| e.calls)
                .collect();
            assert!(
                claims
                    .iter()
                    .any(|c| c.reason == reason && c.targets.is_empty()),
                "{file}: {claims:?}"
            );
        }
    }

    #[test]
    fn methods_do_not_inherit_the_legacy_resolvers_confidence() {
        let g = graph("app.py", "class Base:\n    def run(self):\n        pass\n    def caller(self):\n        self.run()\nclass Child(Base):\n    def run(self):\n        pass\n");
        let caller = g.nodes().find(|n| n.name == "caller").unwrap();
        assert!(g
            .call_evidence(caller.id)
            .unwrap()
            .calls
            .iter()
            .all(|c| c.class == CallClass::Unknown));
    }
}
