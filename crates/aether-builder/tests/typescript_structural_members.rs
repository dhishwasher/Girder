//! Frozen TypeScript structural callable member identity policy v1:
//! `docs/observations/stage3-typescript-audit/structural-members/policy.md`.
//! The manifest-driven contract replays every pinned identity, refusal,
//! declaration-only span and B1 boundary; the focused tests below pin the
//! highest-risk rules directly (collision propagation, B1 evidence for call
//! and `new` subtrees, identity/span/form/containment, declaration-only
//! exclusions, no proof escalation, and the retained original ambiguity).

use aether_builder::GraphBuilder;
use aether_graph::{CallClaim, CallClass, EdgeKind, Node, NodeKind, SemanticGraph};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const B1_REASON: &str = "typescript-refused-structural-member-subtree";
const INSIDE_REASON: &str = "typescript-call-inside-refused-structural-member-subtree";

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/typescript-structural-member-corpus/v1")
}

fn build(file: &str, source: &str) -> SemanticGraph {
    let mut graph = SemanticGraph::new();
    GraphBuilder::new().load_file(&mut graph, file, source);
    graph
}

/// First definition byte after a marker: skips whitespace and a leading
/// `export` keyword, matching the manifest's marker rule.
fn offset_after(source: &str, marker: &str) -> usize {
    assert_eq!(
        source.matches(marker).count(),
        1,
        "marker {marker} not unique"
    );
    let mut offset = source.find(marker).unwrap() + marker.len();
    loop {
        let rest = &source[offset..];
        let trimmed = rest.trim_start();
        offset += rest.len() - trimmed.len();
        if let Some(after) = trimmed.strip_prefix("export ") {
            offset += trimmed.len() - after.len();
            continue;
        }
        return offset;
    }
}

fn functions_at(graph: &SemanticGraph, offset: usize) -> Vec<&Node> {
    graph
        .nodes()
        .filter(|n| n.kind == NodeKind::Function && n.span.start_byte == offset)
        .collect()
}

fn all_claims(graph: &SemanticGraph) -> Vec<(String, CallClaim)> {
    graph
        .nodes()
        .filter_map(|node| {
            graph
                .call_evidence(node.id)
                .ok()
                .map(|evidence| (node.path.clone(), evidence.calls))
        })
        .flat_map(|(path, calls)| calls.into_iter().map(move |claim| (path.clone(), claim)))
        .collect()
}

fn paths(graph: &SemanticGraph, kind: NodeKind) -> HashSet<String> {
    graph
        .nodes()
        .filter(|n| n.kind == kind)
        .map(|n| n.path.clone())
        .collect()
}

fn no_duplicate_paths(graph: &SemanticGraph) -> bool {
    let mut seen = HashSet::new();
    graph.nodes().all(|n| seen.insert(n.path.clone()))
}

