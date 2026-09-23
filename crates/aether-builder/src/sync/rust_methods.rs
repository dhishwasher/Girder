//! Crate-wide method-call Must-proof (Stage 3 Rust). Consumes the per-file
//! facts `mapper::method_index` collects; the actual rules -- everything
//! that decides Must vs. leaving the existing Unknown claim alone -- live
//! here, not in the extractor, so they can see every file at once.
//!
//! See `docs/observations/stage3-rust-audit/after-assert-macro-fix/` for the
//! full design spec and its two corrections. This is a conservative,
//! textual-over-approximation implementation of that spec: every check that
//! can't be answered with confidence from the collected facts alone leaves
//! the call Unknown, never guesses Must.

use super::FileState;
use crate::mapper::method_index::{ReceiverKind, RustMethodFacts};
use aether_graph::{CallClaim, CallClass, CallEvidence, NodeId, SemanticGraph};
use std::collections::{HashMap, HashSet};

/// Method names the std prelude could supply on almost any type -- a trait
/// method here could always be an earlier- or same-step competitor this
/// extractor cannot see (the trait's declaration is external), so the
/// method name is disqualifying regardless of what the crate-wide index
/// otherwise shows.
const STD_PRELUDE_METHOD_NAMES: &[&str] = &[
    "clone",
    "clone_from",
    "into",
    "from",
    "try_into",
    "try_from",
    "as_ref",
    "as_mut",
    "borrow",
    "borrow_mut",
    "eq",
    "ne",
    "cmp",
    "partial_cmp",
    "fmt",
    "hash",
    "default",
    "drop",
    "next",
    "iter",
    "iter_mut",
    "into_iter",
    "deref",
    "deref_mut",
    "index",
    "index_mut",
    "to_string",
    "to_owned",
];

/// External crate items a named (non-glob) import may safely bring into
/// the caller's file without this extractor treating it as an unverifiable
/// trait: common std container/type names that are never themselves method
/// providers relevant to a receiver of an unrelated, in-crate type.
const KNOWN_SAFE_EXTERNAL_IMPORTS: &[&str] = &[
    "HashMap",
    "HashSet",
    "BTreeMap",
    "BTreeSet",
    "VecDeque",
    "BinaryHeap",
    "String",
    "Vec",
    "Box",
    "Rc",
    "Arc",
    "Cell",
    "RefCell",
    "Mutex",
    "RwLock",
    "PathBuf",
    "Path",
];

fn bare_type_name(annotated: &str) -> Option<&str> {
    let trimmed = annotated.trim();
    if trimmed.starts_with('&')
        || trimmed.starts_with("dyn ")
        || trimmed.starts_with("impl ")
        || trimmed.contains('*')
    {
        return None; // No autoref/autoderef/opaque-type reasoning in this design.
    }
    let head = trimmed.split(['<', ' ']).next().unwrap_or(trimmed);
    // A path type (`std::collections::HashMap`) isn't a single-segment `T`
    // this design's bare-name resolution handles; only a plain identifier
    // (possibly generic) is in scope.
    if head.contains("::") || head.is_empty() {
        return None;
    }
    Some(head)
}

struct CrateFacts {
    /// Bare type-namespace name -> how many distinct definitions exist
    /// crate-wide (struct/enum/union/trait/type-alias, every kind, so
    /// uniqueness stays sound for a struct-only receiver).
    type_item_counts: HashMap<String, usize>,
    /// Bare name -> true if any *external* named import anywhere in the
    /// crate binds that same bare name (an external item could collide
    /// with an in-crate type of the same bare name at the point of use).
    externally_aliased_names: HashSet<String>,
    /// (bare type name, method name) -> inherent method facts, crate-wide.
    inherent_methods:
        HashMap<(String, String), Vec<crate::mapper::method_index::InherentMethodFact>>,
    /// method name -> every in-crate trait declaring it, with receiver kind.
    trait_decls_by_method: HashMap<String, Vec<crate::mapper::method_index::TraitMethodDeclFact>>,
    /// trait name -> its impls, crate-wide (blanket impls have `type_name: None`).
    trait_impls_by_trait: HashMap<String, Vec<crate::mapper::method_index::TraitImplFact>>,
    /// Bare type name -> true if any Deref/DerefMut impl exists for it.
    deref_types: HashSet<String>,
    /// Bare `mod` name -> true only if EVERY declaration of that name
    /// anywhere in the crate is cfg-free and carries no `#[path]`. Deliberately
    /// bare-name-keyed, not per-file: if any module anywhere named "tests"
    /// is cfg-gated, every file whose own path chain includes a "tests"
    /// segment is treated as unverifiable, even if the actual declaring
    /// `mod tests;` for that file is a different, clean one. Over-rejection
    /// only, matching the rest of this design's bare-name simplifications.
    mod_name_clean: HashMap<String, bool>,
}

