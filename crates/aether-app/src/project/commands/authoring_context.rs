//! Context assembly shared by `bitcode do` ([`super::author`]) and
//! `bitcode context` ([`super::context_cmd`]): node selection, the
//! `{path, language, source}` context a model sees, and the authoring JSON
//! Schema that constrains a legal plan step for those nodes.
//!
//! `docs/authoring-cost.md` measures token cost against exactly this
//! context shape. `do` and `context` must never drift from each other, so
//! both call these functions instead of each building their own copy.

use serde_json::{json, Value};

/// How many concept-search hits to show the model. First-run evidence (see
/// gap #15 in `docs/core-gap-analysis.md`): a five-candidate list let four
/// noise nodes scoring 0.05-0.09 sit in the schema enum next to the one real
/// 0.17 hit, and every attempt addressed a noise node. Three keeps room for a
/// real runner-up without diluting the enum with near-zero scores as badly.
pub(crate) const TOP_K: usize = 3;
/// Discard any search hit scoring below this fraction of the top hit's
/// score. On the run that motivated this constant, the top hit was 0.17 and
/// the noise sat at 0.09/0.09/0.05/0.05 (see gap #15 in
/// `docs/core-gap-analysis.md`); a 0.5 floor is the starting point for
/// separating a real hit from noise like that. Named so the ratio isn't a
/// magic literal buried in the filter — retune here if a future run shows
/// 0.5 admits noise or excludes a real runner-up.
pub(crate) const NODE_SCORE_FLOOR_RATIO: f32 = 0.5;

/// One node chosen for authoring. `score` is `None` when the node was
/// pinned via `--nodes` (bypasses scoring entirely) and `Some` when concept
/// search chose it.
#[derive(Debug)]
pub(crate) struct SelectedNode {
    pub(crate) node: aether_graph::Node,
    pub(crate) score: Option<f32>,
}

/// Why node selection failed to produce any nodes to author against.
#[derive(Debug)]
pub(crate) enum SelectionError {
    /// Concept search returned nothing for this intent.
    NoMatches,
    /// A `--nodes` path isn't in the graph.
    UnknownPath(String),
}

