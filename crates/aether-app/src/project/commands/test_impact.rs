use crate::project::config::{ConfiguredCommand, ProjectConfig};
use crate::project::git::semantic_changed_impact;
use crate::project::output_sink::{out, Sink};
use crate::project::process::{run_streamed, BoundedStatus};
use crate::project::source::build_from_dir_with_config;
use aether_graph::NodeId;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub fn test_impact(args: &[String]) -> std::io::Result<()> {
    let root = PathBuf::from(args.first().map(String::as_str).unwrap_or("."));
    let run = args.iter().any(|a| a == "--run");
    let quiet = args.iter().any(|a| a == "--quiet");
    let classified = args.iter().any(|a| a == "--classified");
    // Offline-evaluator-only: not exposed via the MCP schema. Skips the
    // display cap on classified must/may/unknown *paths* (boundary items
    // stay capped regardless, to bound output size) so measurement tooling
    // can see the true, uncapped classification instead of a display-capped
    // slice, per the frozen policy's "explicit unbounded output mode ...
    // for offline evaluators".
    let unbounded = args.iter().any(|a| a == "--unbounded");
    let out_path = args
        .windows(2)
        .find(|w| w[0] == "--out")
        .map(|w| PathBuf::from(&w[1]));
    let explicit: Vec<&str> = {
        let mut words = Vec::new();
        let mut skip_next = false;
        for arg in args.get(1..).unwrap_or_default() {
            if skip_next {
                skip_next = false;
                continue;
            }
            match arg.as_str() {
                "--run" | "--quiet" | "--classified" | "--unbounded" => continue,
                "--out" => {
                    skip_next = true;
                    continue;
                }
                other => words.push(other),
            }
        }
        words
    };

    let mut sink = if out_path.is_some() {
        Sink::Buffer(String::new())
    } else {
        Sink::Stdout
    };
    // `--out` cannot capture `--run`'s subprocess output: `run_streamed`
    // inherits stdout/stderr for a live terminal, bypassing this sink by
    // construction. `--out` still captures the impacted-test listing and
    // commands; live `--run` output keeps streaming to the real terminal.
    let mut run_outcome: Option<String> = None;

    if !quiet {
        out!(sink, "Building semantic graph for {} ...", root.display());
    }
    let config = ProjectConfig::load(&root)?;
    let (graph, _builder, files) = build_from_dir_with_config(&root, &config)?;
    if quiet && classified && !run && out_path.is_none() {
        print!(
            "{}",
            classified_from_graph(&root, &graph, &config, &explicit, unbounded)?
        );
        return Ok(());
    }
    if quiet && !run && out_path.is_none() {
        print!("{}", quiet_from_graph(&root, &graph, &config, &explicit)?);
        return Ok(());
    }

    if !quiet {
        out!(
            sink,
            "  {} file(s), {} nodes, {} edges",
            files,
            graph.node_count(),
            graph.edge_count()
        );
    }

    let mut baseline_test_paths: Vec<String> = Vec::new();

    let origin_ids: Vec<NodeId> = if explicit.is_empty() {
        if let Some(impact) = semantic_changed_impact(&root, &graph)? {
            if impact.changed_files.is_empty()
                && impact.origin_ids.is_empty()
                && impact.baseline_test_paths.is_empty()
            {
                if !quiet {
                    out!(sink, "  no semantic changes detected vs HEAD");
                }
                finish_with_summary(sink, out_path.as_deref(), "0 impacted test(s)")?;
                return Ok(());
            }
            if !impact.changed_files.is_empty() && !quiet {
                out!(sink, "  changed files: {}", impact.changed_files.join(", "));
            }
            baseline_test_paths = impact.baseline_test_paths;
            impact.origin_ids
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "automatic test-impact requires a Git repository; pass explicit node paths instead",
            ));
        }
    } else {
        let mut resolved = Vec::new();
        let mut missing = Vec::new();
        for path in explicit {
            match graph.find_by_path(path) {
                Some(node) => resolved.push(node.id),
                None => missing.push(path),
            }
        }
        if !missing.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("unknown explicit node path(s): {}", missing.join(", ")),
            ));
        }
        resolved
    };

    if origin_ids.is_empty() && baseline_test_paths.is_empty() {
        if !quiet {
            out!(sink, "  no functions found for the changed files");
        }
        finish_with_summary(sink, out_path.as_deref(), "0 impacted test(s)")?;
        return Ok(());
    }

    if !quiet {
        if !origin_ids.is_empty() {
            out!(sink, "\nChanged functions ({}):", origin_ids.len());
            for id in &origin_ids {
                if let Some(n) = graph.get(*id) {
                    out!(sink, "  · {}", n.path);
                }
            }
        } else {
            out!(sink, "\nNo currently present changed functions.");
            out!(
                sink,
                "  Removed functions were detected; using their baseline test coverage."
            );
        }
    }

    let mut test_ids = graph.tests_for_nodes(&origin_ids);
    let mut seen: HashSet<NodeId> = test_ids.iter().copied().collect();
    for path in &baseline_test_paths {
        if let Some(node) = graph.find_by_path(path) {
            if seen.insert(node.id) {
                test_ids.push(node.id);
            }
        }
    }

    if test_ids.is_empty() {
        if !quiet {
            out!(sink, "\nNo tests found in the impact set.");
            out!(
                sink,
                "  The changed functions have no test coverage reachable via the call graph."
            );
            out!(
                sink,
                "  Consider adding tests, or run the full suite to be safe."
            );
        }
        finish_with_summary(sink, out_path.as_deref(), "0 impacted test(s)")?;
        return Ok(());
    }

    let mut rust_tests: Vec<String> = Vec::new();
    let mut python_tests: Vec<String> = Vec::new();
    let mut go_tests: Vec<String> = Vec::new();

    if !quiet {
        out!(sink, "\nImpacted tests ({}):", test_ids.len());
    }
    for id in &test_ids {
        if let Some(n) = graph.get(*id) {
            if quiet {
                // Every id here already came from `tests_for`/`tests_for_nodes`,
                // which identify test nodes by the `is_test` attribute with no
                // language filter — printing must not narrow that set back
                // down, or a language the graph already recognizes as tested
                // goes silently missing from `--quiet` output.
                out!(sink, "{}", n.name);
            } else {
                out!(sink, "  ✓ {} ({})", n.path, n.language);
            }
            match n.language.as_str() {
                "rust" => rust_tests.push(n.name.clone()),
                "python" => python_tests.push(n.name.clone()),
                "go" => go_tests.push(n.name.clone()),
                _ => {}
            }
        }
    }

    if !quiet {
        out!(sink, "\nCommands to run impacted tests only:");
    }
    let mut commands = Vec::new();
    for name in &rust_tests {
        if let Some(command) = config.rust_test_command(name) {
            if !quiet {
                out!(sink, "  {}", command.display());
            }
            commands.push(("Rust", command));
        }
    }
    if !python_tests.is_empty() {
        if let Some(command) = config.python_test_command(&python_tests.join(" or ")) {
            if !quiet {
                out!(sink, "  {}", command.display());
            }
            commands.push(("Python", command));
        }
    }
    if !go_tests.is_empty() {
        if let Some(command) = config.go_test_command(&go_tests.join("|")) {
            if !quiet {
                out!(sink, "  {}", command.display());
            }
            commands.push(("Go", command));
        }
    }
    if commands.is_empty() && !quiet {
        out!(sink, "  (test commands are disabled in girder.toml)");
    }

    // A real set difference, not count arithmetic: selected ids that are not
    // graph test nodes (e.g. baseline-only paths) must not distort the count.
    let selected: HashSet<NodeId> = test_ids.iter().copied().collect();
    let skipped = graph
        .nodes()
        .filter(|n| n.attr("is_test").is_some() && !selected.contains(&n.id))
        .count();
    if skipped > 0 && !quiet {
        out!(
            sink,
            "\n  ({skipped} other test(s) not in impact set — skipped)"
        );
    }

    if run {
        if commands.is_empty() {
            return Err(std::io::Error::other(
                "cannot run impacted tests because no test commands are configured",
            ));
        }
        if !quiet {
            out!(sink, "\nRunning ...");
        }
        let mut failures = Vec::new();
        for (language, command) in &commands {
            let status = run_command(command, &root, &config)?;
            if !status.success() {
                failures.push(format!(
                    "{language} command exited with {}: {}",
                    status
                        .code()
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "a signal".into()),
                    command.display()
                ));
            }
        }
        run_outcome = Some(if failures.is_empty() {
            format!("{} passed", commands.len())
        } else {
            format!("FAILED: {}", failures.join("; "))
        });
        if !failures.is_empty() {
            let summary = format!(
                "{} impacted test(s), {}",
                test_ids.len(),
                run_outcome.as_deref().unwrap_or_default()
            );
            finish_with_summary(sink, out_path.as_deref(), &summary)?;
            return Err(std::io::Error::other(failures.join("; ")));
        }
    }

    let summary = match &run_outcome {
        Some(outcome) => format!("{} impacted test(s), {outcome}", test_ids.len()),
        None => format!("{} impacted test(s)", test_ids.len()),
    };
    finish_with_summary(sink, out_path.as_deref(), &summary)?;
    Ok(())
}

