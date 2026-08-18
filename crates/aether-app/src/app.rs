//! The egui/eframe application shell: state + window bootstrap + panel layout.

use crate::graph_view::GraphViewState;
use crate::panels;
use crate::project::{
    apply_authored_guarantees, apply_reviewed_collaboration_projection, author, build_context_json,
    discover_collaboration_peers, generate_collaboration_identity, generate_collaboration_secret,
    inspect_collaboration_public_identity, join_collaboration, parse_plan,
    review_collaboration_projection, run_for_authoring_with_plan, search_nodes_for_authoring,
    trust_collaboration_identity, trusted_collaboration_identities, AgentValidationOutcome,
    AuthorEvent, AuthorOutcome, AuthoringRunResult, CollaborationProjectionReview, DiscoveredPeer,
    ExtensionCommandRequest, ExtensionMutation, ExtensionMutationOutcome, ExtensionMutationRequest,
    IdentitySummary, LiveSyncReport, ProjectWorkspace, SyncImpact, TrustChange, ValidationReport,
    DEFAULT_MAX_REPAIRS,
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
enum CollaborationProjectionOutcome {
    Reviewed(CollaborationProjectionReview),
    Applied(String),
}
type CollaborationProjectionReceiver =
    tokio::sync::oneshot::Receiver<std::io::Result<CollaborationProjectionOutcome>>;
type AuthorSearchReceiver = tokio::sync::oneshot::Receiver<std::io::Result<Vec<AuthorSearchHit>>>;
type AuthorRunReceiver = tokio::sync::oneshot::Receiver<std::io::Result<AuthorOutcome>>;
type AuthorRunAuthoredReceiver =
    tokio::sync::oneshot::Receiver<std::io::Result<AuthoringRunResult>>;
type AuthorContextJsonReceiver = tokio::sync::oneshot::Receiver<std::io::Result<String>>;

/// How many `AuthorEvent`s the Author tab's log keeps. A local-model repair
/// loop runs indefinitely in principle (bounded only by provider count ×
/// `max_repairs`), so unlike `transcript` (wholesale-replaced once per
/// swarm run) this log is genuinely appended-to and needs its own cap.
const AUTHOR_LOG_CAP: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RightPanel {
    Agents,
    Extensions,
    Collaboration,
    Author,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthorMode {
    Local,
    External,
}

/// One node search hit rendered as a checkbox in the Author tab, mapped
/// down from `authoring_context::AuthoringContext` at search time — the
/// panel only ever needs the path/score/selection, not the multi-KB JSON
/// schema/source context selection also produces (an eventual Run
/// re-derives all of that itself from the checked paths).
#[derive(Debug, Clone)]
pub(crate) struct AuthorSearchHit {
    pub(crate) path: String,
    pub(crate) score: Option<f32>,
    pub(crate) selected: bool,
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
    pub(crate) collaboration_presence_input: String,
    pub(crate) collaboration_discovery_input: String,
    pub(crate) collaboration_discovered_peers: Vec<DiscoveredPeer>,
    pub(crate) collaboration_identity_enabled: bool,
    pub(crate) collaboration_identity_input: String,
    pub(crate) collaboration_identity_public_input: String,
    pub(crate) collaboration_trust_input: String,
    pub(crate) collaboration_peer_identity_input: String,
    pub(crate) collaboration_identity_review: Option<IdentitySummary>,
    pub(crate) collaboration_trusted_identities: Vec<IdentitySummary>,
    pub(crate) collaboration_status: String,
    collaboration_rx: Option<CollaborationReceiver>,
    collaboration_projection_rx: Option<CollaborationProjectionReceiver>,
    collaboration_projection_approval: Option<String>,
    /// Nodes currently lit by the impact ripple: node_id -> hop distance from changed node.
    /// Distance 0 = the edited function itself; 1 = direct callers; 2 = their callers; etc.
    pub(crate) impact_nodes: HashMap<NodeId, u32>,
    /// When the current ripple animation started. None when no ripple is active.
    pub(crate) ripple_start: Option<Instant>,
    /// Byte offset to select after graph-to-editor navigation.
    pub(crate) editor_jump: Option<usize>,

    // ── Author tab ───────────────────────────────────────────────────────────
    pub(crate) author_intent: String,
    pub(crate) author_search_results: Vec<AuthorSearchHit>,
    author_search_rx: Option<AuthorSearchReceiver>,
    pub(crate) author_mode: AuthorMode,
    pub(crate) author_dry_run: bool,
    pub(crate) author_max_repairs: usize,
    pub(crate) author_log: std::collections::VecDeque<AuthorEvent>,
    author_progress_rx: Option<tokio::sync::mpsc::UnboundedReceiver<AuthorEvent>>,
    author_run_rx: Option<AuthorRunReceiver>,
    pub(crate) author_pasted_plan: String,
    pub(crate) author_authored_by: String,
    author_run_authored_rx: Option<AuthorRunAuthoredReceiver>,
    author_context_json_rx: Option<AuthorContextJsonReceiver>,
    pub(crate) author_live_run_confirming: bool,
    pub(crate) author_last_result: Option<String>,
    pub(crate) author_last_error: Option<String>,

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
            collaboration_presence_input: String::new(),
            collaboration_discovery_input: ".bitcode/peers".into(),
            collaboration_discovered_peers: Vec::new(),
            collaboration_identity_enabled: false,
            collaboration_identity_input: ".bitcode/collaboration.identity".into(),
            collaboration_identity_public_input: ".bitcode/collaboration.identity.pub".into(),
            collaboration_trust_input: ".bitcode/collaboration.trust".into(),
            collaboration_peer_identity_input: ".bitcode/peer.identity.pub".into(),
            collaboration_identity_review: None,
            collaboration_trusted_identities: Vec::new(),
            collaboration_status:
                "Initialize a graph replica or inspect an existing collaboration bundle.".into(),
            collaboration_rx: None,
            collaboration_projection_rx: None,
            collaboration_projection_approval: None,
            impact_nodes: HashMap::new(),
            ripple_start: None,
            editor_jump: None,
            author_intent: String::new(),
            author_search_results: Vec::new(),
            author_search_rx: None,
            author_mode: AuthorMode::Local,
            author_dry_run: true,
            author_max_repairs: DEFAULT_MAX_REPAIRS,
            author_log: std::collections::VecDeque::new(),
            author_progress_rx: None,
            author_run_rx: None,
            author_pasted_plan: String::new(),
            author_authored_by: String::new(),
            author_run_authored_rx: None,
            author_context_json_rx: None,
            author_live_run_confirming: false,
            author_last_result: None,
            author_last_error: None,
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
        if self.author_busy() {
            self.set_workspace_error("Wait for the authoring operation before opening a project.");
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
                self.collaboration_discovered_peers.clear();
                self.collaboration_identity_review = None;
                self.collaboration_trusted_identities.clear();
                self.author_intent.clear();
                self.author_search_results.clear();
                self.author_log.clear();
                self.author_pasted_plan.clear();
                self.author_authored_by.clear();
                self.author_live_run_confirming = false;
                self.author_last_result = None;
                self.author_last_error = None;
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
        if self.author_busy() {
            self.set_workspace_error("Wait for the authoring operation before reloading.");
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
                self.collaboration_discovered_peers.clear();
                self.collaboration_identity_review = None;
                self.collaboration_trusted_identities.clear();
                self.author_search_results.clear();
                self.author_log.clear();
                self.author_live_run_confirming = false;
                self.author_last_result = None;
                self.author_last_error = None;
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

    /// Runs the GUI equivalent of `--nodes`' search half: the same
    /// `search_nodes_for_authoring` (built on `authoring_context::
    /// build_authoring_context`, the exact function `bitcode do` and
    /// `bitcode context` both call) against a graph rebuilt fresh from disk,
    /// so what the checkboxes show is exactly what a subsequent Run will
    /// search against. Runs on a plain OS thread (no `.await` anywhere in
    /// this path), matching `start_collaboration_join`.
    /// Read-only: never gated on `author_busy()`. A repeated click always
    /// supersedes whatever search is still in flight — the previous
    /// receiver is dropped, so an older, slower search can never overwrite
    /// a newer one's results (the classic out-of-order-async-response
    /// trap), and the button is never disabled for a reason unrelated to
    /// searching itself.
    pub(crate) fn run_author_search(&mut self) {
        let intent = self.author_intent.trim().to_string();
        if intent.is_empty() {
            self.author_last_error = Some("Enter an intent before searching.".into());
            return;
        }
        self.author_last_error = None;
        self.author_last_result = None;
        let root = self.workspace.root().to_path_buf();
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let result = search_nodes_for_authoring(&root, &intent).map(|hits| {
                // Only the top hit starts checked. gap #15's score floor
                // exists specifically to shrink what the model can touch;
                // defaulting every hit to checked (including runner-ups
                // that only barely cleared NODE_SCORE_FLOOR_RATIO) worked
                // against that on real intents — e.g. "make hello end with
                // an exclamation mark" pulled in an unrelated debugger
                // function as a legal edit target. The user opts more
                // nodes in deliberately; they should never have to opt
                // stray ones out.
                hits.into_iter()
                    .enumerate()
                    .map(|(index, (path, score))| AuthorSearchHit {
                        path,
                        score,
                        selected: index == 0,
                    })
                    .collect()
            });
            let _ = tx.send(result);
        });
        self.author_search_rx = Some(rx);
    }

    /// Mode 1 ("local model") Run: calls the exact same `author()` function
    /// `bitcode do` calls, pinned to whatever search hits are currently
    /// checked. A non-dry run requires clicking Run twice — the first click
    /// only arms `author_live_run_confirming`, mirroring the
    /// remove-extension confirm/cancel idiom, since a GUI button that
    /// silently writes to the tree has no command line to review first.
    pub(crate) fn run_author(&mut self) {
        if self.author_write_busy() {
            self.author_last_error = Some("Wait for the current authoring run to finish.".into());
            return;
        }
        let intent = self.author_intent.trim().to_string();
        if intent.is_empty() {
            self.author_last_error = Some("Enter an intent before running.".into());
            return;
        }
        let checked: Vec<String> = self
            .author_search_results
            .iter()
            .filter(|hit| hit.selected)
            .map(|hit| hit.path.clone())
            .collect();
        if checked.is_empty() {
            self.author_last_error =
                Some("Search and select at least one node before running.".into());
            return;
        }
        if !self.author_dry_run && !self.author_live_run_confirming {
            self.author_live_run_confirming = true;
            return;
        }
        self.author_live_run_confirming = false;
        self.author_last_error = None;
        self.author_last_result = None;
        self.author_log.clear();

        let root = self.workspace.root().to_path_buf();
        let dry = self.author_dry_run;
        let max_repairs = self.author_max_repairs;
        let router = self.router.clone();
        let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.rt.spawn(async move {
            let result = author(
                &root,
                &intent,
                Some(&checked),
                dry,
                max_repairs,
                &router,
                move |event| {
                    let _ = progress_tx.send(event);
                },
            )
            .await;
            let _ = tx.send(result);
        });
        self.author_progress_rx = Some(progress_rx);
        self.author_run_rx = Some(rx);
    }

    /// Mode 2 ("external model") "Run authored": parses the pasted plan via
    /// the exact same `planfile::parse_plan`, applies the exact same
    /// `--authored` guarantees via `apply_authored_guarantees`, and executes
    /// through `run_for_authoring_with_plan` — the same guarantee logic and
    /// the same executor `plan run --authored` uses, just reached without a
    /// temp file. Same confirm-before-live-run gate as Mode 1.
    pub(crate) fn run_author_authored(&mut self) {
        if self.author_write_busy() {
            self.author_last_error = Some("Wait for the current authoring run to finish.".into());
            return;
        }
        if self.author_pasted_plan.trim().is_empty() {
            self.author_last_error = Some("Paste a plan before running.".into());
            return;
        }
        if !self.author_dry_run && !self.author_live_run_confirming {
            self.author_live_run_confirming = true;
            return;
        }
        self.author_live_run_confirming = false;
        self.author_last_error = None;
        self.author_last_result = None;

        let root = self.workspace.root().to_path_buf();
        let dry = self.author_dry_run;
        let pasted_plan = self.author_pasted_plan.clone();
        let authored_by = {
            let trimmed = self.author_authored_by.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let result = (|| -> std::io::Result<AuthoringRunResult> {
                let mut plan = parse_plan(&pasted_plan)?;
                apply_authored_guarantees(&root, &mut plan)?;
                run_for_authoring_with_plan(&root, plan, dry, None, authored_by.as_deref())
            })();
            let _ = tx.send(result);
        });
        self.author_run_authored_rx = Some(rx);
    }

    /// Puts exactly what `bitcode context --json` would print onto the
    /// clipboard, via the same `build_context_json` that command calls.
    ///
    /// Read-only: never gated on `author_busy()`. `checked`/`pinned` are
    /// recomputed here, at click time, from `author_search_results` —
    /// never cached from an earlier click or an earlier frame. Gating this
    /// on any other in-flight author operation previously meant a click
    /// while, say, a full-repo Search or a slow Run was still in flight got
    /// silently dropped, and whatever was already on the clipboard (from an
    /// earlier click, possibly with different checkboxes and a different
    /// `plan_id`) was left untouched — indistinguishable from "the button
    /// is serving a cached result". A repeated click here now always
    /// supersedes: the previous receiver is dropped (its now-orphaned
    /// thread's `tx.send` is simply ignored), so only the latest click's
    /// result is ever applied to the clipboard.
    pub(crate) fn copy_author_context_json(&mut self) {
        let intent = self.author_intent.trim().to_string();
        if intent.is_empty() {
            self.author_last_error = Some("Enter an intent before copying context.".into());
            return;
        }
        self.author_last_error = None;
        let checked: Vec<String> = self
            .author_search_results
            .iter()
            .filter(|hit| hit.selected)
            .map(|hit| hit.path.clone())
            .collect();
        let pinned = (!checked.is_empty()).then_some(checked);
        let root = self.workspace.root().to_path_buf();
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let result = build_context_json(&root, &intent, pinned.as_deref()).and_then(|value| {
                serde_json::to_string_pretty(&value)
                    .map_err(|error| std::io::Error::other(error.to_string()))
            });
            let _ = tx.send(result);
        });
        self.author_context_json_rx = Some(rx);
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
        self.collaboration_rx.is_some() || self.collaboration_projection_rx.is_some()
    }

    /// Used only to gate opening/reloading a project: swapping `workspace`
    /// out from under any in-flight author background thread (which
    /// captured the old root by value) would leave its eventual result
    /// describing the wrong project.
    pub(crate) fn author_busy(&self) -> bool {
        self.author_search_rx.is_some()
            || self.author_run_rx.is_some()
            || self.author_run_authored_rx.is_some()
            || self.author_context_json_rx.is_some()
    }

    /// Gates Run / Run-authored specifically: unlike Search or Copy Context
    /// JSON (read-only, always safe to supersede), these two write to the
    /// real tree when not dry, so two of them must never run concurrently
    /// against the same project.
    pub(crate) fn author_write_busy(&self) -> bool {
        self.author_run_rx.is_some() || self.author_run_authored_rx.is_some()
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
            let members = replica
                .members()
                .map_err(std::io::Error::other)?
                .into_iter()
                .map(|member| {
                    if &member == replica.actor() {
                        format!("{member} (local)")
                    } else if replica.acknowledgements().any(|(peer, _)| peer == &member) {
                        format!("{member} (acknowledged)")
                    } else {
                        format!("{member} (awaiting acknowledgement)")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!(
                "{}\nactor: {}\nactive members: {}\nversion: {}\ncompacted through: {}\ndurable acknowledgements: {}\noperations: {}\ndurable operation attestations: {} / {}\ngraph: {} nodes / {} edges",
                bundle.display(),
                replica.actor(),
                members,
                version,
                if floor.is_empty() { "none" } else { &floor },
                if acknowledgements.is_empty() {
                    "none"
                } else {
                    &acknowledgements
                },
                replica.operation_count(),
                replica.attestation_count(),
                replica
                    .operations()
                    .filter(|(dot, _)| !dot.actor.is_bootstrap())
                    .count(),
                graph.node_count(),
                graph.edge_count()
            ))
        })();
        self.collaboration_status = result.unwrap_or_else(|error| format!("Error: {error}"));
    }

    pub(crate) fn modify_collaboration_member(&mut self, add: bool) {
        let result = (|| -> std::io::Result<String> {
            self.collaboration_snapshot_ready()?;
            let bundle = self.collaboration_path(&self.collaboration_bundle_input)?;
            let actor = ActorId::new(self.collaboration_actor_input.trim())
                .map_err(std::io::Error::other)?;
            let mut replica = GraphReplica::load(&bundle).map_err(std::io::Error::other)?;
            if add {
                replica
                    .add_member(actor.clone())
                    .map_err(std::io::Error::other)?;
            } else {
                replica
                    .remove_member(&actor)
                    .map_err(std::io::Error::other)?;
            }
            replica.save(&bundle).map_err(std::io::Error::other)?;
            let members = replica
                .members()
                .map_err(std::io::Error::other)?
                .into_iter()
                .map(|member| member.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!(
                "{} member {actor}; active roster: {members}",
                if add { "Added" } else { "Removed" }
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
        let presence = self.collaboration_presence_input.clone();
        let identity = if self.collaboration_identity_enabled {
            let identity = match self.collaboration_path(&self.collaboration_identity_input) {
                Ok(path) => path,
                Err(error) => {
                    self.collaboration_status = format!("Error: {error}");
                    return;
                }
            };
            let trust = match self.collaboration_path(&self.collaboration_trust_input) {
                Ok(path) => path,
                Err(error) => {
                    self.collaboration_status = format!("Error: {error}");
                    return;
                }
            };
            Some((identity, trust))
        } else {
            None
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let (identity_file, trust_store) = identity
                .as_ref()
                .map(|(identity, trust)| (Some(identity.as_path()), Some(trust.as_path())))
                .unwrap_or((None, None));
            let result = join_collaboration(
                &bundle,
                &address,
                &secret,
                None,
                Some(&presence),
                identity_file,
                trust_store,
            );
            let _ = tx.send(result);
        });
        self.collaboration_rx = Some(rx);
        self.collaboration_status = format!(
            "Joining {} with {}...",
            self.collaboration_address_input.trim(),
            if self.collaboration_identity_enabled {
                "group-secret, pinned Ed25519 authentication, and signed-operation verification"
            } else {
                "group-secret authentication (legacy mode)"
            }
        );
    }

    pub(crate) fn scan_collaboration_peers(&mut self) {
        let result = (|| -> std::io::Result<Vec<DiscoveredPeer>> {
            let bundle = self.collaboration_path(&self.collaboration_bundle_input)?;
            let secret = self.collaboration_path(&self.collaboration_secret_input)?;
            let directory = self.collaboration_path(&self.collaboration_discovery_input)?;
            let scan = discover_collaboration_peers(&bundle, &directory, &secret)?;
            let ignored = scan.ignored_entries;
            self.collaboration_status = format!(
                "Discovered {} authenticated active local peer(s); ignored {} invalid, stale, unrelated, or unauthorized entr{}.",
                scan.peers.len(),
                ignored,
                if ignored == 1 { "y" } else { "ies" }
            );
            Ok(scan.peers)
        })();
        match result {
            Ok(peers) => self.collaboration_discovered_peers = peers,
            Err(error) => {
                self.collaboration_discovered_peers.clear();
                self.collaboration_status = format!("Local peer discovery failed: {error}");
            }
        }
    }

    pub(crate) fn generate_actor_identity(&mut self) {
        let result = (|| -> std::io::Result<String> {
            let bundle = self.collaboration_path(&self.collaboration_bundle_input)?;
            let private = self.collaboration_path(&self.collaboration_identity_input)?;
            let public = self.collaboration_path(&self.collaboration_identity_public_input)?;
            for path in [&private, &public] {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            let identity = generate_collaboration_identity(&bundle, &private, &public)?;
            Ok(format!(
                "Generated Ed25519 identity for {}.\nPrivate: {}\nPublic: {}\nSHA-256 fingerprint: {}\nDistribute only the public file and verify this fingerprint out of band.",
                identity.actor,
                private.display(),
                public.display(),
                identity.fingerprint
            ))
        })();
        self.collaboration_status = result.unwrap_or_else(|error| format!("Error: {error}"));
    }

    pub(crate) fn review_peer_identity(&mut self) {
        let result = (|| -> std::io::Result<IdentitySummary> {
            let public = self.collaboration_path(&self.collaboration_peer_identity_input)?;
            inspect_collaboration_public_identity(&public)
        })();
        match result {
            Ok(identity) => {
                self.collaboration_status = format!(
                    "Review public identity for {}.\nEd25519 SHA-256: {}\nVerify this fingerprint out of band, then choose Trust reviewed identity.",
                    identity.actor, identity.fingerprint
                );
                self.collaboration_identity_review = Some(identity);
            }
            Err(error) => {
                self.collaboration_identity_review = None;
                self.collaboration_status = format!("Identity review failed: {error}");
            }
        }
    }

    pub(crate) fn trust_reviewed_identity(&mut self) {
        let Some(approved) = self.collaboration_identity_review.clone() else {
            self.collaboration_status =
                "Review a peer public identity and verify its fingerprint first.".into();
            return;
        };
        let result = (|| -> std::io::Result<(String, Vec<IdentitySummary>)> {
            let public = self.collaboration_path(&self.collaboration_peer_identity_input)?;
            let trust_store = self.collaboration_path(&self.collaboration_trust_input)?;
            if let Some(parent) = trust_store.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let current = inspect_collaboration_public_identity(&public)?;
            if current != approved {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "public identity changed after review; review its new fingerprint",
                ));
            }
            let change =
                trust_collaboration_identity(&trust_store, &public, &approved.fingerprint)?;
            let message = match change {
                TrustChange::Added(identity) => format!(
                    "Pinned {} to Ed25519 SHA-256 {}.",
                    identity.actor, identity.fingerprint
                ),
                TrustChange::AlreadyTrusted(identity) => format!(
                    "{} is already pinned to Ed25519 SHA-256 {}.",
                    identity.actor, identity.fingerprint
                ),
                TrustChange::Rotated { .. } | TrustChange::Removed(_) => {
                    return Err(std::io::Error::other(
                        "unexpected trust mutation while adding an identity",
                    ))
                }
            };
            Ok((message, trusted_collaboration_identities(&trust_store)?))
        })();
        match result {
            Ok((message, identities)) => {
                self.collaboration_identity_review = None;
                self.collaboration_trusted_identities = identities;
                self.collaboration_status = message;
            }
            Err(error) => {
                self.collaboration_identity_review = None;
                self.collaboration_status = format!("Identity trust failed: {error}");
            }
        }
    }

    pub(crate) fn refresh_trusted_identities(&mut self) {
        let result = (|| -> std::io::Result<Vec<IdentitySummary>> {
            let trust_store = self.collaboration_path(&self.collaboration_trust_input)?;
            trusted_collaboration_identities(&trust_store)
        })();
        match result {
            Ok(identities) => {
                self.collaboration_status =
                    format!("Loaded {} pinned actor identity(s).", identities.len());
                self.collaboration_trusted_identities = identities;
            }
            Err(error) => {
                self.collaboration_trusted_identities.clear();
                self.collaboration_status = format!("Identity trust-store load failed: {error}");
            }
        }
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

    pub(crate) fn start_collaboration_projection_review(&mut self) {
        if self.collaboration_busy() {
            self.collaboration_status = "A collaboration operation is already running.".into();
            return;
        }
        if let Err(error) = self.collaboration_snapshot_ready() {
            self.collaboration_status = format!("Error: {error}");
            return;
        }
        let bundle = match self.collaboration_path(&self.collaboration_bundle_input) {
            Ok(path) => path,
            Err(error) => {
                self.collaboration_status = format!("Error: {error}");
                return;
            }
        };
        let root = self.workspace.root().to_path_buf();
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let result = review_collaboration_projection(&root, &bundle)
                .map(CollaborationProjectionOutcome::Reviewed);
            let _ = tx.send(result);
        });
        self.collaboration_projection_approval = None;
        self.collaboration_projection_rx = Some(rx);
        self.collaboration_status =
            "Rebuilding and checking the remote whole-file projection...".into();
    }

    pub(crate) fn start_collaboration_projection_apply(&mut self) {
        if self.collaboration_busy() {
            self.collaboration_status = "A collaboration operation is already running.".into();
            return;
        }
        if let Err(error) = self.collaboration_snapshot_ready() {
            self.collaboration_status = format!("Error: {error}");
            return;
        }
        let Some(approval) = self.collaboration_projection_approval.take() else {
            self.collaboration_status =
                "Review a conflict-free projection before applying it.".into();
            return;
        };
        let bundle = match self.collaboration_path(&self.collaboration_bundle_input) {
            Ok(path) => path,
            Err(error) => {
                self.collaboration_status = format!("Error: {error}");
                return;
            }
        };
        let root = self.workspace.root().to_path_buf();
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let result = apply_reviewed_collaboration_projection(&root, &bundle, &approval)
                .map(CollaborationProjectionOutcome::Applied);
            let _ = tx.send(result);
        });
        self.collaboration_projection_rx = Some(rx);
        self.collaboration_status =
            "Validating the reviewed projection in an isolated workspace...".into();
    }

    pub(crate) fn collaboration_projection_approved(&self) -> bool {
        self.collaboration_projection_approval.is_some()
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
                    let peer_presence = report
                        .peer_presence()
                        .map(|status| format!("status {status:?}"))
                        .unwrap_or_else(|| "no status shared".into());
                    let peer_identity = report
                        .peer_identity_fingerprint()
                        .map(|fingerprint| format!("pinned Ed25519 {fingerprint}"))
                        .unwrap_or_else(|| "legacy group-secret identity".into());
                    self.collaboration_status = format!(
                        "Live synchronization with {} completed ({peer_presence}; {peer_identity}): sent {}, received {}, inserted {}; converged graph {} nodes / {} edges",
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

        if let Some(rx) = self.collaboration_projection_rx.as_mut() {
            match rx.try_recv() {
                Ok(Ok(CollaborationProjectionOutcome::Reviewed(review))) => {
                    self.collaboration_projection_rx = None;
                    self.collaboration_projection_approval = review.approval_digest;
                    self.collaboration_status = review.text;
                }
                Ok(Ok(CollaborationProjectionOutcome::Applied(summary))) => {
                    self.collaboration_projection_rx = None;
                    self.collaboration_projection_approval = None;
                    let root = self.workspace.root().to_path_buf();
                    match ProjectWorkspace::open(root) {
                        Ok(workspace) => {
                            self.workspace = workspace;
                            self.py_file = active_python_path(&self.workspace).unwrap_or_default();
                            self.graph_view.reset_for_project();
                            self.collaboration_status = summary;
                            self.set_workspace_status(workspace_summary(&self.workspace));
                        }
                        Err(error) => {
                            self.collaboration_status =
                                format!("Projection committed, but reload failed: {error}");
                        }
                    }
                }
                Ok(Err(error)) => {
                    self.collaboration_projection_rx = None;
                    self.collaboration_projection_approval = None;
                    self.collaboration_status = format!("Projection failed: {error}");
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.collaboration_projection_rx = None;
                    self.collaboration_projection_approval = None;
                    self.collaboration_status = "Projection operation stopped unexpectedly.".into();
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

        // Poll the Author tab's background node search.
        if let Some(rx) = self.author_search_rx.as_mut() {
            match rx.try_recv() {
                Ok(Ok(hits)) => {
                    self.author_search_rx = None;
                    self.author_search_results = hits;
                }
                Ok(Err(error)) => {
                    self.author_search_rx = None;
                    self.author_last_error = Some(format!("Search failed: {error}"));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.author_search_rx = None;
                    self.author_last_error = Some("Search stopped unexpectedly.".into());
                }
            }
        }

        // Drain the Author tab's local-model run progress into a capped
        // log before checking whether the run itself has finished.
        if let Some(rx) = self.author_progress_rx.as_mut() {
            while let Ok(event) = rx.try_recv() {
                if self.author_log.len() >= AUTHOR_LOG_CAP {
                    self.author_log.pop_front();
                }
                self.author_log.push_back(event);
            }
        }
        if let Some(rx) = self.author_run_rx.as_mut() {
            match rx.try_recv() {
                Ok(Ok(AuthorOutcome::NoMatches)) => {
                    self.author_run_rx = None;
                    self.author_progress_rx = None;
                    self.author_last_result = None;
                    self.author_last_error = Some("No nodes matched; nothing to author.".into());
                }
                Ok(Ok(AuthorOutcome::Authored {
                    provider,
                    model,
                    repairs,
                    report_path,
                    report_json,
                })) => {
                    self.author_run_rx = None;
                    self.author_progress_rx = None;
                    self.author_last_error = None;
                    let mut summary = format!(
                        "plan authored by {provider} ({model}) after {repairs} repair attempt(s)"
                    );
                    if let Some(path) = &report_path {
                        summary.push_str(&format!("\nreport: {}", path.display()));
                    }
                    if let Some(rendered) = &report_json {
                        summary.push_str(&format!(
                            "\nreport (dry run, not written to disk):\n{rendered}"
                        ));
                    }
                    self.author_last_result = Some(summary);
                }
                Ok(Err(error)) => {
                    self.author_run_rx = None;
                    self.author_progress_rx = None;
                    self.author_last_result = None;
                    self.author_last_error = Some(error.to_string());
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.author_run_rx = None;
                    self.author_progress_rx = None;
                    self.author_last_error = Some("Authoring run stopped unexpectedly.".into());
                }
            }
        }

        // Poll the Author tab's "Run authored" flow (Mode 2, external plan).
        if let Some(rx) = self.author_run_authored_rx.as_mut() {
            match rx.try_recv() {
                Ok(Ok(result)) => {
                    self.author_run_authored_rx = None;
                    self.author_last_error = None;
                    let mut summary = if result.passed {
                        "authored plan passed".to_string()
                    } else {
                        format!("authored plan FAILED\n{}", result.diagnostic)
                    };
                    if let Some(path) = &result.report_path {
                        summary.push_str(&format!("\nreport: {}", path.display()));
                    }
                    if let Some(rendered) = &result.report_json {
                        summary.push_str(&format!(
                            "\nreport (dry run, not written to disk):\n{rendered}"
                        ));
                    }
                    self.author_last_result = Some(summary);
                }
                Ok(Err(error)) => {
                    self.author_run_authored_rx = None;
                    self.author_last_result = None;
                    self.author_last_error = Some(error.to_string());
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.author_run_authored_rx = None;
                    self.author_last_error = Some("Authoring run stopped unexpectedly.".into());
                }
            }
        }

        // Poll the Author tab's "Copy context JSON" background build.
        if let Some(rx) = self.author_context_json_rx.as_mut() {
            match rx.try_recv() {
                Ok(Ok(json)) => {
                    self.author_context_json_rx = None;
                    ctx.copy_text(json);
                    self.author_last_error = None;
                }
                Ok(Err(error)) => {
                    self.author_context_json_rx = None;
                    self.author_last_error = Some(format!("Copy context JSON failed: {error}"));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.author_context_json_rx = None;
                    self.author_last_error = Some("Copy context JSON stopped unexpectedly.".into());
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
