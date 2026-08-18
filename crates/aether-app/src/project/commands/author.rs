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

use super::authoring_context::{
    build_authoring_context, collect_words, generate_plan_id, invalid_input, parse_pinned_nodes,
    SelectionError,
};
use crate::project::config::ProjectConfig;
use crate::project::git::git_head_commit;
use crate::project::planfile::{
    reject_measurement_fixture_root, run_for_authoring, AuthoringRunResult,
};
use crate::project::source::build_from_dir_with_config;
use aether_ai::{Prompt, TaskClass};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) const DEFAULT_MAX_REPAIRS: usize = 2;

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

const USAGE: &str =
    "usage: bitcode do <dir> \"<intent...>\" [--dry] [--max-repairs N] [--nodes <path>[,<path>...]]";

/// One observable step of an [`author`] run, emitted through its progress
/// callback instead of printed directly, so `bitcode do` (which prints each
/// variant verbatim) and the GUI's Author tab (which streams each variant
/// into a scrolling log) render the exact same authoring run without either
/// side reimplementing node selection, prompting, or the repair loop.
pub(crate) enum AuthorEvent {
    Loading {
        root: PathBuf,
    },
    Loaded {
        files: usize,
        node_count: usize,
    },
    NodesSelected {
        pinned: bool,
        intent: String,
        nodes: Vec<(String, Option<f32>)>,
    },
    AttemptStarted {
        provider: String,
        attempt: usize,
        max_attempts: usize,
        node_paths: Vec<String>,
    },
    ProviderDeclined {
        diagnostic: String,
        elapsed_secs: u64,
    },
    ProviderResponded {
        provider: String,
        elapsed_secs: u64,
    },
    ModelResponseInvalid {
        diagnostic: String,
    },
    PlanWrapFailed {
        diagnostic: String,
    },
    AttemptFailed {
        diagnostic: String,
    },
}

/// The final result of an [`author`] run.
#[derive(Debug)]
pub(crate) enum AuthorOutcome {
    /// Node selection matched nothing; nothing was authored, and this is
    /// not an error (mirrors `do_intent`'s current `Ok(())` early return).
    NoMatches,
    Authored {
        provider: String,
        model: String,
        repairs: usize,
        report_path: Option<PathBuf>,
        report_json: Option<String>,
    },
}

/// Selects nodes, asks the router for a grammar-constrained plan step, and
/// executes it through `run_for_authoring`, repairing with check output up
/// to `max_repairs` times before escalating to the next provider — this is
/// the entire body of `bitcode do`, extracted so the GUI's Author tab can
/// call the exact same function instead of a second implementation.
///
/// `on_progress` fires once per observable step; it must be `Send` because
/// the GUI runs this inside a `tokio::spawn`ed future, which requires the
/// whole future — including any closure held across the `.await` points
/// below — to be `Send`.
pub(crate) async fn author(
    root: &Path,
    intent: &str,
    pinned_node_paths: Option<&[String]>,
    dry: bool,
    max_repairs: usize,
    router: &aether_ai::Router,
    mut on_progress: impl FnMut(AuthorEvent) + Send,
) -> std::io::Result<AuthorOutcome> {
    reject_measurement_fixture_root(root)?;
    on_progress(AuthorEvent::Loading {
        root: root.to_path_buf(),
    });
    let config = ProjectConfig::load(root)?;
    let (graph, _builder, files) = build_from_dir_with_config(root, &config)?;
    on_progress(AuthorEvent::Loaded {
        files,
        node_count: graph.node_count(),
    });

    let ctx = match build_authoring_context(&graph, intent, pinned_node_paths) {
        Ok(ctx) => ctx,
        Err(SelectionError::NoMatches) => return Ok(AuthorOutcome::NoMatches),
        Err(SelectionError::UnknownPath(path)) => {
            return Err(invalid_input(&format!(
                "--nodes references a path not present in the graph: {path}"
            )));
        }
    };
    on_progress(AuthorEvent::NodesSelected {
        pinned: pinned_node_paths.is_some(),
        intent: intent.to_string(),
        nodes: ctx
            .nodes
            .iter()
            .map(|selected| (selected.node.path.clone(), selected.score))
            .collect(),
    });
    let node_paths = ctx.node_paths;
    let node_context = ctx.node_context;
    let schema = ctx.schema;

    let base_commit = git_head_commit(root)?;
    let candidates = router.candidates(TaskClass::Authoring);
    if candidates.is_empty() {
        return Err(std::io::Error::other(
            "no provider is configured for authoring",
        ));
    }
    let total_providers = candidates.len();

    let plan_id = generate_plan_id();

    let mut calls: Vec<Value> = Vec::new();
    let mut diagnostic = String::new();
    let mut succeeded: Option<(String, String, usize, AuthoringRunResult)> = None;

    'providers: for provider in &candidates {
        for attempt in 0..=max_repairs {
            on_progress(AuthorEvent::AttemptStarted {
                provider: provider.name().to_string(),
                attempt: attempt + 1,
                max_attempts: max_repairs + 1,
                node_paths: node_paths.clone(),
            });
            let prompt = build_prompt(
                intent,
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
                    on_progress(AuthorEvent::ProviderDeclined {
                        diagnostic: diagnostic.clone(),
                        elapsed_secs: started.elapsed().as_secs(),
                    });
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
            on_progress(AuthorEvent::ProviderResponded {
                provider: provider.name().to_string(),
                elapsed_secs: started.elapsed().as_secs(),
            });
            calls.push(json!({
                "provider": provider.name(),
                "model": completion.model,
                "tokens": completion.tokens
            }));

            let step_value: Value = match serde_json::from_str(&completion.text) {
                Ok(value) => value,
                Err(error) => {
                    diagnostic = format!("model response was not valid JSON: {error}");
                    on_progress(AuthorEvent::ModelResponseInvalid {
                        diagnostic: diagnostic.clone(),
                    });
                    continue;
                }
            };
            let plan_value =
                match wrap_step_into_plan(&step_value, &node_paths, &base_commit, intent, &plan_id)
                {
                    Ok(value) => value,
                    Err(error) => {
                        diagnostic = error;
                        on_progress(AuthorEvent::PlanWrapFailed {
                            diagnostic: diagnostic.clone(),
                        });
                        continue;
                    }
                };

            let plan_path = write_temp_json(&plan_value, "plan")?;
            let receipt_path =
                write_temp_json(&json!({"schema_version": 1, "calls": calls}), "receipt")?;
            let run_outcome = run_for_authoring(root, &plan_path, dry, Some(&receipt_path));
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
            on_progress(AuthorEvent::AttemptFailed {
                diagnostic: diagnostic.clone(),
            });
        }
    }

    match succeeded {
        Some((provider_name, model, repairs, result)) => Ok(AuthorOutcome::Authored {
            provider: provider_name,
            model,
            repairs,
            report_path: result.report_path,
            report_json: result.report_json,
        }),
        None => Err(std::io::Error::other(format!(
            "no provider produced a working plan for \"{intent}\" after trying \
             {total_providers} provider(s); last diagnostic: {diagnostic}"
        ))),
    }
}