fn build_crate_facts(
    files: &HashMap<String, FileState>,
    in_crate: impl Fn(&str) -> bool,
) -> CrateFacts {
    let mut type_item_counts: HashMap<String, usize> = HashMap::new();
    let mut externally_aliased_names: HashSet<String> = HashSet::new();
    let mut inherent_methods: HashMap<(String, String), Vec<_>> = HashMap::new();
    let mut trait_decls_by_method: HashMap<String, Vec<_>> = HashMap::new();
    let mut trait_impls_by_trait: HashMap<String, Vec<_>> = HashMap::new();
    let mut deref_types: HashSet<String> = HashSet::new();
    let mut mod_name_clean: HashMap<String, bool> = HashMap::new();

    for state in files.values() {
        let facts: &RustMethodFacts = &state.extraction.rust_method_facts;
        for item in &facts.type_items {
            *type_item_counts.entry(item.name.clone()).or_insert(0) += 1;
        }
        for m in &facts.mod_decls {
            let clean = !m.cfg_gated && !m.has_path_attr;
            let entry = mod_name_clean.entry(m.name.clone()).or_insert(true);
            *entry = *entry && clean;
        }
        for import in &facts.named_imports {
            if !in_crate(&import.first_segment) {
                externally_aliased_names.insert(import.local.clone());
            }
        }
        for method in &facts.inherent_methods {
            inherent_methods
                .entry((method.type_name.clone(), method.method_name.clone()))
                .or_default()
                .push(method.clone());
        }
        for decl in &facts.trait_decls {
            trait_decls_by_method
                .entry(decl.method_name.clone())
                .or_default()
                .push(decl.clone());
        }
        for imp in &facts.trait_impls {
            trait_impls_by_trait
                .entry(imp.trait_name.clone())
                .or_default()
                .push(imp.clone());
        }
        for d in &facts.deref_impls {
            deref_types.insert(d.type_name.clone());
        }
    }

    CrateFacts {
        type_item_counts,
        externally_aliased_names,
        inherent_methods,
        trait_decls_by_method,
        trait_impls_by_trait,
        deref_types,
        mod_name_clean,
    }
}

/// Whether `file`'s ancestor `mod X;` declaration chain is entirely clean:
/// every path segment (excluding the crate root itself) has at least one
/// declaration recorded crate-wide, and every declaration of that bare name
/// anywhere is cfg-free and has no `#[path]`. A segment with no recorded
/// declaration at all fails closed -- it cannot be positively confirmed
/// clean, so it is not treated as clean by default.
fn module_path_chain_is_clean(file: &str, mod_name_clean: &HashMap<String, bool>) -> bool {
    let path = super::rust_module_path_for(file);
    path.strip_prefix("crate")
        .unwrap_or(&path)
        .split("::")
        .filter(|segment| !segment.is_empty())
        .all(|segment| mod_name_clean.get(segment).copied().unwrap_or(false))
}

/// Mirrors `GraphBuilder::path_occurrences` (counted before graph upsert, so
/// a genuine crate-wide semantic-path collision isn't hidden by upsert
/// silently merging two distinct declarations into one node).
fn path_occurrences(files: &HashMap<String, FileState>, path: &str) -> usize {
    files
        .values()
        .flat_map(|state| state.paths.iter())
        .filter(|candidate| candidate.as_str() == path)
        .count()
}

/// Whether every glob import in `facts` resolves within the indexed crate.
/// Scoped to the caller's own file, not traced recursively through
/// re-export chains elsewhere in the crate -- a residual, disclosed
/// simplification (see the design spec's second correction and the
/// `glob-safety-checked-file-local-only` assumption this pass attaches to
/// every claim it produces).
fn file_globs_are_in_crate(
    facts: &RustMethodFacts,
    in_crate_first_segment: impl Fn(&str) -> bool,
) -> bool {
    facts
        .glob_imports
        .iter()
        .all(|path| in_crate_first_segment(path.split("::").next().unwrap_or(path)))
}

/// Whether `name` is bound, in the caller's own file, by a named import
/// whose source the extractor cannot positively rule out as a trait --
/// i.e. any external named import not on the small known-safe list. A glob
/// bringing an unindexed trait into scope is handled separately.
fn file_has_unverifiable_external_import(
    facts: &RustMethodFacts,
    in_crate_first_segment: impl Fn(&str) -> bool,
) -> bool {
    facts.named_imports.iter().any(|import| {
        !in_crate_first_segment(&import.first_segment)
            && !KNOWN_SAFE_EXTERNAL_IMPORTS.contains(&import.local.as_str())
    })
}

