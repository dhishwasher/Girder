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

def test_provenance():
    apply(SessionIdentity())
"#;
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph, "models.py", models);
        builder.load_file(&mut graph, "service.py", service);

        let apply = NodeId::from_path("crate::service::apply");
        let inspect_dotted = NodeId::from_path("crate::service::inspect_dotted");
        let inspect_forward = NodeId::from_path("crate::service::inspect_forward");
        let inspect_default = NodeId::from_path("crate::service::inspect_default");
        let session_inspect = NodeId::from_path("crate::models::SessionIdentity::inspect");
        let verify = NodeId::from_path("crate::models::SessionIdentity::verify_selected_operation");
        let decoy_inspect = NodeId::from_path("crate::models::DecoyIdentity::inspect");
        let provenance_test = NodeId::from_path("crate::service::test_provenance");

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
