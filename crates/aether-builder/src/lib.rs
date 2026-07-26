//! # aether-builder
//!
//! Turns source text into semantic-graph nodes/edges using tree-sitter, and
//! keeps the graph in sync as text changes. Text is treated strictly as an
//! *input projection*: the builder's job is to fold edits back into the graph
//! (the source of truth) and let other projections re-render.

pub mod highlight;
pub mod mapper;
pub mod parser;
pub mod sync;

pub use highlight::{spans, HlKind, HlSpan};
pub use mapper::{extract, module_path_for, BuildOutput};
pub use parser::{IncrementalParser, Lang};
pub use sync::GraphBuilder;

#[cfg(test)]
mod tests {
    use super::*;
    use aether_graph::{Edge, EdgeKind, Node, NodeId, NodeKind, SemanticGraph};

    const SAMPLE_RS: &str = r#"
struct Point {
    x: i64,
    y: i64,
}

fn add(a: i64, b: i64) -> i64 {
    a + b
}

fn sum_list(xs: &[i64]) -> i64 {
    let mut total = 0;
    for x in xs {
        total = add(total, *x);
    }
    total
}

fn main() {
    let r = sum_list(&[1, 2, 3]);
    println!("{}", r);
}
"#;

    #[test]
    fn extracts_functions_types_and_calls() {
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/math.rs", SAMPLE_RS);

        // Module + 3 functions + 1 struct + 2 fields.
        assert!(graph.find_by_path("crate::math").is_some());
        assert!(graph.find_by_path("crate::math::add").is_some());
        assert!(graph.find_by_path("crate::math::sum_list").is_some());
        assert!(graph.find_by_path("crate::math::main").is_some());
        let point = graph.find_by_path("crate::math::Point").unwrap();
        assert_eq!(point.kind, NodeKind::Type);
        assert!(graph.find_by_path("crate::math::Point::x").is_some());

        // sum_list calls add; main calls sum_list.
        let sum_id = NodeId::from_path("crate::math::sum_list");
        let add_id = NodeId::from_path("crate::math::add");
        let calls: Vec<_> = graph
            .neighbors(sum_id, Some(EdgeKind::Calls))
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert!(calls.contains(&add_id), "sum_list should call add");

        // Impact: changing `add` should reach sum_list (1) and main (2).
        let impact = graph.impact_of(add_id);
        assert!(impact.affected.contains_key(&sum_id));
        assert!(impact
            .affected
            .contains_key(&NodeId::from_path("crate::math::main")));
    }

    #[test]
    fn incremental_update_removes_deleted_function() {
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/math.rs", SAMPLE_RS);
        assert!(graph.find_by_path("crate::math::sum_list").is_some());

        // Edit: drop sum_list entirely. The graph must lose that node.
        let edited = SAMPLE_RS.replace(
            "fn sum_list(xs: &[i64]) -> i64 {\n    let mut total = 0;\n    for x in xs {\n        total = add(total, *x);\n    }\n    total\n}\n\n",
            "",
        );
        builder.update_file(&mut graph, "src/math.rs", &edited);
        assert!(
            graph.find_by_path("crate::math::sum_list").is_none(),
            "deleted function should be pruned from the graph"
        );
        // Surviving nodes remain.
        assert!(graph.find_by_path("crate::math::add").is_some());
    }

    #[test]
    fn module_paths_are_directory_aware() {
        assert_eq!(module_path_for("src/math.rs"), "crate::math");
        assert_eq!(module_path_for("src/net/client.rs"), "crate::net::client");
        assert_eq!(module_path_for("app/main.py"), "crate::app::main");
        assert_eq!(module_path_for("lib.rs"), "crate::lib");
    }