fn finish_with_summary(
    sink: Sink,
    out_path: Option<&std::path::Path>,
    summary: &str,
) -> std::io::Result<()> {
    if let (Sink::Buffer(_), Some(path)) = (&sink, out_path) {
        println!("test-impact: {summary} -> {}", path.display());
    }
    sink.finish(out_path)
}

/// Execute one configured test command with hard time and output bounds.
///
/// Output streams live to the terminal and stdin stays inherited, but the
/// child runs in its own process group: exceeding the configured timeout or
/// output budget kills the entire tree and fails with an explicit
/// classification instead of leaving descendants running.
fn run_command(
    command: &ConfiguredCommand,
    root: &std::path::Path,
    config: &ProjectConfig,
) -> std::io::Result<std::process::ExitStatus> {
    let mut child = std::process::Command::new(&command.program);
    child.args(&command.args).current_dir(root);
    let timeout = std::time::Duration::from_secs(config.tests.run_timeout_seconds);
    let run = run_streamed(child, timeout, config.tests.run_max_output_bytes).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("could not start {}: {error}", command.display()),
        )
    })?;
    match run.status {
        BoundedStatus::Completed(status) => Ok(status),
        BoundedStatus::TimedOut => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!(
                "test command `{}` timed out after {}s; its process tree was killed",
                command.display(),
                timeout.as_secs()
            ),
        )),
        BoundedStatus::OutputLimited => Err(std::io::Error::other(format!(
            "test command `{}` produced more than {} bytes of output; \
             its process tree was killed",
            command.display(),
            config.tests.run_max_output_bytes
        ))),
        BoundedStatus::Cancelled => Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            format!("test command `{}` was cancelled", command.display()),
        )),
    }
}