fn receiver_step_index(kind: ReceiverKind) -> Option<u8> {
    match kind {
        ReceiverKind::Value => Some(0),
        ReceiverKind::RefShared => Some(1),
        ReceiverKind::RefMut => Some(2),
        ReceiverKind::Unrecognized => None,
    }
}

/// Whether `inherent` can be trusted to win at its own autoref step: no
/// in-crate trait declaring the same method name may match at an earlier
/// step, and any trait competitor matching the *same* step must have an
/// impl covering this exact type with textually identical bounds (the
/// design spec's simplified applicability check) and no blanket impl of
/// that trait may exist at all.
fn inherent_wins(
    inherent: &crate::mapper::method_index::InherentMethodFact,
    crate_facts: &CrateFacts,
) -> bool {
    let Some(inherent_step) = receiver_step_index(inherent.receiver) else {
        return false;
    };
    let Some(competitors) = crate_facts.trait_decls_by_method.get(&inherent.method_name) else {
        return true; // No trait anywhere declares this method name at all.
    };
    for decl in competitors {
        if decl.cfg_gated {
            return false; // Can't rule out this declaration existing under some configuration.
        }
        let Some(decl_step) = receiver_step_index(decl.receiver) else {
            return false; // Unrecognized receiver shape: can't order it.
        };
        if decl_step < inherent_step {
            return false; // A strictly earlier-step trait method always wins.
        }
        if decl_step > inherent_step {
            continue; // A strictly later-step trait method never competes.
        }
        // Same step: only a real threat if the trait is actually blanket-
        // implemented, or implemented directly for this exact type.
        let Some(impls) = crate_facts.trait_impls_by_trait.get(&decl.trait_name) else {
            continue;
        };
        for imp in impls {
            let applies_to_this_type =
                imp.type_name.as_deref() == Some(inherent.type_name.as_str());
            let is_blanket = imp.type_name.is_none();
            if !applies_to_this_type && !is_blanket {
                // This impl of the trait covers some other, unrelated type
                // (e.g. `impl Build for StableGraph`, cfg-gated or not, when
                // `inherent` is `Graph::add_node`) -- irrelevant to whether
                // *this* inherent method is safe, regardless of its own cfg
                // status. Checking cfg/bounds here would let an unrelated
                // type's cfg-gated impl block an entirely different type's
                // otherwise-sound proof.
                continue;
            }
            if imp.cfg_gated {
                return false; // Either the blanket impl or our exact type's impl must be cfg-free.
            }
            if is_blanket {
                return false; // A blanket impl always applies; can't be ruled out.
            }
            if imp.bounds != inherent.bounds {
                return false; // Bounds differ: can't confirm both apply identically.
            }
            // applies_to_this_type && imp.bounds == inherent.bounds: same
            // step, same applicability -- inherent wins here, keep checking
            // the rest of this trait's other impls and other competitors.
        }
    }
    true
}