    #[test]
    fn field_access_emits_dataflow_and_impact() {
        let py = "class Calculator:\n    total = 0\n    def add(self, v):\n        self.total = self.total + v\n        return self.total\n";
        let mut g = SemanticGraph::new();
        let mut b = GraphBuilder::new();
        b.load_file(&mut g, "src/calc.py", py);

        let method = NodeId::from_path("crate::calc::Calculator::add");
        let field = NodeId::from_path("crate::calc::Calculator::total");
        // The method has a DataFlow edge to the field it touches.
        let flows: Vec<_> = g
            .neighbors(method, Some(EdgeKind::DataFlow))
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert!(flows.contains(&field), "add should flow to total");
        // Impact propagates through DataFlow: changing the field reaches the method.
        assert!(
            g.impact_of(field).affected.contains_key(&method),
            "changing total should impact add"
        );
    }

    #[test]
    fn methods_belong_to_their_type() {
        // Python class method.
        let py = "class Calculator:\n    def add(self, v):\n        return v\n";
        let mut g = SemanticGraph::new();
        let mut b = GraphBuilder::new();
        b.load_file(&mut g, "src/calc.py", py);
        let calc = NodeId::from_path("crate::calc::Calculator");
        let method = NodeId::from_path("crate::calc::Calculator::add");
        assert!(g.get(method).is_some(), "method is scoped to its class");
        let members: Vec<_> = g
            .neighbors(calc, Some(EdgeKind::Contains))
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert!(members.contains(&method), "class contains its method");

        // Rust inherent impl method.
        let rs = "struct Logger;\nimpl Logger { fn write(&self) {} }\n";
        let mut g2 = SemanticGraph::new();
        let mut b2 = GraphBuilder::new();
        b2.load_file(&mut g2, "src/zoo.rs", rs);
        let logger = NodeId::from_path("crate::zoo::Logger");
        let write = NodeId::from_path("crate::zoo::Logger::write");
        assert!(g2.get(write).is_some(), "impl method is scoped to its type");
        let members2: Vec<_> = g2
            .neighbors(logger, Some(EdgeKind::Contains))
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert!(members2.contains(&write), "type contains its impl method");
    }

    #[test]
    fn extracts_python_class_inheritance() {
        let py = "class Animal:\n    def speak(self):\n        return \"\"\n\nclass Dog(Animal):\n    def speak(self):\n        return \"woof\"\n";
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/pets.py", py);

        let dog = NodeId::from_path("crate::pets::Dog");
        let animal = NodeId::from_path("crate::pets::Animal");
        let bases: Vec<_> = graph
            .neighbors(dog, Some(EdgeKind::Inherits))
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert!(bases.contains(&animal), "Dog should inherit Animal");
    }

    #[test]
    fn extracts_rust_trait_impl_as_inherits() {
        let rs = "struct Logger;\ntrait Writer { fn write(&self); }\nimpl Writer for Logger { fn write(&self) {} }\n";
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/zoo.rs", rs);

        let logger = NodeId::from_path("crate::zoo::Logger");
        let writer = NodeId::from_path("crate::zoo::Writer");
        let impls: Vec<_> = graph
            .neighbors(logger, Some(EdgeKind::Inherits))
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert!(impls.contains(&writer), "Logger should implement Writer");
    }

    #[test]
    fn resolves_calls_across_files() {
        // `multiply` is defined in math.rs; `compute` in app.rs calls it.
        // Cross-file resolution must link compute -> multiply.
        let math_rs = "fn multiply(a: i64, b: i64) -> i64 { a * b }\n";
        let app_rs = "fn compute() -> i64 { multiply(6, 7) }\n";

        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/math.rs", math_rs);
        builder.load_file(&mut graph, "src/app.rs", app_rs);

        let compute = NodeId::from_path("crate::app::compute");
        let multiply = NodeId::from_path("crate::math::multiply");
        let calls: Vec<_> = graph
            .neighbors(compute, Some(EdgeKind::Calls))
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert!(
            calls.contains(&multiply),
            "compute (src/app.rs) should call multiply (src/math.rs) across files"
        );

        // And impact flows across the file boundary: changing multiply hits compute.
        assert!(graph.impact_of(multiply).affected.contains_key(&compute));
    }

