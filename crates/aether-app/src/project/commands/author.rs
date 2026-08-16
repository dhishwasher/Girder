//! `bitcode do <dir> "<intent...>" [--dry] [--max-repairs N] [--nodes
//! <path>[,<path>...]]` — connects the plan executor to a model. Selects the
//! graph nodes most relevant to the intent via the existing concept-search
//! ranking (or, with `--nodes`, uses exactly the given paths and skips search
//! entirely), asks the router for a grammar-constrained plan step addressing
//! only those nodes, executes it through the *existing* plan executor
//! (`planfile::run_for_authoring`), and on failure repairs with the check
//! output up to `--max-repairs` times before escalating to the next provider
//! in the router's chain.
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

/// How many concept-search hits to show the model. First-run evidence (see
/// gap #15 in `docs/core-gap-analysis.md`): a five-candidate list let four
/// noise nodes scoring 0.05-0.09 sit in the schema enum next to the one real
/// 0.17 hit, and every attempt addressed a noise node. Three keeps room for a
/// real runner-up without diluting the enum with near-zero scores as badly.
const TOP_K: usize = 3;
/// Discard any search hit scoring below this fraction of the top hit's
/// score. On the run that motivated this constant, the top hit was 0.17 and
/// the noise sat at 0.09/0.09/0.05/0.05 (see gap #15 in
/// `docs/core-gap-analysis.md`); a 0.5 floor is the starting point for
/// separating a real hit from noise like that. Named so the ratio isn't a
/// magic literal buried in the filter — retune here if a future run shows
/// 0.5 admits noise or excludes a real runner-up.
const NODE_SCORE_FLOOR_RATIO: f32 = 0.5;
const DEFAULT_MAX_REPAIRS: usize = 2;

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

const USAGE: &str =
    "usage: bitcode do <dir> \"<intent...>\" [--dry] [--max-repairs N] [--nodes <path>[,<path>...]]";

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
    let nodes_value = args.windows(2).find(|window| window[0] == "--nodes");
    if args.iter().any(|arg| arg == "--nodes") && nodes_value.is_none() {
        return Err(invalid_input("--nodes requires a value"));
    }
    let pinned_node_paths: Option<Vec<String>> = match nodes_value {
        Some(window) => {
            let paths: Vec<String> = window[1]
                .split(',')
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(String::from)
                .collect();
            if paths.is_empty() {
                return Err(invalid_input("--nodes requires at least one node path"));
            }
            Some(paths)
        }
        None => None,
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

    let nodes: Vec<aether_graph::Node> = match &pinned_node_paths {
        Some(paths) => {
            println!("\nUsing pinned nodes for \"{intent}\":");
            let mut pinned = Vec::with_capacity(paths.len());
            for path in paths {
                let Some(node) = graph.find_by_path(path) else {
                    return Err(invalid_input(&format!(
                        "--nodes references a path not present in the graph: {path}"
                    )));
                };
                println!("  {}", node.path);
                pinned.push(node.clone());
            }
            pinned
        }
        None => {
            let hits = graph.semantic_search(&intent, TOP_K);
            if hits.is_empty() {
                println!("\nNo nodes matched \"{intent}\"; nothing to author.");
                return Ok(());
            }
            let hits = apply_score_floor(hits);
            println!("\nSelecting nodes for \"{intent}\":");
            let mut selected = Vec::with_capacity(hits.len());
            for (id, score) in &hits {
                if let Some(node) = graph.get(*id) {
                    println!("  {score:.2}  {}", node.path);
                    selected.push(node.clone());
                }
            }
            selected
        }
    };
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
                &node_paths,
                &schema,
                (attempt > 0).then_some(diagnostic.as_str()),
            );
            let started = std::time::Instant::now();
            let completion = match provider.complete(prompt).await {
                Ok(completion) => completion,
                Err(error) => {
                    diagnostic = format!("{} declined: {error}", provider.name());
                    println!("  {diagnostic} ({}s elapsed)", started.elapsed().as_secs());
                    // A decline is permanent for this provider (no key, no
                    // host, or an unconditional refusal) — retrying the same
                    // prompt against it won't help, so move on immediately
                    // instead of burning the repair budget. A timeout is one
                    // such decline: OllamaProvider now bounds every request
                    // (OLLAMA_TIMEOUT_SECS), so a slow local model surfaces
                    // here as an ordinary error instead of hanging forever.
                    break;
                }
            };
            println!(
                "  {} responded in {}s",
                provider.name(),
                started.elapsed().as_secs()
            );
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

