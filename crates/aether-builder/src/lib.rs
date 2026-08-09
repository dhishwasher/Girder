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
        let module = graph.find_by_path("crate::math").unwrap();
        assert_eq!(module.source, SAMPLE_RS);
        assert_eq!(module.attr("source_projection"), Some("file-v1"));
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
        assert_eq!(graph.find_by_path("crate::math").unwrap().source, edited);
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
    fn resolves_factory_return_types_across_files() {
        let projection = r#"
struct ProjectionPlan;
impl ProjectionPlan {
    fn writes(&self) {}
    fn commit(self) {}
}

fn plan_authored_functions() -> Result<ProjectionPlan, ()> {
    Ok(ProjectionPlan)
}
"#;
        let collaboration = r#"
struct CollaborationProjectionPlan;
impl CollaborationProjectionPlan {
    fn writes(&self) {}
    fn commit(self) {}
}
"#;
        let app = r#"
fn apply() -> Result<(), ()> {
    let plan = plan_authored_functions()?;
    plan.writes();
    plan.commit();
    Ok(())
}

fn apply_unwrapped() {
    let plan = plan_authored_functions().expect("plan");
    plan.commit();
}

fn apply_mapped() -> Result<(), ()> {
    let plan = plan_authored_functions().map_err(|error| error)?;
    plan.writes();
    plan.commit();
    Ok(())
}
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/projection.rs", projection);
        builder.load_file(&mut graph, "src/collaboration.rs", collaboration);
        builder.load_file(&mut graph, "src/app.rs", app);

        let factory = graph
            .find_by_path("crate::projection::plan_authored_functions")
            .unwrap();
        assert_eq!(
            factory.attr("return_type"),
            Some("Result<ProjectionPlan, ()>")
        );

        let projection_writes = NodeId::from_path("crate::projection::ProjectionPlan::writes");
        let projection_commit = NodeId::from_path("crate::projection::ProjectionPlan::commit");
        let collaboration_writes =
            NodeId::from_path("crate::collaboration::CollaborationProjectionPlan::writes");
        let collaboration_commit =
            NodeId::from_path("crate::collaboration::CollaborationProjectionPlan::commit");
        for caller in [
            NodeId::from_path("crate::app::apply"),
            NodeId::from_path("crate::app::apply_unwrapped"),
            NodeId::from_path("crate::app::apply_mapped"),
        ] {
            let calls: Vec<_> = graph
                .neighbors(caller, Some(EdgeKind::Calls))
                .into_iter()
                .map(|node| node.id)
                .collect();
            assert!(
                calls.contains(&projection_commit),
                "{caller:?} should resolve ProjectionPlan::commit; calls={calls:?}"
            );
            assert!(!calls.contains(&collaboration_commit));
            if caller != NodeId::from_path("crate::app::apply_unwrapped") {
                assert!(calls.contains(&projection_writes));
                assert!(!calls.contains(&collaboration_writes));
            }
        }
    }

    #[test]
    fn resolves_self_factories_through_generic_result_wrappers() {
        let rs = r#"
struct GraphReplica;
impl GraphReplica {
    fn load() -> Result<Self, ()> { Ok(Self) }
    fn fork(&mut self) -> Result<Self, ()> { Ok(Self) }
    fn members(&self) {}
    fn remove_member(&mut self) -> Result<(), ()> { Ok(()) }
}

struct OtherReplica;
impl OtherReplica {
    fn fork(&mut self) -> Result<Self, ()> { Ok(Self) }
    fn members(&self) {}
    fn remove_member(&mut self) -> Result<(), ()> { Ok(()) }
}

fn collaboration_result<T>(result: Result<T, ()>) -> Result<T, ()> { result }
fn replace_result<T>(_result: Result<T, ()>) -> Result<OtherReplica, ()> {
    Ok(OtherReplica)
}
fn collect_result<T>(_result: Result<T, ()>) -> Result<Vec<T>, ()> {
    Ok(Vec::new())
}

fn inspect() -> Result<(), ()> {
    let mut alice = collaboration_result(GraphReplica::load())?;
    let mut bob = alice.fork()?;
    bob.members();
    collaboration_result(bob.remove_member())?;
    Ok(())
}

fn inspect_replaced() -> Result<(), ()> {
    let other = replace_result(GraphReplica::load())?;
    other.members();
    Ok(())
}

fn inspect_collected() -> Result<(), ()> {
    let collected = collect_result(GraphReplica::load())?;
    collected.members();
    Ok(())
}
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/collaboration.rs", rs);

        let wrapper = graph
            .find_by_path("crate::collaboration::collaboration_result")
            .unwrap();
        assert_eq!(wrapper.attr("type_parameters"), Some("<T>"));
        assert_eq!(wrapper.attr("first_parameter_type"), Some("Result<T, ()>"));

        let inspect = NodeId::from_path("crate::collaboration::inspect");
        let calls: Vec<_> = graph
            .neighbors(inspect, Some(EdgeKind::Calls))
            .into_iter()
            .map(|node| node.id)
            .collect();
        for expected in [
            "crate::collaboration::GraphReplica::fork",
            "crate::collaboration::GraphReplica::members",
            "crate::collaboration::GraphReplica::remove_member",
        ] {
            assert!(
                calls.contains(&NodeId::from_path(expected)),
                "inspect should call {expected}; calls={calls:?}"
            );
        }
        for incorrect in [
            "crate::collaboration::OtherReplica::fork",
            "crate::collaboration::OtherReplica::members",
            "crate::collaboration::OtherReplica::remove_member",
        ] {
            assert!(
                !calls.contains(&NodeId::from_path(incorrect)),
                "inspect must not call {incorrect}; calls={calls:?}"
            );
        }

        let replaced = NodeId::from_path("crate::collaboration::inspect_replaced");
        let replaced_calls: Vec<_> = graph
            .neighbors(replaced, Some(EdgeKind::Calls))
            .into_iter()
            .map(|node| node.id)
            .collect();
        assert!(replaced_calls.contains(&NodeId::from_path(
            "crate::collaboration::OtherReplica::members"
        )));
        assert!(!replaced_calls.contains(&NodeId::from_path(
            "crate::collaboration::GraphReplica::members"
        )));

        let collected = NodeId::from_path("crate::collaboration::inspect_collected");
        let collected_calls: Vec<_> = graph
            .neighbors(collected, Some(EdgeKind::Calls))
            .into_iter()
            .map(|node| node.id)
            .collect();
        assert!(!collected_calls.contains(&NodeId::from_path(
            "crate::collaboration::GraphReplica::members"
        )));
        assert!(!collected_calls.contains(&NodeId::from_path(
            "crate::collaboration::OtherReplica::members"
        )));
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
    fn batch_load_resolves_project_references_after_all_files() {
        let app_rs = "fn compute() -> i64 { multiply(6, 7) }\n";
        let math_rs = "fn multiply(a: i64, b: i64) -> i64 { a * b }\n";
        let base_py = "class Base:\n    pass\n";
        let derived_py = "class Derived(Base):\n    pass\n";
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();

        builder.load_files(
            &mut graph,
            [
                ("src/app.rs", app_rs),
                ("src/math.rs", math_rs),
                ("python/derived.py", derived_py),
                ("python/base.py", base_py),
            ],
        );

        let compute = NodeId::from_path("crate::app::compute");
        let multiply = NodeId::from_path("crate::math::multiply");
        assert_eq!(
            graph
                .neighbors(compute, Some(EdgeKind::Calls))
                .into_iter()
                .map(|node| node.id)
                .collect::<Vec<_>>(),
            vec![multiply]
        );
        let derived = NodeId::from_path("crate::python::derived::Derived");
        let base = NodeId::from_path("crate::python::base::Base");
        assert_eq!(
            graph
                .neighbors(derived, Some(EdgeKind::Inherits))
                .into_iter()
                .map(|node| node.id)
                .collect::<Vec<_>>(),
            vec![base]
        );
    }

    #[test]
    fn resolves_renamed_imports_and_transitive_reexports_exactly() {
        let transport = r#"
pub(crate) fn join() {}
pub(crate) fn leave() {}
"#;
        let project = r#"
pub(crate) use collaboration_transport::join as join_collaboration;
"#;
        let app = r#"
use crate::project::join_collaboration;
use crate::project::collaboration_transport::leave as connect_elsewhere;

fn start() {
    std::thread::spawn(move || join_collaboration());
}

fn direct() {
    connect_elsewhere();
}
"#;
        let decoys = r#"
fn join() {}
fn leave() {}
fn join_collaboration() {}
fn connect_elsewhere() {}
"#;

        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(
            &mut graph,
            "src/project/collaboration_transport.rs",
            transport,
        );
        builder.load_file(&mut graph, "src/decoys.rs", decoys);
        builder.load_file(&mut graph, "src/project.rs", project);
        builder.load_file(&mut graph, "src/app.rs", app);

        let transport_join = NodeId::from_path("crate::project::collaboration_transport::join");
        let transport_leave = NodeId::from_path("crate::project::collaboration_transport::leave");
        let decoy_join = NodeId::from_path("crate::decoys::join_collaboration");
        let decoy_connect = NodeId::from_path("crate::decoys::connect_elsewhere");
        let start = NodeId::from_path("crate::app::start");
        let direct = NodeId::from_path("crate::app::direct");
        let calls_from = |graph: &SemanticGraph, caller| {
            graph
                .neighbors(caller, Some(EdgeKind::Calls))
                .into_iter()
                .map(|node| node.id)
                .collect::<Vec<_>>()
        };

        let start_calls = calls_from(&graph, start);
        assert!(start_calls.contains(&transport_join));
        assert!(!start_calls.contains(&decoy_join));
        let direct_calls = calls_from(&graph, direct);
        assert!(direct_calls.contains(&transport_leave));
        assert!(!direct_calls.contains(&decoy_connect));

        builder.update_file(
            &mut graph,
            "src/project.rs",
            "pub(crate) use collaboration_transport::leave as join_collaboration;\n",
        );
        let updated = calls_from(&graph, start);
        assert!(updated.contains(&transport_leave));
        assert!(!updated.contains(&transport_join));
        assert!(!updated.contains(&decoy_join));
    }

    #[test]
    fn resolves_qualified_reexports_through_mod_and_crate_root_files() {
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(
            &mut graph,
            "src/project/commands/graph.rs",
            "pub(crate) fn analyze() {}\n",
        );
        builder.load_file(
            &mut graph,
            "src/project/commands/mod.rs",
            "pub(crate) use graph::analyze;\n",
        );
        builder.load_file(
            &mut graph,
            "src/project.rs",
            "pub(crate) use commands::analyze;\n",
        );
        builder.load_file(&mut graph, "src/decoy.rs", "fn analyze() {}\n");
        builder.load_file(
            &mut graph,
            "src/main.rs",
            "fn dispatch() { project::analyze(); }\n",
        );

        let dispatch = NodeId::from_path("crate::main::dispatch");
        let command = NodeId::from_path("crate::project::commands::graph::analyze");
        let decoy = NodeId::from_path("crate::decoy::analyze");
        let calls = graph
            .neighbors(dispatch, Some(EdgeKind::Calls))
            .into_iter()
            .map(|node| node.id)
            .collect::<Vec<_>>();
        assert!(calls.contains(&command));
        assert!(!calls.contains(&decoy));
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
    fn contains(&self, _query: &str) -> bool { true }
}

struct CapabilityDelta;
impl CapabilityDelta {
    fn is_empty(&self) -> bool { true }
}

struct GraphDelta;
impl GraphDelta {
    fn is_empty(&self) -> bool { true }
}

fn spawn() {}

fn browse(catalog: &MarketplaceCatalog) {
    catalog.search("graph");
    std::thread::spawn(|| {});
}

fn macro_browse(catalog: &MarketplaceCatalog, text: &str) {
    assert!({ catalog.search("graph"); text.contains("graph") });
}

fn inspect_delta(delta: &CapabilityDelta) -> bool {
    delta.is_empty()
}

fn inspect_replica() {
    let replica = GraphReplica::from_graph();
    let staged = replica.clone();
    staged.materialize();
}

struct GraphReplica;
impl GraphReplica {
    fn from_graph() -> Self { Self }
    fn materialize(&self) {}
}
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/catalog.rs", rs);

        let marketplace_search = NodeId::from_path("crate::catalog::MarketplaceCatalog::search");
        let semantic_search = NodeId::from_path("crate::catalog::SemanticGraph::search");
        let refresh = NodeId::from_path("crate::catalog::MarketplaceCatalog::refresh");
        let browse = NodeId::from_path("crate::catalog::browse");
        let macro_browse = NodeId::from_path("crate::catalog::macro_browse");
        let inspect_delta = NodeId::from_path("crate::catalog::inspect_delta");
        let inspect_replica = NodeId::from_path("crate::catalog::inspect_replica");

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

        let macro_calls: Vec<_> = graph
            .neighbors(macro_browse, Some(EdgeKind::Calls))
            .into_iter()
            .map(|node| node.id)
            .collect();
        assert!(
            macro_calls.contains(&marketplace_search),
            "macro token-tree calls should retain a usable receiver hint"
        );
        assert!(
            !macro_calls.contains(&NodeId::from_path(
                "crate::catalog::SemanticGraph::contains"
            )),
            "qualified macro calls must not use the unqualified unique-name fallback"
        );
        let delta_calls: Vec<_> = graph
            .neighbors(inspect_delta, Some(EdgeKind::Calls))
            .into_iter()
            .map(|node| node.id)
            .collect();
        assert_eq!(
            delta_calls,
            vec![NodeId::from_path(
                "crate::catalog::CapabilityDelta::is_empty"
            )],
            "Rust parameter annotations should disambiguate same-suffix receiver names"
        );
        assert!(
            graph
                .neighbors(inspect_replica, Some(EdgeKind::Calls))
                .iter()
                .any(|node| {
                    node.id == NodeId::from_path("crate::catalog::GraphReplica::materialize")
                }),
            "constructor-initialized locals should retain their receiver type"
        );

        let impact = graph.impact_of(marketplace_search);
        assert!(impact.affected.contains_key(&refresh));
        assert!(impact.affected.contains_key(&browse));
        assert!(impact.affected.contains_key(&macro_browse));

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
    fn macro_literal_text_does_not_create_call_edges() {
        let source = r##"
fn embedded_only_as_text() {}
fn actual_macro_call() -> bool { true }

#[test]
fn macro_fixture() {
    let _source = format!(r#"fn generated() { embedded_only_as_text(); }"#);
    assert!(actual_macro_call());
}
"##;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/macro_fixture.rs", source);

        let fixture = NodeId::from_path("crate::macro_fixture::macro_fixture");
        let embedded = NodeId::from_path("crate::macro_fixture::embedded_only_as_text");
        let actual = NodeId::from_path("crate::macro_fixture::actual_macro_call");
        let calls = |graph: &SemanticGraph| {
            graph
                .neighbors(fixture, Some(EdgeKind::Calls))
                .into_iter()
                .map(|node| node.id)
                .collect::<Vec<_>>()
        };

        assert!(!calls(&graph).contains(&embedded));
        assert!(calls(&graph).contains(&actual));
        assert_eq!(graph.tests_for(actual), vec![fixture]);

        builder.update_file(
            &mut graph,
            "src/macro_fixture.rs",
            &source.replace(
                "    assert!(actual_macro_call());",
                "    assert!(\"actual_macro_call()\".len() > 0);",
            ),
        );
        assert!(!calls(&graph).contains(&actual));
        assert!(
            graph.tests_for(actual).is_empty(),
            "incremental refresh must remove the stale real macro call edge"
        );
    }

    #[test]
    fn resolves_rust_if_let_narrowed_receiver_types() {
        let source = r#"
struct SessionIdentity;
impl SessionIdentity {
    fn inspect(&self) {}
    fn verify_delta_provenance(&self) {}
}

struct OtherIdentity;
impl OtherIdentity {
    fn inspect(&self) {}
}

fn apply(identity: Option<&SessionIdentity>) {
    if let Some(identity) = identity {
        identity.inspect();
        identity.verify_delta_provenance();
    }
}

fn inspect_ok(result: Result<&SessionIdentity, &OtherIdentity>) {
    if let Ok(identity) = result {
        identity.inspect();
    }
}

fn inspect_err(result: Result<&SessionIdentity, &OtherIdentity>) {
    if let Err(identity) = result {
        identity.inspect();
    }
}

#[test]
fn provenance_test() {
    apply(Some(&SessionIdentity));
}
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/session.rs", source);

        let apply = NodeId::from_path("crate::session::apply");
        let inspect_ok = NodeId::from_path("crate::session::inspect_ok");
        let inspect_err = NodeId::from_path("crate::session::inspect_err");
        let session_inspect = NodeId::from_path("crate::session::SessionIdentity::inspect");
        let other_inspect = NodeId::from_path("crate::session::OtherIdentity::inspect");
        let verify = NodeId::from_path("crate::session::SessionIdentity::verify_delta_provenance");
        let provenance_test = NodeId::from_path("crate::session::provenance_test");

        let calls = |graph: &SemanticGraph, caller| {
            graph
                .neighbors(caller, Some(EdgeKind::Calls))
                .into_iter()
                .map(|node| node.id)
                .collect::<Vec<_>>()
        };
        let apply_calls = calls(&graph, apply);
        assert!(
            apply_calls.contains(&session_inspect),
            "Some(binding) must narrow the consequence receiver to Option's inner type"
        );
        assert!(apply_calls.contains(&verify));
        assert!(!apply_calls.contains(&other_inspect));
        assert_eq!(calls(&graph, inspect_ok), vec![session_inspect]);
        assert_eq!(calls(&graph, inspect_err), vec![other_inspect]);
        assert_eq!(graph.tests_for(verify), vec![provenance_test]);

        builder.update_file(
            &mut graph,
            "src/session.rs",
            &source.replace(
                r#"    if let Some(identity) = identity {
        identity.inspect();
        identity.verify_delta_provenance();
    }
"#,
                "",
            ),
        );
        let updated_calls = calls(&graph, apply);
        assert!(!updated_calls.contains(&session_inspect));
        assert!(!updated_calls.contains(&verify));
        assert!(
            graph.tests_for(verify).is_empty(),
            "incremental refresh must remove the stale narrowed call edge"
        );
    }

    #[test]
    fn resolves_rust_match_arm_narrowed_receiver_types() {
        let source = r#"
struct SessionIdentity;
impl SessionIdentity {
    fn inspect(&self) {}
    fn is_valid(&self) -> bool { true }
    fn verify_selected_operation(&self) {}
}

struct DecoyIdentity;
impl DecoyIdentity {
    fn inspect(&self) {}
    fn is_valid(&self) -> bool { false }
}

struct Failure;
impl Failure {
    fn inspect(&self) {}
}

fn apply(identity: Option<&SessionIdentity>) {
    match identity {
        Some(identity) if identity.is_valid() => {
            identity.inspect();
            identity.verify_selected_operation();
        }
        _ => {}
    }
}

fn inspect_result(result: Result<&SessionIdentity, &Failure>) {
    match result {
        Ok(identity) => identity.inspect(),
        Err(failure) => failure.inspect(),
    }
}

#[test]
fn provenance_test() {
    apply(Some(&SessionIdentity));
}
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/session.rs", source);

        let apply = NodeId::from_path("crate::session::apply");
        let inspect_result = NodeId::from_path("crate::session::inspect_result");
        let session_inspect = NodeId::from_path("crate::session::SessionIdentity::inspect");
        let session_is_valid = NodeId::from_path("crate::session::SessionIdentity::is_valid");
        let verify =
            NodeId::from_path("crate::session::SessionIdentity::verify_selected_operation");
        let decoy_inspect = NodeId::from_path("crate::session::DecoyIdentity::inspect");
        let decoy_is_valid = NodeId::from_path("crate::session::DecoyIdentity::is_valid");
        let failure_inspect = NodeId::from_path("crate::session::Failure::inspect");
        let provenance_test = NodeId::from_path("crate::session::provenance_test");

        let calls = |graph: &SemanticGraph, caller| {
            graph
                .neighbors(caller, Some(EdgeKind::Calls))
                .into_iter()
                .map(|node| node.id)
                .collect::<Vec<_>>()
        };
        let apply_calls = calls(&graph, apply);
        assert!(apply_calls.contains(&session_inspect));
        assert!(apply_calls.contains(&session_is_valid));
        assert!(apply_calls.contains(&verify));
        assert!(!apply_calls.contains(&decoy_inspect));
        assert!(!apply_calls.contains(&decoy_is_valid));

        let result_calls = calls(&graph, inspect_result);
        assert!(result_calls.contains(&session_inspect));
        assert!(result_calls.contains(&failure_inspect));
        assert!(!result_calls.contains(&decoy_inspect));
        assert_eq!(graph.tests_for(verify), vec![provenance_test]);

        builder.update_file(
            &mut graph,
            "src/session.rs",
            &source.replace("            identity.verify_selected_operation();\n", ""),
        );
        assert!(!calls(&graph, apply).contains(&verify));
        assert!(
            graph.tests_for(verify).is_empty(),
            "incremental refresh must remove the stale match-arm call edge"
        );
    }

    #[test]
    fn resolves_rust_let_else_narrowed_receiver_types() {
        let source = r#"
struct SessionIdentity;
impl SessionIdentity {
    fn inspect(&self) {}
    fn verify_selected_operation(&self) {}
}

struct DecoyIdentity;
impl DecoyIdentity {
    fn inspect(&self) {}
}

struct Failure;
impl Failure {
    fn inspect(&self) {}
}

fn apply(identity: Option<&SessionIdentity>) {
    let Some(identity) = identity else {
        return;
    };
    identity.inspect();
    identity.verify_selected_operation();
}

fn inspect_ok(result: Result<&SessionIdentity, &Failure>) {
    let Ok(identity) = result else {
        return;
    };
    identity.inspect();
}

fn inspect_err(result: Result<&SessionIdentity, &Failure>) {
    let Err(failure) = result else {
        return;
    };
    failure.inspect();
}

fn nested_scope(identity: Option<&SessionIdentity>, decoy: &DecoyIdentity) {
    {
        let Some(identity) = identity else {
            return;
        };
        identity.inspect();
    }
    decoy.inspect();
}

#[test]
fn provenance_test() {
    apply(Some(&SessionIdentity));
}
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/session.rs", source);

        let apply = NodeId::from_path("crate::session::apply");
        let inspect_ok = NodeId::from_path("crate::session::inspect_ok");
        let inspect_err = NodeId::from_path("crate::session::inspect_err");
        let nested_scope = NodeId::from_path("crate::session::nested_scope");
        let session_inspect = NodeId::from_path("crate::session::SessionIdentity::inspect");
        let verify =
            NodeId::from_path("crate::session::SessionIdentity::verify_selected_operation");
        let decoy_inspect = NodeId::from_path("crate::session::DecoyIdentity::inspect");
        let failure_inspect = NodeId::from_path("crate::session::Failure::inspect");
        let provenance_test = NodeId::from_path("crate::session::provenance_test");

        let calls = |graph: &SemanticGraph, caller| {
            graph
                .neighbors(caller, Some(EdgeKind::Calls))
                .into_iter()
                .map(|node| node.id)
                .collect::<Vec<_>>()
        };
        let apply_calls = calls(&graph, apply);
        assert!(apply_calls.contains(&session_inspect));
        assert!(apply_calls.contains(&verify));
        assert!(!apply_calls.contains(&decoy_inspect));
        assert_eq!(calls(&graph, inspect_ok), vec![session_inspect]);
        assert_eq!(calls(&graph, inspect_err), vec![failure_inspect]);

        let nested_calls = calls(&graph, nested_scope);
        assert!(nested_calls.contains(&session_inspect));
        assert!(nested_calls.contains(&decoy_inspect));
        assert_eq!(graph.tests_for(verify), vec![provenance_test]);

        builder.update_file(
            &mut graph,
            "src/session.rs",
            &source.replace("    identity.verify_selected_operation();\n", ""),
        );
        assert!(!calls(&graph, apply).contains(&verify));
        assert!(
            graph.tests_for(verify).is_empty(),
            "incremental refresh must remove the stale let-else call edge"
        );
    }

    #[test]
    fn resolves_rust_let_chain_narrowed_receiver_types() {
        let source = r#"
struct SessionIdentity;
impl SessionIdentity {
    fn inspect(&self) {}
    fn is_valid(&self) -> bool { true }
    fn verify_selected_operation(&self) {}
}

struct DecoyIdentity;
impl DecoyIdentity {
    fn inspect(&self) {}
    fn is_valid(&self) -> bool { false }
}

struct Failure;
impl Failure {
    fn inspect(&self) {}
}

fn ready() -> bool { true }

fn apply(identity: Option<&SessionIdentity>) {
    if ready() && let Some(identity) = identity && identity.is_valid() {
        identity.inspect();
        identity.verify_selected_operation();
    }
}

fn inspect_ok(result: Result<&SessionIdentity, &Failure>) {
    if let Ok(identity) = result && identity.is_valid() {
        identity.inspect();
    }
}

fn ordered(identity: Option<&SessionIdentity>, decoy: &DecoyIdentity) {
    if decoy.is_valid() && let Some(identity) = identity && identity.is_valid() {
        identity.inspect();
    }
}

fn poll(identity: Option<&SessionIdentity>) {
    while let Some(identity) = identity && identity.is_valid() {
        identity.inspect();
        break;
    }
}

#[test]
fn provenance_test() {
    apply(Some(&SessionIdentity));
}
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/session.rs", source);

        let apply = NodeId::from_path("crate::session::apply");
        let inspect_ok = NodeId::from_path("crate::session::inspect_ok");
        let ordered = NodeId::from_path("crate::session::ordered");
        let poll = NodeId::from_path("crate::session::poll");
        let ready = NodeId::from_path("crate::session::ready");
        let session_inspect = NodeId::from_path("crate::session::SessionIdentity::inspect");
        let session_is_valid = NodeId::from_path("crate::session::SessionIdentity::is_valid");
        let verify =
            NodeId::from_path("crate::session::SessionIdentity::verify_selected_operation");
        let decoy_inspect = NodeId::from_path("crate::session::DecoyIdentity::inspect");
        let decoy_is_valid = NodeId::from_path("crate::session::DecoyIdentity::is_valid");
        let provenance_test = NodeId::from_path("crate::session::provenance_test");

        let calls = |graph: &SemanticGraph, caller| {
            graph
                .neighbors(caller, Some(EdgeKind::Calls))
                .into_iter()
                .map(|node| node.id)
                .collect::<Vec<_>>()
        };
        let apply_calls = calls(&graph, apply);
        assert!(apply_calls.contains(&ready));
        assert!(apply_calls.contains(&session_is_valid));
        assert!(apply_calls.contains(&session_inspect));
        assert!(apply_calls.contains(&verify));
        assert!(!apply_calls.contains(&decoy_is_valid));
        assert!(!apply_calls.contains(&decoy_inspect));

        let ok_calls = calls(&graph, inspect_ok);
        assert!(ok_calls.contains(&session_is_valid));
        assert!(ok_calls.contains(&session_inspect));
        assert!(!ok_calls.contains(&decoy_is_valid));
        assert!(!ok_calls.contains(&decoy_inspect));

        let ordered_calls = calls(&graph, ordered);
        assert!(ordered_calls.contains(&decoy_is_valid));
        assert!(ordered_calls.contains(&session_is_valid));
        assert!(ordered_calls.contains(&session_inspect));
        assert!(!ordered_calls.contains(&decoy_inspect));

        let poll_calls = calls(&graph, poll);
        assert!(poll_calls.contains(&session_is_valid));
        assert!(poll_calls.contains(&session_inspect));
        assert_eq!(graph.tests_for(verify), vec![provenance_test]);

        builder.update_file(
            &mut graph,
            "src/session.rs",
            &source.replace("        identity.verify_selected_operation();\n", ""),
        );
        assert!(!calls(&graph, apply).contains(&verify));
        assert!(
            graph.tests_for(verify).is_empty(),
            "incremental refresh must remove the stale let-chain call edge"
        );
    }

    #[test]
    fn rust_test_functions_marked_is_test() {
        let rs = r#"
fn add(a: i64, b: i64) -> i64 { a + b }

#[test]
fn test_add() { assert_eq!(add(1, 2), 3); }

#[tokio::test]
async fn test_add_async() { assert_eq!(add(1, 2), 3); }

#[tokio :: test]
async fn test_add_spaced_path() { assert_eq!(add(1, 2), 3); }

#[tokio /* path comment */ :: test]
async fn test_add_commented_path() { assert_eq!(add(1, 2), 3); }

#[rstest]
fn test_add_parameterized() { assert_eq!(add(1, 2), 3); }

#[test]
// The outer attribute still applies across a regular comment.
fn test_add_after_line_comment() { assert_eq!(add(1, 2), 3); }

#[test]
/* The outer attribute still applies across a block comment. */
fn test_add_after_block_comment() { assert_eq!(add(1, 2), 3); }

#[test]
/// The outer attribute still applies across a doc comment.
fn test_add_after_doc_comment() { assert_eq!(add(1, 2), 3); }

#[cfg(test)]
fn cfg_test_helper() {}

#[cfg_attr(feature = "integration", test)]
fn conditional_test_helper() {}

#[contest]
fn contest_helper() {}

#[doc = "test helper"]
fn documented_helper() {}

#[cfg(not(unix))]
#[test]
fn test_not_unix_only() {}

#[cfg(unix)]
#[test]
fn test_unix_only() {}

#[cfg(all(unix, not(windows)))]
#[test]
fn test_composed_cfg() {}

#[cfg(any(windows, target_os = "fuchsia"))]
#[test]
fn test_foreign_os_only() {}

#[cfg(feature = "extra")]
#[test]
fn test_unknown_feature() {}

#[cfg(test)]
#[test]
fn test_under_cfg_test() {}

fn outer() {
    #[test]
    fn test_nested() {}
}
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

        for test in [
            "test_add_spaced_path",
            "test_add_commented_path",
            "test_add_after_line_comment",
            "test_add_after_block_comment",
            "test_add_after_doc_comment",
        ] {
            let node = graph.find_by_path(&format!("crate::math::{test}")).unwrap();
            assert_eq!(
                node.attr("is_test"),
                Some("true"),
                "valid test attribute spelling should mark {test}"
            );
        }

        let parameterized = graph
            .find_by_path("crate::math::test_add_parameterized")
            .unwrap();
        assert_eq!(
            parameterized.attr("is_test"),
            Some("true"),
            "#[rstest] should be marked"
        );

        let add = graph.find_by_path("crate::math::add").unwrap();
        assert_eq!(add.attr("is_test"), None, "regular fn should not be marked");

        for helper in [
            "cfg_test_helper",
            "conditional_test_helper",
            "contest_helper",
            "documented_helper",
        ] {
            let node = graph
                .find_by_path(&format!("crate::math::{helper}"))
                .unwrap();
            assert_eq!(
                node.attr("is_test"),
                None,
                "attribute text must not make {helper} a test"
            );
        }

        // Host-conditional compilation: a cfg that is provably false here
        // withholds test identity because Cargo never compiles the function;
        // unknown predicates stay fail-open.
        if cfg!(unix) {
            for absent in ["test_not_unix_only", "test_foreign_os_only"] {
                let node = graph
                    .find_by_path(&format!("crate::math::{absent}"))
                    .unwrap();
                assert_eq!(
                    node.attr("is_test"),
                    None,
                    "{absent} is not compiled on this host and must not be a test"
                );
            }
            for present in ["test_unix_only", "test_composed_cfg"] {
                let node = graph
                    .find_by_path(&format!("crate::math::{present}"))
                    .unwrap();
                assert_eq!(
                    node.attr("is_test"),
                    Some("true"),
                    "{present} is compiled on this host and must stay a test"
                );
            }
        }
        for fail_open in ["test_unknown_feature", "test_under_cfg_test"] {
            let node = graph
                .find_by_path(&format!("crate::math::{fail_open}"))
                .unwrap();
            assert_eq!(
                node.attr("is_test"),
                Some("true"),
                "{fail_open} must stay a test (unknown or test-profile cfg)"
            );
        }

        // libtest never collects a fn nested inside another fn; the nested
        // definition is scoped to its enclosing function, not the module.
        let nested = graph
            .find_by_path("crate::math::outer::test_nested")
            .unwrap();
        assert_eq!(
            nested.attr("is_test"),
            None,
            "a nested #[test] fn is never collected"
        );
        assert!(graph.find_by_path("crate::math::test_nested").is_none());
    }

    #[test]
    fn python_test_identity_follows_framework_collection() {
        let py = "def test_add():\n    assert add(1,2)==3\n\ndef testfoo():\n    pass\n\ndef add(a,b):\n    return a+b\n\ndef helper():\n    pass\n\ndef outer():\n    def test_nested():\n        pass\n\nclass TestSuite:\n    def test_method(self):\n        pass\n";

        // In a collectable test file, module-level `test*` defs and class
        // methods are tests; nested defs and non-test names are not.
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "test_math.py", py);

        for test in ["test_add", "testfoo"] {
            let node = graph
                .find_by_path(&format!("crate::test_math::{test}"))
                .unwrap();
            assert_eq!(
                node.attr("is_test"),
                Some("true"),
                "collectable {test} should be marked"
            );
        }
        let method = graph
            .find_by_path("crate::test_math::TestSuite::test_method")
            .unwrap();
        assert_eq!(
            method.attr("is_test"),
            Some("true"),
            "class test methods are collected"
        );
        for plain in ["add", "helper", "outer"] {
            let node = graph
                .find_by_path(&format!("crate::test_math::{plain}"))
                .unwrap();
            assert_eq!(node.attr("is_test"), None, "{plain} is not a test");
        }
        let nested = graph
            .find_by_path("crate::test_math::outer::test_nested")
            .unwrap();
        assert_eq!(
            nested.attr("is_test"),
            None,
            "pytest never collects a def nested inside a function"
        );
        assert!(graph
            .find_by_path("crate::test_math::test_nested")
            .is_none());

        // pytest's second default pattern also collects.
        let mut suffix_graph = SemanticGraph::new();
        let mut suffix_builder = GraphBuilder::new();
        suffix_builder.load_file(
            &mut suffix_graph,
            "math_test.py",
            "def test_add():\n    pass\n",
        );
        let suffix_test = suffix_graph
            .find_by_path("crate::math_test::test_add")
            .unwrap();
        assert_eq!(suffix_test.attr("is_test"), Some("true"));

        // The same defs in a non-test file are never collected by pytest or
        // unittest and must not enter the test universe.
        let mut plain_graph = SemanticGraph::new();
        let mut plain_builder = GraphBuilder::new();
        plain_builder.load_file(&mut plain_graph, "src/math.py", py);
        let uncollectable = plain_graph.find_by_path("crate::math::test_add").unwrap();
        assert_eq!(
            uncollectable.attr("is_test"),
            None,
            "a test-shaped def outside test files is not collectable"
        );
    }

    #[test]
    fn resolves_python_annotated_receiver_types_across_files() {
        let models = r#"
class SessionIdentity:
    def inspect(self):
        return True

    def verify_selected_operation(self):
        return True

class DecoyIdentity:
    def inspect(self):
        return False
"#;
        let service = r#"
import models
from models import DecoyIdentity, SessionIdentity

def apply(identity: SessionIdentity):
    identity.inspect()
    identity.verify_selected_operation()

def inspect_dotted(identity: models.DecoyIdentity):
    identity.inspect()

def inspect_forward(identity: "SessionIdentity"):
    identity.inspect()

def inspect_default(identity: DecoyIdentity = DecoyIdentity()):
    identity.inspect()

"#;
        let tests = r#"
from models import SessionIdentity
from service import apply

def test_provenance():
    apply(SessionIdentity())
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "models.py", models);
        builder.load_file(&mut graph, "service.py", service);
        builder.load_file(&mut graph, "test_service.py", tests);

        let apply = NodeId::from_path("crate::service::apply");
        let inspect_dotted = NodeId::from_path("crate::service::inspect_dotted");
        let inspect_forward = NodeId::from_path("crate::service::inspect_forward");
        let inspect_default = NodeId::from_path("crate::service::inspect_default");
        let session_inspect = NodeId::from_path("crate::models::SessionIdentity::inspect");
        let verify = NodeId::from_path("crate::models::SessionIdentity::verify_selected_operation");
        let decoy_inspect = NodeId::from_path("crate::models::DecoyIdentity::inspect");
        let provenance_test = NodeId::from_path("crate::test_service::test_provenance");

        let calls = |graph: &SemanticGraph, caller| {
            graph
                .neighbors(caller, Some(EdgeKind::Calls))
                .into_iter()
                .map(|node| node.id)
                .collect::<Vec<_>>()
        };
        let apply_calls = calls(&graph, apply);
        assert!(apply_calls.contains(&session_inspect));
        assert!(apply_calls.contains(&verify));
        assert!(!apply_calls.contains(&decoy_inspect));
        assert_eq!(calls(&graph, inspect_dotted), vec![decoy_inspect]);
        assert_eq!(calls(&graph, inspect_forward), vec![session_inspect]);
        assert_eq!(calls(&graph, inspect_default), vec![decoy_inspect]);
        assert_eq!(graph.tests_for(session_inspect), vec![provenance_test]);

        builder.update_file(
            &mut graph,
            "service.py",
            &service.replacen("    identity.inspect()\n", "", 1),
        );
        assert!(!calls(&graph, apply).contains(&session_inspect));
        assert!(
            graph.tests_for(session_inspect).is_empty(),
            "incremental refresh must remove the stale annotated receiver edge"
        );
    }

    #[test]
    fn resolves_unambiguous_python_nullable_receiver_types() {
        let models = r#"
class SessionIdentity:
    def inspect(self):
        return True

    def verify_selected_operation(self):
        return True

class DecoyIdentity:
    def inspect(self):
        return False
"#;
        let service = r#"
import custom as typing
import typing as typed
from custom import Optional as CustomOptional, Union as CustomUnion
from typing import TYPE_CHECKING, Optional, Optional as Maybe, Union, Union as Choice
from models import DecoyIdentity, SessionIdentity, SessionIdentity as Session
from custom import Optional as LateOptional
from typing import Optional as EarlyOptional
from models import SessionIdentity as OrderedIdentity

def alias_before_shadow(ordered_identity: OrderedIdentity | None):
    ordered_identity.inspect()

from models import DecoyIdentity as OrderedIdentity

def alias_after_shadow(ordered_identity: OrderedIdentity | None):
    ordered_identity.inspect()

def alias_before_import(future_identity: FutureIdentity | None):
    future_identity.verify_selected_operation()

from models import SessionIdentity as FutureIdentity

def before_late_import(session_identity: LateOptional[SessionIdentity]):
    session_identity.verify_selected_operation()

def before_later_shadow(early_identity: EarlyOptional[SessionIdentity]):
    early_identity.inspect()

from typing import Optional as LateOptional
from custom import Optional as EarlyOptional

def after_late_import(late_identity: LateOptional[SessionIdentity]):
    late_identity.inspect()

if runtime_flag:
    from custom import Optional as BranchOptional
    import models as branch_models
else:
    from typing import Optional as BranchOptional
    import custom as branch_models

def branch_ambiguous(session_identity: BranchOptional[SessionIdentity]):
    session_identity.verify_selected_operation()

def branch_dotted():
    branch_models.SessionIdentity.verify_selected_operation()

if TYPE_CHECKING:
    from typing import Optional as CheckedOptional

from custom import TYPE_CHECKING as UntrustedChecking

if UntrustedChecking:
    from typing import Optional as UntrustedGuardOptional

if runtime_flag:
    from custom import Optional as ElifOptional
elif other_flag:
    from typing import Optional as ElifOptional
else:
    from typing import Optional as ElifOptional

if runtime_flag:
    from typing import Optional as NoElseOptional
elif other_flag:
    from typing import Optional as NoElseOptional

def pep604(identity: SessionIdentity | None):
    if identity is None:
        return False
    identity.inspect()
    return identity.verify_selected_operation()

def optional(identity: Optional[SessionIdentity]):
    if identity is not None:
        identity.inspect()

def union(identity: Union[None, SessionIdentity]):
    if identity is None:
        return
    identity.inspect()

def forward(identity: "(SessionIdentity | None)"):
    if identity is not None:
        identity.inspect()

def aliased_optional(identity: Maybe[SessionIdentity]):
    if identity is not None:
        identity.inspect()

def aliased_union(identity: Choice[Session | SessionIdentity | "None"]):
    if identity is not None:
        identity.inspect()

def qualified(identity: typed.Optional[SessionIdentity]):
    if identity is not None:
        identity.inspect()

def type_checking_import(identity: CheckedOptional[SessionIdentity]):
    if identity is not None:
        identity.inspect()

def untrusted_type_checking(session_identity: UntrustedGuardOptional[SessionIdentity]):
    session_identity.verify_selected_operation()

def elif_wrapper(session_identity: ElifOptional[SessionIdentity]):
    session_identity.verify_selected_operation()

def no_else_wrapper(session_identity: NoElseOptional[SessionIdentity]):
    session_identity.verify_selected_operation()

def function_local_import(identity: SessionIdentity):
    from typing import Optional as LocalOptional
    selected: LocalOptional[SessionIdentity] = identity
    selected.inspect()

def function_local_order(identity):
    from custom import Optional as OrderedOptional
    session_identity: OrderedOptional[SessionIdentity] = identity
    session_identity.verify_selected_operation()
    from typing import Optional as OrderedOptional
    selected: OrderedOptional[SessionIdentity] = identity
    selected.inspect()
    OrderedOptional = CustomOptional
    session_identity: OrderedOptional[SessionIdentity] = identity
    session_identity.verify_selected_operation()

def parameter_annotation_shadow(Optional, identity: Optional[SessionIdentity]):
    identity.inspect()

def parameter_body_shadow(Optional, identity):
    session_identity: Optional[SessionIdentity] = identity
    session_identity.verify_selected_operation()

def shadowed_type_checking(TYPE_CHECKING, identity):
    if TYPE_CHECKING:
        from typing import Optional as ShadowedGuardOptional
    session_identity: ShadowedGuardOptional[SessionIdentity] = identity
    session_identity.verify_selected_operation()

def ambiguous(identity: SessionIdentity | DecoyIdentity):
    identity.inspect()

def ambiguous_union(identity: Union[SessionIdentity, DecoyIdentity, None]):
    identity.inspect()

def ambiguous_named(session_identity: SessionIdentity | DecoyIdentity):
    session_identity.inspect()

def custom_optional(identity: CustomOptional[SessionIdentity]):
    identity.inspect()

def custom_named(session_identity: CustomOptional[SessionIdentity]):
    session_identity.verify_selected_operation()

def custom_union(identity: CustomUnion[SessionIdentity, None]):
    identity.inspect()

def shadowed_qualified(identity: typing.Optional[SessionIdentity]):
    identity.inspect()

def dotted_suffix(identity: SessionIdentity | None, box):
    box.identity.inspect()

def shadowed_import_root(typed):
    typed.identity.verify_selected_operation()

"#;
        let tests = r#"
from models import SessionIdentity
from service import pep604

def test_provenance():
    pep604(SessionIdentity())
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "models.py", models);
        builder.load_file(&mut graph, "service.py", service);
        builder.load_file(&mut graph, "test_service.py", tests);

        let pep604 = NodeId::from_path("crate::service::pep604");
        let optional = NodeId::from_path("crate::service::optional");
        let union = NodeId::from_path("crate::service::union");
        let forward = NodeId::from_path("crate::service::forward");
        let aliased_optional = NodeId::from_path("crate::service::aliased_optional");
        let aliased_union = NodeId::from_path("crate::service::aliased_union");
        let qualified = NodeId::from_path("crate::service::qualified");
        let type_checking_import = NodeId::from_path("crate::service::type_checking_import");
        let untrusted_type_checking = NodeId::from_path("crate::service::untrusted_type_checking");
        let elif_wrapper = NodeId::from_path("crate::service::elif_wrapper");
        let no_else_wrapper = NodeId::from_path("crate::service::no_else_wrapper");
        let shadowed_type_checking = NodeId::from_path("crate::service::shadowed_type_checking");
        let function_local_import = NodeId::from_path("crate::service::function_local_import");
        let function_local_order = NodeId::from_path("crate::service::function_local_order");
        let before_late_import = NodeId::from_path("crate::service::before_late_import");
        let before_later_shadow = NodeId::from_path("crate::service::before_later_shadow");
        let after_late_import = NodeId::from_path("crate::service::after_late_import");
        let alias_before_shadow = NodeId::from_path("crate::service::alias_before_shadow");
        let alias_after_shadow = NodeId::from_path("crate::service::alias_after_shadow");
        let alias_before_import = NodeId::from_path("crate::service::alias_before_import");
        let branch_ambiguous = NodeId::from_path("crate::service::branch_ambiguous");
        let branch_dotted = NodeId::from_path("crate::service::branch_dotted");
        let parameter_annotation_shadow =
            NodeId::from_path("crate::service::parameter_annotation_shadow");
        let parameter_body_shadow = NodeId::from_path("crate::service::parameter_body_shadow");
        let ambiguous = NodeId::from_path("crate::service::ambiguous");
        let ambiguous_union = NodeId::from_path("crate::service::ambiguous_union");
        let ambiguous_named = NodeId::from_path("crate::service::ambiguous_named");
        let custom_optional = NodeId::from_path("crate::service::custom_optional");
        let custom_named = NodeId::from_path("crate::service::custom_named");
        let custom_union = NodeId::from_path("crate::service::custom_union");
        let shadowed_qualified = NodeId::from_path("crate::service::shadowed_qualified");
        let dotted_suffix = NodeId::from_path("crate::service::dotted_suffix");
        let shadowed_import_root = NodeId::from_path("crate::service::shadowed_import_root");
        let session_inspect = NodeId::from_path("crate::models::SessionIdentity::inspect");
        let verify = NodeId::from_path("crate::models::SessionIdentity::verify_selected_operation");
        let decoy_inspect = NodeId::from_path("crate::models::DecoyIdentity::inspect");
        let provenance_test = NodeId::from_path("crate::test_service::test_provenance");

        let calls = |graph: &SemanticGraph, caller| {
            graph
                .neighbors(caller, Some(EdgeKind::Calls))
                .into_iter()
                .map(|node| node.id)
                .collect::<Vec<_>>()
        };
        let pep604_calls = calls(&graph, pep604);
        assert!(pep604_calls.contains(&session_inspect));
        assert!(pep604_calls.contains(&verify));
        assert!(!pep604_calls.contains(&decoy_inspect));
        assert!(calls(&graph, before_late_import).is_empty());
        assert_eq!(calls(&graph, before_later_shadow), vec![session_inspect]);
        assert_eq!(calls(&graph, after_late_import), vec![session_inspect]);
        assert_eq!(calls(&graph, function_local_order), vec![session_inspect]);
        assert_eq!(calls(&graph, alias_before_shadow), vec![session_inspect]);
        assert_eq!(calls(&graph, alias_after_shadow), vec![decoy_inspect]);
        assert!(calls(&graph, alias_before_import).is_empty());
        for caller in [
            optional,
            union,
            forward,
            aliased_optional,
            aliased_union,
            qualified,
            type_checking_import,
            function_local_import,
            parameter_annotation_shadow,
        ] {
            assert_eq!(calls(&graph, caller), vec![session_inspect]);
        }
        for (name, caller) in [
            ("ambiguous", ambiguous),
            ("ambiguous_union", ambiguous_union),
            ("ambiguous_named", ambiguous_named),
            ("custom_optional", custom_optional),
            ("custom_named", custom_named),
            ("custom_union", custom_union),
            ("shadowed_qualified", shadowed_qualified),
            ("dotted_suffix", dotted_suffix),
            ("shadowed_import_root", shadowed_import_root),
            ("parameter_body_shadow", parameter_body_shadow),
            ("branch_ambiguous", branch_ambiguous),
            ("branch_dotted", branch_dotted),
            ("untrusted_type_checking", untrusted_type_checking),
            ("elif_wrapper", elif_wrapper),
            ("no_else_wrapper", no_else_wrapper),
            ("shadowed_type_checking", shadowed_type_checking),
        ] {
            let unexpected = calls(&graph, caller);
            assert!(
                unexpected.is_empty(),
                "ambiguous or untrusted receiver in {name} must remain unresolved: {unexpected:?}"
            );
        }
        assert_eq!(graph.tests_for(session_inspect), vec![provenance_test]);

        let without_pep604_call = service.replacen("    identity.inspect()\n", "", 1);
        builder.update_file(&mut graph, "service.py", &without_pep604_call);
        assert!(!calls(&graph, pep604).contains(&session_inspect));
        assert!(
            graph.tests_for(session_inspect).is_empty(),
            "incremental refresh must remove the stale nullable receiver edge"
        );

        let untrusted_late = without_pep604_call.replacen(
            "from typing import Optional as LateOptional",
            "from custom import Optional as LateOptional",
            1,
        );
        builder.update_file(&mut graph, "service.py", &untrusted_late);
        assert!(
            calls(&graph, after_late_import).is_empty(),
            "incremental refresh must remove an edge when wrapper provenance becomes untrusted"
        );
    }

    #[test]
    fn resolves_sound_python_isinstance_receiver_narrowing() {
        let source = r#"
class AliasPath:
    def convert_to_aliases(self):
        return []

class AliasChoices:
    def convert_to_aliases(self):
        aliases = []
        for c in self.choices:
            if isinstance(c, AliasPath):
                aliases.append(c.convert_to_aliases())
        return aliases

class DecoyPath:
    def convert_to_aliases(self):
        return []

def parameter_shadow(isinstance, choices):
    for c in choices:
        if isinstance(c, AliasPath):
            c.convert_to_aliases()

def local_builtin_shadow(choices):
    for c in choices:
        if isinstance(c, AliasPath):
            c.convert_to_aliases()
    isinstance = lambda value, kind: True

def local_type_shadow(AliasPath, choices):
    for c in choices:
        if isinstance(c, AliasPath):
            c.convert_to_aliases()

def tuple_guard(choices):
    for c in choices:
        if isinstance(c, (AliasPath, DecoyPath)):
            c.convert_to_aliases()

def boolean_guard(choices, ready):
    for c in choices:
        if isinstance(c, AliasPath) or ready:
            c.convert_to_aliases()

def negative_guard(choices):
    for c in choices:
        if not isinstance(c, AliasPath):
            c.convert_to_aliases()

def else_branch(choices):
    for c in choices:
        if isinstance(c, AliasPath):
            pass
        else:
            c.convert_to_aliases()

def after_guard(choices):
    for c in choices:
        if isinstance(c, AliasPath):
            pass
        c.convert_to_aliases()

def for_target_rebinding(c, arbitrary_items):
    if isinstance(c, AliasPath):
        for c in arbitrary_items:
            c.convert_to_aliases()

def with_target_rebinding(c, manager):
    if isinstance(c, AliasPath):
        with manager as c:
            c.convert_to_aliases()

def except_target_rebinding(c):
    if isinstance(c, AliasPath):
        try:
            pass
        except Exception as c:
            c.convert_to_aliases()

def case_target_rebinding(c, value):
    if isinstance(c, AliasPath):
        match value:
            case c:
                c.convert_to_aliases()

def comprehension_rebinding(c, arbitrary_items):
    if isinstance(c, AliasPath):
        [c.convert_to_aliases() for c in arbitrary_items]
        {c.convert_to_aliases() for c in arbitrary_items}
        {c: c.convert_to_aliases() for c in arbitrary_items}
        tuple(c.convert_to_aliases() for c in arbitrary_items)

def lambda_rebinding(c):
    if isinstance(c, AliasPath):
        callback = lambda c: c.convert_to_aliases()

def direct_rebinding(choices):
    for c in choices:
        if isinstance(c, AliasPath):
            c = DecoyPath()
            c.convert_to_aliases()

def compound_rebinding(choices, ready):
    for c in choices:
        if isinstance(c, AliasPath):
            if ready:
                c = DecoyPath()
            c.convert_to_aliases()
"#;
        let shadowed_module = r#"
class ModuleAliasPath:
    def convert_to_aliases(self):
        return []

def isinstance(value, kind):
    return True

def module_shadow(choices):
    for c in choices:
        if isinstance(c, ModuleAliasPath):
            c.convert_to_aliases()
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "aliases.py", source);

        let calls = |graph: &SemanticGraph, caller| {
            graph
                .neighbors(caller, Some(EdgeKind::Calls))
                .into_iter()
                .map(|node| node.id)
                .collect::<Vec<_>>()
        };
        let alias_path = NodeId::from_path("crate::aliases::AliasPath::convert_to_aliases");
        let alias_choices = NodeId::from_path("crate::aliases::AliasChoices::convert_to_aliases");
        let decoy_path = NodeId::from_path("crate::aliases::DecoyPath::convert_to_aliases");

        assert_eq!(calls(&graph, alias_choices), vec![alias_path]);
        for function in [
            "parameter_shadow",
            "local_builtin_shadow",
            "local_type_shadow",
            "tuple_guard",
            "boolean_guard",
            "negative_guard",
            "else_branch",
            "after_guard",
            "for_target_rebinding",
            "with_target_rebinding",
            "except_target_rebinding",
            "case_target_rebinding",
            "comprehension_rebinding",
            "lambda_rebinding",
            "compound_rebinding",
        ] {
            let caller = NodeId::from_path(&format!("crate::aliases::{function}"));
            assert!(
                calls(&graph, caller).is_empty(),
                "unproven narrowing in {function} must not emit a call edge"
            );
        }
        assert_eq!(
            calls(
                &graph,
                NodeId::from_path("crate::aliases::direct_rebinding")
            ),
            vec![decoy_path]
        );
        let mut shadowed_graph = SemanticGraph::new();
        let mut shadowed_builder = GraphBuilder::new();
        shadowed_builder.load_file(&mut shadowed_graph, "shadowed.py", shadowed_module);
        let shadowed_calls = calls(
            &shadowed_graph,
            NodeId::from_path("crate::shadowed::module_shadow"),
        );
        assert!(
            !shadowed_calls.contains(&NodeId::from_path(
                "crate::shadowed::ModuleAliasPath::convert_to_aliases",
            )),
            "a module-level isinstance binding must disable receiver narrowing"
        );

        builder.update_file(
            &mut graph,
            "aliases.py",
            &source.replacen(
                "if isinstance(c, AliasPath):",
                "if check_type(c, AliasPath):",
                1,
            ),
        );
        assert!(
            calls(&graph, alias_choices).is_empty(),
            "incremental refresh must remove the stale narrowed receiver edge"
        );
    }

    #[test]
    fn resolves_python_constructor_assignment_receiver_types() {
        let models = r#"
class SessionIdentity:
    def inspect(self):
        return True

    def verify_selected_operation(self):
        return True

class DecoyIdentity:
    def inspect(self):
        return False
"#;
        let service = r#"
import models
from models import DecoyIdentity, SessionIdentity

def unknown():
    return None

def apply():
    identity = SessionIdentity()
    identity.inspect()
    identity.verify_selected_operation()

def inspect_dotted():
    identity = models.DecoyIdentity()
    identity.inspect()

def ordered():
    identity = DecoyIdentity()
    identity.inspect()
    identity = SessionIdentity()
    identity.inspect()
    identity.verify_selected_operation()

def invalidated():
    identity = SessionIdentity()
    identity = unknown()
    identity.inspect()

def annotated_local():
    identity: DecoyIdentity = unknown()
    identity.inspect()

"#;
        let tests = r#"
from service import apply

def test_provenance():
    apply()
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "models.py", models);
        builder.load_file(&mut graph, "service.py", service);
        builder.load_file(&mut graph, "test_service.py", tests);

        let apply = NodeId::from_path("crate::service::apply");
        let inspect_dotted = NodeId::from_path("crate::service::inspect_dotted");
        let ordered = NodeId::from_path("crate::service::ordered");
        let invalidated = NodeId::from_path("crate::service::invalidated");
        let annotated_local = NodeId::from_path("crate::service::annotated_local");
        let unknown = NodeId::from_path("crate::service::unknown");
        let session_inspect = NodeId::from_path("crate::models::SessionIdentity::inspect");
        let verify = NodeId::from_path("crate::models::SessionIdentity::verify_selected_operation");
        let decoy_inspect = NodeId::from_path("crate::models::DecoyIdentity::inspect");
        let provenance_test = NodeId::from_path("crate::test_service::test_provenance");

        let calls = |graph: &SemanticGraph, caller| {
            graph
                .neighbors(caller, Some(EdgeKind::Calls))
                .into_iter()
                .map(|node| node.id)
                .collect::<Vec<_>>()
        };
        let apply_calls = calls(&graph, apply);
        assert!(apply_calls.contains(&session_inspect));
        assert!(apply_calls.contains(&verify));
        assert!(!apply_calls.contains(&decoy_inspect));
        assert_eq!(calls(&graph, inspect_dotted), vec![decoy_inspect]);

        let ordered_calls = calls(&graph, ordered);
        assert!(ordered_calls.contains(&session_inspect));
        assert!(ordered_calls.contains(&decoy_inspect));
        assert!(ordered_calls.contains(&verify));

        let invalidated_calls = calls(&graph, invalidated);
        assert!(invalidated_calls.contains(&unknown));
        assert!(!invalidated_calls.contains(&session_inspect));
        assert!(!invalidated_calls.contains(&decoy_inspect));

        let annotated_calls = calls(&graph, annotated_local);
        assert!(annotated_calls.contains(&unknown));
        assert!(annotated_calls.contains(&decoy_inspect));
        assert_eq!(graph.tests_for(session_inspect), vec![provenance_test]);

        builder.update_file(
            &mut graph,
            "service.py",
            &service.replacen("    identity.inspect()\n", "", 1),
        );
        assert!(!calls(&graph, apply).contains(&session_inspect));
        assert!(
            graph.tests_for(session_inspect).is_empty(),
            "incremental refresh must remove the stale constructor receiver edge"
        );
    }

    #[test]
    fn resolves_python_aliased_import_receiver_types() {
        let models = r#"
class SessionIdentity:
    def inspect(self):
        return True

    def verify_selected_operation(self):
        return True

class DecoyIdentity:
    def inspect(self):
        return False
"#;
        let service = r#"
import models as domain
from models import DecoyIdentity as Decoy, SessionIdentity as Session

def apply(identity: Session):
    identity.inspect()
    identity.verify_selected_operation()

def assigned():
    identity = Session()
    identity.inspect()
    identity.verify_selected_operation()

def inspect_decoy(identity: Decoy):
    identity.inspect()

def assigned_decoy():
    identity = Decoy()
    identity.inspect()

def dotted(identity: domain.DecoyIdentity):
    identity.inspect()

"#;
        let tests = r#"
from models import SessionIdentity as Session
from service import apply

def test_provenance():
    apply(Session())
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "models.py", models);
        builder.load_file(&mut graph, "service.py", service);
        builder.load_file(&mut graph, "test_service.py", tests);

        let apply = NodeId::from_path("crate::service::apply");
        let assigned = NodeId::from_path("crate::service::assigned");
        let inspect_decoy = NodeId::from_path("crate::service::inspect_decoy");
        let assigned_decoy = NodeId::from_path("crate::service::assigned_decoy");
        let dotted = NodeId::from_path("crate::service::dotted");
        let session_inspect = NodeId::from_path("crate::models::SessionIdentity::inspect");
        let verify = NodeId::from_path("crate::models::SessionIdentity::verify_selected_operation");
        let decoy_inspect = NodeId::from_path("crate::models::DecoyIdentity::inspect");
        let provenance_test = NodeId::from_path("crate::test_service::test_provenance");

        let calls = |graph: &SemanticGraph, caller| {
            graph
                .neighbors(caller, Some(EdgeKind::Calls))
                .into_iter()
                .map(|node| node.id)
                .collect::<Vec<_>>()
        };
        for caller in [apply, assigned] {
            let caller_calls = calls(&graph, caller);
            assert!(caller_calls.contains(&session_inspect));
            assert!(caller_calls.contains(&verify));
            assert!(!caller_calls.contains(&decoy_inspect));
        }
        assert_eq!(calls(&graph, inspect_decoy), vec![decoy_inspect]);
        assert_eq!(calls(&graph, assigned_decoy), vec![decoy_inspect]);
        assert_eq!(calls(&graph, dotted), vec![decoy_inspect]);
        assert_eq!(graph.tests_for(session_inspect), vec![provenance_test]);

        builder.update_file(
            &mut graph,
            "service.py",
            &service.replacen("    identity.inspect()\n", "", 1),
        );
        assert!(!calls(&graph, apply).contains(&session_inspect));
        assert!(
            graph.tests_for(session_inspect).is_empty(),
            "incremental refresh must remove the stale aliased receiver edge"
        );
    }

    #[test]
    fn argument_routes_prune_provably_unreachable_cli_tests() {
        let binary = r#"
use std::env;

fn main() {
    match env::args().nth(1).as_deref() {
        Some("selected") => route_selected(),
        Some("unrelated") => route_unrelated(),
        _ => {}
    }
}

fn route_selected() {
    shared_helper();
}

fn route_unrelated() {}

fn shared_helper() {}
"#;
        let cli_tests = r#"
use std::process::Command;

fn run_cli(route: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_demo"))
        .arg(route)
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn test_selected() {
    assert!(!run_cli("selected").is_empty());
}

#[test]
fn test_unrelated() {
    run_cli("unrelated");
}
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/main.rs", binary);
        builder.load_file(&mut graph, "tests/cli.rs", cli_tests);

        let main = NodeId::from_path("crate::main::main");
        let shared_helper = NodeId::from_path("crate::main::shared_helper");
        let run_cli = NodeId::from_path("crate::tests::cli::run_cli");
        let test_selected = NodeId::from_path("crate::tests::cli::test_selected");
        let test_unrelated = NodeId::from_path("crate::tests::cli::test_unrelated");

        // Resolution records the provable route metadata.
        let main_node = graph.get(main).unwrap();
        assert_eq!(
            main_node.attr(&aether_graph::route_guard_key(
                "crate::main::route_selected"
            )),
            Some("selected"),
            "the dispatch arm guards its callee"
        );
        assert_eq!(
            main_node.attr(&aether_graph::route_guard_key(
                "crate::main::route_unrelated"
            )),
            Some("unrelated"),
        );
        assert_eq!(
            graph
                .get(test_selected)
                .unwrap()
                .attr(&aether_graph::entry_route_key("crate::main::main")),
            Some("selected"),
            "the literal caller of the launch helper records its route"
        );
        assert!(
            graph
                .get(run_cli)
                .unwrap()
                .attr(&aether_graph::entry_route_params_key("crate::main::main"))
                .is_some(),
            "the parameterized helper is marked route-carried"
        );

        // The measured consequence: a change reachable only through the
        // "selected" dispatch arm selects exactly its own launcher test.
        let selected_tests = graph.tests_for(shared_helper);
        assert!(selected_tests.contains(&test_selected));
        assert!(
            !selected_tests.contains(&test_unrelated),
            "a launcher with a provably different argv route must be pruned"
        );

        // Recall stays intact for main itself: both tests execute it.
        let entry_tests = graph.tests_for(main);
        assert!(entry_tests.contains(&test_selected));
        assert!(entry_tests.contains(&test_unrelated));

        // Removing the launch route re-derives the metadata: with a
        // non-literal caller the substitution becomes unprovable and every
        // launcher stays selected (fail open).
        let unproved_tests = cli_tests.replace(
            "run_cli(\"unrelated\")",
            "run_cli(std::env::var(\"ROUTE\").unwrap().as_str())",
        );
        builder.update_file(&mut graph, "tests/cli.rs", &unproved_tests);
        assert!(
            graph
                .get(test_selected)
                .unwrap()
                .attr(&aether_graph::entry_route_key("crate::main::main"))
                .is_none(),
            "one unprovable caller withdraws every synthesized route"
        );
        let fail_open = graph.tests_for(shared_helper);
        assert!(fail_open.contains(&test_selected));
        assert!(
            fail_open.contains(&test_unrelated),
            "unprovable routes must not prune anything"
        );
    }

    #[test]
    fn resolves_cargo_binary_subprocess_entrypoints_without_guessing() {
        let binary = r#"
fn main() {
    dispatch();
}

fn dispatch() {}
"#;
        let cli_tests = r#"
use std::process::Command;

fn run_binary() {
    Command::new(env!("CARGO_BIN_EXE_demo")).output().unwrap();
}

fn run_qualified_binary() {
    std::process::Command::new(env!("CARGO_BIN_EXE_demo"))
        .output()
        .unwrap();
}

fn run_git() {
    Command::new("git").output().unwrap();
}

fn run_other_env() {
    Command::new(env!("HOME")).output().unwrap();
}

#[test]
fn cli_route() {
    run_binary();
}

#[test]
fn qualified_cli_route() {
    run_qualified_binary();
}

#[test]
fn git_helper() {
    run_git();
    run_other_env();
}
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "src/main.rs", binary);
        builder.load_file(&mut graph, "tests/cli.rs", cli_tests);

        let main = NodeId::from_path("crate::main::main");
        let run_binary = NodeId::from_path("crate::tests::cli::run_binary");
        let run_qualified_binary = NodeId::from_path("crate::tests::cli::run_qualified_binary");
        let cli_route = NodeId::from_path("crate::tests::cli::cli_route");
        let qualified_cli_route = NodeId::from_path("crate::tests::cli::qualified_cli_route");
        let git_helper = NodeId::from_path("crate::tests::cli::git_helper");

        for launcher in [run_binary, run_qualified_binary] {
            assert!(
                graph
                    .neighbors(launcher, Some(EdgeKind::Calls))
                    .iter()
                    .any(|node| node.id == main),
                "exact Cargo binary launch must call the unique Rust entrypoint"
            );
        }
        let entrypoint_tests = graph.tests_for(main);
        assert_eq!(
            entrypoint_tests.len(),
            2,
            "only tests that execute the Cargo binary should cover main"
        );
        assert!(entrypoint_tests.contains(&cli_route));
        assert!(entrypoint_tests.contains(&qualified_cli_route));
        assert!(!entrypoint_tests.contains(&git_helper));

        builder.update_file(
            &mut graph,
            "tests/cli.rs",
            &cli_tests.replace("CARGO_BIN_EXE_demo", "NOT_A_CARGO_BINARY"),
        );
        assert!(
            graph.tests_for(main).is_empty(),
            "incremental refresh must remove stale subprocess entrypoint edges"
        );

        let mut ambiguous_graph = SemanticGraph::new();
        let mut ambiguous_builder = GraphBuilder::new();
        ambiguous_builder.load_file(&mut ambiguous_graph, "src/main.rs", "fn main() {}\n");
        ambiguous_builder.load_file(&mut ambiguous_graph, "src/bin/other.rs", "fn main() {}\n");
        ambiguous_builder.load_file(&mut ambiguous_graph, "tests/cli.rs", cli_tests);
        for entrypoint in [
            NodeId::from_path("crate::main::main"),
            NodeId::from_path("crate::bin::other::main"),
        ] {
            assert!(
                ambiguous_graph.tests_for(entrypoint).is_empty(),
                "multiple Rust entrypoints must remain unresolved"
            );
        }
        ambiguous_builder.update_file(
            &mut ambiguous_graph,
            "tests/cli.rs",
            &cli_tests.replace("CARGO_BIN_EXE_demo", "CARGO_BIN_EXE_other"),
        );
        assert!(ambiguous_graph
            .tests_for(NodeId::from_path("crate::main::main"))
            .is_empty());
        assert_eq!(
            ambiguous_graph
                .tests_for(NodeId::from_path("crate::bin::other::main"))
                .len(),
            2,
            "an exact src/bin Cargo target must disambiguate multiple entrypoints"
        );

        let mut directory_graph = SemanticGraph::new();
        let mut directory_builder = GraphBuilder::new();
        directory_builder.load_file(
            &mut directory_graph,
            "src/bin/nested/main.rs",
            "fn main() {}\n",
        );
        directory_builder.load_file(
            &mut directory_graph,
            "tests/cli.rs",
            &cli_tests.replace("CARGO_BIN_EXE_demo", "CARGO_BIN_EXE_nested"),
        );
        assert_eq!(
            directory_graph
                .tests_for(NodeId::from_path("crate::bin::nested::main::main"))
                .len(),
            2
        );

        let mut custom_graph = SemanticGraph::new();
        let mut custom_builder = GraphBuilder::new();
        custom_builder.load_file(&mut custom_graph, "tools/cli.rs", "fn main() {}\n");
        custom_builder.load_file(&mut custom_graph, "tests/cli.rs", cli_tests);
        assert!(
            custom_graph
                .tests_for(NodeId::from_path("crate::tools::cli::main"))
                .is_empty(),
            "custom binary paths require manifest metadata and must remain unresolved"
        );
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

        assert_eq!(
            graph
                .find_by_path("crate::lib::run")
                .unwrap()
                .attr("return_type"),
            Some("i64")
        );
        builder.update_file(&mut graph, "src/lib.rs", "fn run() { let _ = 2; }\n");

        let run = graph.find_by_path("crate::lib::run").unwrap();
        assert_eq!(run.source, "fn run() { let _ = 2; }");
        assert_eq!(run.attr("summary"), Some("durable summary"));
        assert_eq!(run.attr("return_type"), None);
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
