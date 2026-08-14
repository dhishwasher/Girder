//! `bitcode do <dir> "<intent...>" [--dry] [--max-repairs N]` — connects the
//! plan executor to a model. Selects the graph nodes most relevant to the
//! intent via the existing concept-search ranking, asks the router for a
//! grammar-constrained plan step addressing only those nodes, executes it
//! through the *existing* plan executor (`planfile::run_for_authoring`), and
//! on failure repairs with the check output up to `--max-repairs` times
//! before escalating to the next provider in the router's chain.
//!
//! This module never applies an edit, runs a check, commits, or rolls back
//! itself — all of that stays inside `planfile`, unmodified.

use crate::project::config::ProjectConfig;
use crate::project::git::git_head_commit;
use crate::project::planfile::{run_for_authoring, AuthoringRunResult};
use crate::project::source::build_from_dir_with_config;
use aether_ai::{Prompt, TaskClass};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// How many concept-search hits to show the model. Matches the "top few
/// nodes" the design settled on: enough to give the model room to pick the
/// right target, few enough that the printed selection stays readable.
const TOP_K: usize = 5;
const DEFAULT_MAX_REPAIRS: usize = 2;

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

const USAGE: &str = "usage: bitcode do <dir> \"<intent...>\" [--dry] [--max-repairs N]";