/// Node selection shared by `bitcode do` and `bitcode context`: `--nodes`
/// pins exact paths (fails closed on any path not present in the graph);
/// otherwise concept search with [`TOP_K`] and [`NODE_SCORE_FLOOR_RATIO`].
/// No I/O beyond graph lookups and no printing — callers own their own
/// console output (`do` narrates as it goes; `context` prints only the
/// final JSON object).
pub(crate) fn select_nodes(
    graph: &aether_graph::SemanticGraph,
    intent: &str,
    pinned_node_paths: Option<&[String]>,
) -> Result<Vec<SelectedNode>, SelectionError> {
    match pinned_node_paths {
        Some(paths) => {
            let mut selected = Vec::with_capacity(paths.len());
            for path in paths {
                let Some(node) = graph.find_by_path(path) else {
                    return Err(SelectionError::UnknownPath(path.clone()));
                };
                selected.push(SelectedNode {
                    node: node.clone(),
                    score: None,
                });
            }
            Ok(selected)
        }
        None => {
            let hits = graph.semantic_search(intent, TOP_K);
            if hits.is_empty() {
                return Err(SelectionError::NoMatches);
            }
            let hits = apply_score_floor(hits);
            let mut selected = Vec::with_capacity(hits.len());
            for (id, score) in hits {
                if let Some(node) = graph.get(id) {
                    selected.push(SelectedNode {
                        node: node.clone(),
                        score: Some(score),
                    });
                }
            }
            Ok(selected)
        }
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

/// Everything a caller needs to author a plan step against the selected
/// nodes: the nodes themselves (with selection scores, for printing), the
/// flat path list (the schema's node enum and node-attribution in repair
/// prompts both key off this), the `{path, language, source}` JSON a model
/// prompt embeds verbatim, and the schema that constrains a legal step for
/// exactly these nodes.
pub(crate) struct AuthoringContext {
    pub(crate) nodes: Vec<SelectedNode>,
    pub(crate) node_paths: Vec<String>,
    pub(crate) node_context: Vec<Value>,
    pub(crate) schema: Value,
}

/// The `{path, language, source}` shape a model prompt embeds verbatim for
/// one node. Shared by `node_context` above and `context --with-tests`'
/// covering-test entries, so a test's context shape can never drift from a
/// selected node's.
pub(crate) fn node_context_entry(node: &aether_graph::Node) -> Value {
    json!({
        "path": node.path,
        "language": node.language,
        "source": node.source
    })
}

pub(crate) fn build_authoring_context(
    graph: &aether_graph::SemanticGraph,
    intent: &str,
    pinned_node_paths: Option<&[String]>,
) -> Result<AuthoringContext, SelectionError> {
    let nodes = select_nodes(graph, intent, pinned_node_paths)?;
    let node_paths: Vec<String> = nodes
        .iter()
        .map(|selected| selected.node.path.clone())
        .collect();
    let node_context: Vec<Value> = nodes
        .iter()
        .map(|selected| node_context_entry(&selected.node))
        .collect();
    let schema = step_schema(&node_paths);
    Ok(AuthoringContext {
        nodes,
        node_paths,
        node_context,
        schema,
    })
}

/// Fresh-from-disk node search for the GUI's Author tab, matching the CLI's
/// own path from a root directory to a candidate list exactly: builds the
/// graph from disk (not any live in-memory graph — so what the GUI's Search
/// preview shows is exactly what an eventual [`super::author`] run would
/// search against) and runs the same `TOP_K`/`NODE_SCORE_FLOOR_RATIO`
/// selection `bitcode do` uses. Search never pins nodes, so "no matches" is
/// this function's own legitimate empty result, not an error — unlike
/// [`build_authoring_context`], which a pinned caller can fail with
/// [`SelectionError::UnknownPath`].
#[cfg(feature = "gui")]
pub(crate) fn search_nodes_for_authoring(
    root: &std::path::Path,
    intent: &str,
) -> std::io::Result<Vec<(String, Option<f32>)>> {
    let config = crate::project::config::ProjectConfig::load(root)?;
    let (graph, _builder, _files) =
        crate::project::source::build_from_dir_with_config(root, &config)?;
    match build_authoring_context(&graph, intent, None) {
        Ok(ctx) => Ok(ctx
            .nodes
            .into_iter()
            .map(|selected| (selected.node.path, selected.score))
            .collect()),
        Err(SelectionError::NoMatches) => Ok(Vec::new()),
        Err(SelectionError::UnknownPath(path)) => {
            unreachable!("search never pins nodes, so no path can be unknown: {path}")
        }
    }
}

/// A flat, discriminant-selected shape rather than a `oneOf` union:
/// `tools/plan_executor_oracle.py`'s `authoring_plan_json_schema` avoids
/// unions entirely (it builds a schema from one concrete example per fixed
/// task), which is evidence that a JSON-Schema union is not a shape to lean
/// on for grammar-constrained local decoding. Since a plan-authoring model
/// doesn't know the operation ahead of time the way that fixed corpus does,
/// every operation field is present and required; only the one named by
/// `operation` is read back out (in `author::convert_edit`/`convert_check`
/// for a local model's response — an external model gets this schema
/// unfiltered via `bitcode context`).
pub(crate) fn step_schema(node_paths: &[String]) -> Value {
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
/// check the model cannot see, remove, or replace (see
/// `author::wrap_step_into_plan` for a local model's plan, and `plan run
/// --authored` for an external model's).
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
            // per the rationale on `step_schema`), but it can no longer be
            // empty.
            "run": {"type": "string", "minLength": 1},
            "expect_exit": {"type": "integer"}
        }
    })
}