    #[test]
    fn resolves_receiver_qualified_methods_and_type_scoped_callers() {
        let rs = r#"
struct MarketplaceCatalog;
impl MarketplaceCatalog {
    fn search(&self, _query: &str) {}
    fn refresh(&self) { self.search("impact"); }
}

struct SemanticGraph;
impl SemanticGraph {
    fn search(&self, _query: &str) {}
}

fn spawn() {}

fn browse(catalog: &MarketplaceCatalog) {
    catalog.search("graph");
    std::thread::spawn(|| {});
}
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/catalog.rs", rs);

        let marketplace_search = NodeId::from_path("crate::catalog::MarketplaceCatalog::search");
        let semantic_search = NodeId::from_path("crate::catalog::SemanticGraph::search");
        let refresh = NodeId::from_path("crate::catalog::MarketplaceCatalog::refresh");
        let browse = NodeId::from_path("crate::catalog::browse");

        let refresh_calls: Vec<_> = graph
            .neighbors(refresh, Some(EdgeKind::Calls))
            .into_iter()
            .map(|node| node.id)
            .collect();
        assert!(
            refresh_calls.contains(&marketplace_search),
            "self.search should resolve to the enclosing type's method"
        );

        let browse_calls: Vec<_> = graph
            .neighbors(browse, Some(EdgeKind::Calls))
            .into_iter()
            .map(|node| node.id)
            .collect();
        assert!(
            browse_calls.contains(&marketplace_search),
            "catalog.search should use the receiver as a type hint"
        );
        assert!(
            !browse_calls.contains(&semantic_search),
            "receiver-aware resolution should not link every same-named method"
        );
        assert!(
            !browse_calls.contains(&NodeId::from_path("crate::catalog::spawn")),
            "an unknown qualified receiver must not fall back to a same-named function"
        );

        let impact = graph.impact_of(marketplace_search);
        assert!(impact.affected.contains_key(&refresh));
        assert!(impact.affected.contains_key(&browse));

        let py = r#"
class MarketplaceCatalog:
    def search(self, query):
        return query
    def refresh(self):
        return self.search("impact")

class SemanticGraph:
    def search(self, query):
        return query

def browse(catalog):
    return catalog.search("graph")
"#;
        let mut python_graph = SemanticGraph::new();
        let mut python_builder = GraphBuilder::new();
        python_builder.load_file(&mut python_graph, "src/catalog.py", py);

        let python_search = NodeId::from_path("crate::catalog::MarketplaceCatalog::search");
        let python_refresh = NodeId::from_path("crate::catalog::MarketplaceCatalog::refresh");
        let python_browse = NodeId::from_path("crate::catalog::browse");
        let python_impact = python_graph.impact_of(python_search);
        assert!(python_impact.affected.contains_key(&python_refresh));
        assert!(python_impact.affected.contains_key(&python_browse));
    }

    #[test]
    fn rust_test_functions_marked_is_test() {
        let rs = r#"
fn add(a: i64, b: i64) -> i64 { a + b }

#[test]
fn test_add() { assert_eq!(add(1, 2), 3); }

#[tokio::test]
async fn test_add_async() { assert_eq!(add(1, 2), 3); }

fn helper() {}
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/math.rs", rs);

        let test_add = graph.find_by_path("crate::math::test_add").unwrap();
        assert_eq!(
            test_add.attr("is_test"),
            Some("true"),
            "#[test] fn should be marked"
        );

        let test_async = graph.find_by_path("crate::math::test_add_async").unwrap();
        assert_eq!(
            test_async.attr("is_test"),
            Some("true"),
            "#[tokio::test] should be marked"
        );

        let add = graph.find_by_path("crate::math::add").unwrap();
        assert_eq!(add.attr("is_test"), None, "regular fn should not be marked");

        let helper = graph.find_by_path("crate::math::helper").unwrap();
        assert_eq!(
            helper.attr("is_test"),
            None,
            "helper fn should not be marked"
        );
    }

    #[test]
    fn python_test_functions_marked_is_test() {
        let py = "def test_add():\n    assert add(1,2)==3\n\ndef add(a,b):\n    return a+b\n\ndef helper():\n    pass\n";
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/math.py", py);

        let test_add = graph.find_by_path("crate::math::test_add").unwrap();
        assert_eq!(
            test_add.attr("is_test"),
            Some("true"),
            "test_ fn should be marked"
        );

        let add = graph.find_by_path("crate::math::add").unwrap();
        assert_eq!(add.attr("is_test"), None, "regular fn should not be marked");
    }

