pub fn debug(args: &[String]) -> std::io::Result<()> {
    use aether_debugger::python_tracer::PyTimeline;

    let Some(file) = args.first() else {
        eprintln!("usage: girder debug <file.py> [--what-if <var>=<val> at <step>]");
        return Ok(());
    };

    let path = std::path::Path::new(file);
    if !path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{file}: file not found"),
        ));
    }

    println!("Tracing {file} ...");
    let mut timeline = PyTimeline::record(path)?;
    let steps = timeline.branch(0).map(|b| b.trace.len()).unwrap_or(0);
    println!("  {steps} step(s) recorded\n");

    // Print main-branch trace.
    if let Some(branch) = timeline.branch(0) {
        println!("Execution trace:");
        for step in &branch.trace.steps {
            let mark = if step.intervened { "  ★" } else { "" };
            println!("  step {:4}: {}{}", step.seq, step.description, mark);
        }
        // Show final locals from the last step.
        if let Some(last) = branch.trace.steps.last() {
            if !last.locals.is_empty() {
                println!("\nFinal state:");
                for (k, v) in &last.locals {
                    println!("  {k} = {v}");
                }
            }
        }
    }

    // Parse --what-if <var>=<val> at <step>
    if let Some(wi_pos) = args.iter().position(|a| a == "--what-if") {
        let var_val = args.get(wi_pos + 1);
        let at_kw = args.get(wi_pos + 2).map(String::as_str);
        let step_str = args.get(wi_pos + 3);

        match (var_val, at_kw, step_str) {
            (Some(vv), Some("at"), Some(s)) => {
                let Some((var, val)) = vv.split_once('=') else {
                    eprintln!("  ! expected <var>=<val>, got: {vv}");
                    return Ok(());
                };
                let Ok(at_step) = s.parse::<usize>() else {
                    eprintln!("  ! expected integer step, got: {s}");
                    return Ok(());
                };

                println!("\nWhat-if branch: {var} = {val} injected at step {at_step}");
                println!("  Re-running ...");

                match timeline.fork_what_if(
                    0,
                    at_step,
                    var,
                    val,
                    &format!("what-if {var}={val}@{at_step}"),
                ) {
                    Ok(branch_id) => {
                        let wb = timeline.branch(branch_id).unwrap();
                        println!("  {} step(s) in what-if branch", wb.trace.len());
                        if let Some(div) = timeline.first_divergence(0, branch_id) {
                            println!("  first divergence at step {div}");
                        }
                        println!("\nWhat-if trace:");
                        for step in &wb.trace.steps {
                            let mark = if step.intervened { "  ★" } else { "" };
                            println!("  step {:4}: {}{}", step.seq, step.description, mark);
                        }
                        if let Some(last) = wb.trace.steps.last() {
                            if !last.locals.is_empty() {
                                println!("\nFinal state (what-if branch):");
                                for (k, v) in &last.locals {
                                    println!("  {k} = {v}");
                                }
                            }
                        }
                    }
                    Err(e) => eprintln!("  ! what-if failed: {e}"),
                }
            }
            _ => eprintln!("  ! usage: --what-if <var>=<val> at <step>"),
        }
    }

    Ok(())
}
