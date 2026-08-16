//! `bitcode context <dir> [--nodes <path>[,<path>...]] ["<intent>"] --json`
//! — read-only: builds the graph, selects nodes exactly as `bitcode do`
//! would (same [`TOP_K`](super::authoring_context::TOP_K)/
//! [`NODE_SCORE_FLOOR_RATIO`](super::authoring_context::NODE_SCORE_FLOOR_RATIO)),
//! and prints the same `{path, language, source}` node context and
//! authoring JSON Schema `do` would send to a model, plus a plan skeleton,
//! as one JSON object on stdout. No model call, no network, no writes —
//! this is `do` split at the model boundary, for pasting into an external
//! chat model that isn't wired in as a provider.

use super::authoring_context::{
    build_authoring_context, collect_words, generate_plan_id, invalid_input, parse_pinned_nodes,
    plan_skeleton, SelectionError,
};
use crate::project::config::ProjectConfig;
use crate::project::git::git_head_commit;
use crate::project::source::build_from_dir_with_config;
use serde_json::json;
use std::path::PathBuf;

const USAGE: &str =
    "usage: bitcode context <dir> [--nodes <path>[,<path>...]] [\"<intent>\"] --json";

pub fn context(args: &[String]) -> std::io::Result<()> {
    let Some(root_arg) = args.first() else {
        eprintln!("{USAGE}");
        return Ok(());
    };
    let root = PathBuf::from(root_arg);

    // `--json` is the only output mode implemented: this command exists to
    // emit exactly one pipeable/pasteable JSON object, so there is no
    // human-readable fallback to silently produce instead.
    if !args.iter().any(|arg| arg == "--json") {
        return Err(invalid_input("bitcode context requires --json"));
    }

    let pinned_node_paths = parse_pinned_nodes(args)?;
    let intent = collect_words(args, &["--json"], &["--nodes"]);

    let config = ProjectConfig::load(&root)?;
    let (graph, _builder, _files) = build_from_dir_with_config(&root, &config)?;

    let ctx = match build_authoring_context(&graph, &intent, pinned_node_paths.as_deref()) {
        Ok(ctx) => ctx,
        Err(SelectionError::NoMatches) => {
            return Err(invalid_input(&format!("no nodes matched \"{intent}\"")));
        }
        Err(SelectionError::UnknownPath(path)) => {
            return Err(invalid_input(&format!(
                "--nodes references a path not present in the graph: {path}"
            )));
        }
    };

    let base_commit = git_head_commit(&root)?;
    let plan_id = generate_plan_id();
    let output = json!({
        "intent": intent,
        "base_commit": base_commit,
        "nodes": ctx.node_context,
        "schema": ctx.schema,
        "plan_skeleton": plan_skeleton(&base_commit, &intent, &plan_id),
    });
    let rendered = serde_json::to_string_pretty(&output)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    println!("{rendered}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_a_dir_argument() {
        assert!(context(&[]).is_ok());
    }
}