pub(super) fn upgrade_method_call_evidence(
    files: &HashMap<String, FileState>,
    graph: &mut SemanticGraph,
    rust_package_name: Option<&str>,
) {
    let in_crate = |segment: &str| -> bool {
        matches!(segment, "crate" | "self" | "super") || Some(segment) == rust_package_name
    };
    let crate_facts = build_crate_facts(files, in_crate);

    // Reset every candidate call's caller to this file's own fresh,
    // pristine evidence before evaluating anything, so a Must upgrade from
    // a prior resolve doesn't survive once the crate-wide facts that
    // justified it no longer hold (e.g. a second inherent impl appears
    // elsewhere, making the target newly ambiguous) -- the loop below only
    // ever adds/replaces a claim, it never reverts one on its own.
    for state in files.values() {
        let facts = &state.extraction.rust_method_facts;
        for call in &facts.candidate_calls {
            let Some(pristine) = state.extraction.nodes.get(call.owner_index) else {
                continue;
            };
            let Some(value) = pristine.attr(aether_graph::CALL_EVIDENCE_ATTRIBUTE) else {
                continue;
            };
            if let Some(node) = graph.get_mut(pristine.id) {
                node.set_attr(aether_graph::CALL_EVIDENCE_ATTRIBUTE, value.to_string());
            }
        }
    }

    for state in files.values() {
        let facts = &state.extraction.rust_method_facts;
        if facts.candidate_calls.is_empty() {
            continue;
        }
        let globs_safe = file_globs_are_in_crate(facts, in_crate);
        let unverifiable_import = file_has_unverifiable_external_import(facts, in_crate);
        // The caller's own same-file gates annotate() already computed:
        // an unexpanded macro/proc-macro attribute, a duplicate semantic
        // path, or a parse error anywhere in this file could rewrite or
        // duplicate the caller regardless of what the crate-wide index
        // otherwise shows about the receiver's type.
        let caller_file_unsafe =
            facts.file_transformed_scope || facts.file_duplicate_paths || facts.file_parse_error;

        for call in &facts.candidate_calls {
            if !globs_safe || unverifiable_import || caller_file_unsafe {
                continue;
            }
            if facts
                .macro_owner_function_indices
                .contains(&call.owner_index)
            {
                continue; // Caller function itself contains an untrusted macro invocation.
            }
            if STD_PRELUDE_METHOD_NAMES.contains(&call.method_name.as_str()) {
                continue;
            }
            let Some(annotated) = &call.annotated_type else {
                continue;
            };
            let Some(bare_type) = bare_type_name(annotated) else {
                continue;
            };

            // Constraint 2: T must resolve, via a named non-glob in-crate
            // import or an in-file definition, to the bare name being the
            // sole type-namespace item of that name crate-wide, with no
            // external item anywhere aliased to the same bare name.
            let defined_in_file = facts.type_items.iter().any(|item| item.name == bare_type);
            let imported_in_crate = facts
                .named_imports
                .iter()
                .any(|import| import.local == bare_type && in_crate(&import.first_segment));
            if !defined_in_file && !imported_in_crate {
                continue;
            }
            if crate_facts.externally_aliased_names.contains(bare_type) {
                continue;
            }
            if crate_facts.type_item_counts.get(bare_type).copied() != Some(1) {
                continue;
            }
            if crate_facts.deref_types.contains(bare_type) {
                continue;
            }

            // Constraint 3: exactly one inherent method of this name.
            let Some(candidates) = crate_facts
                .inherent_methods
                .get(&(bare_type.to_string(), call.method_name.clone()))
            else {
                continue;
            };
            if candidates.len() != 1 {
                continue;
            }
            let inherent = &candidates[0];
            if !inherent.is_pub
                || inherent.cfg_gated
                || !inherent.generic_over_all_type_params
                || inherent.receiver == ReceiverKind::Unrecognized
            {
                continue;
            }
            if !module_path_chain_is_clean(&inherent.file, &crate_facts.mod_name_clean) {
                continue; // Target's own ancestor `mod X;` chain isn't positively confirmed clean.
            }
            let Some(target_id) = inherent.target_id else {
                continue;
            };
            if graph
                .get(target_id)
                .is_some_and(|node| path_occurrences(files, &node.path) != 1)
            {
                continue; // The target's own semantic path is duplicated somewhere crate-wide.
            }

            // Constraint 4: no earlier- or unsafely-same-step trait competitor.
            if !inherent_wins(inherent, &crate_facts) {
                continue;
            }

            upgrade_claim(graph, state, call, target_id);
        }
    }
}

fn upgrade_claim(
    graph: &mut SemanticGraph,
    state: &FileState,
    call: &crate::mapper::method_index::CandidateMethodCall,
    target_id: NodeId,
) {
    let Some(caller_node) = state.extraction.nodes.get(call.owner_index) else {
        return;
    };
    let caller_id = caller_node.id;
    let Ok(evidence) = graph.call_evidence(caller_id) else {
        return; // Missing/stale/invalid evidence: leave it exactly as-is.
    };
    let already_upgraded = evidence
        .calls
        .iter()
        .any(|c| c.site == call.call_span && c.class == CallClass::Must);
    if already_upgraded {
        return;
    }
    let mut calls: Vec<CallClaim> = evidence
        .calls
        .into_iter()
        .filter(|c| c.site != call.call_span)
        .collect();
    calls.push(CallClaim {
        site: call.call_span,
        class: CallClass::Must,
        targets: vec![target_id],
        reason: "proven-inherent-method-on-annotated-receiver".into(),
        coverage_gap: false,
    });
    let mut assumptions = evidence.assumptions;
    let disclosure = "glob-safety-checked-file-local-only";
    if !assumptions.iter().any(|a| a == disclosure) {
        assumptions.push(disclosure.into());
    }
    let Some(node) = graph.get_mut(caller_id) else {
        return;
    };
    let _ = CallEvidence::new(node, calls, assumptions).attach(node);
}

#[cfg(test)]
mod tests {
    use super::super::GraphBuilder;
    use aether_graph::{CallClass, SemanticGraph};