/// Real Plan Format v2 step shape (see `planfile::schema`) — unlike
/// [`step_schema`]'s flat shape (translated by `author::convert_edit`/
/// `convert_check` before it ever reaches a plan file), `bitcode context`
/// hands this to an external model that writes a plan file directly, with
/// no translation layer, straight into `load_plan`. So this describes
/// exactly what `load_plan` accepts: an edit is `oneOf` the one
/// discriminator field the model actually chose (`schema.rs`'s custom
/// `Edit` deserializer rejects zero or more than one of
/// `replace_node`/`rename_node`/`delete_node`/`insert_into_module` present
/// at once), and a check is tagged by `kind` with only that kind's own
/// fields allowed (`schema.rs`'s `Check` is `#[serde(tag = "kind",
/// deny_unknown_fields)]`). Curated to the same operations/check kinds as
/// [`step_schema`] for parity, just shaped so the loader actually accepts
/// it.
pub(crate) fn plan_schema(node_paths: &[String]) -> Value {
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
                "items": plan_edit_schema(node_paths)
            },
            "checks": {
                "type": "array",
                "minItems": 0,
                "maxItems": 3,
                "items": plan_check_schema(node_paths)
            }
        }
    })
}

fn plan_edit_schema(node_paths: &[String]) -> Value {
    let node = json!({"type": "string", "enum": node_paths});
    json!({
        "type": "object",
        "oneOf": [
            {
                "additionalProperties": false,
                "required": ["node", "replace_node"],
                "properties": {"node": node.clone(), "replace_node": {"type": "string"}}
            },
            {
                "additionalProperties": false,
                "required": ["node", "rename_node"],
                "properties": {"node": node.clone(), "rename_node": {"type": "string"}}
            },
            {
                "additionalProperties": false,
                "required": ["node", "delete_node"],
                "properties": {"node": node.clone(), "delete_node": {"type": "boolean"}}
            },
            {
                "additionalProperties": false,
                "required": ["node", "insert_into_module"],
                "properties": {"node": node, "insert_into_module": {"type": "string"}}
            }
        ]
    })
}

fn plan_check_schema(node_paths: &[String]) -> Value {
    let node = json!({"type": "string", "enum": node_paths});
    json!({
        "type": "object",
        "oneOf": [
            {
                "additionalProperties": false,
                "required": ["kind", "node"],
                "properties": {
                    "kind": {"const": "graph.node_exists"},
                    "node": node.clone()
                }
            },
            {
                "additionalProperties": false,
                "required": ["kind", "node"],
                "properties": {
                    "kind": {"const": "graph.node_absent"},
                    "node": node.clone()
                }
            },
            {
                "additionalProperties": false,
                "required": ["kind", "node"],
                "properties": {
                    "kind": {"const": "graph.callers_of"},
                    "node": node.clone(),
                    "expect": {"type": "array", "items": {"type": "string"}}
                }
            },
            {
                "additionalProperties": false,
                "required": ["kind", "node"],
                "properties": {
                    "kind": {"const": "graph.callees_of"},
                    "node": node,
                    "expect": {"type": "array", "items": {"type": "string"}}
                }
            },
            {
                "additionalProperties": false,
                "required": ["kind", "run"],
                "properties": {
                    "kind": {"const": "command"},
                    "run": {"type": "string", "minLength": 1},
                    "expect_exit": {"type": "integer"}
                }
            }
        ]
    })
}

pub(crate) fn generate_plan_id() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("do-{millis}-{}", std::process::id())
}

/// The envelope `bitcode context` hands an external model to fill in:
/// harness-owned fields plus one placeholder step with empty edits and the
/// mandatory `tests.impacted` check already present. The external model
/// fills in only `id`, `description`, and `edits` — exactly the "creative
/// part" `author::wrap_step_into_plan` extracts from a local model's
/// response, and `plan run --authored` re-enforces on the way back in in
/// case the external model touched what it shouldn't have.
pub(crate) fn plan_skeleton(base_commit: &str, intent: &str, plan_id: &str) -> Value {
    json!({
        "plan_version": 2,
        "plan_id": plan_id,
        "intent": intent,
        "base_commit": base_commit,
        "on_failure": "rollback_plan",
        "steps": [{
            "id": "step-1",
            "description": "",
            "edits": [],
            "checks": [{"kind": "tests.impacted", "expect": "all_pass"}]
        }]
    })
}

