//! The egui/eframe application shell: state + window bootstrap + panel layout.

use crate::graph_view::GraphViewState;
use crate::panels;
use crate::project::{AgentValidationOutcome, ProjectWorkspace, SyncImpact};
use aether_agents::{MsgKind, Orchestrator, SwarmContext, SwarmMessage};
use aether_debugger::{buggy_demo_program, python_tracer::PyTimeline, Timeline};
use aether_graph::NodeId;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

type PythonTraceSteps = Vec<(usize, String, bool)>;
type PythonTraceResult = Result<PythonTraceSteps, String>;
type PythonTraceReceiver = tokio::sync::oneshot::Receiver<PythonTraceResult>;
type AgentValidationResult = std::io::Result<AgentValidationOutcome>;
type AgentValidationReceiver = tokio::sync::oneshot::Receiver<AgentValidationResult>;

/// All live IDE state. The graph is shared (Arc<Mutex>) so the agent swarm can
/// mutate it concurrently while the UI renders projections of it.
pub struct AetherApp {
    pub(crate) rt: tokio::runtime::Runtime,
    pub(crate) router: aether_ai::Router,
    pub(crate) workspace: ProjectWorkspace,
    pub(crate) project_path_input: String,
    pub(crate) file_filter: String,
    pub(crate) graph_view: GraphViewState,
    pub(crate) workspace_status: String,
    pub(crate) workspace_status_is_error: bool,
    pub(crate) intent: String,
    pub(crate) transcript: Vec<(String, String)>,
    pub(crate) timeline: Timeline,
    pub(crate) selected_branch: usize,
    /// Pending swarm result. Set when a swarm is running; cleared when it completes.
    pub(crate) swarm_rx: Option<tokio::sync::oneshot::Receiver<Vec<SwarmMessage>>>,
    pub(crate) agent_validation_rx: Option<AgentValidationReceiver>,
    agent_validation_cancel: Option<Arc<AtomicBool>>,
    /// Nodes currently lit by the impact ripple: node_id -> hop distance from changed node.
    /// Distance 0 = the edited function itself; 1 = direct callers; 2 = their callers; etc.
    pub(crate) impact_nodes: HashMap<NodeId, u32>,
    /// When the current ripple animation started. None when no ripple is active.
    pub(crate) ripple_start: Option<Instant>,
    /// Byte offset to select after graph-to-editor navigation.
    pub(crate) editor_jump: Option<usize>,

    // ── Python real tracer ────────────────────────────────────────────────────
    /// Path of the Python file the user wants to trace.
    pub(crate) py_file: String,
    /// Steps from the most recent Python trace: (seq, description, intervened).
    pub(crate) py_steps: Vec<(usize, String, bool)>,
    /// Pending result from a background `python3` subprocess.
    pub(crate) py_trace_rx: Option<PythonTraceReceiver>,
}

