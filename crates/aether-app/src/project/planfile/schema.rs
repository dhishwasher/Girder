//! Serde structs for the Bit Code Plan Format v1 JSON schema. Parsing is
//! deliberately permissive about which check `kind`s exist (all documented
//! kinds parse from Phase 1 onward) even though earlier phases only execute
//! a subset — this avoids re-touching the schema every time a new check
//! kind is implemented.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Plan {
    pub(crate) plan_version: u32,
    pub(crate) plan_id: String,
    pub(crate) intent: String,
    #[serde(default)]
    pub(crate) author: Option<String>,
    pub(crate) base_commit: String,
    #[serde(default)]
    pub(crate) on_failure: OnFailure,
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OnFailure {
    #[default]
    RollbackPlan,
    RollbackStep,
    Stop,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Step {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) edits: Vec<Edit>,
    #[serde(default)]
    pub(crate) checks: Vec<Check>,
}

fn default_occurrences() -> u32 {
    1
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum Edit {
    Substitute {
        path: String,
        #[serde(rename = "match")]
        match_text: String,
        replace: String,
        #[serde(default = "default_occurrences")]
        occurrences: u32,
    },
    Create {
        path: String,
        create: String,
    },
    Delete {
        path: String,
        delete: bool,
    },
}

impl Edit {
    pub(crate) fn path(&self) -> &str {
        match self {
            Edit::Substitute { path, .. } => path,
            Edit::Create { path, .. } => path,
            Edit::Delete { path, .. } => path,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    #[default]
    Exact,
    Superset,
    Subset,
    Absent,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExpectResult {
    #[default]
    Pass,
    Fail,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TestExpect {
    #[default]
    AllPass,
    AllFail,
}

fn default_expect_exit() -> i32 {
    0
}

fn default_timeout_secs() -> u64 {
    120
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind")]
pub(crate) enum Check {
    #[serde(rename = "graph.callers_of")]
    GraphCallersOf {
        node: String,
        #[serde(default)]
        expect: Vec<String>,
        #[serde(default)]
        mode: Mode,
    },
    #[serde(rename = "graph.callees_of")]
    GraphCalleesOf {
        node: String,
        #[serde(default)]
        expect: Vec<String>,
        #[serde(default)]
        mode: Mode,
    },
    #[serde(rename = "graph.tests_for")]
    GraphTestsFor {
        node: String,
        #[serde(default)]
        expect: Vec<String>,
        #[serde(default)]
        mode: Mode,
    },
    #[serde(rename = "graph.node_exists")]
    GraphNodeExists { node: String },
    #[serde(rename = "graph.node_absent")]
    GraphNodeAbsent { node: String },
    #[serde(rename = "graph.no_new_edges_into")]
    GraphNoNewEdgesInto { node: String },
    #[serde(rename = "graph.edge_delta")]
    GraphEdgeDelta {
        #[serde(default)]
        max_added: usize,
        #[serde(default)]
        max_removed: usize,
    },
    #[serde(rename = "graph.unresolved")]
    GraphUnresolved {
        node: String,
        from: String,
        #[serde(default)]
        expect_result: ExpectResult,
    },
    #[serde(rename = "tests.impacted")]
    TestsImpacted {
        #[serde(default)]
        expect: TestExpect,
    },
    #[serde(rename = "tests.named")]
    TestsNamed {
        tests: Vec<String>,
        #[serde(default)]
        expect: TestExpect,
    },
    #[serde(rename = "tests.full")]
    TestsFull {
        #[serde(default)]
        expect: TestExpect,
    },
    #[serde(rename = "command")]
    Command {
        run: String,
        #[serde(default = "default_expect_exit")]
        expect_exit: i32,
        #[serde(default = "default_timeout_secs")]
        timeout_secs: u64,
    },
    // EXTENSION POINT: `oracle`/`benchmark` parse today (so a plan
    // referencing them is not a hard parse error and `plan explain` can
    // still summarize them) but are not executed — `executor::run_check`
    // reports them as not-yet-implemented. Wiring them up means shelling
    // out to tools/core_trustworthiness_oracle.py and
    // tools/core_representative_benchmark.py and interpreting each
    // script's pass/fail verdict; out of scope for this executor's v1.
    #[serde(rename = "oracle")]
    Oracle {
        #[serde(default)]
        #[allow(dead_code)]
        min_precision: f64,
        #[serde(default)]
        #[allow(dead_code)]
        min_recall: f64,
        #[serde(default)]
        #[allow(dead_code)]
        languages: Vec<String>,
    },
    #[serde(rename = "benchmark")]
    Benchmark {
        #[allow(dead_code)]
        policy: String,
    },
}

impl Check {
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Check::GraphCallersOf { .. } => "graph.callers_of",
            Check::GraphCalleesOf { .. } => "graph.callees_of",
            Check::GraphTestsFor { .. } => "graph.tests_for",
            Check::GraphNodeExists { .. } => "graph.node_exists",
            Check::GraphNodeAbsent { .. } => "graph.node_absent",
            Check::GraphNoNewEdgesInto { .. } => "graph.no_new_edges_into",
            Check::GraphEdgeDelta { .. } => "graph.edge_delta",
            Check::GraphUnresolved { .. } => "graph.unresolved",
            Check::TestsImpacted { .. } => "tests.impacted",
            Check::TestsNamed { .. } => "tests.named",
            Check::TestsFull { .. } => "tests.full",
            Check::Command { .. } => "command",
            Check::Oracle { .. } => "oracle",
            Check::Benchmark { .. } => "benchmark",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_worked_example_from_the_spec() {
        let json = r#"
        {
          "plan_version": 1,
          "plan_id": "tighten-dotted-receiver-guard",
          "intent": "Stop trusting capitalized attribute tails as type evidence.",
          "base_commit": "b8f3a24",
          "on_failure": "rollback_plan",
          "steps": [
            {
              "id": "prove-the-gap",
              "description": "Add a fixture that currently produces a false edge.",
              "edits": [
                { "path": "crates/aether-builder/src/lib.rs",
                  "match": "def dotted_suffix(x):",
                  "replace": "def capitalized_suffix(x):\n    pass\n\ndef dotted_suffix(x):",
                  "occurrences": 1 }
              ],
              "checks": [
                { "kind": "command", "run": "cargo build -p aether-builder", "expect_exit": 0 },
                { "kind": "graph.unresolved",
                  "from": "crate::service::capitalized_suffix",
                  "node": "crate::models::SessionIdentity::inspect",
                  "expect_result": "fail" }
              ]
            },
            {
              "id": "close-it",
              "description": "Remove the capitalization heuristic.",
              "edits": [
                { "path": "crates/aether-builder/src/mapper.rs",
                  "match": "import_bindings.contains(root) || tail.starts_with(char::is_uppercase)",
                  "replace": "import_bindings.contains(root)" }
              ],
              "checks": [
                { "kind": "command", "run": "cargo build -p aether-builder", "expect_exit": 0 },
                { "kind": "graph.unresolved",
                  "from": "crate::service::capitalized_suffix",
                  "node": "crate::models::SessionIdentity::inspect" },
                { "kind": "tests.impacted", "expect": "all_pass" },
                { "kind": "oracle", "min_precision": 1.0, "min_recall": 1.0 }
              ]
            }
          ]
        }
        "#;

        let plan: Plan = serde_json::from_str(json).unwrap();
        assert_eq!(plan.plan_id, "tighten-dotted-receiver-guard");
        assert_eq!(plan.on_failure, OnFailure::RollbackPlan);
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].edits.len(), 1);
        match &plan.steps[0].edits[0] {
            Edit::Substitute { occurrences, .. } => assert_eq!(*occurrences, 1),
            other => panic!("expected a substitute edit, got {other:?}"),
        }
        assert_eq!(plan.steps[0].checks.len(), 2);
        assert_eq!(plan.steps[0].checks[0].kind(), "command");
        match &plan.steps[0].checks[1] {
            Check::GraphUnresolved { expect_result, .. } => {
                assert_eq!(*expect_result, ExpectResult::Fail)
            }
            other => panic!("expected graph.unresolved, got {other:?}"),
        }
        match &plan.steps[1].checks[1] {
            Check::GraphUnresolved { expect_result, .. } => {
                assert_eq!(*expect_result, ExpectResult::Pass)
            }
            other => panic!("expected graph.unresolved, got {other:?}"),
        }
    }

    #[test]
    fn on_failure_defaults_to_rollback_plan_when_absent() {
        let json = r#"
        {"plan_version":1,"plan_id":"p","intent":"i","base_commit":"abc","steps":[]}
        "#;
        let plan: Plan = serde_json::from_str(json).unwrap();
        assert_eq!(plan.on_failure, OnFailure::RollbackPlan);
    }

    #[test]
    fn create_and_delete_edits_parse() {
        let json = r#"
        {"path": "new.rs", "create": "fn x() {}\n"}
        "#;
        match serde_json::from_str::<Edit>(json).unwrap() {
            Edit::Create { path, create } => {
                assert_eq!(path, "new.rs");
                assert_eq!(create, "fn x() {}\n");
            }
            other => panic!("expected create edit, got {other:?}"),
        }

        let json = r#"{"path": "old.rs", "delete": true}"#;
        match serde_json::from_str::<Edit>(json).unwrap() {
            Edit::Delete { path, delete } => {
                assert_eq!(path, "old.rs");
                assert!(delete);
            }
            other => panic!("expected delete edit, got {other:?}"),
        }
    }
}
