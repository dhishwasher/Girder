//! Serde structs for the Bit Code Plan Format v1 JSON schema. Parsing is
//! deliberately permissive about which check `kind`s exist (all documented
//! kinds parse from Phase 1 onward) even though earlier phases only execute
//! a subset — this avoids re-touching the schema every time a new check
//! kind is implemented.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

#[derive(Debug, Clone)]
pub(crate) struct Plan {
    pub(crate) plan_version: u32,
    pub(crate) plan_id: String,
    pub(crate) intent: String,
    pub(crate) author: Option<String>,
    pub(crate) base_commit: String,
    pub(crate) on_failure: OnFailure,
    pub(crate) steps: Vec<Step>,
}

#[derive(Deserialize)]
struct RawPlan {
    plan_version: u32,
    plan_id: String,
    intent: String,
    #[serde(default)]
    author: Option<String>,
    base_commit: String,
    #[serde(default)]
    on_failure: OnFailure,
    steps: Vec<Step>,
}

impl<'de> Deserialize<'de> for Plan {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| D::Error::custom("plan must be a JSON object"))?;
        let plan_id = object
            .get("plan_id")
            .and_then(Value::as_str)
            .unwrap_or("<missing plan_id>")
            .to_string();
        let step_id = object
            .get("steps")
            .and_then(Value::as_array)
            .and_then(|steps| steps.first())
            .and_then(|step| step.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("<missing step id>")
            .to_string();
        const KNOWN_FIELDS: &[&str] = &[
            "plan_version",
            "plan_id",
            "intent",
            "author",
            "base_commit",
            "on_failure",
            "steps",
        ];
        if let Some(field) = object
            .keys()
            .find(|field| !KNOWN_FIELDS.contains(&field.as_str()))
        {
            return Err(D::Error::custom(format!(
                "plan {plan_id:?}, step {step_id:?}: unknown plan field {field:?}; expected one of {}",
                KNOWN_FIELDS.join(", ")
            )));
        }
        let raw: RawPlan = serde_json::from_value(value).map_err(|error| {
            D::Error::custom(format!("plan {plan_id:?}, step {step_id:?}: {error}"))
        })?;
        Ok(Self {
            plan_version: raw.plan_version,
            plan_id: raw.plan_id,
            intent: raw.intent,
            author: raw.author,
            base_commit: raw.base_commit,
            on_failure: raw.on_failure,
            steps: raw.steps,
        })
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OnFailure {
    #[default]
    RollbackPlan,
    RollbackStep,
    Stop,
}

#[derive(Debug, Clone)]
pub(crate) struct Step {
    pub(crate) id: String,
    pub(crate) description: String,
    pub(crate) edits: Vec<Edit>,
    pub(crate) checks: Vec<Check>,
}

#[derive(Deserialize)]
struct RawStep {
    id: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    edits: Vec<Value>,
    #[serde(default)]
    checks: Vec<Value>,
}

impl<'de> Deserialize<'de> for Step {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| D::Error::custom("step must be a JSON object"))?;
        let step_id = object
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("<missing step id>")
            .to_string();
        const KNOWN_FIELDS: &[&str] = &["id", "description", "edits", "checks"];
        if let Some(field) = object
            .keys()
            .find(|field| !KNOWN_FIELDS.contains(&field.as_str()))
        {
            return Err(D::Error::custom(format!(
                "step {step_id:?}: unknown step field {field:?}; expected one of {}",
                KNOWN_FIELDS.join(", ")
            )));
        }
        let raw: RawStep = serde_json::from_value(value)
            .map_err(|error| D::Error::custom(format!("step {step_id:?}: {error}")))?;
        let edits = raw
            .edits
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                let path = value
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("<missing path>")
                    .to_string();
                serde_json::from_value(value).map_err(|error| {
                    D::Error::custom(format!(
                        "step {:?}: edits[{index}] path {path:?}: {error}",
                        raw.id
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let checks = raw
            .checks
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                let kind = value
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("<missing kind>")
                    .to_string();
                serde_json::from_value(value).map_err(|error| {
                    D::Error::custom(format!(
                        "step {:?}: checks[{index}] kind {kind:?}: {error}",
                        raw.id
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            id: raw.id,
            description: raw.description,
            edits,
            checks,
        })
    }
}

fn default_occurrences() -> u32 {
    1
}

#[derive(Debug, Clone)]
pub(crate) enum Edit {
    Substitute {
        path: String,
        match_text: String,
        replace: String,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubstituteEdit {
    path: String,
    #[serde(rename = "match")]
    match_text: String,
    replace: String,
    #[serde(default = "default_occurrences")]
    occurrences: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateEdit {
    path: String,
    create: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteEdit {
    path: String,
    delete: bool,
}

impl<'de> Deserialize<'de> for Edit {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| D::Error::custom("edit must be a JSON object"))?;
        const KNOWN_FIELDS: &[&str] = &[
            "path",
            "match",
            "replace",
            "occurrences",
            "create",
            "delete",
        ];
        if let Some(field) = object
            .keys()
            .find(|field| !KNOWN_FIELDS.contains(&field.as_str()))
        {
            return Err(D::Error::custom(format!(
                "unknown edit field {field:?}; expected one of {}",
                KNOWN_FIELDS.join(", ")
            )));
        }

        let discriminators: Vec<&str> = ["match", "create", "delete"]
            .into_iter()
            .filter(|field| object.contains_key(*field))
            .collect();
        if discriminators.len() != 1 {
            return Err(D::Error::custom(format!(
                "edit must contain exactly one discriminator field (match, create, or delete); found {discriminators:?}"
            )));
        }

        match discriminators[0] {
            "match" => {
                let raw: SubstituteEdit = serde_json::from_value(value)
                    .map_err(|error| D::Error::custom(error.to_string()))?;
                Ok(Self::Substitute {
                    path: raw.path,
                    match_text: raw.match_text,
                    replace: raw.replace,
                    occurrences: raw.occurrences,
                })
            }
            "create" => {
                let raw: CreateEdit = serde_json::from_value(value)
                    .map_err(|error| D::Error::custom(error.to_string()))?;
                Ok(Self::Create {
                    path: raw.path,
                    create: raw.create,
                })
            }
            "delete" => {
                let raw: DeleteEdit = serde_json::from_value(value)
                    .map_err(|error| D::Error::custom(error.to_string()))?;
                Ok(Self::Delete {
                    path: raw.path,
                    delete: raw.delete,
                })
            }
            _ => unreachable!("the discriminator list is fixed above"),
        }
    }
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
#[serde(tag = "kind", deny_unknown_fields)]
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

impl Plan {
    pub(crate) fn validate(&self) -> Result<(), String> {
        for step in &self.steps {
            for (index, check) in step.checks.iter().enumerate() {
                let invalid = match check {
                    Check::GraphCallersOf { expect, mode, .. }
                    | Check::GraphCalleesOf { expect, mode, .. }
                    | Check::GraphTestsFor { expect, mode, .. } => {
                        expect.is_empty() && matches!(mode, Mode::Superset | Mode::Absent)
                    }
                    _ => false,
                };
                if invalid {
                    return Err(format!(
                        "step {:?}: checks[{index}] {} uses mode {:?} with an empty expect set; this check would verify nothing",
                        step.id,
                        check.kind(),
                        match check {
                            Check::GraphCallersOf { mode, .. }
                            | Check::GraphCalleesOf { mode, .. }
                            | Check::GraphTestsFor { mode, .. } => mode,
                            _ => unreachable!("invalid is true only for set checks"),
                        }
                    ));
                }
            }
        }
        Ok(())
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

    #[test]
    fn misspelled_check_field_is_rejected_with_step_context() {
        let json = r#"{
          "plan_version": 1,
          "plan_id": "p",
          "intent": "reject typo",
          "base_commit": "abc",
          "steps": [{
            "id": "verify-callers",
            "checks": [{
              "kind": "graph.callers_of",
              "node": "crate::target",
              "expct": ["crate::caller"]
            }]
          }]
        }"#;
        let error = serde_json::from_str::<Plan>(json).unwrap_err().to_string();
        assert!(error.contains("verify-callers"), "{error}");
        assert!(error.contains("checks[0]"), "{error}");
        assert!(error.contains("expct"), "{error}");
    }

    #[test]
    fn empty_superset_and_absent_checks_are_rejected_but_exact_is_meaningful() {
        for mode in ["superset", "absent"] {
            let json = format!(
                r#"{{
                  "plan_version": 1,
                  "plan_id": "p",
                  "intent": "reject vacuity",
                  "base_commit": "abc",
                  "steps": [{{
                    "id": "no-vacuous-pass",
                    "checks": [{{
                      "kind": "graph.callers_of",
                      "node": "crate::target",
                      "expect": [],
                      "mode": "{mode}"
                    }}]
                  }}]
                }}"#
            );
            let plan: Plan = serde_json::from_str(&json).unwrap();
            let error = plan.validate().unwrap_err();
            assert!(error.contains("no-vacuous-pass"), "{error}");
            assert!(error.contains("empty expect"), "{error}");
        }

        let exact: Plan = serde_json::from_str(
            r#"{
              "plan_version": 1,
              "plan_id": "p",
              "intent": "empty is meaningful",
              "base_commit": "abc",
              "steps": [{
                "id": "exact-empty",
                "checks": [{
                  "kind": "graph.callers_of",
                  "node": "crate::target",
                  "expect": [],
                  "mode": "exact"
                }]
              }]
            }"#,
        )
        .unwrap();
        assert!(exact.validate().is_ok());
    }

    #[test]
    fn malformed_edit_error_names_the_step_path_and_field() {
        let json = r#"{
          "plan_version": 1,
          "plan_id": "p",
          "intent": "reject malformed edit",
          "base_commit": "abc",
          "steps": [{
            "id": "rewrite-session",
            "edits": [{
              "path": "src/session.rs",
              "match": "old",
              "replce": "new"
            }]
          }]
        }"#;
        let error = serde_json::from_str::<Plan>(json).unwrap_err().to_string();
        assert!(error.contains("rewrite-session"), "{error}");
        assert!(error.contains("src/session.rs"), "{error}");
        assert!(error.contains("replce"), "{error}");
    }

    #[test]
    fn unknown_step_field_error_retains_the_step_id() {
        let json = r#"{
          "plan_version": 1,
          "plan_id": "p",
          "intent": "reject malformed step",
          "base_commit": "abc",
          "steps": [{
            "id": "diagnose-malformed",
            "unknown_step_field": true
          }]
        }"#;
        let error = serde_json::from_str::<Plan>(json).unwrap_err().to_string();
        assert!(error.contains("diagnose-malformed"), "{error}");
        assert!(error.contains("unknown_step_field"), "{error}");
    }

    #[test]
    fn unknown_plan_field_error_retains_plan_and_step_context() {
        let json = r#"{
          "plan_version": 1,
          "plan_id": "contextual-plan",
          "intent": "reject malformed plan",
          "base_commit": "abc",
          "steps": [{"id": "diagnose-malformed"}],
          "unexpected_plan_field": true
        }"#;
        let error = serde_json::from_str::<Plan>(json).unwrap_err().to_string();
        assert!(error.contains("contextual-plan"), "{error}");
        assert!(error.contains("diagnose-malformed"), "{error}");
        assert!(error.contains("unexpected_plan_field"), "{error}");
    }
}