pub async fn do_intent(args: &[String]) -> std::io::Result<()> {
    let Some(root_arg) = args.first() else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let root = PathBuf::from(root_arg);
    let dry = args.iter().any(|arg| arg == "--dry");
    let max_repairs_value = args.windows(2).find(|window| window[0] == "--max-repairs");
    if args.iter().any(|arg| arg == "--max-repairs") && max_repairs_value.is_none() {
        return Err(invalid_input("--max-repairs requires a value"));
    }
    let max_repairs = match max_repairs_value {
        Some(window) => window[1]
            .parse::<usize>()
            .map_err(|_| invalid_input("--max-repairs requires a non-negative integer"))?,
        None => DEFAULT_MAX_REPAIRS,
    };
    let intent = collect_intent(args);
    if intent.trim().is_empty() {
        eprintln!("{USAGE}");
        return Ok(());
    }

    println!("Loading {} ...", root.display());
    let config = ProjectConfig::load(&root)?;
    let (graph, _builder, files) = build_from_dir_with_config(&root, &config)?;
    println!("  {files} file(s), {} nodes", graph.node_count());

    let hits = graph.semantic_search(&intent, TOP_K);
    if hits.is_empty() {
        println!("\nNo nodes matched \"{intent}\"; nothing to author.");
        return Ok(());
    }
    println!("\nSelecting nodes for \"{intent}\":");
    let mut nodes = Vec::with_capacity(hits.len());
    for (id, score) in &hits {
        if let Some(node) = graph.get(*id) {
            println!("  {score:.2}  {}", node.path);
            nodes.push(node.clone());
        }
    }
    let node_paths: Vec<String> = nodes.iter().map(|node| node.path.clone()).collect();
    let node_context: Vec<Value> = nodes
        .iter()
        .map(|node| json!({"path": node.path, "language": node.language, "source": node.source}))
        .collect();

    let base_commit = git_head_commit(&root)?;
    let router = aether_ai::default_router();
    let candidates = router.candidates(TaskClass::Authoring);
    if candidates.is_empty() {
        return Err(std::io::Error::other(
            "no provider is configured for authoring",
        ));
    }
    let total_providers = candidates.len();

    let schema = step_schema(&node_paths);
    let plan_id = generate_plan_id();

    let mut calls: Vec<Value> = Vec::new();
    let mut diagnostic = String::new();
    let mut succeeded: Option<(String, String, usize, AuthoringRunResult)> = None;

    'providers: for provider in &candidates {
        for attempt in 0..=max_repairs {
            println!(
                "\n[{}] attempt {}/{} — nodes: {}",
                provider.name(),
                attempt + 1,
                max_repairs + 1,
                node_paths.join(", ")
            );
            let prompt = build_prompt(
                &intent,
                &node_context,
                &schema,
                (attempt > 0).then_some(diagnostic.as_str()),
            );
            let completion = match provider.complete(prompt).await {
                Ok(completion) => completion,
                Err(error) => {
                    diagnostic = format!("{} declined: {error}", provider.name());
                    println!("  {diagnostic}");
                    // A decline is permanent for this provider (no key, no
                    // host, or an unconditional refusal) — retrying the same
                    // prompt against it won't help, so move on immediately
                    // instead of burning the repair budget.
                    break;
                }
            };
            calls.push(json!({
                "provider": provider.name(),
                "model": completion.model,
                "tokens": completion.tokens
            }));

            let step_value: Value = match serde_json::from_str(&completion.text) {
                Ok(value) => value,
                Err(error) => {
                    diagnostic = format!("model response was not valid JSON: {error}");
                    println!("  {diagnostic}");
                    continue;
                }
            };
            let plan_value = match wrap_step_into_plan(
                &step_value,
                &node_paths,
                &base_commit,
                &intent,
                &plan_id,
            ) {
                Ok(value) => value,
                Err(error) => {
                    diagnostic = error;
                    println!("  {diagnostic}");
                    continue;
                }
            };

            let plan_path = write_temp_json(&plan_value, "plan")?;
            let receipt_path =
                write_temp_json(&json!({"schema_version": 1, "calls": calls}), "receipt")?;
            let run_outcome = run_for_authoring(&root, &plan_path, dry, Some(&receipt_path));
            let _ = std::fs::remove_file(&plan_path);
            let _ = std::fs::remove_file(&receipt_path);
            let result = run_outcome?;

            if result.passed {
                succeeded = Some((
                    provider.name().to_string(),
                    completion.model.clone(),
                    attempt,
                    result,
                ));
                break 'providers;
            }
            diagnostic = result.diagnostic;
            println!("  attempt failed:\n{}", indent(&diagnostic));
        }
    }

    match succeeded {
        Some((provider_name, model, repairs, result)) => {
            println!(
                "\nplan authored by {provider_name} ({model}) after {repairs} repair attempt(s)"
            );
            if let Some(path) = &result.report_path {
                println!("report: {}", path.display());
            }
            if let Some(rendered) = &result.report_json {
                println!("\nreport (dry run, not written to disk):\n{rendered}");
            }
            Ok(())
        }
        None => Err(std::io::Error::other(format!(
            "no provider produced a working plan for \"{intent}\" after trying \
             {total_providers} provider(s); last diagnostic: {diagnostic}"
        ))),
    }
}

fn collect_intent(args: &[String]) -> String {
    let mut words = Vec::new();
    let mut skip_next = false;
    for arg in args.iter().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--dry" {
            continue;
        }
        if arg == "--max-repairs" {
            skip_next = true;
            continue;
        }
        words.push(arg.as_str());
    }
    words.join(" ")
}

fn generate_plan_id() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("do-{millis}-{}", std::process::id())
}

fn invalid_input(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_temp_json(value: &Value, label: &str) -> std::io::Result<PathBuf> {
    let unique = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "bitcode-do-{label}-{}-{unique}.json",
        std::process::id()
    ));
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    std::fs::write(&path, bytes)?;
    Ok(path)
}