    /// Mirrors the verified real-world case (petgraph's
    /// `tests/floyd_warshall.rs:11`, `graph.add_node(())`): an explicitly
    /// annotated receiver, a single inherent method, and a same-step trait
    /// competitor with identical bounds that must not block it.
    /// Mirrors the real petgraph layout exactly, not just its syntax: the
    /// inherent impl and the same-step trait impl live in *different*
    /// files/module scopes (`graph_impl.rs` vs `data.rs`, like petgraph's
    /// `src/graph_impl/mod.rs` vs `src/data.rs`), so their computed
    /// semantic paths don't collide the way they would if both impls sat
    /// in the same file's module scope (an artificial collision an earlier
    /// version of this fixture had, caught by the target-side
    /// duplicate-path guard correctly rejecting it).
    fn positive_case_files() -> [(&'static str, &'static str); 3] {
        [
            (
                "src/lib.rs",
                "pub struct Graph<Ty> { _p: std::marker::PhantomData<Ty> }\n\
                 impl<Ty> Graph<Ty> {\n\
                 \x20\x20\x20\x20pub fn add_node(&mut self, weight: i32) -> i32 { weight }\n\
                 }\n\
                 pub struct Directed;\n",
            ),
            (
                "src/data.rs",
                "pub trait Build { fn add_node(&mut self, w: i32) -> i32; }\n\
                 impl<Ty> Build for crate::Graph<Ty> {\n\
                 \x20\x20\x20\x20fn add_node(&mut self, weight: i32) -> i32 { self.add_node(weight) }\n\
                 }\n",
            ),
            (
                "tests/positive.rs",
                "use crate::{Graph, Directed};\n\
                 #[test]\n\
                 fn t() {\n\
                 \x20\x20\x20\x20let mut graph: Graph<Directed> = Graph { _p: std::marker::PhantomData };\n\
                 \x20\x20\x20\x20graph.add_node(1);\n\
                 }\n",
            ),
        ]
    }