    #[test]
    fn test_impact_finds_minimal_test_set() {
        // add is called by test_add (marked) and by sum_list (not marked).
        // Only test_add should appear in the impact set.
        let rs = r#"
fn add(a: i64, b: i64) -> i64 { a + b }

fn sum_list(xs: &[i64]) -> i64 {
    xs.iter().fold(0, |acc, x| add(acc, *x))
}

#[test]
fn test_add() { assert_eq!(add(2, 3), 5); }
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/math.rs", rs);

        let add = NodeId::from_path("crate::math::add");
        let test_add = NodeId::from_path("crate::math::test_add");

        let tests = graph.tests_for(add);
        assert!(
            tests.contains(&test_add),
            "test_add should be in impact set of add"
        );
        assert_eq!(
            tests.len(),
            1,
            "sum_list is not a test and should be excluded"
        );
    }

    #[test]
    fn parses_python_too() {
        let py = "def greet(name):\n    return hello(name)\n\ndef hello(name):\n    return name\n";
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "app/main.py", py);
        assert!(graph.find_by_path("crate::app::main::greet").is_some());
        let greet = NodeId::from_path("crate::app::main::greet");
        let hello = NodeId::from_path("crate::app::main::hello");
        let calls: Vec<_> = graph
            .neighbors(greet, Some(EdgeKind::Calls))
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert!(calls.contains(&hello));
    }

    #[test]
    fn source_refresh_preserves_agent_metadata() {
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/lib.rs", "fn run() -> i64 { 1 }\n");
        graph
            .find_by_path("crate::lib::run")
            .map(|node| node.id)
            .and_then(|id| graph.get_mut(id))
            .unwrap()
            .set_attr("summary", "durable summary");

        builder.update_file(&mut graph, "src/lib.rs", "fn run() -> i64 { 2 }\n");

        let run = graph.find_by_path("crate::lib::run").unwrap();
        assert_eq!(run.source, "fn run() -> i64 { 2 }");
        assert_eq!(run.attr("summary"), Some("durable summary"));
    }

    #[test]
    fn source_refresh_removes_stale_dataflow_edges() {
        let initial = "class Counter:\n    total = 0\n    other = 0\n    def read(self):\n        return self.total\n";
        let edited = "class Counter:\n    total = 0\n    other = 0\n    def read(self):\n        return self.other\n";
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "counter.py", initial);
        let read = NodeId::from_path("crate::counter::Counter::read");
        let total = NodeId::from_path("crate::counter::Counter::total");
        let other = NodeId::from_path("crate::counter::Counter::other");

        builder.update_file(&mut graph, "counter.py", edited);

        let flows: Vec<_> = graph
            .neighbors(read, Some(EdgeKind::DataFlow))
            .into_iter()
            .map(|neighbor| neighbor.id)
            .collect();
        assert!(!flows.contains(&total));
        assert!(flows.contains(&other));
    }

    #[test]
    fn source_refresh_preserves_calls_involving_graph_owned_nodes() {
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/lib.rs", "fn existing() {}\n");
        let existing = NodeId::from_path("crate::lib::existing");
        let mut generated = Node::new(NodeKind::Function, "generated", "crate::forge::generated")
            .with_language("rust")
            .with_source("fn generated() { existing() }");
        generated.file = Some("src/forge.rs".into());
        generated.set_attr("authored_by", "Coder");
        let generated = graph.upsert_node(generated);
        graph
            .add_edge(generated, existing, Edge::new(EdgeKind::Calls))
            .unwrap();

        builder.update_file(
            &mut graph,
            "src/lib.rs",
            "fn existing() { println!(\"ok\"); }\n",
        );

        let calls: Vec<_> = graph
            .neighbors(generated, Some(EdgeKind::Calls))
            .into_iter()
            .map(|neighbor| neighbor.id)
            .collect();
        assert!(calls.contains(&existing));
    }
}