fn build_prompt(
    intent: &str,
    node_context: &[Value],
    schema: &Value,
    repair_diagnostic: Option<&str>,
) -> Prompt {
    let system = "You are Bit Code's plan author. You may only edit the graph nodes shown to \
                  you; respond with JSON only, matching the supplied schema exactly."
        .to_string();
    let mut user = format!(
        "Intent: {intent}\n\
         Nodes available (path, language, current source) — edit only these:\n{}\n\n\
         Author one plan step. For each edit, set \"operation\" to exactly one of \
         replace_node, rename_node, delete_node, or insert_into_module; only the field \
         with that same name is used, so leave the other three empty/false. Prefer \
         graph.node_exists, graph.node_absent, graph.callers_of, or graph.callees_of \
         checks over \"command\" — a command check can trigger a full project build. A \
         tests.impacted check runs automatically after your plan; you do not need to add \
         one yourself. Return JSON only, no prose.",
        serde_json::to_string(&Value::Array(node_context.to_vec())).unwrap_or_default()
    );
    if let Some(diagnostic) = repair_diagnostic {
        user.push_str(&format!(
            "\n\nThe previous attempt failed:\n{diagnostic}\nReturn a corrected step object only."
        ));
    }
    Prompt::new(TaskClass::Authoring, system, user).with_response_schema(schema.clone())
}

/// A flat, discriminant-selected shape rather than a `oneOf` union:
/// `tools/plan_executor_oracle.py`'s `authoring_plan_json_schema` avoids
/// unions entirely (it builds a schema from one concrete example per fixed
/// task), which is evidence that a JSON-Schema union is not a shape to lean
/// on for grammar-constrained local decoding. Since `bitcode do` doesn't
/// know the operation ahead of time the way that fixed corpus does, every
/// operation field is present and required; only the one named by
/// `operation` is read back out in `convert_edit`/`convert_check`.
fn step_schema(node_paths: &[String]) -> Value {
    let max_edits = node_paths.len().clamp(1, 3);
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "description", "edits", "checks"],
        "properties": {
            "id": {"type": "string"},
            "description": {"type": "string"},
            "edits": {
                "type": "array",
                "minItems": 1,
                "maxItems": max_edits,
                "items": edit_schema(node_paths)
            },
            "checks": {
                "type": "array",
                "minItems": 0,
                "maxItems": 3,
                "items": check_schema(node_paths)
            }
        }
    })
}

fn edit_schema(node_paths: &[String]) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "node", "operation", "replace_node", "rename_node", "delete_node",
            "insert_into_module"
        ],
        "properties": {
            "node": {"type": "string", "enum": node_paths},
            "operation": {
                "type": "string",
                "enum": ["replace_node", "rename_node", "delete_node", "insert_into_module"]
            },
            "replace_node": {"type": "string"},
            "rename_node": {"type": "string"},
            "delete_node": {"type": "boolean"},
            "insert_into_module": {"type": "string"}
        }
    })
}

/// Deliberately a curated subset of the check kinds `plan run` supports —
/// enough to sanity-check a graph edit (does the node still exist / was it
/// removed / who calls it) plus an escape hatch to a command, without
/// bloating every check item with the full kind menu's field union. Every
/// authored plan additionally gets a harness-injected `tests.impacted`
/// check the model cannot see, remove, or replace (see `wrap_step_into_plan`).
fn check_schema(node_paths: &[String]) -> Value {
    let mut node_enum = node_paths.to_vec();
    node_enum.push(String::new());
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["kind", "node", "expect", "run", "expect_exit"],
        "properties": {
            "kind": {
                "type": "string",
                "enum": [
                    "graph.node_exists",
                    "graph.node_absent",
                    "graph.callers_of",
                    "graph.callees_of",
                    "command"
                ]
            },
            "node": {"type": "string", "enum": node_enum},
            "expect": {"type": "array", "items": {"type": "string"}},
            "run": {"type": "string"},
            "expect_exit": {"type": "integer"}
        }
    })
}

