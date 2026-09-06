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
        let path = format!("crate::example.test::{title}");
        let test = graph.find_by_path(&path).unwrap();
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
