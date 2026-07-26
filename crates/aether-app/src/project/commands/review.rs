use crate::project::git::build_baseline_graph;
use crate::project::source::build_from_dir;
use aether_graph::{NodeId, NodeKind};
use std::path::PathBuf;

pub fn review(args: &[String]) -> std::io::Result<()> {
    let root = PathBuf::from(args.first().map(String::as_str).unwrap_or("."));
    let since = args
        .windows(2)
        .find(|w| w[0] == "--since")
        .map(|w| w[1].as_str())
        .unwrap_or("HEAD");

    // Build current graph from working tree.
    println!("Building current graph for {} ...", root.display());
    let (current, _builder, files) = build_from_dir(&root)?;
    println!("  {} file(s), {} nodes", files, current.node_count());

    // Build baseline graph from the git ref, file by file.
    println!("Building baseline graph from {since} ...");
    let baseline = build_baseline_graph(&root, since)?;
    println!("  {} nodes in baseline", baseline.node_count());

    // Diff the two graphs.
    let diff = current.diff_from(&baseline);

    if diff.is_empty() {
        println!("\nNo semantic changes detected vs {since}.");
        return Ok(());
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

    println!();
    println!("Semantic Review — {} vs {since}", root.display());
    println!("{}", "═".repeat(52));
    print!("  ");
    if !diff.added.is_empty() {
        print!("{} added  ", diff.added.len());
    }
    if !diff.modified.is_empty() {
        print!("{} modified  ", diff.modified.len());
    }
    if !diff.removed.is_empty() {
        print!("{} removed  ", diff.removed.len());
    }
    println!();
    println!(
        "  {} edge change(s)  ·  {} node(s) in blast radius",
        total_edges,
        impacted.len()
    );

    if !diff.added.is_empty() {
        println!("\nAdded [{}]:", diff.added.len());
        for c in &diff.added {
            println!("  + {}  [{:?}, {}]", c.path, c.kind, c.language);
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
                println!("      impacts: {}", preview.join("  ·  "));
            }
            // Test coverage for new code.
            let tests = current.tests_for(c.id);
            if tests.is_empty() {
                println!("      ! no tests cover this");
            } else {
                let names: Vec<String> = tests
                    .iter()
                    .filter_map(|&id| current.get(id).map(|n| n.name.clone()))
                    .collect();
                println!("      tests: {}", names.join(", "));
            }
        }
    }

    if !diff.modified.is_empty() {
        println!("\nModified [{}]:", diff.modified.len());
        for c in &diff.modified {
            println!("  ~ {}  [{:?}, {}]", c.path, c.kind, c.language);
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
                println!("      impacts: {}", preview.join("  ·  "));
            }
            let tests = current.tests_for(c.id);
            if tests.is_empty() {
                println!("      ! no tests cover this");
            } else {
                let names: Vec<String> = tests
                    .iter()
                    .filter_map(|&id| current.get(id).map(|n| n.name.clone()))
                    .collect();
                println!("      tests: {}", names.join(", "));
            }
        }
    }

    if !diff.removed.is_empty() {
        println!("\nRemoved [{}]:", diff.removed.len());
        for c in &diff.removed {
            println!("  - {}  [{:?}, {}]", c.path, c.kind, c.language);
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
                    println!("      previously impacted: {}", preview.join("  ·  "));
                }
                let tests = baseline.tests_for(c.id);
                if tests.is_empty() {
                    println!("      ! no baseline tests covered this");
                } else {
                    let names: Vec<String> = tests
                        .iter()
                        .filter_map(|&id| baseline.get(id).map(|n| n.name.clone()))
                        .collect();
                    println!("      baseline tests: {}", names.join(", "));
                }
            }
        }
    }

    if !diff.added_edges.is_empty() {
        println!("\nNew edges [{}]:", diff.added_edges.len());
        for (src, dst, kind) in &diff.added_edges {
            let src_name = current.get(*src).map(|n| n.path.as_str()).unwrap_or("?");
            let dst_name = current.get(*dst).map(|n| n.path.as_str()).unwrap_or("?");
            println!("  + {src_name} ──{kind:?}──▶ {dst_name}");
        }
    }

    if !diff.removed_edges.is_empty() {
        println!("\nRemoved edges [{}]:", diff.removed_edges.len());
        for (src, dst, kind) in &diff.removed_edges {
            let src_name = baseline.get(*src).map(|n| n.path.as_str()).unwrap_or("?");
            let dst_name = baseline.get(*dst).map(|n| n.path.as_str()).unwrap_or("?");
            println!("  - {src_name} ──{kind:?}──▶ {dst_name}");
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
        println!("\nTest gaps — changed functions with no test coverage:");
        for path in &uncovered {
            println!("  ! {path}");
        }
    }

    Ok(())
}
