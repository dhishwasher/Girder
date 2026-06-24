//! The egui/eframe application shell: state + window bootstrap + panel layout.

use crate::panels;
use aether_agents::{Orchestrator, SwarmContext};
use aether_builder::{module_path_for, GraphBuilder};
use aether_debugger::{buggy_demo_program, Timeline};
use aether_graph::SemanticGraph;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) const SAMPLE_RS: &str = r#"struct Point { x: i64, y: i64 }

fn add(a: i64, b: i64) -> i64 {
    a + b
}

fn sum_list(xs: &[i64]) -> i64 {
    let mut total = 0;
    for x in xs {
        total = add(total, *x);
    }
    total
}

fn main() {
    let r = sum_list(&[1, 2, 3]);
    println!("{}", r);
}
"#;

/// All live IDE state. The graph is shared (Arc<Mutex>) so the agent swarm can
/// mutate it concurrently while the UI renders projections of it.
pub struct AetherApp {
    pub(crate) rt: tokio::runtime::Runtime,
    pub(crate) router: aether_ai::Router,
    pub(crate) graph: Arc<Mutex<SemanticGraph>>,
    pub(crate) builder: GraphBuilder,
    pub(crate) file: String,
    pub(crate) code: String,
    pub(crate) intent: String,
    pub(crate) transcript: Vec<(String, String)>,
    pub(crate) timeline: Timeline,
    pub(crate) selected_branch: usize,
}

impl AetherApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let file = "src/math.rs".to_string();
        let graph = Arc::new(Mutex::new(SemanticGraph::new()));
        let mut builder = GraphBuilder::new();
        builder.load_file(&mut graph.lock().unwrap(), &file, SAMPLE_RS);

        AetherApp {
            rt: tokio::runtime::Runtime::new().expect("tokio runtime"),
            router: aether_ai::default_router(),
            graph,
            builder,
            file,
            code: SAMPLE_RS.to_string(),
            intent: "Add a multiply function to the math module".to_string(),
            transcript: Vec::new(),
            timeline: Timeline::record(buggy_demo_program()),
            selected_branch: 0,
        }
    }

    /// Re-fold the (possibly edited) editor buffer back into the graph. This is
    /// the editor→graph half of bidirectional sync.
    pub(crate) fn sync_code_to_graph(&mut self) {
        let code = self.code.clone();
        let file = self.file.clone();
        let mut graph = self.graph.lock().unwrap();
        self.builder.update_file(&mut graph, &file, &code);
    }

    /// Run the agent swarm on the current intent, blocking briefly. Agents
    /// mutate `self.graph` directly; we just collect the transcript here.
    pub(crate) fn run_swarm(&mut self) {
        let module = module_path_for(&self.file);
        let ctx = Arc::new(SwarmContext::new(
            self.router.clone(),
            self.graph.clone(),
            &module,
            &self.file,
        ));
        let orchestrator = Orchestrator::new(ctx).with_default_swarm();
        let intent = self.intent.clone();
        let transcript = self
            .rt
            .block_on(orchestrator.run(&intent, Duration::from_secs(5)));

        self.transcript = transcript
            .iter()
            .map(|m| {
                let body = match &m.kind {
                    aether_agents::MsgKind::Intent(t) => format!("intent: {t}"),
                    aether_agents::MsgKind::PlanReady { steps } => {
                        format!("plan ({} steps)", steps.len())
                    }
                    aether_agents::MsgKind::CodeReady { name, .. } => format!("wrote fn {name}"),
                    aether_agents::MsgKind::TestsReady { for_fn, .. } => {
                        format!("tested {for_fn}")
                    }
                    aether_agents::MsgKind::Note { text } => text.clone(),
                };
                (m.from.label().to_string(), body)
            })
            .collect();
    }
}

impl eframe::App for AetherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("title").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("⚡ AetherForge IDE");
                ui.separator();
                let (n, e) = {
                    let g = self.graph.lock().unwrap();
                    (g.node_count(), g.edge_count())
                };
                ui.label(format!("semantic graph: {n} nodes · {e} edges"));
            });
        });

        egui::SidePanel::left("graph")
            .resizable(true)
            .default_width(280.0)
            .show(ctx, |ui| panels::graph_panel(self, ui));

        egui::SidePanel::right("agents")
            .resizable(true)
            .default_width(320.0)
            .show(ctx, |ui| panels::agents_panel(self, ui));

        egui::TopBottomPanel::bottom("debugger")
            .resizable(true)
            .default_height(200.0)
            .show(ctx, |ui| panels::debugger_panel(self, ui));

        egui::CentralPanel::default().show(ctx, |ui| panels::editor_panel(self, ui));
    }
}

/// Bootstrap the native window.
pub fn launch() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "AetherForge IDE",
        native_options,
        Box::new(|cc| Ok(Box::new(AetherApp::new(cc)))),
    )
}