    #[test]
    fn annotated_receiver_with_unique_inherent_method_is_proven_must() {
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, positive_case_files());
        let caller = graph.nodes().find(|n| n.name == "t").unwrap();
        let evidence = graph.call_evidence(caller.id).unwrap();
        let target = graph
            .nodes()
            .find(|n| n.name == "add_node" && n.file.as_deref() == Some("src/lib.rs"))
            .unwrap();
        assert!(
            evidence.calls.iter().any(|c| c.class == CallClass::Must
                && c.reason == "proven-inherent-method-on-annotated-receiver"
                && c.targets == vec![target.id]),
            "{evidence:?}"
        );
    }

    #[test]
    fn unannotated_receiver_is_not_proven() {
        let files = [
            positive_case_files()[0],
            (
                "tests/positive.rs",
                "use crate::Graph;\n\
                 #[test]\n\
                 fn t() {\n\
                 \x20\x20\x20\x20let mut graph = Graph::<crate::Directed> { _p: std::marker::PhantomData };\n\
                 \x20\x20\x20\x20graph.add_node(1);\n\
                 }\n",
            ),
        ];
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files);
        let caller = graph.nodes().find(|n| n.name == "t").unwrap();
        let evidence = graph.call_evidence(caller.id).unwrap();
        assert!(
            evidence
                .calls
                .iter()
                .all(|c| c.reason != "proven-inherent-method-on-annotated-receiver"),
            "{evidence:?}"
        );
    }

    #[test]
    fn earlier_step_trait_competitor_blocks_proof() {
        let files = [
            (
                "src/lib.rs",
                "pub trait Peek { fn add_node(&self, w: i32) -> i32; }\n\
                 pub struct Graph;\n\
                 impl Graph {\n\
                 \x20\x20\x20\x20pub fn add_node(&mut self, weight: i32) -> i32 { weight }\n\
                 }\n",
            ),
            (
                "tests/positive.rs",
                "use crate::Graph;\n\
                 #[test]\n\
                 fn t() {\n\
                 \x20\x20\x20\x20let mut graph: Graph = Graph;\n\
                 \x20\x20\x20\x20graph.add_node(1);\n\
                 }\n",
            ),
        ];
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files);
        let caller = graph.nodes().find(|n| n.name == "t").unwrap();
        let evidence = graph.call_evidence(caller.id).unwrap();
        assert!(
            evidence
                .calls
                .iter()
                .all(|c| c.reason != "proven-inherent-method-on-annotated-receiver"),
            "{evidence:?}"
        );
    }

    #[test]
    fn two_inherent_methods_of_the_same_name_block_proof() {
        let files = [
            (
                "src/lib.rs",
                "pub struct Graph;\n\
             impl Graph { pub fn add_node(&mut self, w: i32) -> i32 { w } }\n\
             impl Graph { pub fn add_node(&mut self, w: i32) -> i32 { w * 2 } }\n",
            ),
            (
                "tests/positive.rs",
                "use crate::Graph;\n\
             #[test]\n\
             fn t() {\n\
             \x20\x20\x20\x20let mut graph: Graph = Graph;\n\
             \x20\x20\x20\x20graph.add_node(1);\n\
             }\n",
            ),
        ];
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files);
        let caller = graph.nodes().find(|n| n.name == "t").unwrap();
        let evidence = graph.call_evidence(caller.id).unwrap();
        // Two inherent add_node definitions for the same type: candidates
        // for (Graph, add_node) is not a singleton, so the call must stay
        // unproven regardless of receiver/bounds/step reasoning.
        assert!(
            evidence
                .calls
                .iter()
                .all(|c| c.reason != "proven-inherent-method-on-annotated-receiver"),
            "{evidence:?}"
        );
    }

    #[test]
    fn cfg_gated_inherent_impl_blocks_proof() {
        let files = [
            (
                "src/lib.rs",
                "pub struct Graph;\n\
                 #[cfg(feature = \"x\")]\n\
                 impl Graph {\n\
                 \x20\x20\x20\x20pub fn add_node(&mut self, weight: i32) -> i32 { weight }\n\
                 }\n",
            ),
            (
                "tests/positive.rs",
                "use crate::Graph;\n\
                 #[test]\n\
                 fn t() {\n\
                 \x20\x20\x20\x20let mut graph: Graph = Graph;\n\
                 \x20\x20\x20\x20graph.add_node(1);\n\
                 }\n",
            ),
        ];
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files);
        let caller = graph.nodes().find(|n| n.name == "t").unwrap();
        let evidence = graph.call_evidence(caller.id).unwrap();
        assert!(
            evidence
                .calls
                .iter()
                .all(|c| c.reason != "proven-inherent-method-on-annotated-receiver"),
            "{evidence:?}"
        );
    }

    #[test]
    fn external_glob_import_blocks_proof() {
        let files = [
            (
                "src/lib.rs",
                "pub struct Graph;\n\
                 impl Graph { pub fn add_node(&mut self, w: i32) -> i32 { w } }\n",
            ),
            (
                "tests/positive.rs",
                "use crate::Graph;\n\
                 use some_external_crate::*;\n\
                 #[test]\n\
                 fn t() {\n\
                 \x20\x20\x20\x20let mut graph: Graph = Graph;\n\
                 \x20\x20\x20\x20graph.add_node(1);\n\
                 }\n",
            ),
        ];
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files);
        let caller = graph.nodes().find(|n| n.name == "t").unwrap();
        let evidence = graph.call_evidence(caller.id).unwrap();
        assert!(
            evidence
                .calls
                .iter()
                .all(|c| c.reason != "proven-inherent-method-on-annotated-receiver"),
            "{evidence:?}"
        );
    }

    #[test]
    fn package_name_import_is_recognized_as_in_crate() {
        let mut files = positive_case_files();
        files[2] = (
            "tests/positive.rs",
            "use pkg::{Graph, Directed};\n\
             #[test]\n\
             fn t() {\n\
             \x20\x20\x20\x20let mut graph: Graph<Directed> = Graph { _p: std::marker::PhantomData };\n\
             \x20\x20\x20\x20graph.add_node(1);\n\
             }\n",
        );
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.set_rust_package_name(Some("pkg".to_string()));
        builder.load_files(&mut graph, files);
        let caller = graph.nodes().find(|n| n.name == "t").unwrap();
        let evidence = graph.call_evidence(caller.id).unwrap();
        assert!(
            evidence.calls.iter().any(|c| c.class == CallClass::Must
                && c.reason == "proven-inherent-method-on-annotated-receiver"),
            "{evidence:?}"
        );
    }

    #[test]
    fn external_import_of_same_named_type_blocks_proof() {
        // An in-crate `Graph` exists, but the bare name "Graph" is ALSO
        // bound by a named import whose first segment isn't in-crate --
        // the bare-name-uniqueness simplification requires nothing
        // anywhere aliases the same bare name externally.
        let files = [
            (
                "src/lib.rs",
                "pub struct Graph;\n\
                 impl Graph { pub fn add_node(&mut self, w: i32) -> i32 { w } }\n",
            ),
            ("src/other.rs", "use some_external_crate::Graph;\n"),
            (
                "tests/positive.rs",
                "use crate::Graph;\n\
                 #[test]\n\
                 fn t() {\n\
                 \x20\x20\x20\x20let mut graph: Graph = Graph;\n\
                 \x20\x20\x20\x20graph.add_node(1);\n\
                 }\n",
            ),
        ];
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files);
        let caller = graph.nodes().find(|n| n.name == "t").unwrap();
        let evidence = graph.call_evidence(caller.id).unwrap();
        assert!(
            evidence
                .calls
                .iter()
                .all(|c| c.reason != "proven-inherent-method-on-annotated-receiver"),
            "{evidence:?}"
        );
    }

    #[test]
    fn same_step_trait_impl_with_narrower_bounds_blocks_proof() {
        // Both impls are &mut self (same step), but the trait impl's bounds
        // (none) aren't textually identical to the inherent impl's (T:
        // Clone) -- the simplified applicability check requires exact
        // equality, not subset/superset reasoning.
        let files = [
            (
                "src/lib.rs",
                "pub trait Build { fn add_node(&mut self, w: i32) -> i32; }\n\
                 pub struct Graph<T> { _p: std::marker::PhantomData<T> }\n\
                 impl<T> Graph<T> where T: Clone {\n\
                 \x20\x20\x20\x20pub fn add_node(&mut self, w: i32) -> i32 { w }\n\
                 }\n\
                 impl<T> Build for Graph<T> {\n\
                 \x20\x20\x20\x20fn add_node(&mut self, w: i32) -> i32 { w }\n\
                 }\n\
                 pub struct Marker;\n",
            ),
            (
                "tests/positive.rs",
                "use crate::{Graph, Marker};\n\
                 #[test]\n\
                 fn t() {\n\
                 \x20\x20\x20\x20let mut graph: Graph<Marker> = Graph { _p: std::marker::PhantomData };\n\
                 \x20\x20\x20\x20graph.add_node(1);\n\
                 }\n",
            ),
        ];
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files);
        let caller = graph.nodes().find(|n| n.name == "t").unwrap();
        let evidence = graph.call_evidence(caller.id).unwrap();
        assert!(
            evidence
                .calls
                .iter()
                .all(|c| c.reason != "proven-inherent-method-on-annotated-receiver"),
            "{evidence:?}"
        );
    }

    #[test]
    fn blanket_trait_impl_blocks_proof() {
        let files = [
            (
                "src/lib.rs",
                "pub trait Build { fn add_node(&mut self, w: i32) -> i32; }\n\
                 impl<X> Build for X {\n\
                 \x20\x20\x20\x20fn add_node(&mut self, w: i32) -> i32 { w }\n\
                 }\n\
                 pub struct Graph;\n\
                 impl Graph { pub fn add_node(&mut self, w: i32) -> i32 { w } }\n",
            ),
            (
                "tests/positive.rs",
                "use crate::Graph;\n\
                 #[test]\n\
                 fn t() {\n\
                 \x20\x20\x20\x20let mut graph: Graph = Graph;\n\
                 \x20\x20\x20\x20graph.add_node(1);\n\
                 }\n",
            ),
        ];
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files);
        let caller = graph.nodes().find(|n| n.name == "t").unwrap();
        let evidence = graph.call_evidence(caller.id).unwrap();
        assert!(
            evidence
                .calls
                .iter()
                .all(|c| c.reason != "proven-inherent-method-on-annotated-receiver"),
            "{evidence:?}"
        );
    }

    #[test]
    fn non_exact_receiver_syntax_is_not_proven() {
        // `&mut Graph` and `Box<Graph>` are both rejected: the design only
        // proves an exact `T<...>` binding, no autoref/autoderef reasoning.
        let files = [
            (
                "src/lib.rs",
                "pub struct Graph;\n\
                 impl Graph { pub fn add_node(&mut self, w: i32) -> i32 { w } }\n",
            ),
            (
                "tests/positive.rs",
                "use crate::Graph;\n\
                 #[test]\n\
                 fn ref_receiver() {\n\
                 \x20\x20\x20\x20let mut owner = Graph;\n\
                 \x20\x20\x20\x20let graph: &mut Graph = &mut owner;\n\
                 \x20\x20\x20\x20graph.add_node(1);\n\
                 }\n\
                 #[test]\n\
                 fn box_receiver() {\n\
                 \x20\x20\x20\x20let mut graph: Box<Graph> = Box::new(Graph);\n\
                 \x20\x20\x20\x20graph.add_node(1);\n\
                 }\n",
            ),
        ];
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files);
        for name in ["ref_receiver", "box_receiver"] {
            let caller = graph.nodes().find(|n| n.name == name).unwrap();
            let evidence = graph.call_evidence(caller.id).unwrap();
            assert!(
                evidence
                    .calls
                    .iter()
                    .all(|c| c.reason != "proven-inherent-method-on-annotated-receiver"),
                "{name}: {evidence:?}"
            );
        }
    }

    #[test]
    fn shadowing_in_a_nested_block_blocks_proof() {
        let files = [
            (
                "src/lib.rs",
                "pub struct Graph;\n\
                 pub struct Other;\n\
                 impl Graph { pub fn add_node(&mut self, w: i32) -> i32 { w } }\n",
            ),
            (
                "tests/positive.rs",
                "use crate::{Graph, Other};\n\
                 #[test]\n\
                 fn t() {\n\
                 \x20\x20\x20\x20let mut graph: Graph = Graph;\n\
                 \x20\x20\x20\x20{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20let graph: Other = Other;\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20let _ = graph;\n\
                 \x20\x20\x20\x20}\n\
                 \x20\x20\x20\x20graph.add_node(1);\n\
                 }\n",
            ),
        ];
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files);
        let caller = graph.nodes().find(|n| n.name == "t").unwrap();
        let evidence = graph.call_evidence(caller.id).unwrap();
        assert!(
            evidence
                .calls
                .iter()
                .all(|c| c.reason != "proven-inherent-method-on-annotated-receiver"),
            "{evidence:?}"
        );
    }

    #[test]
    fn transformed_scope_on_the_caller_file_blocks_proof() {
        let files = [
            (
                "src/lib.rs",
                "pub struct Graph;\n\
                 impl Graph { pub fn add_node(&mut self, w: i32) -> i32 { w } }\n",
            ),
            (
                "tests/positive.rs",
                "use crate::Graph;\n\
                 #[derive(Debug)]\n\
                 pub struct Unrelated;\n\
                 #[test]\n\
                 fn t() {\n\
                 \x20\x20\x20\x20let mut graph: Graph = Graph;\n\
                 \x20\x20\x20\x20graph.add_node(1);\n\
                 }\n",
            ),
        ];
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files);
        let caller = graph.nodes().find(|n| n.name == "t").unwrap();
        let evidence = graph.call_evidence(caller.id).unwrap();
        assert!(
            evidence
                .calls
                .iter()
                .all(|c| c.reason != "proven-inherent-method-on-annotated-receiver"),
            "{evidence:?}"
        );
    }

    #[test]
    fn an_untrusted_macro_elsewhere_in_the_caller_function_blocks_proof() {
        let files = [
            (
                "src/lib.rs",
                "pub struct Graph;\n\
                 impl Graph { pub fn add_node(&mut self, w: i32) -> i32 { w } }\n",
            ),
            (
                "tests/positive.rs",
                "use crate::Graph;\n\
                 #[test]\n\
                 fn t() {\n\
                 \x20\x20\x20\x20opaque!();\n\
                 \x20\x20\x20\x20let mut graph: Graph = Graph;\n\
                 \x20\x20\x20\x20graph.add_node(1);\n\
                 }\n",
            ),
        ];
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files);
        let caller = graph.nodes().find(|n| n.name == "t").unwrap();
        let evidence = graph.call_evidence(caller.id).unwrap();
        assert!(
            evidence
                .calls
                .iter()
                .all(|c| c.reason != "proven-inherent-method-on-annotated-receiver"),
            "{evidence:?}"
        );
    }

    #[test]
    fn duplicate_semantic_path_in_the_caller_file_blocks_proof() {
        let files = [
            (
                "src/lib.rs",
                "pub struct Graph;\n\
                 impl Graph { pub fn add_node(&mut self, w: i32) -> i32 { w } }\n",
            ),
            (
                "tests/positive.rs",
                "use crate::Graph;\n\
                 #[test]\n\
                 fn t() {\n\
                 \x20\x20\x20\x20let mut graph: Graph = Graph;\n\
                 \x20\x20\x20\x20graph.add_node(1);\n\
                 }\n\
                 #[test]\n\
                 fn t() {\n\
                 \x20\x20\x20\x20let mut graph: Graph = Graph;\n\
                 \x20\x20\x20\x20graph.add_node(1);\n\
                 }\n",
            ),
        ];
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files);
        let all_claims: Vec<_> = graph
            .nodes()
            .filter_map(|n| graph.call_evidence(n.id).ok())
            .flat_map(|e| e.calls)
            .collect();
        assert!(
            all_claims
                .iter()
                .all(|c| c.reason != "proven-inherent-method-on-annotated-receiver"),
            "{all_claims:?}"
        );
    }

    #[test]
    fn a_second_inherent_impl_added_later_reverts_a_prior_must_to_unknown() {
        use crate::FileChange;
        let files = positive_case_files();
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(&mut graph, files);
        let caller_id = graph.nodes().find(|n| n.name == "t").unwrap().id;
        let before = graph.call_evidence(caller_id).unwrap();
        assert!(
            before
                .calls
                .iter()
                .any(|c| c.reason == "proven-inherent-method-on-annotated-receiver"),
            "setup: expected the positive case to prove Must first: {before:?}"
        );

        // Add a second inherent add_node for Graph<Ty> in a different file:
        // (type_name, method_name) is no longer a singleton crate-wide.
        let second_impl_source = "impl<Ty> Graph<Ty> {\n\
             \x20\x20\x20\x20pub fn add_node(&mut self, weight: i32) -> i32 { weight }\n\
             }\n";
        builder
            .update_files(
                &mut graph,
                &[FileChange::replace(
                    "src/second_impl.rs",
                    second_impl_source,
                )],
                &[],
            )
            .unwrap();

        let after = graph.call_evidence(caller_id).unwrap();
        assert!(
            after
                .calls
                .iter()
                .all(|c| c.reason != "proven-inherent-method-on-annotated-receiver"),
            "expected the Must claim to revert once add_node became ambiguous: {after:?}"
        );
    }
}
