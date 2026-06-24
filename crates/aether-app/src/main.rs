//! AetherForge IDE entry point.
//!
//! Default (`cargo run -p aether-app`) runs the headless end-to-end demo — it
//! needs no display, GPU, or API key. The full egui/wgpu GUI is compiled in with
//! `--features gui` (`cargo run -p aether-app --features gui`).

mod smoke;

#[cfg(feature = "gui")]
mod app;
#[cfg(feature = "gui")]
mod highlight;
#[cfg(feature = "gui")]
mod panels;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    let want_gui = std::env::args().any(|a| a == "--gui");

    if want_gui {
        #[cfg(feature = "gui")]
        {
            if let Err(e) = app::launch() {
                eprintln!("GUI failed to start ({e}); falling back to headless demo.");
                run_headless();
            }
            return;
        }
        #[cfg(not(feature = "gui"))]
        {
            eprintln!(
                "This binary was built without the GUI. Rebuild with \
                 `cargo run -p aether-app --features gui`. Running headless demo instead.\n"
            );
        }
    }

    run_headless();
}

fn run_headless() {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(smoke::run());
}
