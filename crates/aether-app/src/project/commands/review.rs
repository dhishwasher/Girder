use crate::project::config::ProjectConfig;
use crate::project::git::build_baseline_graph_with_config;
use crate::project::output_sink::{out, Sink};
use crate::project::source::build_from_dir_with_config;
use aether_graph::{NodeId, NodeKind};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub fn review(args: &[String]) -> std::io::Result<()> {
    let root = PathBuf::from(args.first().map(String::as_str).unwrap_or("."));
    let since = args
        .windows(2)
        .find(|w| w[0] == "--since")
        .map(|w| w[1].as_str())
        .unwrap_or("HEAD");
    let quiet = args.iter().any(|a| a == "--quiet");
    let out_path = args
        .windows(2)
        .find(|w| w[0] == "--out")
        .map(|w| PathBuf::from(&w[1]));

    // `--quiet` already trims output to just changed paths -- there is
    // nothing larger left for `--out` to usefully redirect.
    if quiet && out_path.is_some() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "girder review: --out cannot be combined with --quiet",
        ));
    }

    let mut sink = if out_path.is_some() {
        Sink::Buffer(String::new())
    } else {
        Sink::Stdout
    };

    let config = ProjectConfig::load(&root)?;
    let (current, _, files) = build_from_dir_with_config(&root, &config)?;
    let summary = review_into(&root, &current, files, &config, since, quiet, &mut sink)?;
    if let Some(path) = &out_path {
        println!("{summary} -> {}", path.display());
    }
    sink.finish(out_path.as_deref())
}