/// Turn the model's flat step JSON into a Plan Format v2 plan: harness-owned
/// fields (`plan_version`, `plan_id`, `intent`, `base_commit`, `on_failure`)
/// are injected here rather than asked of the model — `on_failure` is
/// always `rollback_plan` so a failed repair attempt always reverts to
/// `base_commit` cleanly before the next one, and the other four are
/// mechanical values the model has no reason to get wrong.
fn wrap_step_into_plan(
    step: &Value,
    node_paths: &[String],
    base_commit: &str,
    intent: &str,
    plan_id: &str,
) -> Result<Value, String> {
    let id = step
        .get("id")
        .and_then(Value::as_str)
        .ok_or("step.id missing")?;
    let description = step
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let edits_in = step
        .get("edits")
        .and_then(Value::as_array)
        .ok_or("step.edits missing")?;
    if edits_in.is_empty() {
        return Err("step.edits must not be empty".to_string());
    }
    let mut edits_out = Vec::with_capacity(edits_in.len());
    for edit in edits_in {
        edits_out.push(convert_edit(edit, node_paths)?);
    }

    let empty_checks = Vec::new();
    let checks_in = step
        .get("checks")
        .and_then(Value::as_array)
        .unwrap_or(&empty_checks);
    let mut checks_out = Vec::with_capacity(checks_in.len() + 1);
    for check in checks_in {
        checks_out.push(convert_check(check, node_paths)?);
    }
    // Mandatory regardless of what the model authored: every plan is
    // verified against real tests, not only whatever checks the model
    // chose to write. The model cannot remove or replace this.
    checks_out.push(json!({"kind": "tests.impacted", "expect": "all_pass"}));

    Ok(json!({
        "plan_version": 2,
        "plan_id": plan_id,
        "intent": intent,
        "base_commit": base_commit,
        "on_failure": "rollback_plan",
        "steps": [{
            "id": id,
            "description": description,
            "edits": edits_out,
            "checks": checks_out
        }]
    }))
}

fn convert_edit(edit: &Value, node_paths: &[String]) -> Result<Value, String> {
    let node = edit
        .get("node")
        .and_then(Value::as_str)
        .ok_or("edit.node missing")?;
    if !node_paths.iter().any(|path| path.as_str() == node) {
        return Err(format!(
            "edit references a node that was not offered: {node:?}"
        ));
    }
    let operation = edit
        .get("operation")
        .and_then(Value::as_str)
        .ok_or("edit.operation missing")?;
    match operation {
        "replace_node" => {
            let replacement = edit
                .get("replace_node")
                .and_then(Value::as_str)
                .ok_or("edit.replace_node missing for operation replace_node")?;
            Ok(json!({"node": node, "replace_node": replacement}))
        }
        "rename_node" => {
            let new_name = edit
                .get("rename_node")
                .and_then(Value::as_str)
                .ok_or("edit.rename_node missing for operation rename_node")?;
            Ok(json!({"node": node, "rename_node": new_name}))
        }
        "delete_node" => Ok(json!({"node": node, "delete_node": true})),
        "insert_into_module" => {
            let insertion = edit
                .get("insert_into_module")
                .and_then(Value::as_str)
                .ok_or("edit.insert_into_module missing for operation insert_into_module")?;
            Ok(json!({"node": node, "insert_into_module": insertion}))
        }
        other => Err(format!("unknown edit operation {other:?}")),
    }
}

