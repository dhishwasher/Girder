//! Per-file facts for crate-wide method-call Must-proof (Stage 3 Rust).
//!
//! `annotate()` (in `claims.rs`) proves same-file bare-identifier calls only;
//! it cannot see whether `x.m()`'s receiver type has a single inherent `m`
//! crate-wide, since that requires facts from every file. This module
//! collects exactly the facts a project-wide pass (in `sync::resolve_calls`)
//! needs to make that call soundly, and nothing else: extraction happens once
//! per file, here; the crate-wide index and the actual proof/rejection rules
//! live in `sync.rs`, so a rule change never needs a second implementation of
//! `transformed_scope`/`duplicate_paths`/`macro_owners` (which `annotate()`
//! already computes) to drift against.
//!
//! Every fact here is deliberately a coarse, textual over-approximation where
//! a precise one would need real trait solving (bounds, blanket impls,
//! receiver-adjustment ordering) -- the design intentionally keeps this
//! extractor's reasoning "can I see, from the syntax alone, that nothing
//! disqualifies this candidate", never "have I proven propositionally that it
//! type-checks". See `docs/observations/stage3-rust-audit/after-assert-macro-fix/`
//! for the full design spec and its two corrections.

use super::{node_text, span_of, BuildOutput};
use aether_graph::{NodeId, Span};
use tree_sitter::{Node as TsNode, Tree};

/// How a method/function's first parameter binds its receiver, in Rust's own
/// autoref probe order (`self`/`mut self` before `&self` before `&mut self`).
/// A parameter this can't positively classify (typed self like
/// `self: Box<Self>`, or anything else unrecognized) is `Unrecognized`,
/// which the resolver in `sync.rs` must treat as disqualifying wherever it
/// appears -- on a competitor it can win at an unknown step, and on the
/// candidate inherent method it must never be proven.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReceiverKind {
    Value,
    RefShared,
    RefMut,
    Unrecognized,
}

/// A type-namespace item (struct/enum/union/trait/type alias) defined in this
/// file -- every kind, not just struct, so a crate-wide uniqueness count
/// stays sound even though only a struct receiver is ever proven from.
#[derive(Debug, Clone)]
pub struct TypeItemFact {
    pub name: String,
    /// This item's own attributes only (module-chain cfg is a separate,
    /// crate-wide check in `sync.rs`, since it needs every file's `mod`
    /// declarations).
    pub cfg_gated: bool,
}

#[derive(Debug, Clone)]
pub struct InherentMethodFact {
    pub type_name: String,
    pub method_name: String,
    pub receiver: ReceiverKind,
    pub is_pub: bool,
    /// Merged inline (`impl<T: Bound>`) and `where`-clause bounds, each
    /// trimmed and whitespace-collapsed, sorted, and joined with `;` --
    /// compared for exact textual equality against a same-step competitor's
    /// bounds rather than solved for satisfaction (see the module doc).
    pub bounds: String,
    /// Whether the impl's generic parameter list mentions every one of the
    /// type's own parameters (`impl<N, E, Ty, Ix> Graph<N, E, Ty, Ix>`, not
    /// `impl<N, E> Graph<N, E, Directed, DefaultIx>`) -- a partial
    /// specialization could hide a second, more specific inherent impl that
    /// coherence would otherwise rule out.
    pub generic_over_all_type_params: bool,
    pub cfg_gated: bool,
    /// Resolved at extraction time (this file's own `out.nodes` are already
    /// available), not carried as a raw span -- avoids needing cross-file
    /// span matching in the crate-wide pass.
    pub target_id: Option<NodeId>,
}

#[derive(Debug, Clone)]
pub struct TraitMethodDeclFact {
    pub trait_name: String,
    pub method_name: String,
    pub receiver: ReceiverKind,
    pub cfg_gated: bool,
}

#[derive(Debug, Clone)]
pub struct TraitImplFact {
    pub trait_name: String,
    /// Bare name of the implementing type, or `None` for a blanket impl
    /// (`impl<X> Trait for X`, `X` a bare generic parameter of the impl).
    pub type_name: Option<String>,
    pub bounds: String,
    pub cfg_gated: bool,
}

#[derive(Debug, Clone)]
pub struct DerefImplFact {
    pub type_name: String,
}

#[derive(Debug, Clone)]
pub struct ModDeclFact {
    pub name: String,
    pub cfg_gated: bool,
    /// `#[path = "..."]`: the module's file location isn't derivable from
    /// its name alone, which the crate-wide cfg-chain check relies on.
    pub has_path_attr: bool,
}

