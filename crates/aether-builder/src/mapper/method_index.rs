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
use std::collections::HashSet;
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
    /// This item's own attributes, any enclosing *inline* `mod { ... }`
    /// ancestor's attributes, and whether the file carries a file-level
    /// `#![cfg]` anywhere. Does NOT cover ancestor `mod X;` *declarations*
    /// in other files -- that needs the crate-wide `mod_decls` index and is
    /// checked separately, in `sync/rust_methods.rs`.
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
    /// The file this impl block was found in -- needed to check its
    /// ancestor `mod X;` declaration chain crate-wide (`cfg_gated` above
    /// covers only this file's own attributes and inline `mod` ancestors).
    pub file: String,
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
    /// Identifies the lexical module this `mod X;` declaration itself sits
    /// in (the start byte of the nearest enclosing `mod_item` or `block`,
    /// or `u64::MAX` for the file's own top-level module) -- see
    /// `enclosing_mod_scope`.
    /// A bare `use` segment naming this `mod` is only in-crate for a `use`
    /// declared in the SAME scope; matching by name alone across scopes is
    /// unsound (an inline `mod m { use foo::Gen; }` next to a top-level
    /// `mod foo;` names the *external* crate `foo`, not the sibling module).
    pub scope: u64,
}

#[derive(Debug, Clone)]
pub struct ImportFact {
    pub local: String,
    /// The first `::`-separated segment of the imported path, as written
    /// (`crate`, `self`, `super`, or an external-looking crate name).
    pub first_segment: String,
    pub full_path: String,
    /// The lexical module this `use` declaration itself sits in -- see
    /// `ModDeclFact::scope` and `enclosing_mod_scope`.
    pub scope: u64,
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
    /// This file's own `annotate()` gates (`claims::AnnotateGates`), copied
    /// rather than re-derived: a call whose *caller's* file trips
    /// `transformed_scope`/`duplicate_paths`/a parse error, or whose caller
    /// function is itself in `macro_owners`, must not be proven regardless
    /// of what the crate-wide index otherwise shows -- an unexpanded macro
    /// or proc-macro attribute on the caller could rewrite or duplicate it.
    pub file_transformed_scope: bool,
    pub file_duplicate_paths: bool,
    pub file_parse_error: bool,
    pub macro_owner_function_indices: HashSet<usize>,
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

/// `has_cfg_attribute` on `node` itself, plus every enclosing inline
/// `mod_item` ancestor's own attributes (a `struct`/`impl` inside
/// `#[cfg(feature = "x")] mod y { ... }` is only compiled under that
/// configuration even though the item's own attributes are clean). Does
/// NOT check ancestor `mod X;` *declarations* in other files -- that needs
/// the crate-wide `mod_decls` index and is checked separately, in
/// `sync/rust_methods.rs`.
fn cfg_gated_including_inline_mod_ancestors(node: TsNode, source: &str) -> bool {
    if has_cfg_attribute(node, source) {
        return true;
    }
    let mut current = node.parent();
    while let Some(n) = current {
        if n.kind() == "mod_item" && has_cfg_attribute(n, source) {
            return true;
        }
        current = n.parent();
    }
    false
}

/// The lexical container `node` itself sits in: the start byte of the
/// nearest enclosing `mod_item` OR `block`, or `u64::MAX` (never a real
/// byte offset) for the file's own top-level module. Used to scope
/// `ModDeclFact`/`ImportFact` so a bare `use` segment naming a `mod` is
/// only treated as in-crate when the `mod` is declared in the SAME
/// container, not merely the same file -- `mod m { use foo::Gen; }` next
/// to a top-level `mod foo;` names the external crate `foo`, not the
/// sibling module, even though both are textually in one file. The
/// sentinel for "no enclosing container" must never be a value a real
/// container's own `start_byte()` could produce: an earlier version of
/// this function used `0` for both, which collided whenever a `mod`/
/// `block` was the very first thing in the file (its own `start_byte()`
/// is then also `0`) -- a `use` genuinely at file-root and an unrelated
/// `mod`'s own child could then wrongly compute the same scope id purely
/// from that coincidence of position, not from actually sharing scope.
/// Stopping at `block` too (not just `mod_item`) matters because a `mod`
/// declared inside a function/closure/`unsafe`/`async`/`const` body is
/// block-scoped, not module-scoped -- a module-level `use` cannot see it,
/// so it must not share that block's scope id with anything outside the
/// block. Two items directly in the same immediate module body or the
/// same immediate block are always mutually visible in Rust regardless of
/// declaration order (checked by construction, and against the specific
/// counter-examples this function's fixes were added for -- not
/// exhaustively verified against every corner of Rust's real resolution,
/// e.g. an item in an outer block referenced from a nested inner block,
/// which this conservatively treats as NOT sharing scope even where real
/// Rust would allow it). Two nodes with a DIFFERENT nearest enclosing
/// module-or-block are always treated as not sharing scope, which can only
/// make `in_crate` under-recognize (fail closed to Unknown), never
/// over-recognize a bare segment as in-crate that Rust would not -- this
/// specific claim has been wrong twice before for this same function
/// (widening `in_crate` to crate-wide, then file-wide, mod recognition),
/// so it is stated here only once every known collision source (file vs.
/// crate scope, block vs. module scope, and now the root-sentinel
/// collision) has been found and fixed, not as an a priori guarantee.
fn enclosing_mod_scope(node: TsNode) -> u64 {
    let mut current = node.parent();
    while let Some(n) = current {
        if n.kind() == "mod_item" || n.kind() == "block" {
            return n.start_byte() as u64;
        }
        current = n.parent();
    }
    u64::MAX
}

/// Whether the file carries a `#![cfg(...)]`/`#![cfg_attr(...)]` inner
/// attribute anywhere at all (crate root or nested inside an inline `mod`'s
/// body) -- conservatively disqualifies every item in the file, rather than
/// trying to determine which items a given inner attribute actually scopes
/// over.
fn file_has_inner_cfg(syntax: &[TsNode], source: &str) -> bool {
    syntax
        .iter()
        .any(|n| n.kind() == "inner_attribute_item" && is_cfg_attr_text(node_text(*n, source)))
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

pub(super) fn collect(
    tree: &Tree,
    source: &str,
    file: &str,
    out: &BuildOutput,
    gates: &super::claims::AnnotateGates,
) -> RustMethodFacts {
    let mut facts = RustMethodFacts {
        file_transformed_scope: gates.transformed_scope,
        file_duplicate_paths: gates.duplicate_paths,
        file_parse_error: gates.parse_error,
        macro_owner_function_indices: gates.macro_owners.clone(),
        ..RustMethodFacts::default()
    };
    let root = tree.root_node();
    let mut syntax = Vec::new();
    walk(root, &mut syntax);
    let file_inner_cfg = file_has_inner_cfg(&syntax, source);

    for n in &syntax {
        match n.kind() {
            "struct_item" | "enum_item" | "union_item" | "trait_item" | "type_item" => {
                let Some(name_field) = n.child_by_field_name("name") else {
                    continue;
                };
                facts.type_items.push(TypeItemFact {
                    name: node_text(name_field, source).to_string(),
                    cfg_gated: file_inner_cfg
                        || cfg_gated_including_inline_mod_ancestors(*n, source),
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
                    scope: enclosing_mod_scope(*n),
                });
            }
            "impl_item" => {
                let Some((type_name, is_blanket)) = impl_type_name(*n, source) else {
                    continue;
                };
                let cfg_gated =
                    file_inner_cfg || cfg_gated_including_inline_mod_ancestors(*n, source);
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
                            // Exact "pub" only -- `pub(crate)`, `pub(super)`
                            // and `pub(in ...)` are all restricted
                            // visibility, not the plain public the design
                            // requires, but all three also start with "pub".
                            first.kind() == "visibility_modifier"
                                && node_text(first, source).trim() == "pub"
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
                            file: file.to_string(),
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
        let trait_cfg = file_inner_cfg || cfg_gated_including_inline_mod_ancestors(*n, source);
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
        let scope = enclosing_mod_scope(*n);
        collect_use_clause(argument, source, "", scope, &mut facts);
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
        let mut annotated_type =
            binding_type_if_unique(&syntax, out, owner_index, &receiver_name, *n, source);
        if annotated_type
            .as_deref()
            .and_then(bare_head)
            .is_some_and(|bare| owner_generic_param_names(*n, source).contains(bare))
        {
            // The owning function or its enclosing impl block declares a
            // generic type parameter with this exact bare name (e.g.
            // `fn t<Graph: Build + Default>()`) -- the annotation names
            // that parameter, not the crate-wide type of the same name.
            annotated_type = None;
        }
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

fn collect_use_clause(
    node: TsNode,
    source: &str,
    prefix: &str,
    scope: u64,
    facts: &mut RustMethodFacts,
) {
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
                collect_use_clause(list, source, &path, scope, facts);
            }
        }
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_use_clause(child, source, prefix, scope, facts);
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
                scope,
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
                    scope,
                });
            }
        }
    }
}

