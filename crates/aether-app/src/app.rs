//! The egui/eframe application shell: state + window bootstrap + panel layout.

use crate::graph_view::GraphViewState;
use crate::panels;
use crate::project::{
    generate_collaboration_secret, join_collaboration, AgentValidationOutcome,
    ExtensionCommandRequest, ExtensionMutation, ExtensionMutationOutcome, ExtensionMutationRequest,
    LiveSyncReport, ProjectWorkspace, SyncImpact, ValidationReport,
};
use aether_agents::{MsgKind, Orchestrator, SwarmContext, SwarmMessage};
use aether_debugger::{buggy_demo_program, python_tracer::PyTimeline, Timeline};
use aether_extensions::{
    adaptation_system_prompt, builtin_catalog, generation_system_prompt,
    marketplace_project_context, ExtensionRecipe, MarketplaceCatalog, MarketplaceListing,
};
use aether_graph::{ActorId, GraphReplica, NodeId};
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
type ExtensionGenerationResult =
    Result<(ExtensionRecipe, String, Option<MarketplaceListing>), String>;
type ExtensionGenerationReceiver = tokio::sync::oneshot::Receiver<ExtensionGenerationResult>;
type ExtensionMutationResult = std::io::Result<ExtensionMutationOutcome>;
type ExtensionMutationReceiver = tokio::sync::oneshot::Receiver<ExtensionMutationResult>;
type ExtensionCommandResult = std::io::Result<ValidationReport>;
type ExtensionCommandReceiver = tokio::sync::oneshot::Receiver<ExtensionCommandResult>;
type CollaborationReceiver = tokio::sync::oneshot::Receiver<std::io::Result<LiveSyncReport>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RightPanel {
    Agents,
    Extensions,
    Collaboration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExtensionPanelView {
    Generate,
    Marketplace,
    Installed,
}

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
    pub(crate) extension_intent: String,
    pub(crate) extension_candidate: Option<ExtensionRecipe>,
    pub(crate) extension_candidate_source: Option<MarketplaceListing>,
    pub(crate) marketplace_catalog: MarketplaceCatalog,
    pub(crate) marketplace_query: String,
    pub(crate) extension_panel_view: ExtensionPanelView,
    extension_generation_rx: Option<ExtensionGenerationReceiver>,
    extension_mutation_rx: Option<ExtensionMutationReceiver>,
    extension_mutation_cancel: Option<Arc<AtomicBool>>,
    extension_command_rx: Option<ExtensionCommandReceiver>,
    extension_command_cancel: Option<Arc<AtomicBool>>,
    pub(crate) extension_action_output: String,
    pub(crate) extension_remove_confirmation: Option<String>,
    pub(crate) right_panel: RightPanel,
    pub(crate) collaboration_bundle_input: String,
    pub(crate) collaboration_actor_input: String,
    pub(crate) collaboration_address_input: String,
    pub(crate) collaboration_secret_input: String,
    pub(crate) collaboration_status: String,
    collaboration_rx: Option<CollaborationReceiver>,
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
        let marketplace_catalog =
            builtin_catalog().map_err(|error| std::io::Error::other(error.to_string()))?;
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
            extension_intent: String::new(),
            extension_candidate: None,
            extension_candidate_source: None,
            marketplace_catalog,
            marketplace_query: String::new(),
            extension_panel_view: ExtensionPanelView::Generate,
            extension_generation_rx: None,
            extension_mutation_rx: None,
            extension_mutation_cancel: None,
            extension_command_rx: None,
            extension_command_cancel: None,
            extension_action_output: String::new(),
            extension_remove_confirmation: None,
            right_panel: RightPanel::Agents,
            collaboration_bundle_input: ".bitcode/collaboration.aetherc".into(),
            collaboration_actor_input: String::new(),
            collaboration_address_input: "127.0.0.1:7331".into(),
            collaboration_secret_input: ".bitcode/collaboration.secret".into(),
            collaboration_status:
                "Initialize a graph replica or inspect an existing collaboration bundle.".into(),
            collaboration_rx: None,
            impact_nodes: HashMap::new(),
            ripple_start: None,
            editor_jump: None,
            py_file,
            py_steps: Vec::new(),
            py_trace_rx: None,
        })
    }

    pub(crate) fn open_project(&mut self) {
        if self.extension_busy() {
            self.set_workspace_error("Wait for the extension operation before opening a project.");
            return;
        }
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
                self.extension_intent.clear();
                self.extension_candidate = None;
                self.extension_candidate_source = None;
                self.extension_action_output.clear();
                self.extension_remove_confirmation = None;
                self.set_workspace_status(workspace_summary(&self.workspace));
            }
            Err(error) => self.set_workspace_error(format!("Open failed: {error}")),
        }
    }

    pub(crate) fn save_active_file(&mut self) {
        if self.extension_busy() {
            self.set_workspace_error("Wait for the extension operation before saving.");
            return;
        }
        match self.workspace.save() {
            Ok(path) => self.set_workspace_status(format!("Saved {}", path.display())),
            Err(error) => self.set_workspace_error(format!("Save failed: {error}")),
        }
    }

    pub(crate) fn discard_active_changes(&mut self) {
        if self.extension_busy() {
            self.set_workspace_error("Wait for the extension operation before discarding changes.");
            return;
        }
        match self.workspace.discard_changes() {
            Ok(impact) => {
                self.apply_sync_impact(impact);
                self.set_workspace_status("Discarded unsaved editor changes.");
            }
            Err(error) => self.set_workspace_error(format!("Discard failed: {error}")),
        }
    }

    pub(crate) fn reload_project(&mut self) {
        if self.extension_busy() {
            self.set_workspace_error("Wait for the extension operation before reloading.");
            return;
        }
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
        if self.extension_mutation_rx.is_some() {
            self.set_workspace_error("Wait for the extension change before opening a file.");
            return;
        }
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
        if self.extension_busy() {
            self.set_workspace_error("Wait for the extension operation before dispatching agents.");
            return;
        }
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

    pub(crate) fn generate_extension(&mut self) {
        if self.extension_busy() {
            self.set_workspace_error("An extension operation is already running.");
            return;
        }
        if self.workspace.is_dirty() || self.workspace.has_pending_agent_changes() {
            self.set_workspace_error(
                "Save or resolve pending agent changes before generating an extension.",
            );
            return;
        }
        let intent = self.extension_intent.trim().to_string();
        if intent.is_empty() {
            self.set_workspace_error("Describe the extension to generate.");
            return;
        }
        let (nodes, edges) = {
            let graph = self.workspace.graph().lock().unwrap();
            (graph.node_count(), graph.edge_count())
        };
        let router = self.router.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.rt.spawn(async move {
            let input = serde_json::json!({
                "intent": intent,
                "graph_context": {"nodes": nodes, "edges": edges}
            });
            let mut prompt = aether_ai::Prompt::new(
                aether_ai::TaskClass::Extension,
                generation_system_prompt(),
                input.to_string(),
            );
            prompt.max_tokens = 4_096;
            let result = router
                .complete(prompt)
                .await
                .map_err(|error| error.to_string())
                .and_then(|completion| {
                    ExtensionRecipe::from_json(&completion.text)
                        .map(|recipe| (recipe, completion.model, None))
                        .map_err(|error| error.to_string())
                });
            let _ = tx.send(result);
        });
        self.extension_candidate = None;
        self.extension_candidate_source = None;
        self.extension_generation_rx = Some(rx);
        self.set_workspace_status("Generating a declarative extension recipe...");
    }

    pub(crate) fn adapt_marketplace_listing(&mut self, listing_id: &str) {
        if self.extension_busy() {
            self.set_workspace_error("An extension operation is already running.");
            return;
        }
        if self.workspace.is_dirty() || self.workspace.has_pending_agent_changes() {
            self.set_workspace_error(
                "Save or resolve pending agent changes before adapting an extension.",
            );
            return;
        }
        let Some(listing) = self.marketplace_catalog.listing(listing_id).cloned() else {
            self.set_workspace_error(format!(
                "Marketplace listing {listing_id} is no longer available."
            ));
            return;
        };
        let project = {
            let graph = self.workspace.graph().lock().unwrap();
            marketplace_project_context(&graph)
        };
        let router = self.router.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.rt.spawn(async move {
            let input = serde_json::json!({
                "listing": listing,
                "project": project,
            });
            let mut prompt = aether_ai::Prompt::new(
                aether_ai::TaskClass::Extension,
                adaptation_system_prompt(),
                input.to_string(),
            );
            prompt.max_tokens = 4_096;
            let result = router
                .complete(prompt)
                .await
                .map_err(|error| error.to_string())
                .and_then(|completion| {
                    let recipe = ExtensionRecipe::from_json(&completion.text)
                        .map_err(|error| error.to_string())?;
                    listing
                        .capability_delta(&recipe)
                        .map_err(|error| error.to_string())?;
                    Ok((recipe, completion.model, Some(listing)))
                });
            let _ = tx.send(result);
        });
        self.extension_candidate = None;
        self.extension_candidate_source = None;
        self.extension_generation_rx = Some(rx);
        self.set_workspace_status(format!("Adapting marketplace listing {listing_id}..."));
    }

    pub(crate) fn approve_extension(&mut self) {
        let Some(recipe) = self.extension_candidate.clone() else {
            self.set_workspace_error("There is no extension recipe to approve.");
            return;
        };
        self.start_extension_mutation(ExtensionMutation::Install(recipe));
    }

    pub(crate) fn set_extension_enabled(&mut self, extension_id: String, enabled: bool) {
        self.start_extension_mutation(ExtensionMutation::SetEnabled {
            extension_id,
            enabled,
        });
    }

    pub(crate) fn remove_extension(&mut self, extension_id: String) {
        self.extension_remove_confirmation = None;
        self.start_extension_mutation(ExtensionMutation::Remove { extension_id });
    }

    pub(crate) fn extension_busy(&self) -> bool {
        self.extension_generation_rx.is_some()
            || self.extension_mutation_rx.is_some()
            || self.extension_command_rx.is_some()
    }

    pub(crate) fn collaboration_busy(&self) -> bool {
        self.collaboration_rx.is_some()
    }

    fn collaboration_snapshot_ready(&self) -> std::io::Result<()> {
        if self.workspace.is_dirty() || self.workspace.has_pending_agent_changes() {
            return Err(std::io::Error::other(
                "save or resolve pending workspace changes before snapshot synchronization",
            ));
        }
        Ok(())
    }

    pub(crate) fn initialize_collaboration(&mut self) {
        let result = (|| -> std::io::Result<String> {
            self.collaboration_snapshot_ready()?;
            let bundle = self.collaboration_path(&self.collaboration_bundle_input)?;
            if bundle.exists() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!("refusing to overwrite {}", bundle.display()),
                ));
            }
            let actor = ActorId::new(self.collaboration_actor_input.trim())
                .map_err(std::io::Error::other)?;
            let graph = self
                .workspace
                .graph()
                .lock()
                .map_err(|_| std::io::Error::other("semantic graph lock is poisoned"))?
                .clone();
            if let Some(parent) = bundle.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let replica = GraphReplica::from_graph(actor, &graph);
            replica.save(&bundle).map_err(std::io::Error::other)?;
            Ok(format!(
                "Initialized {}: {} nodes, {} edges, {} operations",
                bundle.display(),
                graph.node_count(),
                graph.edge_count(),
                replica.operation_count()
            ))
        })();
        self.collaboration_status = result.unwrap_or_else(|error| format!("Error: {error}"));
    }

    pub(crate) fn sync_collaboration(&mut self) {
        let result = (|| -> std::io::Result<String> {
            self.collaboration_snapshot_ready()?;
            let bundle = self.collaboration_path(&self.collaboration_bundle_input)?;
            let graph = self
                .workspace
                .graph()
                .lock()
                .map_err(|_| std::io::Error::other("semantic graph lock is poisoned"))?
                .clone();
            let mut replica = GraphReplica::load(&bundle).map_err(std::io::Error::other)?;
            let report = replica.sync_graph(&graph).map_err(std::io::Error::other)?;
            replica.save(&bundle).map_err(std::io::Error::other)?;
            Ok(format!(
                "Synchronized {} as {}: {} operation(s) (+{} / -{} nodes, +{} / -{} edges)",
                bundle.display(),
                replica.actor(),
                report.operation_count(),
                report.nodes_upserted,
                report.nodes_removed,
                report.edges_upserted,
                report.edges_removed
            ))
        })();
        self.collaboration_status = result.unwrap_or_else(|error| format!("Error: {error}"));
    }

    pub(crate) fn inspect_collaboration(&mut self) {
        let result = (|| -> std::io::Result<String> {
            let bundle = self.collaboration_path(&self.collaboration_bundle_input)?;
            let replica = GraphReplica::load(&bundle).map_err(std::io::Error::other)?;
            let graph = replica.materialize().map_err(std::io::Error::other)?;
            let version = replica
                .version()
                .actors()
                .map(|(actor, counter)| format!("{actor}:{counter}"))
                .collect::<Vec<_>>()
                .join(", ");
            let floor = replica
                .history_floor()
                .actors()
                .map(|(actor, counter)| format!("{actor}:{counter}"))
                .collect::<Vec<_>>()
                .join(", ");
            let acknowledgements = replica
                .acknowledgements()
                .map(|(peer, version)| {
                    let version = version
                        .actors()
                        .map(|(actor, counter)| format!("{actor}:{counter}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{peer}=[{version}]")
                })
                .collect::<Vec<_>>()
                .join("; ");
            Ok(format!(
                "{}\nactor: {}\nversion: {}\ncompacted through: {}\ndurable acknowledgements: {}\noperations: {}\ngraph: {} nodes / {} edges",
                bundle.display(),
                replica.actor(),
                version,
                if floor.is_empty() { "none" } else { &floor },
                if acknowledgements.is_empty() {
                    "none"
                } else {
                    &acknowledgements
                },
                replica.operation_count(),
                graph.node_count(),
                graph.edge_count()
            ))
        })();
        self.collaboration_status = result.unwrap_or_else(|error| format!("Error: {error}"));
    }

    pub(crate) fn compact_collaboration(&mut self) {
        let result = (|| -> std::io::Result<String> {
            self.collaboration_snapshot_ready()?;
            let bundle = self.collaboration_path(&self.collaboration_bundle_input)?;
            let mut replica = GraphReplica::load(&bundle).map_err(std::io::Error::other)?;
            let report = replica
                .compact_acknowledged()
                .map_err(std::io::Error::other)?;
            replica.save(&bundle).map_err(std::io::Error::other)?;
            Ok(format!(
                "Compacted {}: removed {} superseded operation(s), {} -> {} retained. Peers older than the history floor require a current bundle.",
                bundle.display(),
                report.removed_operations,
                report.operations_before,
                report.operations_after
            ))
        })();
        self.collaboration_status = result.unwrap_or_else(|error| format!("Error: {error}"));
    }

    pub(crate) fn start_collaboration_join(&mut self) {
        if self.collaboration_busy() {
            self.collaboration_status = "A live collaboration session is already running.".into();
            return;
        }
        let bundle = match self.collaboration_path(&self.collaboration_bundle_input) {
            Ok(path) => path,
            Err(error) => {
                self.collaboration_status = format!("Error: {error}");
                return;
            }
        };
        let secret = match self.collaboration_path(&self.collaboration_secret_input) {
            Ok(path) => path,
            Err(error) => {
                self.collaboration_status = format!("Error: {error}");
                return;
            }
        };
        let address = self.collaboration_address_input.trim().to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let result = join_collaboration(&bundle, &address, &secret, None);
            let _ = tx.send(result);
        });
        self.collaboration_rx = Some(rx);
        self.collaboration_status = format!(
            "Joining {} with mutual authentication...",
            self.collaboration_address_input.trim()
        );
    }

    pub(crate) fn generate_collaboration_secret(&mut self) {
        let result = (|| -> std::io::Result<String> {
            let path = self.collaboration_path(&self.collaboration_secret_input)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            generate_collaboration_secret(&path)?;
            Ok(format!(
                "Created private collaboration secret {} (contents hidden).",
                path.display()
            ))
        })();
        self.collaboration_status = result.unwrap_or_else(|error| format!("Error: {error}"));
    }

    fn collaboration_path(&self, input: &str) -> std::io::Result<PathBuf> {
        let input = input.trim();
        if input.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "collaboration path is empty",
            ));
        }
        let path = PathBuf::from(input);
        Ok(if path.is_absolute() {
            path
        } else {
            self.workspace.root().join(path)
        })
    }

    fn start_extension_mutation(&mut self, mutation: ExtensionMutation) {
        if self.extension_busy() {
            self.set_workspace_error("An extension operation is already running.");
            return;
        }
        let request: ExtensionMutationRequest = match self
            .workspace
            .extension_mutation_request(mutation)
        {
            Ok(request) => request,
            Err(error) => {
                self.set_workspace_error(format!("Could not prepare extension operation: {error}"));
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
                    "extension task failed: {error}"
                )))
            });
            let _ = tx.send(result);
        });
        self.extension_mutation_cancel = Some(cancel);
        self.extension_mutation_rx = Some(rx);
        self.set_workspace_status("Validating extension changes in an isolated workspace...");
    }

    pub(crate) fn ask_extension_graph(&mut self, question: &str) {
        let query = aether_graph::parse_query(question);
        self.extension_action_output = self
            .workspace
            .graph()
            .lock()
            .unwrap()
            .answer_query(&query)
            .display();
        self.set_workspace_status("Extension graph query completed.");
    }

    pub(crate) fn open_extension_file(&mut self, path: &str, line: Option<u32>) {
        self.select_file(path);
        if self.workspace.active_file() != Some(path) {
            return;
        }
        if let Some(line) = line {
            let target_line = line.saturating_sub(1) as usize;
            let offset = self
                .workspace
                .buffer_mut()
                .split_inclusive('\n')
                .take(target_line)
                .map(str::len)
                .sum();
            self.editor_jump = Some(offset);
        }
    }

    pub(crate) fn run_extension_command(&mut self, extension_id: String, contribution_id: String) {
        if self.extension_busy() {
            self.set_workspace_error("An extension operation is already running.");
            return;
        }
        let request: ExtensionCommandRequest = match self
            .workspace
            .extension_command_request(&extension_id, &contribution_id)
        {
            Ok(request) => request,
            Err(error) => {
                self.set_workspace_error(format!(
                    "Could not prepare extension validation: {error}"
                ));
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
                    "extension command task failed: {error}"
                )))
            });
            let _ = tx.send(result);
        });
        self.extension_command_cancel = Some(cancel);
        self.extension_command_rx = Some(rx);
        self.extension_action_output.clear();
        self.set_workspace_status("Running extension validation in an isolated workspace...");
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
        if let Some(cancel) = &self.extension_mutation_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        if let Some(cancel) = &self.extension_command_cancel {
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

        if let Some(rx) = self.collaboration_rx.as_mut() {
            match rx.try_recv() {
                Ok(Ok(report)) => {
                    self.collaboration_rx = None;
                    self.collaboration_status = format!(
                        "Live synchronization with {} completed: sent {}, received {}, inserted {}; converged graph {} nodes / {} edges",
                        report.peer,
                        report.sent_operations,
                        report.received_operations,
                        report.inserted_operations,
                        report.node_count,
                        report.edge_count
                    );
                }
                Ok(Err(error)) => {
                    self.collaboration_rx = None;
                    self.collaboration_status = format!("Live synchronization failed: {error}");
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.collaboration_rx = None;
                    self.collaboration_status = "Live synchronization stopped unexpectedly.".into();
                }
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

        if let Some(rx) = self.extension_generation_rx.as_mut() {
            match rx.try_recv() {
                Ok(Ok((recipe, model, source))) => {
                    let digest = recipe.digest().unwrap_or_else(|_| "invalid".into());
                    self.extension_generation_rx = None;
                    if source.is_some() {
                        self.extension_panel_view = ExtensionPanelView::Generate;
                    }
                    self.extension_candidate = Some(recipe);
                    self.extension_candidate_source = source;
                    self.set_workspace_status(format!(
                        "Generated extension recipe with {model}; review digest {digest}"
                    ));
                }
                Ok(Err(error)) => {
                    self.extension_generation_rx = None;
                    self.extension_candidate_source = None;
                    self.set_workspace_error(format!("Extension generation failed: {error}"));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.extension_generation_rx = None;
                    self.extension_candidate_source = None;
                    self.set_workspace_error("Extension generation stopped unexpectedly.");
                }
            }
        }

        if let Some(rx) = self.extension_mutation_rx.as_mut() {
            match rx.try_recv() {
                Ok(Ok(outcome)) => {
                    self.extension_mutation_rx = None;
                    self.extension_mutation_cancel = None;
                    let message = outcome.message;
                    match self.workspace.reload() {
                        Ok(()) => {
                            self.extension_candidate = None;
                            self.extension_candidate_source = None;
                            self.graph_view.reset_for_project();
                            self.impact_nodes.clear();
                            self.ripple_start = None;
                            self.editor_jump = None;
                            self.set_workspace_status(message);
                        }
                        Err(error) => self.set_workspace_error(format!(
                            "{message}, but workspace refresh failed: {error}"
                        )),
                    }
                }
                Ok(Err(error)) => {
                    self.extension_mutation_rx = None;
                    self.extension_mutation_cancel = None;
                    self.set_workspace_error(format!("Extension operation failed: {error}"));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.extension_mutation_rx = None;
                    self.extension_mutation_cancel = None;
                    self.set_workspace_error("Extension operation stopped unexpectedly.");
                }
            }
        }

        if let Some(rx) = self.extension_command_rx.as_mut() {
            match rx.try_recv() {
                Ok(Ok(report)) => {
                    let passed = report.passed();
                    let summary = report.summary();
                    self.extension_action_output = report
                        .steps
                        .iter()
                        .map(|step| {
                            let command = step.command.as_deref().unwrap_or(&step.label);
                            let output = step.output.trim();
                            if output.is_empty() {
                                format!("[{}] {command}", step.status.label())
                            } else {
                                format!("[{}] {command}\n{output}", step.status.label())
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    self.extension_command_rx = None;
                    self.extension_command_cancel = None;
                    if passed {
                        self.set_workspace_status(summary);
                    } else {
                        self.set_workspace_error(summary);
                    }
                }
                Ok(Err(error)) => {
                    self.extension_command_rx = None;
                    self.extension_command_cancel = None;
                    self.set_workspace_error(format!("Extension validation failed: {error}"));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.extension_command_rx = None;
                    self.extension_command_cancel = None;
                    self.set_workspace_error("Extension validation stopped unexpectedly.");
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
                let extension_busy = self.extension_busy();
                ui.add_enabled(
                    !extension_busy,
                    egui::TextEdit::singleline(&mut self.project_path_input).desired_width(320.0),
                );
                if ui
                    .add_enabled(!extension_busy, egui::Button::new("Open"))
                    .on_hover_text("Open project directory")
                    .clicked()
                {
                    self.open_project();
                }
                let has_file = self.workspace.active_file().is_some();
                let pending_agents = self.workspace.has_pending_agent_changes();
                if ui
                    .add_enabled(
                        has_file && self.workspace.is_dirty() && !pending_agents && !extension_busy,
                        egui::Button::new("Save"),
                    )
                    .on_hover_text("Save source and semantic graph")
                    .clicked()
                {
                    self.save_active_file();
                }
                if ui
                    .add_enabled(
                        has_file && self.workspace.is_dirty() && !pending_agents && !extension_busy,
                        egui::Button::new("Discard"),
                    )
                    .on_hover_text("Discard unsaved editor changes")
                    .clicked()
                {
                    self.discard_active_changes();
                }
                if ui
                    .add_enabled(
                        !self.workspace.is_dirty()
                            && !pending_agents
                            && self.swarm_rx.is_none()
                            && !extension_busy,
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
