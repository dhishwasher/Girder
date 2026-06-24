//! AetherForge IDE entry point.
//!
//! Default (`cargo run -p aether-app`) runs the headless end-to-end demo — it
//! needs no display, GPU, or API key. The full egui/wgpu GUI is compiled in with
//! `--features gui` (`cargo run -p aether-app --features gui`).

mod project;
mod smoke;

#[cfg(feature = "gui")]
mod app;
#[cfg(feature = "gui")]
mod highlight;
#[cfg(feature = "gui")]
mod panels;

const USAGE: &str = "\
AetherForge IDE — the semantic agentic forge

USAGE:
    aetherforge [COMMAND]

COMMANDS:
    demo                      Run the headless end-to-end pipeline demo (default)
    analyze <dir>             Build the semantic graph from a project directory,
                              report likely duplicates, save <dir>/project.aether
    search <dir> <query...>   Concept search: rank functions by relevance to a
                              natural-language query
    forge <dir> <intent...>   Dispatch the agent swarm on a project with a
                              natural-language intent, then save the graph
    inspect <file.aether> [path]
                              Load a saved graph; with a node path, show its
                              impact set
    --gui                     Launch the native egui/wgpu window (feature `gui`)
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
        Some("--gui") => launch_gui_or_fallback(),
        Some("analyze") => report(project::analyze(&args[1..])),
        Some("search") => report(project::search(&args[1..])),
        Some("forge") => {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            report(rt.block_on(project::forge(&args[1..])));
        }
        Some("inspect") => report(project::inspect(&args[1..])),
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

fn launch_gui_or_fallback() {
    #[cfg(feature = "gui")]
    {
        if let Err(e) = app::launch() {
            eprintln!("GUI failed to start ({e}); falling back to headless demo.");
            run_headless();
        }
    }
    #[cfg(not(feature = "gui"))]
    {
        eprintln!(
            "This binary was built without the GUI. Rebuild with \
             `cargo run -p aether-app --features gui`. Running headless demo instead.\n"
        );
        run_headless();
    }
}
