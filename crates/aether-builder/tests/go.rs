use aether_builder::{GraphBuilder, Lang};
use aether_graph::{EdgeKind, NodeId, NodeKind, SemanticGraph};

fn has_edge(graph: &SemanticGraph, from: &str, to: &str, kind: EdgeKind) -> bool {
    graph
        .edges()
        .contains(&(NodeId::from_path(from), NodeId::from_path(to), kind))
}

#[test]
fn routes_go_and_extracts_package_vocabulary() {
    assert_eq!(Lang::from_path("main.go"), Some(Lang::Go));
    assert_eq!(Lang::from_path("main.mod"), None);

    let declarations = r#"
package sample

type Base struct{}
func (b *Base) String() string { return "base" }
func (*Base) Reset() {}

type Contract interface { Execute() }
type Embedded interface { Contract }

type Container struct {
    *Base
    Value string
}
type Alias string
const Limit = 4

func helper() {}
func (c Container) ValueMethod() { helper() }
func (c *Container) PointerMethod() { c.ValueMethod(); c.Base.String() }
"#;
    let tests = r#"
package sample_test
import "testing"
func TestFeature(t *testing.T) {}
func BenchmarkFeature(b *testing.B) {}
func FuzzFeature(f *testing.F) {}
func ExampleFeature() {}
func helperForTest() {}
"#;

    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();
    builder.set_go_module_path(Some("example.com/sample".to_string()));
    builder.load_files(
        &mut graph,
        [("model.go", declarations), ("model_test.go", tests)],
    );

    for (path, kind) in [
        ("crate", NodeKind::Module),
        ("crate::Base", NodeKind::Type),
        ("crate::Base::Reset", NodeKind::Function),
        ("crate::Contract", NodeKind::Type),
        ("crate::Embedded", NodeKind::Type),
        ("crate::Container", NodeKind::Type),
        ("crate::Alias", NodeKind::Type),
        ("crate::Limit", NodeKind::Field),
        ("crate::Container::Value", NodeKind::Field),
        ("crate::Container::ValueMethod", NodeKind::Function),
        ("crate::Container::PointerMethod", NodeKind::Function),
    ] {
        assert_eq!(graph.find_by_path(path).unwrap().kind, kind, "{path}");
    }

    assert!(has_edge(
        &graph,
        "crate::Container",
        "crate::Base",
        EdgeKind::Inherits
    ));
    assert!(has_edge(
        &graph,
        "crate::Embedded",
        "crate::Contract",
        EdgeKind::Inherits
    ));
    assert!(has_edge(
        &graph,
        "crate::Container::ValueMethod",
        "crate::helper",
        EdgeKind::Calls
    ));
    assert!(has_edge(
        &graph,
        "crate::Container::PointerMethod",
        "crate::Container::ValueMethod",
        EdgeKind::Calls
    ));
    assert!(has_edge(
        &graph,
        "crate::Container::PointerMethod",
        "crate::Base::String",
        EdgeKind::Calls
    ));

    for path in [
        "crate::TestFeature",
        "crate::BenchmarkFeature",
        "crate::FuzzFeature",
        "crate::ExampleFeature",
    ] {
        assert_eq!(
            graph.find_by_path(path).unwrap().attr("is_test"),
            Some("true")
        );
    }
    assert_eq!(
        graph
            .find_by_path("crate::helperForTest")
            .unwrap()
            .attr("is_test"),
        None
    );
}

#[test]
fn resolves_go_imports_by_module_path_and_fails_closed_on_shadowing() {
    let library = r#"
package library
func Target() {}
"#;
    let dotted = r#"
package dotted
func DotTarget() {}
"#;
    let hooks = r#"
package hooks
func init() {}
"#;
    let other = r#"
package other
func Target() {}
"#;
    let consumer = r#"
package consumer
import (
    x "example.com/project/library"
    . "example.com/project/dotted"
    _ "example.com/project/hooks"
)
func Target() {}
func Local() { Target() }
func Imported() { x.Target(); DotTarget() }
func Shadowed(Target func()) { Target() }
"#;

    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();
    builder.set_go_module_path(Some("example.com/project".to_string()));
    builder.load_files(
        &mut graph,
        [
            ("library/library.go", library),
            ("dotted/dotted.go", dotted),
            ("hooks/hooks.go", hooks),
            ("other/other.go", other),
            ("consumer/consumer.go", consumer),
        ],
    );

    assert!(has_edge(
        &graph,
        "crate::consumer::Local",
        "crate::consumer::Target",
        EdgeKind::Calls
    ));
    assert!(has_edge(
        &graph,
        "crate::consumer::Imported",
        "crate::library::Target",
        EdgeKind::Calls
    ));
    assert!(has_edge(
        &graph,
        "crate::consumer::Imported",
        "crate::dotted::DotTarget",
        EdgeKind::Calls
    ));
    assert!(!has_edge(
        &graph,
        "crate::consumer::Imported",
        "crate::other::Target",
        EdgeKind::Calls
    ));
    assert!(!has_edge(
        &graph,
        "crate::consumer::Shadowed",
        "crate::consumer::Target",
        EdgeKind::Calls
    ));
}