/// Whether `name` is bound anywhere within a pattern subtree -- recurses
/// through every destructuring form the grammar has (`tuple_pattern`,
/// `tuple_struct_pattern`, `struct_pattern`, `slice_pattern`, `ref_pattern`,
/// `mut_pattern`, `reference_pattern`, `captured_pattern`, `or_pattern`),
/// not just a top-level bare identifier. `struct_pattern`/
/// `tuple_struct_pattern`'s own `type` field (the enum variant or struct
/// name being matched) is explicitly excluded -- it names a type, not a
/// binding. `field_pattern`'s `name` field is a binding only in shorthand
/// form (`Foo { graph }`); when it has its own `pattern` field
/// (`Foo { graph: g }`), only that nested pattern is a binding site.
fn pattern_binds_name(pattern: TsNode, source: &str, name: &str) -> bool {
    match pattern.kind() {
        "identifier" => node_text(pattern, source) == name,
        "field_pattern" => {
            if let Some(inner) = pattern.child_by_field_name("pattern") {
                pattern_binds_name(inner, source, name)
            } else {
                pattern
                    .child_by_field_name("name")
                    .is_some_and(|n| node_text(n, source) == name)
            }
        }
        "tuple_struct_pattern" | "struct_pattern" => {
            let type_field_id = pattern.child_by_field_name("type").map(|t| t.id());
            let mut cursor = pattern.walk();
            let children: Vec<TsNode> = pattern.named_children(&mut cursor).collect();
            children.into_iter().any(|child| {
                type_field_id != Some(child.id()) && pattern_binds_name(child, source, name)
            })
        }
        _ => {
            let mut cursor = pattern.walk();
            let children: Vec<TsNode> = pattern.named_children(&mut cursor).collect();
            children
                .into_iter()
                .any(|child| pattern_binds_name(child, source, name))
        }
    }
}