/// Keep only hits scoring at least [`NODE_SCORE_FLOOR_RATIO`] of the top
/// hit's score. `hits` must already be sorted best-first (as
/// `semantic_search` returns them) — the floor is relative to `hits[0]`.
fn apply_score_floor(hits: Vec<(aether_graph::NodeId, f32)>) -> Vec<(aether_graph::NodeId, f32)> {
    let Some(&(_, top_score)) = hits.first() else {
        return hits;
    };
    let floor = top_score * NODE_SCORE_FLOOR_RATIO;
    hits.into_iter()
        .filter(|&(_, score)| score >= floor)
        .collect()
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
        if arg == "--max-repairs" || arg == "--nodes" {
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
    node_paths: &[String],
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
         with that same name is used, so leave the other three empty/false. A \
         tests.impacted check runs automatically after your plan, so the checks array may \
         be left empty — do this unless you have a specific graph.* check in mind. Do NOT \
         use a \"command\" check: it can trigger a full project build and is almost never \
         the right choice here. If you do add a check, it must be graph.node_exists, \
         graph.node_absent, graph.callers_of, or graph.callees_of. Every check field is \
         required regardless of kind, including \"run\" — for a non-command check, set \
         \"run\" to a single placeholder character such as \"-\" (it is ignored), never \
         the empty string. Return JSON only, no prose.",
        serde_json::to_string(&Value::Array(node_context.to_vec())).unwrap_or_default()
    );
    if let Some(diagnostic) = repair_diagnostic {
        // A diagnostic that names one of the offered nodes means the node
        // choice, not just the edit kind, may be wrong — attempt 3 of the
        // qwen2.5-coder:1.5b run (gap #15 in docs/core-gap-analysis.md) kept
        // the same bad node across repairs because the repair prompt only
        // ever said an edit failed, never that the node itself looked wrong.
        // Restating the intent and the node list with the failure pinned to
        // that node gives the model a reason to reconsider it instead of
        // just swapping the operation.
        match node_paths
            .iter()
            .find(|path| diagnostic.contains(path.as_str()))
        {
            Some(implicated) => {
                user.push_str(&format!(
                    "\n\nThe previous attempt failed, and the failure is attributed to node \
                     `{implicated}`:\n{diagnostic}\n\
                     Re-read the intent — \"{intent}\" — and reconsider whether `{implicated}` \
                     is actually the right node. Nodes offered: {}. If it is not, choose a \
                     different offered node instead of only changing the operation on the same \
                     node. Return a corrected step object only.",
                    node_paths.join(", ")
                ));
            }
            None => {
                user.push_str(&format!(
                    "\n\nThe previous attempt failed:\n{diagnostic}\nReturn a corrected step object only."
                ));
            }
        }
    }
    let mut prompt =
        Prompt::new(TaskClass::Authoring, system, user).with_response_schema(schema.clone());
    // Set from measured local throughput, not the truncation this bound was
    // first raised for. qwen2.5-coder:1.5b generates at 1.49 tok/s on this
    // machine (95 tokens in 63.7s, via /api/generate eval_count/
    // eval_duration), so 1024 tokens is roughly an 11-minute ceiling per
    // attempt (1024 / 1.49 tok/s ~= 687s). The gap #15 truncation ("EOF
    // while parsing a string at line 22 column 1216") failed at ~1216
    // characters of JSON — well under 1024 tokens' worth — so it was never
    // actually a max_tokens problem; raising this to 8192 (a 91-minute
    // ceiling at this throughput) didn't address the real cause, which was
    // the Ollama provider having no request timeout (now fixed via
    // OLLAMA_TIMEOUT_SECS in aether-ai). This value is throughput-derived:
    // revisit it if the local model changes.
    prompt.max_tokens = 1024;
    prompt
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
            // A command check with an empty run is useless and previously
            // reached `convert_check` as "command check missing a
            // non-empty run" — a repairable diagnostic, but one the
            // grammar should reject up front instead. Every check kind
            // still must supply some string here (the shape stays flat
            // per the module-level rationale on `step_schema`), but it can
            // no longer be empty.
            "run": {"type": "string", "minLength": 1},
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
    fn collect_intent_skips_nodes_flag_and_its_value() {
        let args = vec![
            ".".to_string(),
            "add".to_string(),
            "validation".to_string(),
            "--nodes".to_string(),
            "crate::calc::greet,crate::calc::hello".to_string(),
            "to".to_string(),
            "greet".to_string(),
        ];
        assert_eq!(collect_intent(&args), "add validation to greet");
    }

    #[test]
    fn apply_score_floor_drops_hits_under_half_the_top_score() {
        let top = aether_graph::NodeId::from_path("crate::calc::greet");
        let noise_a = aether_graph::NodeId::from_path("crate::tools::noise_a");
        let noise_b = aether_graph::NodeId::from_path("crate::tools::noise_b");
        // Mirrors the shape of the observed run (top hit 0.17, noise well
        // below it): every noise score here is strictly under half of 0.17.
        let hits = vec![(top, 0.17), (noise_a, 0.08), (noise_b, 0.05)];
        let kept = apply_score_floor(hits);
        assert_eq!(kept, vec![(top, 0.17)]);
    }

    #[test]
    fn apply_score_floor_keeps_a_close_runner_up() {
        let top = aether_graph::NodeId::from_path("crate::calc::greet");
        let runner_up = aether_graph::NodeId::from_path("crate::calc::greet_loudly");
        let hits = vec![(top, 0.20), (runner_up, 0.15)];
        let kept = apply_score_floor(hits);
        assert_eq!(kept, vec![(top, 0.20), (runner_up, 0.15)]);
    }

    #[test]
    fn build_prompt_attributes_a_repair_failure_to_the_node_it_names() {
        let node_context = vec![json!({
            "path": "crate::calc::greet",
            "language": "python",
            "source": "def greet(): pass"
        })];
        let schema = json!({"type": "object"});
        let diagnostic = "    FAIL graph.node_exists: crate::calc::greet not found";
        let prompt = build_prompt(
            "add validation to greet",
            &node_context,
            &node_paths(),
            &schema,
            Some(diagnostic),
        );
        assert!(
            prompt
                .user
                .contains("attributed to node `crate::calc::greet`"),
            "{}",
            prompt.user
        );
        assert!(prompt.user.contains("add validation to greet"));
        assert!(prompt.user.contains(&node_paths().join(", ")));
    }

    #[test]
    fn build_prompt_falls_back_to_the_plain_diagnostic_when_no_node_is_named() {
        let node_context = vec![json!({
            "path": "crate::calc::greet",
            "language": "python",
            "source": "def greet(): pass"
        })];
        let schema = json!({"type": "object"});
        let diagnostic = "model response was not valid JSON: EOF while parsing a string";
        let prompt = build_prompt(
            "add validation to greet",
            &node_context,
            &node_paths(),
            &schema,
            Some(diagnostic),
        );
        assert!(!prompt.user.contains("attributed to node"));
        assert!(prompt.user.contains(diagnostic));
    }

    #[test]
    fn build_prompt_sets_the_throughput_derived_max_tokens() {
        let node_context = vec![json!({
            "path": "crate::calc::greet",
            "language": "python",
            "source": "def greet(): pass"
        })];
        let schema = json!({"type": "object"});
        let prompt = build_prompt(
            "add validation",
            &node_context,
            &node_paths(),
            &schema,
            None,
        );
        assert_eq!(prompt.max_tokens, 1024);
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

    #[test]
    fn check_schema_rejects_an_empty_run_string() {
        let schema = check_schema(&node_paths());
        assert_eq!(schema["properties"]["run"]["minLength"], 1);
    }
}