/// The `bitcode new` counterpart to [`plan_edit_schema`]: a plan step here
/// can only create a file, never address an existing graph node, because
/// there is no graph yet for `bitcode new`'s target to have nodes in. Kept
/// as a one-branch `oneOf` for shape parity with `plan_edit_schema`'s
/// discriminated union rather than a flat object, so a caller that expects
/// "an edit is one of several shapes" doesn't need a special case for this
/// one. `path` has no enum the way `plan_edit_schema`'s `node` does — the
/// whole point is authoring a path that doesn't exist in the graph yet;
/// `Edit::Create`'s own overwrite/escape checks in `edit.rs` are the real
/// enforcement, exactly as they already are for any other caller of
/// `Edit::Create`.
pub(crate) fn creation_edit_schema() -> Value {
    json!({
        "type": "object",
        "oneOf": [
            {
                "additionalProperties": false,
                "required": ["path", "create"],
                "properties": {
                    "path": {"type": "string"},
                    "create": {"type": "string"}
                }
            }
        ]
    })
}

/// The `bitcode new` counterpart to [`plan_schema`]: every edit is
/// [`creation_edit_schema`] (a `create`, never a node-addressed op), and
/// every check the schema allows is `command` — gap 24's rule (a step
/// containing an `Edit::Create` must carry a `command` check in the same
/// step; see `docs/core-gap-analysis.md` item 24 and
/// `planfile::schema::Plan::validate`) is enforced here structurally
/// (`checks` requires at least one item and `kind` has no value but
/// `"command"`), not only discovered on rejection from `Plan::validate()`
/// after a model has already committed to a wrong shape. `tests.impacted`
/// is deliberately absent from the `kind` enum: it is always vacuous for a
/// node a `create` edit just introduced (no prior callers, no prior
/// tests), so offering it here would let a model reach for the exact
/// vacuous-pass shape gap 24 closed.
pub(crate) fn creation_plan_schema() -> Value {
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
                "items": creation_edit_schema()
            },
            "checks": {
                "type": "array",
                "minItems": 1,
                "maxItems": 3,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["kind", "run"],
                    "properties": {
                        "kind": {"type": "string", "enum": ["command"]},
                        "run": {"type": "string", "minLength": 1},
                        "expect_exit": {"type": "integer"}
                    }
                }
            }
        }
    })
}

/// The `bitcode new` counterpart to [`plan_skeleton`]: same envelope
/// shape, but `checks: []` instead of a pre-seeded `tests.impacted` —
/// gap 24 established that check is always vacuous for a node a `create`
/// edit just introduced, so seeding it here would be the exact "injecting
/// a check known to be vacuous is worse than injecting nothing" mistake
/// gap 24 closed at both of its other injection points. The model must
/// supply its own real `command` check; [`creation_plan_schema`]'s
/// `checks.minItems: 1` plus its `kind` enum of `["command"]` means it
/// cannot skip this the way a vacuous default would otherwise let it.
pub(crate) fn creation_plan_skeleton(base_commit: &str, intent: &str, plan_id: &str) -> Value {
    json!({
        "plan_version": 2,
        "plan_id": plan_id,
        "intent": intent,
        "base_commit": base_commit,
        "on_failure": "rollback_plan",
        "steps": [{
            "id": "step-1",
            "description": "",
            "edits": [],
            "checks": []
        }]
    })
}

pub(crate) fn invalid_input(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
}

/// Parse `--nodes <path>[,<path>...]`, shared by `bitcode do` and
/// `bitcode context`.
pub(crate) fn parse_pinned_nodes(args: &[String]) -> std::io::Result<Option<Vec<String>>> {
    let nodes_value = args.windows(2).find(|window| window[0] == "--nodes");
    if args.iter().any(|arg| arg == "--nodes") && nodes_value.is_none() {
        return Err(invalid_input("--nodes requires a value"));
    }
    match nodes_value {
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
            Ok(Some(paths))
        }
        None => Ok(None),
    }
}

