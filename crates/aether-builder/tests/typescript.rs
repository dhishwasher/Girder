use aether_builder::{GraphBuilder, Lang};
use aether_graph::{EdgeKind, NodeId, NodeKind, SemanticGraph};

fn has_edge(graph: &SemanticGraph, from: &str, to: &str, kind: EdgeKind) -> bool {
    let from = NodeId::from_path(from);
    let to = NodeId::from_path(to);
    graph.edges().contains(&(from, to, kind))
}

#[test]
fn routes_typescript_extensions_and_uses_tsx_grammar() {
    assert_eq!(Lang::from_path("src/lib.ts"), Some(Lang::TypeScript));
    assert_eq!(Lang::from_path("src/lib.mts"), Some(Lang::TypeScript));
    assert_eq!(Lang::from_path("src/lib.cts"), Some(Lang::TypeScript));
    assert_eq!(Lang::from_path("src/view.tsx"), Some(Lang::Tsx));
    assert_eq!(Lang::from_path("src/view.js"), None);

    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();
    builder.load_file(
        &mut graph,
        "src/view.tsx",
        "export function View() { return <main>Hello</main>; }",
    );
    let view = graph.find_by_path("crate::view::View").unwrap();
    assert_eq!(view.kind, NodeKind::Function);
    assert_eq!(view.language, "typescript");
}

#[test]
fn extracts_definitions_imports_reexports_calls_and_heritage() {
    let base = r#"
export interface Contract { value: string; }
export class Base {}
export function helper() {}
export default helper;
"#;
    let barrel = r#"
export * from "./base";
export { helper as renamed } from "./base";
export { default } from "./base";
"#;
    let main = r#"
import defaultHelper, { Base as Parent, Contract, renamed } from "./barrel";
import * as ns from "./barrel";

export type Alias<T> = { value: T };
export interface Shape extends Contract { count: number; }
export class Child extends Parent implements Contract {
  value = "ok";
  method() { renamed(); ns.helper(); }
  callback = () => defaultHelper();
}
"#;

    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();
    builder.load_files(
        &mut graph,
        [("base.ts", base), ("barrel.ts", barrel), ("main.ts", main)],
    );

    for (path, kind) in [
        ("crate::main::Alias", NodeKind::Type),
        ("crate::main::Shape", NodeKind::Type),
        ("crate::main::Shape::count", NodeKind::Field),
        ("crate::main::Child", NodeKind::Type),
        ("crate::main::Child::method", NodeKind::Function),
        ("crate::main::Child::callback", NodeKind::Function),
    ] {
        assert_eq!(graph.find_by_path(path).unwrap().kind, kind, "{path}");
    }

    assert!(has_edge(
        &graph,
        "crate::main::Child",
        "crate::base::Base",
        EdgeKind::Inherits
    ));
    assert!(has_edge(
        &graph,
        "crate::main::Shape",
        "crate::base::Contract",
        EdgeKind::Inherits
    ));
    assert!(has_edge(
        &graph,
        "crate::main::Child",
        "crate::base::Contract",
        EdgeKind::Inherits
    ));
    assert!(has_edge(
        &graph,
        "crate::main::Child::method",
        "crate::base::helper",
        EdgeKind::Calls
    ));
    assert!(has_edge(
        &graph,
        "crate::main::Child::callback",
        "crate::base::helper",
        EdgeKind::Calls
    ));
}

#[test]
fn discovers_jest_vitest_tests_without_treating_jsx_as_a_call() {
    let source = r#"
function helper() {}
function Widget() { return <span />; }
describe("component", () => {
  it("renders", () => { helper(); return <Widget />; });
  test("works", function () { helper(); });
});
"#;
    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();
    builder.load_file(&mut graph, "src/example.test.tsx", source);

    for title in ["renders", "works"] {
        let test = graph.nodes().find(|node| node.name == title).unwrap();
        let path = test.path.clone();
        assert!(path.starts_with("crate::example.test::@describe["));
        assert_eq!(test.attr("is_test"), Some("true"));
        assert!(has_edge(
            &graph,
            &path,
            "crate::example.test::helper",
            EdgeKind::Calls
        ));
        assert!(!has_edge(
            &graph,
            &path,
            "crate::example.test::Widget",
            EdgeKind::Calls
        ));
    }
}

#[test]
fn declaration_files_include_plain_types_but_not_ambient_augmentation() {
    let source = r#"
/// <reference path="legacy.d.ts" />
export interface Visible { value: string; }
export type VisibleAlias = Visible | string;
declare global { interface HiddenGlobal { secret: string; } }
declare module "package-name" { interface HiddenAugmentation { secret: string; } }
"#;
    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();
    builder.load_file(&mut graph, "source/contracts.d.ts", source);

    assert!(graph
        .find_by_path("crate::source::contracts::Visible")
        .is_some());
    assert!(graph
        .find_by_path("crate::source::contracts::VisibleAlias")
        .is_some());
    assert!(graph
        .nodes()
        .all(|node| !matches!(node.name.as_str(), "HiddenGlobal" | "HiddenAugmentation")));
}