#[derive(Debug, Clone)]
pub struct ImportFact {
    pub local: String,
    /// The first `::`-separated segment of the imported path, as written
    /// (`crate`, `self`, `super`, or an external-looking crate name).
    pub first_segment: String,
    pub full_path: String,
}

/// A method call whose receiver is a bare identifier -- the only shape this
/// design ever proves from. `x.m()` where `x` is anything else (a chained
/// call, a field access, a literal) is out of scope; `sync.rs` never even
/// attempts those.
#[derive(Debug, Clone)]
pub struct CandidateMethodCall {
    pub call_span: Span,
    pub receiver_name: String,
    pub method_name: String,
    pub owner_index: usize,
    /// The receiver's declared type, in the exact syntax written after `:` in
    /// its `let` binding, only when that binding is the receiver name's sole
    /// occurrence among every name-binding position in the owning function
    /// (see `binding_type_if_unique`). `None` means: not provable, no matter
    /// what the crate-wide index says.
    pub annotated_type: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct RustMethodFacts {
    pub type_items: Vec<TypeItemFact>,
    pub inherent_methods: Vec<InherentMethodFact>,
    pub trait_decls: Vec<TraitMethodDeclFact>,
    pub trait_impls: Vec<TraitImplFact>,
    pub deref_impls: Vec<DerefImplFact>,
    pub mod_decls: Vec<ModDeclFact>,
    pub named_imports: Vec<ImportFact>,
    /// Full path prefix before `::*`, e.g. `petgraph::prelude` for
    /// `use petgraph::prelude::*;`.
    pub glob_imports: Vec<String>,
    pub candidate_calls: Vec<CandidateMethodCall>,
}

fn walk<'tree>(node: TsNode<'tree>, nodes: &mut Vec<TsNode<'tree>>) {
    nodes.push(node);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk(child, nodes);
    }
}

fn is_cfg_attr_text(text: &str) -> bool {
    let inner = text
        .trim()
        .trim_start_matches("#!")
        .trim_start_matches('#')
        .trim_start()
        .trim_start_matches('[');
    inner.starts_with("cfg(") || inner.starts_with("cfg_attr(") || inner.trim() == "cfg"
}

/// Whether any attribute belonging to `node` (its own `attribute_item`
/// children, found by scanning preceding siblings up to the nearest
/// non-attribute sibling) is `#[cfg(...)]`/`#[cfg_attr(...)]`.
fn has_cfg_attribute(node: TsNode, source: &str) -> bool {
    let mut sibling = node.prev_sibling();
    while let Some(n) = sibling {
        if n.kind() != "attribute_item" && n.kind() != "inner_attribute_item" {
            break;
        }
        if is_cfg_attr_text(node_text(n, source)) {
            return true;
        }
        sibling = n.prev_sibling();
    }
    false
}

fn receiver_kind_of_parameters(params: TsNode, source: &str) -> Option<ReceiverKind> {
    let mut cursor = params.walk();
    let first = params.named_children(&mut cursor).next()?;
    match first.kind() {
        "self_parameter" => {
            let text = node_text(first, source);
            if text.contains("&mut") {
                Some(ReceiverKind::RefMut)
            } else if text.starts_with('&') {
                Some(ReceiverKind::RefShared)
            } else {
                Some(ReceiverKind::Value)
            }
        }
        "parameter" => {
            let pattern = first.child_by_field_name("pattern")?;
            (pattern.kind() == "self").then_some(ReceiverKind::Unrecognized)
        }
        _ => None,
    }
}