impl AetherApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        initial_root: impl AsRef<Path>,
    ) -> std::io::Result<Self> {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let workspace = ProjectWorkspace::open(initial_root)?;
        let project_path_input = workspace.root().display().to_string();
        let workspace_status = workspace_summary(&workspace);
        let py_file = active_python_path(&workspace).unwrap_or_default();

        Ok(AetherApp {
            rt: tokio::runtime::Runtime::new().expect("tokio runtime"),
            router: aether_ai::default_router(),
            workspace,
            project_path_input,
            file_filter: String::new(),
            graph_view: GraphViewState::default(),
            workspace_status,
            workspace_status_is_error: false,
            intent: String::new(),
            transcript: Vec::new(),
            timeline: Timeline::record(buggy_demo_program()),
            selected_branch: 0,
            swarm_rx: None,
            agent_validation_rx: None,
            agent_validation_cancel: None,
            impact_nodes: HashMap::new(),
            ripple_start: None,
            editor_jump: None,
            py_file,
            py_steps: Vec::new(),
            py_trace_rx: None,
        })
    }

    pub(crate) fn open_project(&mut self) {
        if self.workspace.has_pending_agent_changes() {
            self.set_workspace_error(
                "Commit or roll back pending agent changes before opening another project.",
            );
            return;
        }
        if self.workspace.is_dirty() {
            self.set_workspace_error(
                "Save or discard the active file before opening another project.",
            );
            return;
        }
        match ProjectWorkspace::open(&self.project_path_input) {
            Ok(workspace) => {
                self.workspace = workspace;
                self.project_path_input = self.workspace.root().display().to_string();
                self.file_filter.clear();
                self.graph_view.reset_for_project();
                self.transcript.clear();
                self.impact_nodes.clear();
                self.ripple_start = None;
                self.editor_jump = None;
                self.py_file = active_python_path(&self.workspace).unwrap_or_default();
                self.py_steps.clear();
                self.set_workspace_status(workspace_summary(&self.workspace));
            }
            Err(error) => self.set_workspace_error(format!("Open failed: {error}")),
        }
    }

    pub(crate) fn save_active_file(&mut self) {
        match self.workspace.save() {
            Ok(path) => self.set_workspace_status(format!("Saved {}", path.display())),
            Err(error) => self.set_workspace_error(format!("Save failed: {error}")),
        }
    }

    pub(crate) fn discard_active_changes(&mut self) {
        match self.workspace.discard_changes() {
            Ok(impact) => {
                self.apply_sync_impact(impact);
                self.set_workspace_status("Discarded unsaved editor changes.");
            }
            Err(error) => self.set_workspace_error(format!("Discard failed: {error}")),
        }
    }

    pub(crate) fn reload_project(&mut self) {
        match self.workspace.reload() {
            Ok(()) => {
                self.py_file = active_python_path(&self.workspace).unwrap_or_default();
                self.py_steps.clear();
                self.impact_nodes.clear();
                self.ripple_start = None;
                self.graph_view.reset_for_project();
                self.editor_jump = None;
                self.set_workspace_status(workspace_summary(&self.workspace));
            }
            Err(error) => self.set_workspace_error(format!("Reload failed: {error}")),
        }
    }

    pub(crate) fn select_file(&mut self, relative: &str) {
        match self.workspace.select_file(relative) {
            Ok(()) => {
                self.py_file = active_python_path(&self.workspace).unwrap_or_default();
                self.py_steps.clear();
                self.set_workspace_status(format!("Opened {relative}"));
            }
            Err(error) => self.set_workspace_error(format!("File switch failed: {error}")),
        }
    }

    pub(crate) fn navigate_to_graph_node(&mut self, id: NodeId) {
        let target = {
            let graph = self.workspace.graph().lock().unwrap();
            graph.get(id).and_then(|node| {
                node.file.clone().map(|file| {
                    (
                        file,
                        node.span.start_byte,
                        node.span.start_row + 1,
                        node.path.clone(),
                    )
                })
            })
        };
        let Some((file, start_byte, row, path)) = target else {
            self.set_workspace_error("The selected graph node has no source projection.");
            return;
        };
        match self.workspace.select_file(&file) {
            Ok(()) => {
                self.editor_jump = Some(start_byte);
                self.set_workspace_status(format!("Opened {path} at {file}:{row}"));
            }
            Err(error) => self.set_workspace_error(format!("Graph navigation failed: {error}")),
        }
    }

    /// Spawn a background `python3` subprocess to trace `self.py_file`.
    /// The result lands in `py_trace_rx` and is harvested in `update()`.
    pub(crate) fn trace_python_file(&mut self) {
        let path = std::path::PathBuf::from(&self.py_file);
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.rt.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                PyTimeline::record(&path)
                    .map(|tl| {
                        tl.branch(0)
                            .map(|b| {
                                b.trace
                                    .steps
                                    .iter()
                                    .map(|s| (s.seq, s.description.clone(), s.intervened))
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                    })
                    .map_err(|e| e.to_string())
            })
            .await;
            let _ = tx.send(result.unwrap_or_else(|e| Err(e.to_string())));
        });
        self.py_trace_rx = Some(rx);
    }

    /// Re-fold the (possibly edited) editor buffer back into the graph (editor→graph
    /// sync), then compute the impact set of any changed nodes and kick off the
    /// ripple animation so the graph panel lights up in real time.
    pub(crate) fn sync_code_to_graph(&mut self) {
        match self.workspace.sync_buffer_to_graph() {
            Ok(impact) => self.apply_sync_impact(impact),
            Err(error) => self.set_workspace_error(format!("Graph sync failed: {error}")),
        }
    }

    /// Dispatch the agent swarm on the current intent. The swarm runs as a
    /// background tokio task so the egui render thread is never blocked.
    /// Results are collected in `update` via `swarm_rx`.
    pub(crate) fn run_swarm(&mut self) {
        if self.workspace.active_file().is_none() {
            self.set_workspace_error("Open a source file before dispatching the swarm.");
            return;
        }
        if self.workspace.is_dirty() {
            self.set_workspace_error(
                "Save or discard editor changes before dispatching the swarm.",
            );
            return;
        }
        if self.intent.trim().is_empty() {
            self.set_workspace_error("Enter an intent before dispatching the swarm.");
            return;
        }
        if let Err(error) = self.workspace.begin_agent_transaction() {
            self.set_workspace_error(format!("Could not start agent transaction: {error}"));
            return;
        }
        let (module, file) = self.workspace.agent_target();
        let module = module.to_string();
        let file = file.to_string();
        let timeout = self.workspace.agent_timeout();
        let ctx = Arc::new(SwarmContext::new(
            self.router.clone(),
            self.workspace.graph().clone(),
            &module,
            &file,
        ));
        let orchestrator = Orchestrator::new(ctx).with_default_swarm();
        let intent = self.intent.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.rt.spawn(async move {
            let messages = orchestrator.run(&intent, timeout).await;
            let _ = tx.send(messages);
        });
        self.swarm_rx = Some(rx);
    }

    pub(crate) fn commit_agent_changes(&mut self) {
        if self.agent_validation_rx.is_some() {
            self.set_workspace_error("Wait for candidate validation before committing.");
            return;
        }
        match self.workspace.commit_agent_changes() {
            Ok(files) => {
                self.impact_nodes.clear();
                self.ripple_start = None;
                let detail = if files.is_empty() {
                    "durable graph metadata".to_string()
                } else {
                    files
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                self.set_workspace_status(format!("Committed agent changes: {detail}"));
            }
            Err(error) => self.set_workspace_error(format!("Agent commit failed: {error}")),
        }
    }

    pub(crate) fn rollback_agent_changes(&mut self) {
        self.cancel_agent_validation();
        match self.workspace.rollback_agent_changes() {
            Ok(()) => {
                self.impact_nodes.clear();
                self.ripple_start = None;
                self.set_workspace_status("Rolled back pending agent changes.");
            }
            Err(error) => self.set_workspace_error(format!("Agent rollback failed: {error}")),
        }
    }

    pub(crate) fn start_agent_validation(&mut self) {
        self.cancel_agent_validation();
        let request = match self.workspace.agent_validation_request() {
            Ok(request) => request,
            Err(error) => {
                self.set_workspace_error(format!("Could not prepare validation: {error}"));
                return;
            }
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let task_cancel = cancel.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.rt.spawn(async move {
            let result = tokio::task::spawn_blocking(move || request.run(&task_cancel)).await;
            let result = result.unwrap_or_else(|error| {
                Err(std::io::Error::other(format!(
                    "validation task failed: {error}"
                )))
            });
            let _ = tx.send(result);
        });
        self.agent_validation_cancel = Some(cancel);
        self.agent_validation_rx = Some(rx);
        self.set_workspace_status("Validating agent candidate in an isolated workspace...");
    }

    pub(crate) fn agent_validation_running(&self) -> bool {
        self.agent_validation_rx.is_some()
    }

    fn cancel_agent_validation(&mut self) {
        if let Some(cancel) = self.agent_validation_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.agent_validation_rx = None;
    }

    fn apply_sync_impact(&mut self, impact: SyncImpact) {
        if !impact.nodes.is_empty() {
            self.impact_nodes = impact.nodes;
            self.ripple_start = Some(Instant::now());
        }
    }

    fn set_workspace_status(&mut self, message: impl Into<String>) {
        self.workspace_status = message.into();
        self.workspace_status_is_error = false;
    }

    fn set_workspace_error(&mut self, message: impl Into<String>) {
        self.workspace_status = message.into();
        self.workspace_status_is_error = true;
    }

    pub(crate) fn messages_to_transcript(messages: Vec<SwarmMessage>) -> Vec<(String, String)> {
        messages
            .iter()
            .map(|m| {
                let body = match &m.kind {
                    MsgKind::Intent(t) => format!("intent: {t}"),
                    MsgKind::PlanReady { steps } => format!("plan ({} steps)", steps.len()),
                    MsgKind::FeatureSpec { fn_specs, .. } => {
                        let names: Vec<&str> = fn_specs.iter().map(|s| s.name.as_str()).collect();
                        format!("spec: fn {}", names.join(", fn "))
                    }
                    MsgKind::CodeReady { name, .. } => format!("wrote fn {name}"),
                    MsgKind::TestsReady { for_fn, .. } => {
                        let short = for_fn.rsplit("::").next().unwrap_or(for_fn);
                        format!("tested {short}")
                    }
                    MsgKind::FeatureComplete { built, .. } => {
                        format!("built {} fn(s): {}", built.len(), built.join(", "))
                    }
                    MsgKind::Note { text } => text.clone(),
                };
                (m.from.label().to_string(), body)
            })
            .collect()
    }
}

impl Drop for AetherApp {
    fn drop(&mut self) {
        if let Some(cancel) = &self.agent_validation_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl eframe::App for AetherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            self.save_active_file();
        }

        // Drive the impact ripple: keep repainting while the animation is live,
        // then clear the state so the graph panel returns to its resting look.
        const RIPPLE_SECS: f32 = 2.5;
        if let Some(start) = self.ripple_start {
            if start.elapsed().as_secs_f32() < RIPPLE_SECS {
                ctx.request_repaint();
            } else {
                self.ripple_start = None;
                self.impact_nodes.clear();
            }
        }

        // Poll the background Python tracer task without blocking.
        if let Some(rx) = self.py_trace_rx.as_mut() {
            match rx.try_recv() {
                Ok(Ok(steps)) => {
                    self.py_steps = steps;
                    self.py_trace_rx = None;
                }
                Ok(Err(e)) => {
                    self.transcript
                        .push(("[debugger]".to_string(), format!("trace error: {e}")));
                    self.py_trace_rx = None;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.py_trace_rx = None;
                }
            }
        }

        // Poll isolated candidate validation without blocking the UI.
        if let Some(rx) = self.agent_validation_rx.as_mut() {
            match rx.try_recv() {
                Ok(Ok(outcome)) => {
                    let summary = outcome.report.summary();
                    let passed = outcome.report.passed();
                    self.agent_validation_rx = None;
                    self.agent_validation_cancel = None;
                    match self.workspace.accept_agent_validation(outcome) {
                        Ok(()) if passed => self.set_workspace_status(summary),
                        Ok(()) => self.set_workspace_error(format!(
                            "{summary}; inspect diagnostics or roll back"
                        )),
                        Err(error) => {
                            self.set_workspace_error(format!("Validation was discarded: {error}"))
                        }
                    }
                }
                Ok(Err(error)) => {
                    self.agent_validation_rx = None;
                    self.agent_validation_cancel = None;
                    self.set_workspace_error(format!("Candidate validation failed: {error}"));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.agent_validation_rx = None;
                    self.agent_validation_cancel = None;
                    self.set_workspace_error("Candidate validation stopped unexpectedly.");
                }
            }
        }

        // Poll the background swarm task without blocking.
        if let Some(rx) = self.swarm_rx.as_mut() {
            match rx.try_recv() {
                Ok(messages) => {
                    self.transcript = Self::messages_to_transcript(messages);
                    self.swarm_rx = None;
                    match self.workspace.finish_agent_transaction() {
                        Ok(0) => {
                            self.set_workspace_status("Agent swarm completed with no changes.")
                        }
                        Ok(changes) => {
                            self.set_workspace_status(format!(
                                "Agent swarm completed with {changes} pending graph change(s)."
                            ));
                            self.start_agent_validation();
                        }
                        Err(error) => {
                            let _ = self.workspace.rollback_agent_changes();
                            self.set_workspace_error(format!(
                                "Agent transaction failed and was rolled back: {error}"
                            ));
                        }
                    }
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.swarm_rx = None;
                    let _ = self.workspace.rollback_agent_changes();
                    self.set_workspace_error(
                        "Agent swarm stopped; graph changes were rolled back.",
                    );
                }
            }
        }

        egui::TopBottomPanel::top("title").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Bit Code");
                ui.separator();
                let (n, e) = {
                    let g = self.workspace.graph().lock().unwrap();
                    (g.node_count(), g.edge_count())
                };
                ui.label(format!("semantic graph: {n} nodes · {e} edges"));
                if self.workspace.is_dirty() {
                    ui.colored_label(egui::Color32::from_rgb(0xE5, 0xC0, 0x7B), "modified");
                }
                if self.workspace.has_pending_agent_changes() {
                    ui.colored_label(
                        egui::Color32::from_rgb(0x4E, 0xC9, 0xB0),
                        "agent changes pending",
                    );
                }
            });
            ui.horizontal(|ui| {
                ui.label("Project");
                ui.add(
                    egui::TextEdit::singleline(&mut self.project_path_input).desired_width(320.0),
                );
                if ui
                    .button("Open")
                    .on_hover_text("Open project directory")
                    .clicked()
                {
                    self.open_project();
                }
                let has_file = self.workspace.active_file().is_some();
                let pending_agents = self.workspace.has_pending_agent_changes();
                if ui
                    .add_enabled(
                        has_file && self.workspace.is_dirty() && !pending_agents,
                        egui::Button::new("Save"),
                    )
                    .on_hover_text("Save source and semantic graph")
                    .clicked()
                {
                    self.save_active_file();
                }
                if ui
                    .add_enabled(
                        has_file && self.workspace.is_dirty() && !pending_agents,
                        egui::Button::new("Discard"),
                    )
                    .on_hover_text("Discard unsaved editor changes")
                    .clicked()
                {
                    self.discard_active_changes();
                }
                if ui
                    .add_enabled(
                        !self.workspace.is_dirty() && !pending_agents && self.swarm_rx.is_none(),
                        egui::Button::new("Reload"),
                    )
                    .on_hover_text("Re-index project from disk")
                    .clicked()
                {
                    self.reload_project();
                }
                ui.separator();
                if self.workspace_status_is_error {
                    ui.colored_label(
                        egui::Color32::from_rgb(0xF4, 0x87, 0x71),
                        &self.workspace_status,
                    );
                } else {
                    ui.label(&self.workspace_status);
                }
            });
        });

        let available_width = ctx.available_rect().width();
        let workspace_max = (available_width * 0.40).clamp(300.0, 520.0);
        let agents_max = (available_width * 0.32).clamp(240.0, 440.0);

        egui::SidePanel::left("workspace")
            .resizable(true)
            .default_width(360.0)
            .min_width(240.0)
            .max_width(workspace_max)
            .show(ctx, |ui| panels::workspace_panel(self, ui));

        egui::SidePanel::right("agents")
            .resizable(true)
            .default_width(320.0)
            .min_width(240.0)
            .max_width(agents_max)
            .show(ctx, |ui| panels::agents_panel(self, ui));

        egui::TopBottomPanel::bottom("debugger")
            .resizable(true)
            .default_height(200.0)
            .show(ctx, |ui| panels::debugger_panel(self, ui));

        egui::CentralPanel::default().show(ctx, |ui| panels::editor_panel(self, ui));
    }
}

fn workspace_summary(workspace: &ProjectWorkspace) -> String {
    format!(
        "Indexed {} source file(s) from {}; {}",
        workspace.files().len(),
        workspace.root().display(),
        workspace.open_status()
    )
}

fn active_python_path(workspace: &ProjectWorkspace) -> Option<String> {
    let file = workspace.active_file()?;
    (file.ends_with(".py")).then(|| workspace.root().join(file).display().to_string())
}

/// Bootstrap the native window.
pub fn launch(initial_root: Option<PathBuf>) -> eframe::Result<()> {
    let initial_root = initial_root
        .map(Ok)
        .unwrap_or_else(std::env::current_dir)
        .map_err(|error| eframe::Error::AppCreation(Box::new(error)))?;
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Bit Code",
        native_options,
        Box::new(move |cc| Ok(Box::new(AetherApp::new(cc, &initial_root)?))),
    )
}