const COLLISION_SOURCE: &str = include_str!(
    "../../../docs/observations/stage3-typescript-audit/collision-repro/typescript-repro/sample.ts"
);
const HARDER_COLLISION_SOURCE: &str = include_str!(
    "../../../docs/observations/stage3-typescript-audit/identity-repair/harder.test.ts"
);

fn registration_graph(source: &str) -> SemanticGraph {
    let mut graph = SemanticGraph::new();
    GraphBuilder::new().load_file(&mut graph, "identity.test.ts", source);
    graph
}

#[test]
fn repeated_titles_retain_both_bodies_and_their_call_evidence() {
    let graph = registration_graph(COLLISION_SOURCE);
    let duplicates: Vec<_> = graph
        .nodes()
        .filter(|node| {
            node.attr("is_test") == Some("true")
                && node.name == "when project is indirectly referenced by solution"
        })
        .collect();
    assert_eq!(duplicates.len(), 2);
    assert_ne!(duplicates[0].id, duplicates[1].id);
    assert_eq!(
        graph
            .nodes()
            .filter(|n| n.attr("is_test").is_some())
            .count(),
        4
    );
    for (own, other) in [
        ("onlyInLost", "onlyInSurvivor"),
        ("onlyInSurvivor", "onlyInLost"),
    ] {
        let test = duplicates
            .iter()
            .find(|n| n.source.contains(&format!("{own}();")))
            .unwrap();
        let offset = COLLISION_SOURCE.find(&format!("{own}();")).unwrap();
        assert!(graph
            .call_evidence(test.id)
            .unwrap()
            .calls
            .iter()
            .any(|claim| claim.site.start_byte == offset));
        assert!(has_edge(
            &graph,
            &test.path,
            &format!("crate::identity.test::{own}"),
            EdgeKind::Calls
        ));
        assert!(!has_edge(
            &graph,
            &test.path,
            &format!("crate::identity.test::{other}"),
            EdgeKind::Calls
        ));
    }
}

#[test]
fn identical_suites_sibling_tests_and_delimiter_titles_do_not_merge() {
    let graph = registration_graph(HARDER_COLLISION_SOURCE);
    let tests: Vec<_> = graph
        .nodes()
        .filter(|n| n.attr("is_test") == Some("true"))
        .collect();
    assert_eq!(tests.len(), 5);
    assert_eq!(
        tests
            .iter()
            .map(|n| n.id)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        5
    );
    assert_eq!(tests.iter().filter(|n| n.name == "duplicate").count(), 3);
    for target in ["first", "second", "third", "fourth", "fifth"] {
        let call = format!("{target}();");
        let test = tests.iter().find(|n| n.source.contains(&call)).unwrap();
        let owner = if matches!(target, "first" | "second") {
            graph
                .find_by_path(&format!("{}::local", test.path))
                .unwrap()
        } else {
            test
        };
        let offset = HARDER_COLLISION_SOURCE.find(&call).unwrap();
        assert!(graph
            .call_evidence(owner.id)
            .unwrap()
            .calls
            .iter()
            .any(|claim| claim.site.start_byte == offset));
        assert!(has_edge(
            &graph,
            &owner.path,
            &format!("crate::identity.test::{target}"),
            EdgeKind::Calls
        ));
        for other in ["first", "second", "third", "fourth", "fifth"]
            .into_iter()
            .filter(|s| *s != target)
        {
            assert!(!has_edge(
                &graph,
                &owner.path,
                &format!("crate::identity.test::{other}"),
                EdgeKind::Calls
            ));
        }
        if owner.id != test.id {
            assert!(has_edge(
                &graph,
                &test.path,
                &owner.path,
                EdgeKind::Contains
            ));
            let invocation = test.span.start_byte + test.source.rfind("local();").unwrap();
            let evidence = graph.call_evidence(test.id).unwrap();
            let claim = evidence
                .calls
                .iter()
                .find(|c| c.site.start_byte == invocation)
                .unwrap();
            assert!(claim.targets.iter().all(|target| *target == owner.id));
            for other in graph
                .nodes()
                .filter(|n| n.name == "local" && n.id != owner.id)
            {
                assert!(!has_edge(&graph, &test.path, &other.path, EdgeKind::Calls));
            }
        }
    }
}

#[test]
fn registration_ids_survive_body_edits_offsets_and_unrelated_siblings() {
    let before = registration_graph(HARDER_COLLISION_SOURCE);
    let edited = format!(
        "\n// shift all byte positions\n{}",
        HARDER_COLLISION_SOURCE
            .replace("first();", "first(); const bodyEdit = 1;")
            .replacen(
                "describe('same'",
                "test('unrelated', () => {});\ndescribe('same'",
                1
            )
    );
    let after = registration_graph(&edited);
    for node in before.nodes().filter(|n| n.kind == NodeKind::Function) {
        let current = after
            .find_by_path(&node.path)
            .expect("existing function path remains present");
        assert_eq!(current.id, node.id);
        assert_eq!(current.name, node.name);
        assert!(current.span.start_byte > node.span.start_byte);
    }
    assert_eq!(
        after
            .nodes()
            .filter(|n| n.attr("is_test").is_some())
            .count(),
        6
    );
}