/// Resolves the same origin set `quiet_from_graph`/`classified_from_graph` and
/// the non-quiet `test_impact` path all start from: explicit node paths when
/// given, otherwise the current git diff.
fn resolve_origins(
    root: &Path,
    graph: &aether_graph::SemanticGraph,
    config: &ProjectConfig,
    explicit: &[&str],
) -> std::io::Result<(Vec<NodeId>, Vec<String>)> {
    if explicit.is_empty() {
        let impact = crate::project::git::semantic_changed_impact_with_config(root, graph, config)?
            .ok_or_else(|| {
                std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "automatic test-impact requires a Git repository; pass explicit node paths instead",
        )
            })?;
        Ok((impact.origin_ids, impact.baseline_test_paths))
    } else {
        let mut ids = Vec::new();
        let mut missing = Vec::new();
        for path in explicit {
            if let Some(node) = graph.find_by_path(path) {
                ids.push(node.id);
            } else {
                missing.push(*path);
            }
        }
        if !missing.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("unknown explicit node path(s): {}", missing.join(", ")),
            ));
        }
        Ok((ids, Vec::new()))
    }
}

/// Sorts node ids by their graph path (breaking ties by id), the ordering
/// the frozen classification policy requires for every labeled set.
fn sort_by_path(graph: &aether_graph::SemanticGraph, ids: &mut [NodeId]) {
    ids.sort_by(|a, b| {
        let left = graph.get(*a).map(|n| n.path.as_str()).unwrap_or_default();
        let right = graph.get(*b).map(|n| n.path.as_str()).unwrap_or_default();
        left.cmp(right).then_with(|| a.cmp(b))
    });
}