pub(super) fn review_into(
    root: &Path,
    current: &aether_graph::SemanticGraph,
    files: usize,
    config: &ProjectConfig,
    since: &str,
    quiet: bool,
    sink: &mut Sink,
) -> std::io::Result<String> {
    // Build current graph from working tree.
    if !quiet {
        out!(sink, "Building current graph for {} ...", root.display());
    }
    if !quiet {
        out!(sink, "  {} file(s), {} nodes", files, current.node_count());
    }

    // Build baseline graph from the git ref, file by file.
    if !quiet {
        out!(sink, "Building baseline graph from {since} ...");
    }
    let baseline = build_baseline_graph_with_config(root, since, config)?;
    if !quiet {
        out!(sink, "  {} nodes in baseline", baseline.node_count());
    }

    // Diff the two graphs.
    let diff = current.diff_from(&baseline);

    if diff.is_empty() {
        if !quiet {
            out!(sink, "\nNo semantic changes detected vs {since}.");
        }
        return Ok(format!("review: no semantic changes vs {since}"));
    }

    if quiet {
        for c in diff
            .added
            .iter()
            .chain(diff.modified.iter())
            .chain(diff.removed.iter())
        {
            out!(sink, "{}", c.path);
        }
        return Ok(String::new());
    }

    let total_edges = diff.added_edges.len() + diff.removed_edges.len();

    // Compute total impact radius across all changed nodes.
    let mut impacted: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
    for id in diff.changed_node_ids() {
        if current.get(id).is_some() {
            for &nid in current.impact_of(id).affected.keys() {
                impacted.insert(nid);
            }
        }
    }

    out!(sink, "");
    out!(sink, "Semantic Review — {} vs {since}", root.display());
    out!(sink, "{}", "═".repeat(52));
    let mut summary_parts = Vec::new();
    if !diff.added.is_empty() {
        summary_parts.push(format!("{} added", diff.added.len()));
    }
    if !diff.modified.is_empty() {
        summary_parts.push(format!("{} modified", diff.modified.len()));
    }
    if !diff.removed.is_empty() {
        summary_parts.push(format!("{} removed", diff.removed.len()));
    }
    let summary_line = format!("  {}", summary_parts.join("  "));
    out!(sink, "{summary_line}");
    out!(
        sink,
        "  {} edge change(s)  ·  {} node(s) in blast radius",
        total_edges,
        impacted.len()
    );

    if !diff.added.is_empty() {
        out!(sink, "\nAdded [{}]:", diff.added.len());
        for c in &diff.added {
            out!(sink, "  + {}  [{:?}, {}]", c.path, c.kind, c.language);
            // Show impact of new nodes.
            let impact = current.impact_of(c.id);
            if !impact.is_empty() {
                let ranked = impact.ranked();
                let preview: Vec<String> = ranked
                    .iter()
                    .take(3)
                    .filter_map(|(id, dist)| {
                        current.get(*id).map(|n| format!("{} (d{})", n.path, dist))
                    })
                    .collect();
                out!(sink, "      impacts: {}", preview.join("  ·  "));
            }
            // Test coverage for new code.
            let tests = current.tests_for(c.id);
            if tests.is_empty() {
                out!(sink, "      ! no tests cover this");
            } else {
                let names: Vec<String> = tests
                    .iter()
                    .filter_map(|&id| current.get(id).map(|n| n.name.clone()))
                    .collect();
                out!(sink, "      tests: {}", names.join(", "));
            }
        }
    }

    if !diff.modified.is_empty() {
        out!(sink, "\nModified [{}]:", diff.modified.len());
        for c in &diff.modified {
            out!(sink, "  ~ {}  [{:?}, {}]", c.path, c.kind, c.language);
            let impact = current.impact_of(c.id);
            if !impact.is_empty() {
                let ranked = impact.ranked();
                let preview: Vec<String> = ranked
                    .iter()
                    .take(3)
                    .filter_map(|(id, dist)| {
                        current.get(*id).map(|n| format!("{} (d{})", n.path, dist))
                    })
                    .collect();
                out!(sink, "      impacts: {}", preview.join("  ·  "));
            }
            let tests = current.tests_for(c.id);
            if tests.is_empty() {
                out!(sink, "      ! no tests cover this");
            } else {
                let names: Vec<String> = tests
                    .iter()
                    .filter_map(|&id| current.get(id).map(|n| n.name.clone()))
                    .collect();
                out!(sink, "      tests: {}", names.join(", "));
            }
        }
    }

    if !diff.removed.is_empty() {
        out!(sink, "\nRemoved [{}]:", diff.removed.len());
        for c in &diff.removed {
            out!(sink, "  - {}  [{:?}, {}]", c.path, c.kind, c.language);
            if c.kind == NodeKind::Function {
                let impact = baseline.impact_of(c.id);
                if !impact.is_empty() {
                    let ranked = impact.ranked();
                    let preview: Vec<String> = ranked
                        .iter()
                        .take(3)
                        .filter_map(|(id, dist)| {
                            baseline.get(*id).map(|n| format!("{} (d{})", n.path, dist))
                        })
                        .collect();
                    out!(sink, "      previously impacted: {}", preview.join("  ·  "));
                }
                let tests = baseline.tests_for(c.id);
                if tests.is_empty() {
                    out!(sink, "      ! no baseline tests covered this");
                } else {
                    let names: Vec<String> = tests
                        .iter()
                        .filter_map(|&id| baseline.get(id).map(|n| n.name.clone()))
                        .collect();
                    out!(sink, "      baseline tests: {}", names.join(", "));
                }
            }
        }
    }

    if !diff.added_edges.is_empty() {
        out!(sink, "\nNew edges [{}]:", diff.added_edges.len());
        for (src, dst, kind) in &diff.added_edges {
            let src_name = current.get(*src).map(|n| n.path.as_str()).unwrap_or("?");
            let dst_name = current.get(*dst).map(|n| n.path.as_str()).unwrap_or("?");
            out!(sink, "  + {src_name} ──{kind:?}──▶ {dst_name}");
        }
    }

    if !diff.removed_edges.is_empty() {
        out!(sink, "\nRemoved edges [{}]:", diff.removed_edges.len());
        for (src, dst, kind) in &diff.removed_edges {
            let src_name = baseline.get(*src).map(|n| n.path.as_str()).unwrap_or("?");
            let dst_name = baseline.get(*dst).map(|n| n.path.as_str()).unwrap_or("?");
            out!(sink, "  - {src_name} ──{kind:?}──▶ {dst_name}");
        }
    }

    // Test gap summary.
    let uncovered: Vec<String> = diff
        .changed_node_ids()
        .iter()
        .filter(|&&id| {
            current
                .get(id)
                .map(|n| n.kind == NodeKind::Function)
                .unwrap_or(false)
                && current.tests_for(id).is_empty()
        })
        .filter_map(|&id| current.get(id).map(|n| n.path.clone()))
        .collect();

    if !uncovered.is_empty() {
        out!(
            sink,
            "\nTest gaps — changed functions with no test coverage:"
        );
        for path in &uncovered {
            out!(sink, "  ! {path}");
        }
    }

    Ok(format!(
        "review: {}, {} test gap(s)",
        summary_parts.join(", "),
        uncovered.len()
    ))
}
