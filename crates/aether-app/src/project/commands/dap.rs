use crate::project::source::build_from_dir;

pub async fn dap(args: &[String]) -> std::io::Result<()> {
    use aether_dap::{DapError, DebugManager};

    let program = match args.first() {
        Some(p) => p.clone(),
        None => {
            eprintln!("usage: bitcode dap <program> [--adapter python|rust|<path>] [--break-at <node::path>]");
            return Ok(());
        }
    };

    // Parse flags.
    let adapter_flag = args
        .windows(2)
        .find(|w| w[0] == "--adapter")
        .map(|w| w[1].as_str())
        .unwrap_or("python");
    let break_at: Vec<&str> = args
        .windows(2)
        .filter(|w| w[0] == "--break-at")
        .map(|w| w[1].as_str())
        .collect();
    let dry_run = args.iter().any(|a| a == "--dry-run");

    let (adapter_cmd, adapter_args): (&str, Vec<&str>) = match adapter_flag {
        "python" => ("python", vec!["-m", "debugpy.adapter"]),
        "rust" => ("codelldb", vec!["--port", "0", "--adapter"]),
        path => (path, vec![]),
    };

    println!("DAP debug session plan");
    println!("  adapter : {adapter_cmd} {}", adapter_args.join(" "));
    println!("  program : {program}");

    // Build semantic graph from the program's directory so we can resolve
    // --break-at node paths to file:line pairs.
    let prog_path = std::path::PathBuf::from(&program);
    let root = prog_path.parent().unwrap_or(std::path::Path::new("."));
    let (graph, _builder, files) = build_from_dir(root)?;
    println!(
        "  graph   : {files} file(s), {} nodes\n",
        graph.node_count()
    );

    // Resolve --break-at paths to file:line.
    let mut resolved_breakpoints: Vec<(String, String, u32)> = Vec::new();
    for node_path in &break_at {
        match graph.find_by_path(node_path) {
            Some(node) if node.file.is_some() => {
                let file = node.file.as_deref().unwrap();
                let line = node.span.start_row as u32 + 1;
                println!("  breakpoint: {} → {}:{}", node_path, file, line);
                resolved_breakpoints.push((node_path.to_string(), file.to_string(), line));
            }
            Some(_) => println!("  ! {node_path}: node found but has no source location"),
            None => println!("  ! {node_path}: not found in graph"),
        }
    }

    if dry_run {
        println!("\n(dry run — no adapter launched)");
        return Ok(());
    }

    // Launch the real debug session.
    let graph = std::sync::Arc::new(std::sync::Mutex::new(graph));
    let mut mgr = DebugManager::new(graph.clone());

    let adapter_args_refs: Vec<&str> = adapter_args.clone();
    let session_id = match mgr
        .launch_adapter(adapter_cmd, &adapter_args_refs, adapter_flag)
        .await
    {
        Ok(id) => {
            println!("Session {} initialized.", id);
            id
        }
        Err(e) => {
            eprintln!("  ! could not start adapter: {e}");
            eprintln!("  Tip: install the adapter (e.g. `pip install debugpy`) and retry.");
            eprintln!("  Use --dry-run to see the plan without launching.");
            return Ok(());
        }
    };

    // Begin launch. DAP adapters emit `initialized` after this request, opening
    // the configuration window in which breakpoints must be sent.
    let launch_args = serde_json::json!({
        "program": program,
        "stopOnEntry": true,
    });
    if let Some(session) = mgr.session_mut(session_id) {
        if let Err(e) = session.launch(launch_args).await {
            eprintln!("  ! launch failed: {e}");
            return Ok(());
        }
    }

    // Set breakpoints during the adapter's configuration window.
    for (node_path, file, line) in &resolved_breakpoints {
        use aether_dap::types::SourceBreakpoint;
        use std::path::Path;
        if let Some(session) = mgr.session_mut(session_id) {
            match session
                .set_breakpoints(Path::new(file), &[SourceBreakpoint::at_line(*line)])
                .await
            {
                Ok(bps) => {
                    let verified = bps.iter().filter(|b| b.verified).count();
                    println!(
                        "  ✓ breakpoint at {node_path} ({verified}/{} verified)",
                        bps.len()
                    );
                }
                Err(e) => eprintln!("  ! set_breakpoints failed: {e}"),
            }
        }
    }

    // Complete configuration and allow the debuggee to run.
    if let Some(session) = mgr.session_mut(session_id) {
        if let Err(e) = session.configuration_done().await {
            eprintln!("  ! configurationDone failed: {e}");
            return Ok(());
        }
    }

    println!("\nRunning — waiting for first stop ...");

    // Wait for stop, print stack trace with graph context.
    let stopped = {
        let session = mgr.session_mut(session_id).unwrap();
        match session.wait_for_stopped().await {
            Ok(s) => s,
            Err(DapError::AdapterExited) => {
                println!("  Debuggee exited without stopping.");
                return Ok(());
            }
            Err(e) => {
                eprintln!("  ! wait_for_stopped: {e}");
                return Ok(());
            }
        }
    };

    println!("Stopped: {}", stopped.reason);
    if let Some(node_id) = mgr.on_stopped(session_id, &stopped).await.unwrap_or(None) {
        let g = graph.lock().unwrap();
        if let Some(node) = g.get(node_id) {
            println!("  graph node: {} [{:?}]", node.path, node.kind);
        }
    }

    let thread_id = stopped.thread_id.unwrap_or(1);
    if let Some(session) = mgr.session(session_id) {
        match session.stack_trace(thread_id, 5).await {
            Ok(frames) => {
                println!("\nStack trace:");
                for (i, frame) in frames.iter().enumerate() {
                    let file = frame
                        .source
                        .as_ref()
                        .and_then(|s| s.path.as_deref())
                        .unwrap_or("?");
                    println!("  #{i} {} at {}:{}", frame.name, file, frame.line);
                }
            }
            Err(e) => eprintln!("  ! stack_trace: {e}"),
        }
    }

    // Disconnect cleanly.
    if let Some(session) = mgr.session_mut(session_id) {
        let _ = session.disconnect().await;
    }
    println!("\nSession terminated.");
    Ok(())
}