/// The read-only quiet command shared with MCP: the conservative union of
/// every test whose coverage could not be *excluded* — must, may, and
/// unknown alike — per docs/call-classification-policy.md ("Quiet CLI test
/// selection returns the conservative union of known test paths in all
/// three classes ... an empty result never means nothing needs testing").
///
/// This is deliberately NOT the same test set `tests_for_nodes`/`impact_of`
/// compute on their own (those stay unchanged for their other callers, e.g.
/// the advisory hook) — this union is evidence-classified, so it grows to
/// include every test once a single unresolved call anywhere in the graph
/// makes exclusion unprovable. On real code that is common, so this
/// selection is frequently close to the full test inventory; that is the
/// intended, honest behavior while Must/May coverage is still thin (Stage 1),
/// not a bug. No test subprocess is launched here.
pub(super) fn quiet_from_graph(
    root: &Path,
    graph: &aether_graph::SemanticGraph,
    config: &ProjectConfig,
    explicit: &[&str],
) -> std::io::Result<String> {
    let (origin_ids, baseline_paths) = resolve_origins(root, graph, config, explicit)?;
    let classified = graph.classified_impact(&origin_ids).tests(graph);
    let mut ids: Vec<NodeId> = classified
        .must
        .into_iter()
        .chain(classified.may)
        .chain(classified.unknown)
        .collect();
    let mut seen: HashSet<NodeId> = ids.iter().copied().collect();
    for path in baseline_paths {
        if let Some(node) = graph.find_by_path(&path) {
            if seen.insert(node.id) {
                ids.push(node.id);
            }
        }
    }
    sort_by_path(graph, &mut ids);
    if !classified.boundaries.is_empty() {
        eprintln!(
            "test-impact: {} unresolved call-evidence boundar{} found; this \
             selection is the conservative must∪may∪unknown union, not a \
             targeted answer — pass --classified to see why each test is \
             included (docs/call-classification-policy.md)",
            classified.boundaries.len(),
            if classified.boundaries.len() == 1 {
                "y"
            } else {
                "ies"
            }
        );
    }
    Ok(ids
        .into_iter()
        .filter_map(|id| graph.get(id))
        .map(|n| format!("{}\n", n.name))
        .collect())
}

/// Cap on how many paths or boundary records one section lists in full before
/// degrading to a count — mirrors `orient`'s `MAX_LISTED_PER_SECTION`. `count`
/// is always the true total; only the listed items are capped. `--unbounded`
/// (CLI-only, not exposed via MCP) lifts the cap on the must/may/unknown
/// *path* lists for offline measurement tooling; boundary items stay capped
/// regardless; see `docs/call-classification-policy.md`'s "An explicit
/// unbounded output mode may be used by offline evaluators."
const MAX_LISTED: usize = 50;