/// Merged, normalized bounds text for an `impl_item`: each inline
/// `type_parameters` bound and each `where_clause` predicate, trimmed,
/// whitespace-collapsed, sorted, and joined -- compared for exact equality,
/// never solved for satisfaction (see the module doc).
fn normalized_bounds(impl_node: TsNode, source: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut cursor = impl_node.walk();
    for child in impl_node.named_children(&mut cursor) {
        match child.kind() {
            "type_parameters" => {
                let mut inner_cursor = child.walk();
                for param in child.named_children(&mut inner_cursor) {
                    if param.kind() == "constrained_type_parameter" {
                        parts.push(collapse_whitespace(node_text(param, source)));
                    }
                }
            }
            "where_clause" => {
                let mut inner_cursor = child.walk();
                for predicate in child.named_children(&mut inner_cursor) {
                    if predicate.kind() == "where_predicate" {
                        parts.push(collapse_whitespace(node_text(predicate, source)));
                    }
                }
            }
            _ => {}
        }
    }
    parts.sort();
    parts.join(";")
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Bare type name from an `impl_item`'s `type` field text (`Graph<N, E, Ty,
/// Ix>` -> `Graph`), or `None` when the type field is itself a bare
/// (potentially generic) identifier matching one of the impl's own type
/// parameters -- a blanket impl (`impl<X> Trait for X`).
fn impl_type_name(impl_node: TsNode, source: &str) -> Option<(String, bool)> {
    let type_field = impl_node.child_by_field_name("type")?;
    let type_params: Vec<&str> = impl_node
        .child_by_field_name("type_parameters")
        .map(|tp| {
            let mut cursor = tp.walk();
            tp.named_children(&mut cursor)
                .filter_map(|c| {
                    if c.kind() == "type_identifier" {
                        Some(node_text(c, source))
                    } else if c.kind() == "constrained_type_parameter" {
                        c.child_by_field_name("left").map(|l| node_text(l, source))
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let text = node_text(type_field, source);
    let bare = super::last_ident(text.split(['<', ' ']).next().unwrap_or(text));
    let is_blanket = type_field.kind() == "type_identifier" && type_params.contains(&bare);
    if is_blanket {
        return Some((bare.to_string(), true));
    }
    Some((bare.to_string(), false))
}

/// Whether the impl's `type_parameters` list mentions every one of the
/// target type's own generic parameter names, found from `Graph<N, E, Ty,
/// Ix>`'s angle-bracket argument list text.
fn generic_over_all_params(impl_node: TsNode, source: &str) -> bool {
    let Some(type_field) = impl_node.child_by_field_name("type") else {
        return false;
    };
    let text = node_text(type_field, source);
    let Some(args) = text
        .find('<')
        .and_then(|start| text.rfind('>').map(|end| &text[start + 1..end]))
    else {
        return true; // No generic arguments at all: nothing to under-specialize.
    };
    let target_params: Vec<&str> = args.split(',').map(str::trim).collect();
    let impl_params: std::collections::HashSet<&str> = impl_node
        .child_by_field_name("type_parameters")
        .map(|tp| {
            let mut cursor = tp.walk();
            tp.named_children(&mut cursor)
                .filter_map(|c| match c.kind() {
                    "type_identifier" => Some(node_text(c, source)),
                    "constrained_type_parameter" | "optional_type_parameter" => {
                        c.child_by_field_name("left").map(|l| node_text(l, source))
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    target_params
        .iter()
        .all(|param| impl_params.contains(param))
}

pub(super) fn collect(tree: &Tree, source: &str, out: &BuildOutput) -> RustMethodFacts {
    let mut facts = RustMethodFacts::default();
    let root = tree.root_node();
    let mut syntax = Vec::new();
    walk(root, &mut syntax);

    for n in &syntax {
        match n.kind() {
            "struct_item" | "enum_item" | "union_item" | "trait_item" | "type_item" => {
                let Some(name_field) = n.child_by_field_name("name") else {
                    continue;
                };
                facts.type_items.push(TypeItemFact {
                    name: node_text(name_field, source).to_string(),
                    cfg_gated: has_cfg_attribute(*n, source),
                });
            }
            "mod_item" => {
                let Some(name_field) = n.child_by_field_name("name") else {
                    continue;
                };
                let has_path_attr = {
                    let mut sib = n.prev_sibling();
                    let mut found = false;
                    while let Some(s) = sib {
                        if s.kind() != "attribute_item" {
                            break;
                        }
                        if node_text(s, source).contains("path") {
                            found = true;
                        }
                        sib = s.prev_sibling();
                    }
                    found
                };
                facts.mod_decls.push(ModDeclFact {
                    name: node_text(name_field, source).to_string(),
                    cfg_gated: has_cfg_attribute(*n, source),
                    has_path_attr,
                });
            }
            "impl_item" => {
                let Some((type_name, is_blanket)) = impl_type_name(*n, source) else {
                    continue;
                };
                let cfg_gated = has_cfg_attribute(*n, source);
                let bounds = normalized_bounds(*n, source);
                let trait_field = n.child_by_field_name("trait");
                if let Some(trait_node) = trait_field {
                    let trait_name = super::last_ident(node_text(trait_node, source)).to_string();
                    if (trait_name == "Deref" || trait_name == "DerefMut") && !is_blanket {
                        facts.deref_impls.push(DerefImplFact {
                            type_name: type_name.clone(),
                        });
                    }
                    facts.trait_impls.push(TraitImplFact {
                        trait_name,
                        type_name: if is_blanket { None } else { Some(type_name) },
                        bounds,
                        cfg_gated,
                    });
                } else {
                    // Inherent impl: record each directly-declared method.
                    let Some(body) = n.child_by_field_name("body") else {
                        continue;
                    };
                    let generic_ok = generic_over_all_params(*n, source);
                    let mut cursor = body.walk();
                    for item in body.named_children(&mut cursor) {
                        if item.kind() != "function_item" {
                            continue;
                        }
                        let Some(params) = item.child_by_field_name("parameters") else {
                            continue;
                        };
                        let Some(receiver) = receiver_kind_of_parameters(params, source) else {
                            continue; // No receiver: an associated function, not a method.
                        };
                        let Some(name_field) = item.child_by_field_name("name") else {
                            continue;
                        };
                        let mut cur = item.walk();
                        let first_child = item.children(&mut cur).next();
                        let is_pub = first_child.is_some_and(|first| {
                            first.kind() == "visibility_modifier"
                                && node_text(first, source).starts_with("pub")
                        });
                        let target_id = out
                            .nodes
                            .iter()
                            .find(|node| {
                                node.kind == aether_graph::NodeKind::Function
                                    && node.span.start_byte == item.start_byte()
                                    && node.span.end_byte == item.end_byte()
                            })
                            .map(|node| node.id);
                        facts.inherent_methods.push(InherentMethodFact {
                            type_name: type_name.clone(),
                            method_name: node_text(name_field, source).to_string(),
                            receiver,
                            is_pub,
                            bounds: bounds.clone(),
                            generic_over_all_type_params: generic_ok,
                            cfg_gated,
                            target_id,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    // Trait method declarations: both function_item (default bodies) and
    // function_signature_item (required, bodyless methods -- a scan over
    // function_item alone would miss e.g. `Build::add_node`).
    for n in &syntax {
        if n.kind() != "trait_item" {
            continue;
        }
        let Some(name_field) = n.child_by_field_name("name") else {
            continue;
        };
        let trait_name = node_text(name_field, source).to_string();
        let trait_cfg = has_cfg_attribute(*n, source);
        let Some(body) = n.child_by_field_name("body") else {
            continue;
        };
        let mut cursor = body.walk();
        for item in body.named_children(&mut cursor) {
            if !matches!(item.kind(), "function_item" | "function_signature_item") {
                continue;
            }
            let Some(params) = item.child_by_field_name("parameters") else {
                continue;
            };
            let Some(receiver) = receiver_kind_of_parameters(params, source) else {
                continue;
            };
            let Some(method_name_field) = item.child_by_field_name("name") else {
                continue;
            };
            facts.trait_decls.push(TraitMethodDeclFact {
                trait_name: trait_name.clone(),
                method_name: node_text(method_name_field, source).to_string(),
                receiver,
                cfg_gated: trait_cfg,
            });
        }
    }

    // Imports: named vs. glob, reusing the same use-tree walk shape as
    // `collect_rust_imports` (mapper.rs) but recording globs too, which that
    // function deliberately does not.
    for n in &syntax {
        if n.kind() != "use_declaration" {
            continue;
        }
        let Some(argument) = n.child_by_field_name("argument") else {
            continue;
        };
        collect_use_clause(argument, source, "", &mut facts);
    }

    // Candidate method calls: `x.m()` where `x` is a bare identifier.
    for n in &syntax {
        if n.kind() != "call_expression" {
            continue;
        }
        let Some(function) = n.child_by_field_name("function") else {
            continue;
        };
        if function.kind() != "field_expression" {
            continue;
        }
        let Some(value) = function.child_by_field_name("value") else {
            continue;
        };
        if value.kind() != "identifier" {
            continue;
        }
        let Some(field) = function.child_by_field_name("field") else {
            continue;
        };
        let owner_index = out
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                node.kind == aether_graph::NodeKind::Function
                    && node.span.start_byte <= n.start_byte()
                    && node.span.end_byte >= n.end_byte()
            })
            .min_by_key(|(_, node)| node.span.end_byte - node.span.start_byte)
            .map(|(index, _)| index);
        let Some(owner_index) = owner_index else {
            continue; // No enclosing function: not a call this design proves.
        };
        let receiver_name = node_text(value, source).to_string();
        let annotated_type =
            binding_type_if_unique(&syntax, out, owner_index, &receiver_name, source);
        facts.candidate_calls.push(CandidateMethodCall {
            call_span: span_of(*n),
            receiver_name,
            method_name: node_text(field, source).to_string(),
            owner_index,
            annotated_type,
        });
    }

    facts
}

fn collect_use_clause(node: TsNode, source: &str, prefix: &str, facts: &mut RustMethodFacts) {
    fn combine(prefix: &str, suffix: &str) -> String {
        let suffix = suffix.trim();
        if prefix.is_empty() || suffix == "crate" || suffix.starts_with("crate::") {
            suffix.to_string()
        } else {
            format!("{prefix}::{suffix}")
        }
    }
    match node.kind() {
        "scoped_use_list" => {
            let path = node
                .child_by_field_name("path")
                .map(|p| combine(prefix, node_text(p, source)))
                .unwrap_or_else(|| prefix.to_string());
            if let Some(list) = node.child_by_field_name("list") {
                collect_use_clause(list, source, &path, facts);
            }
        }
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_use_clause(child, source, prefix, facts);
            }
        }
        "use_as_clause" => {
            let (Some(path), Some(alias)) = (
                node.child_by_field_name("path"),
                node.child_by_field_name("alias"),
            ) else {
                return;
            };
            let full_path = combine(prefix, node_text(path, source));
            let first_segment = full_path.split("::").next().unwrap_or("").to_string();
            facts.named_imports.push(ImportFact {
                local: node_text(alias, source).trim().to_string(),
                first_segment,
                full_path,
            });
        }
        "use_wildcard" => {
            let path_text = node_text(node, source);
            let prefix_text = path_text.trim_end_matches('*').trim_end_matches("::");
            let full = combine(prefix, prefix_text);
            facts.glob_imports.push(full);
        }
        _ => {
            let full_path = combine(prefix, node_text(node, source));
            let local = super::last_ident(&full_path).to_string();
            if !local.is_empty() {
                let first_segment = full_path.split("::").next().unwrap_or("").to_string();
                facts.named_imports.push(ImportFact {
                    local,
                    first_segment,
                    full_path,
                });
            }
        }
    }
}

/// The receiver's declared type text, only when `name`'s only binding
/// occurrence anywhere in the owning function is exactly one explicitly
/// typed `let` pattern -- every other name-binding position (function/
/// closure parameters, `for`/`match`/`if let`/`while let` patterns, a second
/// `let` of the same name) makes this `None`, since any of those could be
/// the one actually reaching the call, or could shadow the annotated one.
fn binding_type_if_unique(
    syntax: &[TsNode],
    out: &BuildOutput,
    owner_index: usize,
    name: &str,
    source: &str,
) -> Option<String> {
    let owner_span = out.nodes.get(owner_index)?.span;
    let mut annotated: Option<(usize, String)> = None; // (start_byte, type text)
    let mut binding_count = 0usize;
    for n in syntax {
        if n.start_byte() < owner_span.start_byte || n.end_byte() > owner_span.end_byte {
            continue;
        }
        match n.kind() {
            "let_declaration" => {
                let Some(pattern) = n.child_by_field_name("pattern") else {
                    continue;
                };
                if pattern.kind() != "identifier" || node_text(pattern, source) != name {
                    continue;
                }
                binding_count += 1;
                if let Some(type_field) = n.child_by_field_name("type") {
                    annotated = Some((n.start_byte(), node_text(type_field, source).to_string()));
                }
            }
            "parameter" | "self_parameter" => {
                let pattern = n.child_by_field_name("pattern").unwrap_or(*n);
                if pattern.kind() == "identifier" && node_text(pattern, source) == name {
                    binding_count += 1;
                }
            }
            "closure_parameters" => {
                // Direct patterns here have no enclosing field (grammar.js:
                // `sepBy(',', choice($._pattern, $.parameter))`); an
                // identifier pattern is a named child directly, a typed one
                // is a `parameter` with its own `pattern` field.
                let mut inner = n.walk();
                for child in n.named_children(&mut inner) {
                    let bound = if child.kind() == "parameter" {
                        child.child_by_field_name("pattern")
                    } else {
                        Some(child)
                    };
                    if bound
                        .is_some_and(|b| b.kind() == "identifier" && node_text(b, source) == name)
                    {
                        binding_count += 1;
                    }
                }
            }
            "for_expression" | "match_pattern" | "let_condition"
                if node_text(*n, source)
                    .split(|c: char| !(c == '_' || c.is_ascii_alphanumeric()))
                    .any(|word| word == name) =>
            {
                binding_count += 1;
            }
            _ => {}
        }
    }
    if binding_count != 1 {
        return None;
    }
    annotated.map(|(_, type_text)| type_text)
}