#[test]
fn frozen_structural_member_identity_contract() {
    let root = corpus_root();
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).unwrap())
            .unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 31);
    let mut failures = Vec::new();
    let mut stability = HashMap::<(String, String), String>::new();
    let (mut identities, mut nulls, mut declarations, mut boundaries) = (0, 0, 0, 0);
    for case in cases {
        let file = case["file"].as_str().unwrap();
        let module = case["module_path"].as_str().unwrap();
        let source = std::fs::read_to_string(root.join(file)).unwrap();
        let graph = build(file, &source);
        let claims = all_claims(&graph);
        if !no_duplicate_paths(&graph) {
            failures.push(format!("{file}: duplicate semantic paths remain"));
        }
        for member in case["members"].as_array().into_iter().flatten() {
            let marker = member["marker"].as_str().unwrap();
            let offset = offset_after(&source, marker);
            let found = functions_at(&graph, offset);
            match member["expected_identity"].as_str() {
                Some(expected) => {
                    identities += 1;
                    let ok = found.len() == 1
                        && found[0].path == expected
                        && graph.nodes().filter(|n| n.path == expected).count() == 1;
                    if !ok {
                        let got: Vec<_> = found.iter().map(|n| &n.path).collect();
                        failures.push(format!("{file} {marker}: expected {expected}, got {got:?}"));
                        continue;
                    }
                    let form = member["form"].as_str().unwrap();
                    if expected.rsplit("::").nth(1) == Some("@object") {
                        if found[0].attr("member_form") != Some(form) {
                            failures.push(format!(
                                "{file} {marker}: member_form {:?} != {form}",
                                found[0].attr("member_form")
                            ));
                        }
                        let key = expected.rsplit("::").next().unwrap();
                        if found[0].name != key {
                            failures
                                .push(format!("{file} {marker}: display name {}", found[0].name));
                        }
                    }
                    if let Some(suffix) = expected.strip_prefix(&format!("{module}::")) {
                        if file.starts_with("stability/") {
                            let id = marker.to_string();
                            if let Some(previous) =
                                stability.insert((id.clone(), "suffix".into()), suffix.to_string())
                            {
                                if previous != suffix {
                                    failures
                                        .push(format!("stability {id}: {previous} != {suffix}"));
                                }
                            }
                        }
                    }
                }
                None => {
                    nulls += 1;
                    if !found.is_empty() {
                        let got: Vec<_> = found.iter().map(|n| &n.path).collect();
                        failures.push(format!(
                            "{file} {marker}: expected no Function, got {got:?}"
                        ));
                    }
                }
            }
        }
        for declaration in case["declaration_only"].as_array().into_iter().flatten() {
            declarations += 1;
            let marker = declaration["marker"].as_str().unwrap();
            let offset = offset_after(&source, marker);
            let at: Vec<_> = graph
                .nodes()
                .filter(|n| n.span.start_byte == offset)
                .collect();
            if at.iter().any(|n| n.kind == NodeKind::Function) {
                failures.push(format!(
                    "{file} {marker}: declaration-only span is a Function"
                ));
            }
            let ids: HashSet<_> = at.iter().map(|n| n.id).collect();
            if claims
                .iter()
                .any(|(_, claim)| claim.targets.iter().any(|target| ids.contains(target)))
            {
                failures.push(format!(
                    "{file} {marker}: declaration-only span is a target"
                ));
            }
            if at.iter().any(|n| n.kind == NodeKind::Field)
                && declaration["syntax_kind"] != "property_signature"
            {
                failures.push(format!("{file} {marker}: unexpected Field node"));
            }
        }
        for boundary in case["refused_subtree_boundaries"]
            .as_array()
            .into_iter()
            .flatten()
        {
            boundaries += 1;
            let owner = boundary["owner"].as_str().unwrap();
            let subtree = offset_after(&source, boundary["subtree_marker"].as_str().unwrap());
            let gaps: Vec<_> = claims
                .iter()
                .filter(|(_, claim)| claim.reason == B1_REASON && claim.site.start_byte == subtree)
                .collect();
            let ok = gaps.len() == 1
                && gaps[0].0 == owner
                && gaps[0].1.coverage_gap
                && gaps[0].1.class == CallClass::Unknown
                && gaps[0].1.targets.is_empty();
            if !ok {
                failures.push(format!(
                    "{file}: B1 boundary at {subtree} on {owner}: {gaps:?}"
                ));
                continue;
            }
            let site = gaps[0].1.site;
            if let Some(inner) = graph.nodes().find(|n| {
                n.kind != NodeKind::Module
                    && site.start_byte <= n.span.start_byte
                    && n.span.end_byte <= site.end_byte
            }) {
                failures.push(format!(
                    "{file}: T1 node inside refused subtree: {}",
                    inner.path
                ));
            }
            for call in boundary["calls_inside"].as_array().unwrap() {
                let offset = offset_after(&source, call["marker"].as_str().unwrap());
                let at: Vec<_> = claims
                    .iter()
                    .filter(|(_, claim)| !claim.coverage_gap && claim.site.start_byte == offset)
                    .collect();
                let ok = at.len() == 1
                    && at[0].0 == owner
                    && at[0].1.class == CallClass::Unknown
                    && at[0].1.targets.is_empty()
                    && at[0].1.reason == INSIDE_REASON;
                if !ok {
                    failures.push(format!("{file}: inner call at {offset}: {at:?}"));
                }
            }
        }
        // No proof escalation: a structural member is never a proof target.
        let members: HashSet<_> = graph
            .nodes()
            .filter(|n| n.attr("member_form").is_some())
            .map(|n| n.id)
            .collect();
        if claims
            .iter()
            .any(|(_, claim)| claim.targets.iter().any(|target| members.contains(target)))
        {
            failures.push(format!("{file}: a structural member became a call target"));
        }
    }
    assert_eq!(
        (identities, nulls, declarations, boundaries),
        (44, 62, 10, 6),
        "manifest totals changed"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn collision_refuses_members_and_every_descendant_without_flattening() {
    let source = "export {};\n\
function helper() { return 'outer'; }\n\
{ const a = { run: () => { function helper() { return 1; } const box = { go: () => helper() }; class K { m() {} } return box.go(); } }; }\n\
{ const a = { run() { function helper() { return 2; } return helper(); } }; }\n\
const b = { run: () => helper() };\n";
    let graph = build("collide.ts", source);
    let functions = paths(&graph, NodeKind::Function);
    let expected: HashSet<String> = ["crate::collide::helper", "crate::collide::b::@object::run"]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(functions, expected);
    assert!(
        paths(&graph, NodeKind::Type).is_empty(),
        "nested class leaked"
    );
    assert!(no_duplicate_paths(&graph));
    let gaps: Vec<_> = all_claims(&graph)
        .into_iter()
        .filter(|(_, claim)| claim.reason == B1_REASON)
        .collect();
    assert_eq!(gaps.len(), 2, "{gaps:?}");
    assert!(gaps.iter().all(|(owner, _)| owner == "crate::collide"));
}

#[test]
fn refusal_of_a_branch_keeps_unrelated_sibling_branches() {
    let source = "export const api = {\n\
  bad: { 'k': 1, list: () => 1, deeper: { list: () => 2 } },\n\
  good: { list: () => 3 },\n\
  dup: () => 4,\n\
  dup: () => 5,\n\
  ok: () => 6,\n\
};\n";
    let graph = build("branch.ts", source);
    let functions = paths(&graph, NodeKind::Function);
    let expected: HashSet<String> = [
        "crate::branch::api::@object::good::@object::list",
        "crate::branch::api::@object::ok",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(functions, expected);
}

#[test]
fn b1_boundary_exists_for_call_and_new_subtrees_only() {
    let source = "export class Foo {}\n\
export function host() {\n\
  let withNew = { make: () => new Foo() };\n\
  let withCall = { run: () => String(1) };\n\
  let quiet = { run: () => 1 };\n\
  const outer = { inner: { 'x': 1, f: () => ({ g: () => String(2) }) } };\n\
  return [withNew, withCall, quiet, outer];\n\
}\n";
    let graph = build("b1.ts", source);
    let claims = all_claims(&graph);
    let gaps: Vec<_> = claims
        .iter()
        .filter(|(_, claim)| claim.reason == B1_REASON)
        .collect();
    // withNew, withCall, and the maximal refused `inner` literal (its nested
    // owner-less literal is not a second boundary); `quiet` has no call.
    assert_eq!(gaps.len(), 3, "{gaps:?}");
    for (owner, claim) in &gaps {
        assert_eq!(owner, "crate::b1::host");
        assert!(claim.coverage_gap && claim.targets.is_empty());
        assert_eq!(claim.class, CallClass::Unknown);
    }
    let new_site = source.find("new Foo()").unwrap();
    let new_claim = claims
        .iter()
        .find(|(_, claim)| claim.site.start_byte == new_site && !claim.coverage_gap)
        .expect("new expression claim");
    assert_eq!(new_claim.0, "crate::b1::host");
    assert_eq!(new_claim.1.reason, INSIDE_REASON);
    assert_eq!(
        paths(&graph, NodeKind::Function),
        HashSet::from(["crate::b1::host".to_string()])
    );
}

#[test]
fn identity_span_form_and_containment_are_exact() {
    let source = "export function make() {\n\
  const alice = { name: () => 'a', greet() { return 'g'; }, label: function inner() { return 'l'; } };\n\
  return alice;\n\
}\n";
    let graph = build("ident.ts", source);
    let make = graph
        .nodes()
        .find(|n| n.path == "crate::ident::make")
        .unwrap()
        .id;
    for (key, form, start) in [
        ("name", "arrow", "name: () =>"),
        ("greet", "method_shorthand", "greet()"),
        ("label", "function_expression", "label: function"),
    ] {
        let path = format!("crate::ident::make::alice::@object::{key}");
        let node = graph.nodes().find(|n| n.path == path).expect(&path);
        assert_eq!(node.kind, NodeKind::Function);
        assert_eq!(node.name, key);
        assert_eq!(node.attr("member_form"), Some(form));
        assert_eq!(node.span.start_byte, source.find(start).unwrap());
        assert_eq!(node.file.as_deref(), Some("ident.ts"));
        let contained = graph
            .edges()
            .into_iter()
            .any(|(from, to, kind)| from == make && to == node.id && kind == EdgeKind::Contains);
        assert!(contained, "{path} not contained by nearest owner");
    }
    assert!(graph.nodes().all(|n| n.name != "inner"));
}

#[test]
fn declaration_only_signatures_are_never_functions_or_targets() {
    let source = "export interface Named { name(): string; label: () => string; (): void; }\n\
export type T = { area(): number };\n\
export abstract class B { abstract run(): void; step(): void; step(n?: number): void {} }\n\
export function over(): void;\n\
export function over(n?: number): void {}\n\
export function use(n: Named) { over(); return n.name(); }\n";
    let graph = build("decl.ts", source);
    let functions = paths(&graph, NodeKind::Function);
    for absent in [
        "crate::decl::Named::name",
        "crate::decl::Named::label",
        "crate::decl::T::area",
        "crate::decl::B::run",
    ] {
        assert!(!functions.contains(absent), "{absent} became a Function");
    }
    let field = graph
        .nodes()
        .find(|n| n.path == "crate::decl::Named::label")
        .expect("existing declaration-only Field");
    assert_eq!(field.kind, NodeKind::Field);
    let declaration_ids: HashSet<_> = graph
        .nodes()
        .filter(|n| n.kind != NodeKind::Function && n.kind != NodeKind::Module)
        .map(|n| n.id)
        .collect();
    assert!(all_claims(&graph)
        .iter()
        .all(|(_, claim)| claim.targets.iter().all(|t| !declaration_ids.contains(t))));
}

#[test]
fn no_proof_escalation_through_members_or_refused_subtrees() {
    let control =
        "export function target() { return 1; }\nexport function call() { return target(); }\n";
    let graph = build("control.ts", control);
    assert!(
        all_claims(&graph)
            .iter()
            .any(|(_, claim)| claim.class == CallClass::Must),
        "control lexical proof missing"
    );

    let source = "export function target() { return 1; }\n\
export function host() { let cfg = { run: () => target() }; return cfg.run(); }\n\
export const alice = { name: () => 'a' };\n\
export function direct() { return alice.name(); }\n";
    let graph = build("escalate.ts", source);
    let claims = all_claims(&graph);
    let inner = source.find("target() }").unwrap();
    let inner_claim = claims
        .iter()
        .find(|(_, claim)| claim.site.start_byte == inner && !claim.coverage_gap)
        .unwrap();
    assert_eq!(inner_claim.1.class, CallClass::Unknown);
    assert!(inner_claim.1.targets.is_empty());
    let member_call = source.find("alice.name()").unwrap();
    let member_claim = claims
        .iter()
        .find(|(_, claim)| claim.site.start_byte == member_call)
        .unwrap();
    assert_eq!(member_claim.1.class, CallClass::Unknown);
    assert!(member_claim.1.targets.is_empty());
    assert!(claims
        .iter()
        .all(|(_, claim)| claim.class != CallClass::May));
}

#[test]
fn r6_withdraws_flattened_shorthand_paths() {
    let source =
        "export function host() { return [{ name() { return 1; } }, { name() { return 2; } }]; }\n\
export let later = { run() { return 3; } };\n\
export default { fetch() { return 4; } };\n";
    let graph = build("r6.ts", source);
    assert_eq!(
        paths(&graph, NodeKind::Function),
        HashSet::from(["crate::r6::host".to_string()])
    );
    assert!(no_duplicate_paths(&graph));
}

#[test]
fn original_structural_fixture_stays_ambiguous_by_name() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/dispatch-corpus/typescript/structural-object-literal/app.test.ts");
    let source = std::fs::read_to_string(path).unwrap();
    let graph = build("app.test.ts", &source);
    let mut candidates: Vec<_> = graph
        .nodes()
        .filter(|n| n.name == "name" && n.kind == NodeKind::Function)
        .map(|n| n.path.clone())
        .collect();
    candidates.sort();
    assert_eq!(
        candidates,
        [
            "crate::app.test::alice::@object::name",
            "crate::app.test::bob::@object::name"
        ]
    );
    assert!(graph
        .nodes()
        .all(|n| !(n.name == "name" && n.kind != NodeKind::Function)));
}