/// `bitcode do`'s progress callback: prints each [`AuthorEvent`] exactly as
/// `do_intent` printed it before `author` was extracted.
fn print_author_event(event: AuthorEvent) {
    match event {
        AuthorEvent::Loading { root } => println!("Loading {} ...", root.display()),
        AuthorEvent::Loaded { files, node_count } => {
            println!("  {files} file(s), {node_count} nodes")
        }
        AuthorEvent::NodesSelected {
            pinned,
            intent,
            nodes,
        } => {
            if pinned {
                println!("\nUsing pinned nodes for \"{intent}\":");
            } else {
                println!("\nSelecting nodes for \"{intent}\":");
            }
            for (path, score) in &nodes {
                match score {
                    Some(score) => println!("  {score:.2}  {path}"),
                    None => println!("  {path}"),
                }
            }
        }
        AuthorEvent::AttemptStarted {
            provider,
            attempt,
            max_attempts,
            node_paths,
        } => {
            println!(
                "\n[{provider}] attempt {attempt}/{max_attempts} — nodes: {}",
                node_paths.join(", ")
            );
        }
        AuthorEvent::ProviderDeclined {
            diagnostic,
            elapsed_secs,
        } => println!("  {diagnostic} ({elapsed_secs}s elapsed)"),
        AuthorEvent::ProviderResponded {
            provider,
            elapsed_secs,
        } => println!("  {provider} responded in {elapsed_secs}s"),
        AuthorEvent::ModelResponseInvalid { diagnostic } => println!("  {diagnostic}"),
        AuthorEvent::PlanWrapFailed { diagnostic } => println!("  {diagnostic}"),
        AuthorEvent::AttemptFailed { diagnostic } => {
            println!("  attempt failed:\n{}", indent(&diagnostic))
        }
    }
}

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
    let pinned_node_paths = parse_pinned_nodes(args)?;
    let intent = collect_intent(args);
    if intent.trim().is_empty() {
        eprintln!("{USAGE}");
        return Ok(());
    }

    let router = aether_ai::default_router();
    let result = author(
        &root,
        &intent,
        pinned_node_paths.as_deref(),
        dry,
        max_repairs,
        &router,
        print_author_event,
    )
    .await;

    match result {
        Ok(AuthorOutcome::NoMatches) => {
            println!("\nNo nodes matched \"{intent}\"; nothing to author.");
            Ok(())
        }
        Ok(AuthorOutcome::Authored {
            provider,
            model,
            repairs,
            report_path,
            report_json,
        }) => {
            println!("\nplan authored by {provider} ({model}) after {repairs} repair attempt(s)");
            if let Some(path) = &report_path {
                println!("report: {}", path.display());
            }
            if let Some(rendered) = &report_json {
                println!("\nreport (dry run, not written to disk):\n{rendered}");
            }
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn collect_intent(args: &[String]) -> String {
    collect_words(args, &["--dry"], &["--max-repairs", "--nodes"])
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

    #[tokio::test]
    async fn author_rejects_sample_project_before_touching_the_provider_or_disk() {
        // The path need not exist and no router candidate needs to work:
        // reject_measurement_fixture_root runs before any file I/O or
        // provider call, so this is a fast, fixture-free check that
        // `bitcode do` (and the GUI's local-model Run, which calls this
        // same function) refuses sample-project/ as a target.
        let root = std::path::Path::new("/nonexistent/sample-project");
        let router = aether_ai::default_router();
        let result = author(
            root,
            "intent",
            None,
            true,
            0,
            &router,
            |_event: AuthorEvent| {},
        )
        .await;
        let error = result.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("demo-project"), "{error}");
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
}