/// The receiver's declared type text, only when `name`'s only binding
/// occurrence anywhere in the owning function is exactly one explicitly
/// typed `let` pattern that textually precedes `call` and whose own
/// enclosing block contains `call` -- every other name-binding position
/// (function/closure parameters, destructured patterns anywhere, `for`/
/// `match`/`if let`/`while let` patterns, a second `let` of the same name,
/// a `let` whose scope doesn't reach the call) makes this `None`, since any
/// of those could be the one actually reaching the call, or could shadow
/// the annotated one.
fn binding_type_if_unique(
    syntax: &[TsNode],
    out: &BuildOutput,
    owner_index: usize,
    name: &str,
    call: TsNode,
    source: &str,
) -> Option<String> {
    let owner_span = out.nodes.get(owner_index)?.span;
    let mut annotated: Option<String> = None;
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
                if !pattern_binds_name(pattern, source, name) {
                    continue;
                }
                binding_count += 1;
                if pattern.kind() != "identifier" {
                    continue; // A destructured let never annotates a single name's type.
                }
                let Some(type_field) = n.child_by_field_name("type") else {
                    continue;
                };
                let reaches_call = n.end_byte() <= call.start_byte()
                    && n.parent().is_some_and(|block| {
                        block.start_byte() <= call.start_byte()
                            && block.end_byte() >= call.end_byte()
                    });
                if reaches_call {
                    annotated = Some(node_text(type_field, source).to_string());
                }
            }
            "parameter" | "self_parameter" => {
                let pattern = n.child_by_field_name("pattern").unwrap_or(*n);
                if pattern_binds_name(pattern, source, name) {
                    binding_count += 1;
                }
            }
            "closure_parameters" => {
                // Direct patterns here have no enclosing field (grammar.js:
                // `sepBy(',', choice($._pattern, $.parameter))`); a bare
                // pattern is a named child directly, a typed one is a
                // `parameter` with its own `pattern` field.
                let mut inner = n.walk();
                for child in n.named_children(&mut inner) {
                    let bound = if child.kind() == "parameter" {
                        child.child_by_field_name("pattern")
                    } else {
                        Some(child)
                    };
                    if bound.is_some_and(|b| pattern_binds_name(b, source, name)) {
                        binding_count += 1;
                    }
                }
            }
            "for_expression" => {
                if n.child_by_field_name("pattern")
                    .is_some_and(|p| pattern_binds_name(p, source, name))
                {
                    binding_count += 1;
                }
            }
            "match_pattern" | "let_condition"
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
    annotated
}

/// Bare head of a type annotation's text (strips generics/whitespace),
/// mirroring `sync/rust_methods.rs::bare_type_name`'s own extraction but not
/// its rejection rules -- used here only to compare against a generic
/// parameter name, so a conservative extraction (not a full re-validation)
/// is enough.
fn bare_head(annotated: &str) -> Option<&str> {
    let trimmed = annotated.trim();
    let head = trimmed.split(['<', ' ']).next().unwrap_or(trimmed);
    (!head.is_empty() && !head.contains("::")).then_some(head)
}

/// Every generic type-parameter name declared on the function enclosing
/// `call` and, if that function is a method, on the impl block enclosing
/// it -- `fn t<Graph: Build + Default>()`'s `Graph` shadows the crate-wide
/// struct of the same name for the whole function body.
fn owner_generic_param_names(call: TsNode, source: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    let collect_from = |node: TsNode, names: &mut HashSet<String>| {
        let Some(type_params) = node.child_by_field_name("type_parameters") else {
            return;
        };
        let mut cursor = type_params.walk();
        for child in type_params.named_children(&mut cursor) {
            match child.kind() {
                "type_identifier" => {
                    names.insert(node_text(child, source).to_string());
                }
                "constrained_type_parameter" | "optional_type_parameter" => {
                    if let Some(left) = child.child_by_field_name("left") {
                        names.insert(node_text(left, source).to_string());
                    }
                }
                _ => {}
            }
        }
    };
    let mut current = Some(call);
    while let Some(n) = current {
        if n.kind() == "function_item" {
            collect_from(n, &mut names);
            if let Some(impl_block) = n
                .parent()
                .and_then(|body| body.parent())
                .filter(|p| p.kind() == "impl_item")
            {
                collect_from(impl_block, &mut names);
            }
            break;
        }
        current = n.parent();
    }
    names
}