fn convert_check(check: &Value, node_paths: &[String]) -> Result<Value, String> {
    let kind = check
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("check.kind missing")?;
    match kind {
        "graph.node_exists" | "graph.node_absent" => {
            let node = check
                .get("node")
                .and_then(Value::as_str)
                .ok_or("check.node missing")?;
            if !node_paths.iter().any(|path| path.as_str() == node) {
                return Err(format!(
                    "check references a node that was not offered: {node:?}"
                ));
            }
            Ok(json!({"kind": kind, "node": node}))
        }
        "graph.callers_of" | "graph.callees_of" => {
            let node = check
                .get("node")
                .and_then(Value::as_str)
                .ok_or("check.node missing")?;
            if !node_paths.iter().any(|path| path.as_str() == node) {
                return Err(format!(
                    "check references a node that was not offered: {node:?}"
                ));
            }
            let expect: Vec<String> = check
                .get("expect")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            Ok(json!({"kind": kind, "node": node, "expect": expect}))
        }
        "command" => {
            let run = check
                .get("run")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or("command check missing a non-empty run")?;
            // Clamped to i32's range before it ever reaches the plan file:
            // schema.rs deserializes `expect_exit` as `i32`, and an
            // out-of-range value would otherwise turn into a hard
            // `load_plan` failure instead of a repairable diagnostic.
            let expect_exit = check
                .get("expect_exit")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .clamp(i64::from(i32::MIN), i64::from(i32::MAX));
            Ok(json!({"kind": "command", "run": run, "expect_exit": expect_exit}))
        }
        other => Err(format!("unknown check kind {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node_paths() -> Vec<String> {
        vec![
            "crate::calc::greet".to_string(),
            "crate::calc::hello".to_string(),
        ]
    }

    #[test]
    fn collect_intent_skips_flags_and_their_values() {
        let args = vec![
            ".".to_string(),
            "add".to_string(),
            "--dry".to_string(),
            "validation".to_string(),
            "--max-repairs".to_string(),
            "5".to_string(),
            "to".to_string(),
            "greet".to_string(),
        ];
        assert_eq!(collect_intent(&args), "add validation to greet");
    }

    #[test]
    fn wrap_step_into_plan_injects_harness_owned_fields_and_the_mandatory_check() {
        let step = json!({
            "id": "s1",
            "description": "d",
            "edits": [{
                "node": "crate::calc::greet",
                "operation": "replace_node",
                "replace_node": "def greet(name):\n    return name",
                "rename_node": "",
                "delete_node": false,
                "insert_into_module": ""
            }],
            "checks": []
        });
        let plan = wrap_step_into_plan(
            &step,
            &node_paths(),
            "deadbeef",
            "add validation",
            "do-test",
        )
        .unwrap();

        assert_eq!(plan["plan_version"], 2);
        assert_eq!(plan["on_failure"], "rollback_plan");
        assert_eq!(plan["base_commit"], "deadbeef");
        let checks = plan["steps"][0]["checks"].as_array().unwrap();
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0]["kind"], "tests.impacted");
        let edits = plan["steps"][0]["edits"].as_array().unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0]["node"], "crate::calc::greet");
        assert_eq!(
            edits[0]["replace_node"],
            "def greet(name):\n    return name"
        );
        assert!(edits[0].get("rename_node").is_none());
    }

    #[test]
    fn convert_edit_rejects_a_node_outside_the_offered_set() {
        let edit = json!({
            "node": "crate::calc::somewhere_else",
            "operation": "delete_node",
            "replace_node": "",
            "rename_node": "",
            "delete_node": true,
            "insert_into_module": ""
        });
        let error = convert_edit(&edit, &node_paths()).unwrap_err();
        assert!(error.contains("not offered"), "{error}");
    }

    #[test]
    fn model_cannot_remove_or_replace_the_mandatory_impacted_test_check() {
        let step = json!({
            "id": "s1",
            "description": "d",
            "edits": [{
                "node": "crate::calc::greet",
                "operation": "delete_node",
                "replace_node": "",
                "rename_node": "",
                "delete_node": true,
                "insert_into_module": ""
            }],
            "checks": [{
                "kind": "command",
                "node": "",
                "expect": [],
                "run": "true",
                "expect_exit": 0
            }]
        });
        let plan = wrap_step_into_plan(&step, &node_paths(), "deadbeef", "delete greet", "do-test")
            .unwrap();
        let checks = plan["steps"][0]["checks"].as_array().unwrap();
        assert_eq!(checks.len(), 2);
        assert!(checks.iter().any(|check| check["kind"] == "command"));
        assert!(checks.iter().any(|check| check["kind"] == "tests.impacted"));
    }

    #[test]
    fn step_schema_bounds_edits_to_the_number_of_offered_nodes() {
        let schema = step_schema(&node_paths());
        assert_eq!(schema["properties"]["edits"]["maxItems"], 2);
        assert_eq!(schema["properties"]["edits"]["minItems"], 1);
    }
}