/// Three-way split of *why* a boundary exists, per the frozen policy's
/// "report all three kinds with reasons": a call site whose target could not
/// be resolved; a whole-file/module coverage gap (unexpanded macro, parse
/// error, a language construct this extractor doesn't certify); or evidence
/// that is missing, stale, invalid, or backed only by an uncertified legacy
/// edge. Matches the reason strings `mapper/claims.rs` and
/// `aether-graph/src/claims.rs` actually emit.
pub(super) fn boundary_category(reason: &str, coverage_gap: bool) -> &'static str {
    const MISSING_OR_INVALID_EVIDENCE: &[&str] = &[
        "missing-node",
        "missing-call-evidence",
        "invalid-call-evidence",
        "stale-call-evidence",
        "unsupported-call-evidence-version",
        "invalid-call-claim",
    ];
    if MISSING_OR_INVALID_EVIDENCE.contains(&reason) || reason.starts_with("uncertified-") {
        "missing_or_invalid_evidence"
    } else if coverage_gap {
        "coverage_gap"
    } else {
        "unresolved_call_site"
    }
}

/// Labeled Must/May/Unknown test selection, per the frozen classification
/// policy (`docs/call-classification-policy.md`): the tests reachable from
/// `origin_ids` via independently certified call evidence, split by the
/// strength of that evidence, plus every unresolved boundary that could hide
/// a reachable test. A test reached only through a removed origin's baseline
/// coverage has no evidence path to classify, so it is conservatively placed
/// in `unknown` rather than silently dropped.
pub(super) fn classified_from_graph(
    root: &Path,
    graph: &aether_graph::SemanticGraph,
    config: &ProjectConfig,
    explicit: &[&str],
    unbounded: bool,
) -> std::io::Result<String> {
    let (origin_ids, baseline_paths) = resolve_origins(root, graph, config, explicit)?;
    let mut classified = graph.classified_impact(&origin_ids).tests(graph);
    let mut seen: HashSet<NodeId> = classified
        .must
        .iter()
        .chain(&classified.may)
        .chain(&classified.unknown)
        .copied()
        .collect();
    for path in baseline_paths {
        if let Some(node) = graph.find_by_path(&path) {
            if seen.insert(node.id) {
                classified.unknown.push(node.id);
            }
        }
    }
    sort_by_path(graph, &mut classified.unknown);

    let paths = |ids: &[NodeId]| -> serde_json::Value {
        let mut paths: Vec<&str> = ids
            .iter()
            .filter_map(|&id| graph.get(id))
            .map(|n| n.path.as_str())
            .collect();
        paths.sort_unstable();
        let count = paths.len();
        let truncated = !unbounded && count > MAX_LISTED;
        if !unbounded {
            paths.truncate(MAX_LISTED);
        }
        serde_json::json!({"count": count, "paths": paths, "truncated": truncated})
    };
    let boundary_count = classified.boundaries.len();
    let boundary_truncated = boundary_count > MAX_LISTED;
    let mut by_category: std::collections::BTreeMap<&'static str, usize> =
        std::collections::BTreeMap::new();
    for boundary in &classified.boundaries {
        *by_category
            .entry(boundary_category(&boundary.reason, boundary.coverage_gap))
            .or_insert(0) += 1;
    }
    let boundaries: Vec<_> = classified
        .boundaries
        .into_iter()
        .take(MAX_LISTED)
        .filter_map(|b| serde_json::to_value(b).ok())
        .collect();

    let document = serde_json::json!({
        "schema_version": 1,
        "must": paths(&classified.must),
        "may": paths(&classified.may),
        "unknown": paths(&classified.unknown),
        "boundaries": {
            "count": boundary_count,
            "by_category": by_category,
            "items": boundaries,
            "truncated": boundary_truncated,
        },
    });
    serde_json::to_string_pretty(&document)
        .map(|s| s + "\n")
        .map_err(std::io::Error::other)
}
