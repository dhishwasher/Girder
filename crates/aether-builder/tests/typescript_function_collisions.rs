//! TypeScript functions and methods that compute the same semantic path (a getter/setter pair, the
//! same function declared in two blocks, a duplicate declaration) must each keep their own node,
//! body and call evidence. Before this fix the graph kept one node and silently dropped the rest.
//! Only colliding paths change; every non-colliding path is unchanged.

use aether_builder::GraphBuilder;
use aether_graph::{CallClass, EdgeKind, NodeId, SemanticGraph};

fn graph(source: &str) -> SemanticGraph {
    let mut graph = SemanticGraph::new();
    GraphBuilder::new().load_file(&mut graph, "shapes.ts", source);
    graph
}

fn paths(graph: &SemanticGraph) -> Vec<String> {
    let mut paths: Vec<String> = graph.nodes().map(|n| n.path.clone()).collect();
    paths.sort();
    paths
}

fn calls(graph: &SemanticGraph, from: &str, to: &str) -> bool {
    graph.edges().contains(&(
        NodeId::from_path(from),
        NodeId::from_path(to),
        EdgeKind::Calls,
    ))
}

const HELPERS: &str = "function a1(): number { return 1; }\nfunction a2(): number { return 2; }\n";

#[test]
fn a_getter_setter_pair_keeps_both_accessors() {
    let source = format!(
        "{HELPERS}export class Box {{\n  get value(): number {{ return a1(); }}\n  set value(x: number) {{ a2(); }}\n  other(): number {{ return a1(); }}\n}}\n"
    );
    let g = graph(&source);
    let get = g
        .find_by_path("crate::shapes::Box::value@get")
        .expect("getter node");
    let set = g
        .find_by_path("crate::shapes::Box::value@set")
        .expect("setter node");
    assert!(get.source.contains("get value"));
    assert!(set.source.contains("set value"));
    assert!(
        g.find_by_path("crate::shapes::Box::value").is_none(),
        "an ambiguous plain path must not exist"
    );
    assert!(
        calls(&g, &get.path, "crate::shapes::a1") && !calls(&g, &get.path, "crate::shapes::a2")
    );
    assert!(
        calls(&g, &set.path, "crate::shapes::a2") && !calls(&g, &set.path, "crate::shapes::a1")
    );
    // A non-colliding sibling keeps its plain path.
    assert!(g.find_by_path("crate::shapes::Box::other").is_some());
}

#[test]
fn a_lone_accessor_and_a_lone_method_keep_their_plain_paths() {
    let source = format!("{HELPERS}export class C {{\n  get only(): number {{ return a1(); }}\n  run() {{ a2(); }}\n}}\n");
    let g = graph(&source);
    assert!(g.find_by_path("crate::shapes::C::only").is_some());
    assert!(g.find_by_path("crate::shapes::C::run").is_some());
    assert!(
        !paths(&g).iter().any(|p| p.contains('@') || p.contains('#')),
        "{:?}",
        paths(&g)
    );
}

#[test]
fn the_same_function_in_two_blocks_keeps_both_bodies_and_calls() {
    let source = format!(
        "{HELPERS}if (Math.random() > 0.5) {{ function branch() {{ return a1(); }} branch(); }}\nelse {{ function branch() {{ return a2(); }} branch(); }}\n"
    );
    let g = graph(&source);
    let one = g
        .find_by_path("crate::shapes::branch#1")
        .expect("first branch");
    let two = g
        .find_by_path("crate::shapes::branch#2")
        .expect("second branch");
    assert_ne!(one.id, two.id);
    assert!(
        calls(&g, &one.path, "crate::shapes::a1") && !calls(&g, &one.path, "crate::shapes::a2")
    );
    assert!(
        calls(&g, &two.path, "crate::shapes::a2") && !calls(&g, &two.path, "crate::shapes::a1")
    );
}

#[test]
fn duplicate_top_level_declarations_are_both_kept() {
    let source =
        format!("{HELPERS}function dup() {{ return a1(); }}\nfunction dup() {{ return a2(); }}\n");
    let g = graph(&source);
    let one = g.find_by_path("crate::shapes::dup#1").unwrap();
    let two = g.find_by_path("crate::shapes::dup#2").unwrap();
    assert!(one.source.contains("a1()") && two.source.contains("a2()"));
}

#[test]
fn collisions_nested_under_a_colliding_parent_are_resolved_too() {
    let source = format!(
        "{HELPERS}function host() {{\n  if (a1()) {{ function pick() {{ function inner() {{ return a1(); }} function inner() {{ return a2(); }} }} }}\n  else {{ function pick() {{ return a2(); }} }}\n}}\n"
    );
    let g = graph(&source);
    let all = paths(&g);
    let unique: std::collections::HashSet<_> = all.iter().collect();
    assert_eq!(unique.len(), all.len(), "duplicate paths remain: {all:?}");
    assert_eq!(g.nodes().filter(|n| n.name == "inner").count(), 2);
    assert_eq!(g.nodes().filter(|n| n.name == "pick").count(), 2);
}

#[test]
fn non_colliding_paths_do_not_change() {
    let source = format!(
        "{HELPERS}export function outer1() {{ function inner() {{ return a1(); }} return inner(); }}\nexport function outer2() {{ function inner() {{ return a2(); }} return inner(); }}\nexport class P {{ run() {{ a1(); }} }}\nexport class Q {{ run() {{ a2(); }} }}\n"
    );
    let g = graph(&source);
    for path in [
        "crate::shapes::outer1",
        "crate::shapes::outer1::inner",
        "crate::shapes::outer2::inner",
        "crate::shapes::P::run",
        "crate::shapes::Q::run",
    ] {
        assert!(g.find_by_path(path).is_some(), "{path} missing");
    }
    assert!(
        !paths(&g).iter().any(|p| p.contains('@') || p.contains('#')),
        "{:?}",
        paths(&g)
    );
}

#[test]
fn identities_are_stable_across_rebuilds_and_unrelated_edits() {
    let base = format!(
        "{HELPERS}function dup() {{ return a1(); }}\nfunction dup() {{ return a2(); }}\nfunction unrelated() {{ return 1; }}\n"
    );
    let edited = base.replace("return 1; }\n", "return 100; }\n");
    let (a, b) = (graph(&base), graph(&edited));
    assert_eq!(
        a.find_by_path("crate::shapes::dup#1").unwrap().id,
        b.find_by_path("crate::shapes::dup#1").unwrap().id
    );
    assert_eq!(
        a.find_by_path("crate::shapes::dup#2").unwrap().id,
        graph(&base)
            .find_by_path("crate::shapes::dup#2")
            .unwrap()
            .id
    );
}

#[test]
fn no_duplicate_path_gap_remains_and_a_call_to_an_ambiguous_name_is_never_must() {
    let source = format!(
        "{HELPERS}function dup() {{ return a1(); }}\nfunction dup() {{ return a2(); }}\nexport function caller() {{ return dup(); }}\n"
    );
    let g = graph(&source);
    let module = g.find_by_path("crate::shapes").unwrap();
    let module_evidence = g.call_evidence(module.id).unwrap();
    assert!(
        !module_evidence
            .calls
            .iter()
            .any(|c| c.reason == "duplicate-semantic-path"),
        "the duplicate-path gap should be gone once paths are disambiguated"
    );
    let caller = g.find_by_path("crate::shapes::caller").unwrap();
    let claims = &g.call_evidence(caller.id).unwrap().calls;
    assert!(
        claims.iter().all(|c| c.class != CallClass::Must),
        "a call whose name matches two distinct functions must not be proven Must: {claims:?}"
    );
}