/// Join every positional arg after `args[0]` (the `<dir>`) into a
/// whitespace-joined intent string, dropping `bare_flags` (no value) and
/// `flag_pairs` (a flag plus its following value) wherever they appear.
pub(crate) fn collect_words(args: &[String], bare_flags: &[&str], flag_pairs: &[&str]) -> String {
    let mut words = Vec::new();
    let mut skip_next = false;
    for arg in args.iter().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }
        if bare_flags.contains(&arg.as_str()) {
            continue;
        }
        if flag_pairs.contains(&arg.as_str()) {
            skip_next = true;
            continue;
        }
        words.push(arg.as_str());
    }
    words.join(" ")
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

    #[cfg(feature = "gui")]
    fn search_fixture_project(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "bitcode-authoring-context-search-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("calc.rs"),
            "pub fn greet() -> String {\n    \"hi\".to_string()\n}\n",
        )
        .unwrap();
        root
    }

    #[cfg(feature = "gui")]
    #[test]
    fn search_nodes_for_authoring_finds_a_real_node_built_fresh_from_disk() {
        let root = search_fixture_project("finds-a-node");
        let hits = search_nodes_for_authoring(&root, "greet").unwrap();
        assert!(
            hits.iter().any(|(path, _)| path == "crate::calc::greet"),
            "{hits:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(feature = "gui")]
    #[test]
    fn search_nodes_for_authoring_is_empty_not_an_error_on_no_matches() {
        let root = search_fixture_project("no-matches");
        let hits = search_nodes_for_authoring(
            &root,
            "nothing on this earth will match this intent string",
        )
        .unwrap();
        assert!(hits.is_empty());
        let _ = std::fs::remove_dir_all(&root);
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

    #[test]
    fn parse_pinned_nodes_splits_and_trims_comma_separated_paths() {
        let args = vec![
            ".".to_string(),
            "--nodes".to_string(),
            " crate::calc::greet , crate::calc::hello ".to_string(),
        ];
        let paths = parse_pinned_nodes(&args).unwrap().unwrap();
        assert_eq!(paths, node_paths());
    }

    #[test]
    fn parse_pinned_nodes_is_none_when_the_flag_is_absent() {
        let args = vec![".".to_string(), "intent".to_string()];
        assert!(parse_pinned_nodes(&args).unwrap().is_none());
    }

    #[test]
    fn parse_pinned_nodes_rejects_a_missing_value() {
        let args = vec![".".to_string(), "--nodes".to_string()];
        assert!(parse_pinned_nodes(&args).is_err());
    }

    #[test]
    fn collect_words_drops_bare_flags_and_flag_pairs() {
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
        assert_eq!(
            collect_words(&args, &["--dry"], &["--max-repairs", "--nodes"]),
            "add validation to greet"
        );
    }

    #[test]
    fn plan_skeleton_has_empty_edits_and_the_mandatory_check() {
        let skeleton = plan_skeleton("deadbeef", "add validation", "do-test");
        assert_eq!(skeleton["plan_version"], 2);
        assert_eq!(skeleton["plan_id"], "do-test");
        assert_eq!(skeleton["intent"], "add validation");
        assert_eq!(skeleton["base_commit"], "deadbeef");
        assert_eq!(skeleton["on_failure"], "rollback_plan");
        let steps = skeleton["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0]["edits"].as_array().unwrap().len(), 0);
        let checks = steps[0]["checks"].as_array().unwrap();
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0]["kind"], "tests.impacted");
        assert_eq!(checks[0]["expect"], "all_pass");
    }

    #[test]
    fn creation_edit_schema_offers_only_path_and_create_no_node_enum() {
        let schema = creation_edit_schema();
        let branch = &schema["oneOf"][0];
        assert_eq!(branch["required"], serde_json::json!(["path", "create"]));
        assert!(branch["properties"]["path"]["enum"].is_null());
        assert!(!branch["properties"]
            .as_object()
            .unwrap()
            .contains_key("node"));
    }

    #[test]
    fn creation_plan_schema_requires_at_least_one_command_check() {
        let schema = creation_plan_schema();
        assert_eq!(schema["properties"]["checks"]["minItems"], 1);
        assert_eq!(
            schema["properties"]["checks"]["items"]["properties"]["kind"]["enum"],
            serde_json::json!(["command"])
        );
        // tests.impacted must never appear as an offered kind here: it is
        // always vacuous for a node a create edit just introduced (gap 24).
        let allowed_kinds = schema["properties"]["checks"]["items"]["properties"]["kind"]["enum"]
            .as_array()
            .unwrap();
        assert!(!allowed_kinds.iter().any(|kind| kind == "tests.impacted"));
    }

    #[test]
    fn creation_plan_schema_edits_are_creation_edit_schema_only() {
        let schema = creation_plan_schema();
        assert_eq!(schema["properties"]["edits"]["minItems"], 1);
        assert_eq!(
            schema["properties"]["edits"]["items"],
            creation_edit_schema()
        );
    }

    #[test]
    fn creation_plan_skeleton_has_empty_edits_and_no_pre_seeded_checks() {
        let skeleton = creation_plan_skeleton("deadbeef", "make a greeter script", "new-test");
        assert_eq!(skeleton["plan_version"], 2);
        assert_eq!(skeleton["plan_id"], "new-test");
        assert_eq!(skeleton["intent"], "make a greeter script");
        assert_eq!(skeleton["base_commit"], "deadbeef");
        assert_eq!(skeleton["on_failure"], "rollback_plan");
        let steps = skeleton["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0]["edits"].as_array().unwrap().len(), 0);
        // Unlike plan_skeleton, no check is pre-seeded: tests.impacted
        // would be vacuous here, and injecting it would read as a safety
        // net that isn't one (gap 24).
        assert_eq!(steps[0]["checks"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn select_nodes_pins_exact_paths_and_leaves_score_none() {
        let mut graph = aether_graph::SemanticGraph::new();
        graph.upsert_node(
            aether_graph::Node::new(
                aether_graph::NodeKind::Function,
                "greet",
                "crate::calc::greet",
            )
            .with_source("fn greet() {}")
            .with_language("rust"),
        );
        let paths = vec!["crate::calc::greet".to_string()];
        let selected = select_nodes(&graph, "unused", Some(&paths)).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].node.path, "crate::calc::greet");
        assert!(selected[0].score.is_none());
    }

    #[test]
    fn select_nodes_rejects_a_pinned_path_outside_the_graph() {
        let graph = aether_graph::SemanticGraph::new();
        let paths = vec!["crate::calc::missing".to_string()];
        let error = select_nodes(&graph, "unused", Some(&paths)).unwrap_err();
        assert!(
            matches!(error, SelectionError::UnknownPath(path) if path == "crate::calc::missing")
        );
    }

    #[test]
    fn select_nodes_reports_no_matches_for_an_empty_search() {
        let graph = aether_graph::SemanticGraph::new();
        let error = select_nodes(&graph, "nothing will match this", None).unwrap_err();
        assert!(matches!(error, SelectionError::NoMatches));
    }

    #[test]
    fn build_authoring_context_embeds_path_language_and_source() {
        let mut graph = aether_graph::SemanticGraph::new();
        graph.upsert_node(
            aether_graph::Node::new(
                aether_graph::NodeKind::Function,
                "greet",
                "crate::calc::greet",
            )
            .with_source("fn greet() {}")
            .with_language("rust"),
        );
        let paths = vec!["crate::calc::greet".to_string()];
        let ctx = build_authoring_context(&graph, "unused", Some(&paths)).unwrap();
        assert_eq!(ctx.node_paths, vec!["crate::calc::greet".to_string()]);
        assert_eq!(ctx.node_context.len(), 1);
        assert_eq!(ctx.node_context[0]["path"], "crate::calc::greet");
        assert_eq!(ctx.node_context[0]["language"], "rust");
        assert_eq!(ctx.node_context[0]["source"], "fn greet() {}");
        assert_eq!(ctx.schema["properties"]["edits"]["maxItems"], 1);
    }
}
