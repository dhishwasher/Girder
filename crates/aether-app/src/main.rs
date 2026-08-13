//! Bit Code entry point.
//!
//! Default (`cargo run -p aether-app`) runs the headless end-to-end demo — it
//! needs no display, GPU, or API key. The full egui/wgpu GUI is compiled in with
//! `--features gui` (`cargo run -p aether-app --features gui`).

mod project;
mod smoke;

#[cfg(feature = "gui")]
mod app;
#[cfg(feature = "gui")]
mod graph_view;
#[cfg(feature = "gui")]
mod highlight;
#[cfg(feature = "gui")]
mod panels;

const USAGE: &str = "\
Bit Code

USAGE:
    bitcode [COMMAND]

COMMANDS:
    demo                      Run the headless end-to-end pipeline demo (default)
    config <dir> [--init]     Show the validated effective project configuration.
                              --init creates bitcode.toml without overwriting.
    analyze <dir> [--json]    Build the semantic graph from a project directory,
                              report likely duplicates, save the configured graph.
                              --json emits one bounded machine-readable summary.
    search <dir> <query...>   Concept search: rank functions by relevance to a
                              natural-language query
    swarm-plan <dir> <intent...>
                              Preview what the swarm would build: runs the
                              graph-aware Planner, prints the multi-function
                              feature spec, but writes nothing to the graph.
    forge <dir> <intent...>   Dispatch the agent swarm on a project with a
                              natural-language intent, then save the graph
    plan validate <plan.json>
                              Check a Bit Code plan file's preconditions
                              (clean worktree, HEAD == base_commit, edit
                              paths, exact match counts) without writing
                              anything.
    plan explain <plan.json> Print a human-readable summary of a plan file.
                              No execution, no preconditions.
    plan run <plan.json> [--dry] [--authoring-receipt <receipt.json>]
                              Execute a plan file step by step: apply edits
                              in a disposable copy, run each step's checks,
                              commit to the real tree only once they pass.
                              Writes a report to .bitcode/reports/. --dry
                              never writes to the real tree. A versioned
                              authoring receipt adds model/token provenance to
                              the run report without changing Plan Format v1.
    refactor <dir> rename <node::path> <new_name>
                              Semantic rename across the graph (follows Calls
                              edges, not text search), then save
    inspect <file.aether> [path|--json]
                              Load a saved graph; with a node path, show its
                              impact set. --json exports sorted exact graph records.
    review <dir> [--since <ref>] [--quiet]
                              Semantic code review vs a git ref (default: HEAD).
                              Shows added/modified/removed nodes + edges, impact
                              radius, and test coverage gaps — not text diffs.
                              --quiet prints only changed node paths, one per
                              line, and nothing when there are no changes.
    test-impact <dir> [--run] [--quiet] [node::path...]
                              Find the minimal set of tests that cover changed
                              functions (auto-detected via git diff, or explicit
                              node paths). Pass --run to execute them immediately.
                              --quiet prints only the selected test names, one
                              per line, suitable for `cargo test $(bitcode
                              test-impact . --quiet)`.
    collab <operation>        Exchange deterministic semantic-graph CRDT bundles.
                              Operations: init, status, fork, member, sync, merge,
                              compact, review, apply, materialize, secret,
                              identity, host, discover, join, join-peer. Live
                              peers use mutually authenticated loopback sessions;
                              run without an operation for details.
    dap <program> [--adapter python|rust|<path>] [--break-at <node::path>] [--dry-run]
                              Launch a Debug Adapter Protocol session. Spawns
                              the adapter subprocess, negotiates capabilities,
                              translates graph-node breakpoints to file:line,
                              runs the debuggee, and prints a stop report with
                              stack frames annotated from the semantic graph.
                              --dry-run shows the plan without launching.
    query <dir> [<question...>]   Answer a natural-language question about the
                              codebase by traversing the semantic graph. No code
                              is generated. Supports: concept search, impact
                              analysis, callers/callees, node explain, and
                              subgraph neighbourhood queries. With no question,
                              enters an interactive REPL (reads from stdin).
    debug <file.py> [--what-if <var>=<val> at <step>]
                              Real Python execution tracer. Records every variable
                              at every line/call/return via sys.settrace. With
                              --what-if, re-runs with <var> forced to <val>
                              (a Python literal, e.g. 42) at step <N> and shows
                              where execution diverges. Step numbers come from
                              the trace output (inject AFTER the assignment).
    extension <dir> <operation>
                              Manage declarative, capability-bound extensions.
                              Operations: list; generate <intent...> [--approve];
                              install <recipe.json> [--approve];
                              enable|disable|remove <extension-id>;
                              marketplace list|search|show|adapt.
    --gui [dir]               Launch the native egui/wgpu window for a project
    --help                    Show this help
";

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .with_target(false)
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str);

    match cmd {
        Some("--help") | Some("-h") => println!("{USAGE}"),
        Some("--gui") => launch_gui_or_fallback(args.get(1)),
        Some("config") => report(project::config(&args[1..])),
        Some("analyze") => report(project::analyze(&args[1..])),
        Some("search") => report(project::search(&args[1..])),
        Some("swarm-plan") => {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            report(rt.block_on(project::swarm_plan(&args[1..])));
        }
        Some("plan") => match args.get(1).map(String::as_str) {
            Some("validate") => report(project::plan_validate(&args[2..])),
            Some("explain") => report(project::plan_explain(&args[2..])),
            Some("run") => report(project::plan_run(&args[2..])),
            _ => {
                eprintln!("usage: bitcode plan <validate|explain|run> <plan.json> [--dry]\n");
                println!("{USAGE}");
            }
        },
        Some("forge") => {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            report(rt.block_on(project::forge(&args[1..])));
        }
        Some("refactor") => report(project::refactor(&args[1..])),
        Some("inspect") => report(project::inspect(&args[1..])),
        Some("review") => report(project::review(&args[1..])),
        Some("test-impact") => report(project::test_impact(&args[1..])),
        Some("collab") => report(project::collaboration(&args[1..])),
        Some("dap") => {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            report(rt.block_on(project::dap(&args[1..])));
        }
        Some("query") => report(project::query(&args[1..])),
        Some("debug") => report(project::debug(&args[1..])),
        Some("extension") => {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            report(rt.block_on(project::extensions(&args[1..])));
        }
        Some("demo") | None => run_headless(),
        Some(other) => {
            eprintln!("unknown command: {other}\n");
            println!("{USAGE}");
        }
    }
}

fn report(result: std::io::Result<()>) {
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run_headless() {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(smoke::run());
}

fn launch_gui_or_fallback(root: Option<&String>) {
    #[cfg(feature = "gui")]
    {
        let root = root.map(std::path::PathBuf::from);
        if let Err(e) = app::launch(root) {
            eprintln!("GUI failed to start ({e}); falling back to headless demo.");
            run_headless();
        }
    }
    #[cfg(not(feature = "gui"))]
    {
        let _ = root;
        eprintln!(
            "This binary was built without the GUI. Rebuild with \
             `cargo run -p aether-app --features gui`. Running headless demo instead.\n"
        );
        run_headless();
    }
}
