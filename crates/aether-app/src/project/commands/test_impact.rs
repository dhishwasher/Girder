use crate::project::config::{ConfiguredCommand, ProjectConfig};
use crate::project::git::semantic_changed_impact;
use crate::project::process::{run_streamed, BoundedStatus};
use crate::project::source::build_from_dir_with_config;
use aether_graph::NodeId;
use std::collections::HashSet;
use std::path::PathBuf;

pub fn test_impact(args: &[String]) -> std::io::Result<()> {
    let root = PathBuf::from(args.first().map(String::as_str).unwrap_or("."));
    let run = args.iter().any(|a| a == "--run");
    let explicit: Vec<&str> = args
        .get(1..)
        .unwrap_or_default()
        .iter()
        .filter(|a| *a != "--run")
        .map(String::as_str)
        .collect();

    println!("Building semantic graph for {} ...", root.display());
    let config = ProjectConfig::load(&root)?;
    let (graph, _builder, files) = build_from_dir_with_config(&root, &config)?;
    println!(
        "  {} file(s), {} nodes, {} edges",
        files,
        graph.node_count(),
        graph.edge_count()
    );

    let mut baseline_test_paths: Vec<String> = Vec::new();

    let origin_ids: Vec<NodeId> = if explicit.is_empty() {
        if let Some(impact) = semantic_changed_impact(&root, &graph)? {
            if impact.changed_files.is_empty()
                && impact.origin_ids.is_empty()
                && impact.baseline_test_paths.is_empty()
            {
                println!("  no semantic changes detected vs HEAD");
                return Ok(());
            }
            if !impact.changed_files.is_empty() {
                println!("  changed files: {}", impact.changed_files.join(", "));
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
        println!("  no functions found for the changed files");
        return Ok(());
    }

    if !origin_ids.is_empty() {
        println!("\nChanged functions ({}):", origin_ids.len());
        for id in &origin_ids {
            if let Some(n) = graph.get(*id) {
                println!("  · {}", n.path);
            }
        }
    } else {
        println!("\nNo currently present changed functions.");
        println!("  Removed functions were detected; using their baseline test coverage.");
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
        println!("\nNo tests found in the impact set.");
        println!("  The changed functions have no test coverage reachable via the call graph.");
        println!("  Consider adding tests, or run the full suite to be safe.");
        return Ok(());
    }

    let mut rust_tests: Vec<String> = Vec::new();
    let mut python_tests: Vec<String> = Vec::new();

    println!("\nImpacted tests ({}):", test_ids.len());
    for id in &test_ids {
        if let Some(n) = graph.get(*id) {
            println!("  ✓ {} ({})", n.path, n.language);
            match n.language.as_str() {
                "rust" => rust_tests.push(n.name.clone()),
                "python" => python_tests.push(n.name.clone()),
                _ => {}
            }
        }
    }

    println!("\nCommands to run impacted tests only:");
    let mut commands = Vec::new();
    for name in &rust_tests {
        if let Some(command) = config.rust_test_command(name) {
            println!("  {}", command.display());
            commands.push(("Rust", command));
        }
    }
    if !python_tests.is_empty() {
        if let Some(command) = config.python_test_command(&python_tests.join(" or ")) {
            println!("  {}", command.display());
            commands.push(("Python", command));
        }
    }
    if commands.is_empty() {
        println!("  (test commands are disabled in bitcode.toml)");
    }

    let skipped = graph
        .nodes()
        .filter(|n| n.attr("is_test").is_some())
        .count()
        .saturating_sub(test_ids.len());
    if skipped > 0 {
        println!("\n  ({skipped} other test(s) not in impact set — skipped)");
    }

    if run {
        if commands.is_empty() {
            return Err(std::io::Error::other(
                "cannot run impacted tests because no test commands are configured",
            ));
        }
        println!("\nRunning ...");
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
        if !failures.is_empty() {
            return Err(std::io::Error::other(failures.join("; ")));
        }
    }

    Ok(())
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
