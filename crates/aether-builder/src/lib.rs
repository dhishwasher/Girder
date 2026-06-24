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
    use aether_graph::{EdgeKind, NodeId, NodeKind, SemanticGraph};

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
}
