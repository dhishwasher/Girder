#[cfg(feature = "gui")]
use crate::project::commands::extensions::{ExtensionMutation, ExtensionMutationRequest};
use crate::project::config::ProjectConfig;
use crate::project::projection::{
    capture_agent_baseline, plan_authored_functions, ProjectionBaseline,
};
#[cfg(feature = "gui")]
use crate::project::source::read_project_bytes_bounded;
use crate::project::source::{
    collect_sources_with_config, commit_project_writes, graph_project_write, load_graph_snapshot,
    read_project_bytes, recover_project_transactions, ProjectWrite,
};
#[cfg(feature = "gui")]
use crate::project::validation::validate_extension_command;
use crate::project::validation::{validate_candidate, ValidationReport};
use aether_builder::{GraphBuilder, Lang};
#[cfg(feature = "gui")]
use aether_extensions::{
    find_record, records, CommandAction, Contribution, ExtensionError, ExtensionRecord,
    ExtensionState,
};
use aether_graph::{NodeId, SemanticGraph};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectFile {
    relative: String,
    language: Lang,
}

impl ProjectFile {
    #[cfg(test)]
    pub(crate) fn relative(&self) -> &str {
        &self.relative
    }

    pub(crate) fn language(&self) -> Lang {
        self.language
    }
}

#[derive(Debug, Default)]
pub(crate) struct SyncImpact {
    pub(crate) nodes: HashMap<NodeId, u32>,
}

pub(crate) struct ProjectWorkspace {
    root: PathBuf,
    config: ProjectConfig,
    files: Vec<ProjectFile>,
    graph: Arc<Mutex<SemanticGraph>>,
    builder: GraphBuilder,
    active_file: Option<String>,
    buffer: String,
    clean_buffer: String,
    clean_graph_bytes: Option<Vec<u8>>,
    open_status: String,
    agent_transaction: Option<AgentTransaction>,
}

struct AgentTransaction {
    graph: SemanticGraph,
    projection: ProjectionBaseline,
    validation: Option<ValidationReport>,
    validated_graph_bytes: Option<Vec<u8>>,
}

pub(crate) struct AgentValidationRequest {
    root: PathBuf,
    config: ProjectConfig,
    graph: SemanticGraph,
    module: String,
    baseline: ProjectionBaseline,
}

#[cfg(feature = "gui")]
pub(crate) struct ExtensionCommandRequest {
    root: PathBuf,
    config: ProjectConfig,
    argv: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct AgentValidationOutcome {
    pub(crate) report: ValidationReport,
    graph_bytes: Vec<u8>,
}

impl AgentValidationRequest {
    pub(crate) fn run(self, cancel: &Arc<AtomicBool>) -> std::io::Result<AgentValidationOutcome> {
        let graph_bytes = self
            .graph
            .to_bytes()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let plan = plan_authored_functions(
            &self.root,
            &self.config,
            &self.graph,
            &self.module,
            Some(&self.baseline),
        )?;
        let report = validate_candidate(&self.root, &self.config, plan.writes(), cancel)?;
        Ok(AgentValidationOutcome {
            report,
            graph_bytes,
        })
    }
}

#[cfg(feature = "gui")]
impl ExtensionCommandRequest {
    pub(crate) fn run(self, cancel: &Arc<AtomicBool>) -> std::io::Result<ValidationReport> {
        validate_extension_command(&self.root, &self.config, self.argv, cancel)
    }
}

impl ProjectWorkspace {
    pub(crate) fn open(root: impl AsRef<Path>) -> std::io::Result<Self> {
        let root = std::fs::canonicalize(root.as_ref()).map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!(
                    "could not open project {}: {error}",
                    root.as_ref().display()
                ),
            )
        })?;
        if !root.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("project root is not a directory: {}", root.display()),
            ));
        }

        let recovered = recover_project_transactions(&root)?;
        let config = ProjectConfig::load(&root)?;
        let persisted = load_graph_snapshot(&root, &config)?;
        let sources = collect_sources_with_config(&root, &config)?;
        let mut source_graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        let mut files = Vec::with_capacity(sources.len());

        for (absolute, relative) in sources {
            let source = std::fs::read_to_string(&absolute).map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!("could not read {}: {error}", absolute.display()),
                )
            })?;
            let Some(language) = Lang::from_path(&relative) else {
                continue;
            };
            builder.load_file(&mut source_graph, &relative, &source);
            files.push(ProjectFile { relative, language });
        }

        let (graph, graph_status) = match (&persisted.graph, &persisted.error) {
            (Some(durable), _) => {
                let (graph, report) = SemanticGraph::reconcile_persisted(source_graph, durable);
                (
                    graph,
                    format!(
                        "reconciled durable graph ({} metadata node(s), {} graph-owned node(s), {} source update(s))",
                        report.metadata_nodes, report.graph_owned_nodes, report.source_changes
                    ),
                )
            }
            (None, Some(error)) => (
                source_graph,
                format!("rebuilt from source; persisted graph was invalid: {error}"),
            ),
            (None, None) => (source_graph, "built graph from source".to_string()),
        };
        let open_status = if recovered == 0 {
            graph_status
        } else {
            format!("recovered {recovered} interrupted transaction(s); {graph_status}")
        };

        let active_file = preferred_file(&files).map(|file| file.relative.clone());
        let buffer = match &active_file {
            Some(relative) => std::fs::read_to_string(root.join(relative))?,
            None => String::new(),
        };

        Ok(Self {
            root,
            config,
            files,
            graph: Arc::new(Mutex::new(graph)),
            builder,
            active_file,
            clean_buffer: buffer.clone(),
            buffer,
            clean_graph_bytes: persisted.bytes,
            open_status,
            agent_transaction: None,
        })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn files(&self) -> &[ProjectFile] {
        &self.files
    }

    pub(crate) fn graph(&self) -> &Arc<Mutex<SemanticGraph>> {
        &self.graph
    }

    pub(crate) fn open_status(&self) -> &str {
        &self.open_status
    }

    pub(crate) fn agent_target(&self) -> (&str, &str) {
        (
            &self.config.agents.output_module,
            &self.config.agents.output_file,
        )
    }

    #[cfg(feature = "gui")]
    pub(crate) fn agent_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.config.agents.timeout_seconds)
    }

    pub(crate) fn active_file(&self) -> Option<&str> {
        self.active_file.as_deref()
    }

    pub(crate) fn active_language(&self) -> Option<Lang> {
        let active = self.active_file()?;
        self.files
            .iter()
            .find(|file| file.relative == active)
            .map(ProjectFile::language)
    }

    pub(crate) fn buffer(&self) -> &str {
        &self.buffer
    }

    pub(crate) fn buffer_mut(&mut self) -> &mut String {
        &mut self.buffer
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.buffer != self.clean_buffer
    }

    pub(crate) fn select_file(&mut self, relative: &str) -> std::io::Result<()> {
        if self.has_pending_agent_changes() {
            return Err(pending_agent_error("switch files"));
        }
        if self.is_dirty() {
            return Err(dirty_error("switch files"));
        }
        if !self.files.iter().any(|file| file.relative == relative) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("project file is not indexed: {relative}"),
            ));
        }
        if let Some(current) = self.active_file.clone() {
            let current_source = std::fs::read_to_string(self.root.join(&current))?;
            if current_source != self.clean_buffer {
                self.update_graph_file(&current, &current_source)?;
            }
            if current == relative {
                self.clean_buffer = current_source.clone();
                self.buffer = current_source;
                return Ok(());
            }
        }
        let source = std::fs::read_to_string(self.root.join(relative)).map_err(|error| {
            std::io::Error::new(error.kind(), format!("could not read {relative}: {error}"))
        })?;
        self.update_graph_file(relative, &source)?;
        self.active_file = Some(relative.to_string());
        self.clean_buffer = source.clone();
        self.buffer = source;
        Ok(())
    }

    pub(crate) fn sync_buffer_to_graph(&mut self) -> std::io::Result<SyncImpact> {
        if self.has_pending_agent_changes() {
            return Err(pending_agent_error("edit source"));
        }
        let Some(file) = self.active_file.clone() else {
            return Ok(SyncImpact::default());
        };
        let graph_handle = self.graph.clone();
        let mut graph = graph_handle
            .lock()
            .map_err(|_| std::io::Error::other("semantic graph lock is poisoned"))?;
        let before: HashMap<NodeId, String> = graph
            .nodes()
            .map(|node| (node.id, node.source.clone()))
            .collect();
        self.builder.update_file(&mut graph, &file, &self.buffer);

        let changed: Vec<NodeId> = graph
            .nodes()
            .filter(|node| {
                before.get(&node.id).map(String::as_str).unwrap_or_default() != node.source.as_str()
            })
            .map(|node| node.id)
            .collect();

        let mut nodes: HashMap<NodeId, u32> = changed.iter().copied().map(|id| (id, 0)).collect();
        for id in changed {
            for (&affected, &distance) in &graph.impact_of(id).affected {
                nodes
                    .entry(affected)
                    .and_modify(|current| *current = (*current).min(distance))
                    .or_insert(distance);
            }
        }
        Ok(SyncImpact { nodes })
    }

    pub(crate) fn save(&mut self) -> std::io::Result<PathBuf> {
        if self.has_pending_agent_changes() {
            return Err(pending_agent_error("save source"));
        }
        let Some(file) = self.active_file.clone() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "project has no active source file",
            ));
        };
        let disk = std::fs::read_to_string(self.root.join(&file))?;
        if disk != self.clean_buffer {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{file} changed on disk; reload before saving"),
            ));
        }

        self.sync_buffer_to_graph()?;
        let graph = self
            .graph
            .lock()
            .map_err(|_| std::io::Error::other("semantic graph lock is poisoned"))?
            .clone();
        let source_write = ProjectWrite::text(
            &file,
            Some(self.clean_buffer.as_bytes().to_vec()),
            self.buffer.clone(),
        );
        let graph_write = graph_project_write(
            &self.root,
            &self.config,
            &graph,
            self.clean_graph_bytes.clone(),
        )?;
        commit_project_writes(&self.root, vec![source_write, graph_write])?;
        let source_path = self.root.join(&file);
        self.clean_buffer.clone_from(&self.buffer);
        self.clean_graph_bytes = read_project_bytes(&self.root, &self.config.graph.path)?;
        Ok(source_path)
    }

    pub(crate) fn discard_changes(&mut self) -> std::io::Result<SyncImpact> {
        self.buffer.clone_from(&self.clean_buffer);
        self.sync_buffer_to_graph()
    }

    pub(crate) fn reload(&mut self) -> std::io::Result<()> {
        if self.has_pending_agent_changes() {
            return Err(pending_agent_error("reload the project"));
        }
        if self.is_dirty() {
            return Err(dirty_error("reload the project"));
        }
        let active = self.active_file.clone();
        let mut reopened = Self::open(&self.root)?;
        if let Some(active) = active {
            if reopened.files.iter().any(|file| file.relative == active) {
                reopened.select_file(&active)?;
            }
        }
        *self = reopened;
        Ok(())
    }

    pub(crate) fn begin_agent_transaction(&mut self) -> std::io::Result<()> {
        if self.is_dirty() {
            return Err(dirty_error("dispatch agents"));
        }
        if self.agent_transaction.is_some() {
            return Err(pending_agent_error("dispatch another agent run"));
        }
        let graph = self
            .graph
            .lock()
            .map_err(|_| std::io::Error::other("semantic graph lock is poisoned"))?
            .clone();
        let projection = capture_agent_baseline(&self.root, &self.config)?;
        self.agent_transaction = Some(AgentTransaction {
            graph,
            projection,
            validation: None,
            validated_graph_bytes: None,
        });
        Ok(())
    }

    pub(crate) fn finish_agent_transaction(&mut self) -> std::io::Result<usize> {
        let Some(transaction) = &self.agent_transaction else {
            return Ok(0);
        };
        let current = self
            .graph
            .lock()
            .map_err(|_| std::io::Error::other("semantic graph lock is poisoned"))?;
        let current_bytes = current
            .to_bytes()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let baseline_bytes = transaction
            .graph
            .to_bytes()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        if current_bytes == baseline_bytes {
            drop(current);
            self.agent_transaction = None;
            return Ok(0);
        }
        let diff = current.diff_from(&transaction.graph);
        let structural =
            diff.node_change_count() + diff.added_edges.len() + diff.removed_edges.len();
        Ok(structural.max(1))
    }

    pub(crate) fn commit_agent_changes(&mut self) -> std::io::Result<Vec<PathBuf>> {
        if self.is_dirty() {
            return Err(dirty_error("commit agent changes"));
        }
        let Some(transaction) = self.agent_transaction.as_ref() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "there are no pending agent changes",
            ));
        };
        let Some(validation) = transaction.validation.as_ref() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "validate pending agent changes before committing",
            ));
        };
        if !validation.passed() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "agent candidate did not pass validation",
            ));
        }
        let graph = self
            .graph
            .lock()
            .map_err(|_| std::io::Error::other("semantic graph lock is poisoned"))?
            .clone();
        let graph_bytes = graph
            .to_bytes()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        if transaction.validated_graph_bytes.as_deref() != Some(graph_bytes.as_slice()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "pending graph changed after validation; validate it again",
            ));
        }
        let plan = plan_authored_functions(
            &self.root,
            &self.config,
            &graph,
            &self.config.agents.output_module,
            Some(&transaction.projection),
        )?;
        let projected = plan.commit(&self.root)?;

        let active = self.active_file.clone();
        let mut reopened = Self::open(&self.root)?;
        if let Some(active) = active {
            if reopened.files.iter().any(|file| file.relative == active) {
                reopened.select_file(&active)?;
            }
        }
        *self = reopened;
        Ok(projected)
    }

    pub(crate) fn rollback_agent_changes(&mut self) -> std::io::Result<()> {
        let Some(transaction) = self.agent_transaction.take() else {
            return Ok(());
        };
        let mut graph = self
            .graph
            .lock()
            .map_err(|_| std::io::Error::other("semantic graph lock is poisoned"))?;
        *graph = transaction.graph;
        Ok(())
    }

    pub(crate) fn has_pending_agent_changes(&self) -> bool {
        self.agent_transaction.is_some()
    }

    #[cfg(feature = "gui")]
    pub(crate) fn extension_records(
        &self,
    ) -> std::io::Result<Vec<Result<ExtensionRecord, ExtensionError>>> {
        let graph = self
            .graph
            .lock()
            .map_err(|_| std::io::Error::other("semantic graph lock is poisoned"))?;
        Ok(records(&graph))
    }

    #[cfg(feature = "gui")]
    pub(crate) fn extension_mutation_request(
        &self,
        mutation: ExtensionMutation,
    ) -> std::io::Result<ExtensionMutationRequest> {
        if self.is_dirty() {
            return Err(dirty_error("change extensions"));
        }
        if self.has_pending_agent_changes() {
            return Err(pending_agent_error("change extensions"));
        }
        let graph = self
            .graph
            .lock()
            .map_err(|_| std::io::Error::other("semantic graph lock is poisoned"))?
            .clone();
        Ok(ExtensionMutationRequest::new(
            self.root.clone(),
            self.config.clone(),
            read_project_bytes(&self.root, crate::project::config::CONFIG_FILE)?,
            graph,
            self.clean_graph_bytes.clone(),
            mutation,
        ))
    }

    #[cfg(feature = "gui")]
    pub(crate) fn extension_command_request(
        &self,
        extension_id: &str,
        contribution_id: &str,
    ) -> std::io::Result<ExtensionCommandRequest> {
        if self.is_dirty() {
            return Err(dirty_error("run an extension command"));
        }
        if self.has_pending_agent_changes() {
            return Err(pending_agent_error("run an extension command"));
        }
        let graph = self
            .graph
            .lock()
            .map_err(|_| std::io::Error::other("semantic graph lock is poisoned"))?;
        let record = find_record(&graph, extension_id)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("extension {extension_id} is not installed"),
                )
            })?;
        if record.state != ExtensionState::Enabled {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("extension {extension_id} is disabled"),
            ));
        }
        let argv = record
            .recipe
            .contributions
            .iter()
            .find_map(|contribution| match contribution {
                Contribution::Command {
                    id,
                    action: CommandAction::RunValidation { argv },
                    ..
                } if id == contribution_id => Some(argv.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("extension {extension_id} has no validation command {contribution_id}"),
                )
            })?;
        Ok(ExtensionCommandRequest {
            root: self.root.clone(),
            config: self.config.clone(),
            argv,
        })
    }

    #[cfg(feature = "gui")]
    pub(crate) fn read_extension_file(&self, relative: &str) -> std::io::Result<String> {
        const MAX_EXTENSION_FILE_BYTES: usize = 256 * 1024;
        let Some(bytes) =
            read_project_bytes_bounded(&self.root, relative, MAX_EXTENSION_FILE_BYTES)?
        else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("extension file does not exist: {relative}"),
            ));
        };
        if bytes.len() > MAX_EXTENSION_FILE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("extension file {relative} exceeds {MAX_EXTENSION_FILE_BYTES} bytes"),
            ));
        }
        String::from_utf8(bytes).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("extension file {relative} is not UTF-8: {error}"),
            )
        })
    }

    pub(crate) fn agent_validation_request(&mut self) -> std::io::Result<AgentValidationRequest> {
        let Some(transaction) = self.agent_transaction.as_mut() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "there are no pending agent changes",
            ));
        };
        transaction.validation = None;
        transaction.validated_graph_bytes = None;
        let graph = self
            .graph
            .lock()
            .map_err(|_| std::io::Error::other("semantic graph lock is poisoned"))?
            .clone();
        Ok(AgentValidationRequest {
            root: self.root.clone(),
            config: self.config.clone(),
            graph,
            module: self.config.agents.output_module.clone(),
            baseline: transaction.projection.clone(),
        })
    }

    pub(crate) fn accept_agent_validation(
        &mut self,
        outcome: AgentValidationOutcome,
    ) -> std::io::Result<()> {
        let Some(transaction) = self.agent_transaction.as_mut() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "agent transaction ended before validation completed",
            ));
        };
        let current = self
            .graph
            .lock()
            .map_err(|_| std::io::Error::other("semantic graph lock is poisoned"))?
            .to_bytes()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        if current != outcome.graph_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "pending graph changed while validation was running",
            ));
        }
        transaction.validated_graph_bytes = outcome.report.passed().then_some(outcome.graph_bytes);
        transaction.validation = Some(outcome.report);
        Ok(())
    }

    #[cfg(feature = "gui")]
    pub(crate) fn agent_validation_report(&self) -> Option<&ValidationReport> {
        self.agent_transaction
            .as_ref()
            .and_then(|transaction| transaction.validation.as_ref())
    }

    fn update_graph_file(&mut self, relative: &str, source: &str) -> std::io::Result<()> {
        let graph_handle = self.graph.clone();
        let mut graph = graph_handle
            .lock()
            .map_err(|_| std::io::Error::other("semantic graph lock is poisoned"))?;
        self.builder.update_file(&mut graph, relative, source);
        Ok(())
    }
}

fn preferred_file(files: &[ProjectFile]) -> Option<&ProjectFile> {
    const PREFERRED: &[&str] = &["src/lib.rs", "src/main.rs", "main.py", "app.py"];
    PREFERRED
        .iter()
        .find_map(|preferred| files.iter().find(|file| file.relative == *preferred))
        .or_else(|| files.first())
}

fn dirty_error(action: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::WouldBlock,
        format!("save or discard the active file before you {action}"),
    )
}

fn pending_agent_error(action: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::WouldBlock,
        format!("commit or roll back pending agent changes before you {action}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_graph::{Edge, EdgeKind, Node, NodeId, NodeKind};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    struct TempProject(PathBuf);

    impl TempProject {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "girder-workspace-{name}-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("src")).unwrap();
            Self(root)
        }

        fn write(&self, relative: &str, source: &str) {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, source).unwrap();
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn opens_real_project_and_prefers_library_entrypoint() {
        let project = TempProject::new("open");
        project.write("src/z.rs", "pub fn z() {}\n");
        project.write("src/lib.rs", "pub fn entry() {}\n");

        let workspace = ProjectWorkspace::open(&project.0).unwrap();

        assert_eq!(workspace.root(), project.0.canonicalize().unwrap());
        assert_eq!(workspace.active_file(), Some("src/lib.rs"));
        assert_eq!(workspace.active_language(), Some(Lang::Rust));
        assert_eq!(workspace.buffer(), "pub fn entry() {}\n");
        assert_eq!(workspace.files().len(), 2);
        assert_eq!(workspace.files()[0].relative(), "src/lib.rs");
        assert_eq!(workspace.files()[0].language(), Lang::Rust);
        assert!(workspace
            .graph()
            .lock()
            .unwrap()
            .find_by_path("crate::lib::entry")
            .is_some());
    }

    #[test]
    fn save_updates_source_graph_and_persisted_graph() {
        let project = TempProject::new("save");
        project.write("src/lib.rs", "pub fn before() -> i64 { 1 }\n");
        let mut workspace = ProjectWorkspace::open(&project.0).unwrap();
        *workspace.buffer_mut() = "pub fn after() -> i64 { 2 }\n".into();

        let path = workspace.save().unwrap();

        assert_eq!(path, project.0.join("src/lib.rs"));
        assert!(!workspace.is_dirty());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "pub fn after() -> i64 { 2 }\n"
        );
        assert!(workspace
            .graph()
            .lock()
            .unwrap()
            .find_by_path("crate::lib::after")
            .is_some());
        let persisted = SemanticGraph::load(project.0.join("project.aether")).unwrap();
        assert!(persisted.find_by_path("crate::lib::after").is_some());
    }

    #[test]
    fn dirty_buffer_blocks_navigation_and_reload() {
        let project = TempProject::new("dirty");
        project.write("src/lib.rs", "pub fn one() {}\n");
        project.write("src/two.rs", "pub fn two() {}\n");
        let mut workspace = ProjectWorkspace::open(&project.0).unwrap();
        workspace.buffer_mut().push_str("// edit\n");

        assert_eq!(
            workspace.select_file("src/two.rs").unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(
            workspace.reload().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        workspace.discard_changes().unwrap();
        workspace.select_file("src/two.rs").unwrap();
        assert_eq!(workspace.active_file(), Some("src/two.rs"));
    }

    #[test]
    fn save_rejects_external_file_changes() {
        let project = TempProject::new("conflict");
        project.write("src/lib.rs", "pub fn original() {}\n");
        let mut workspace = ProjectWorkspace::open(&project.0).unwrap();
        workspace.buffer_mut().push_str("// local\n");
        project.write("src/lib.rs", "pub fn external() {}\n");

        let error = workspace.save().unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(project.0.join("src/lib.rs")).unwrap(),
            "pub fn external() {}\n"
        );
        assert!(workspace.is_dirty());
    }

    #[test]
    fn sync_reports_transitive_impact() {
        let project = TempProject::new("impact");
        project.write(
            "src/lib.rs",
            "fn add(a: i64, b: i64) -> i64 { a + b }\nfn run() -> i64 { add(1, 2) }\n",
        );
        let mut workspace = ProjectWorkspace::open(&project.0).unwrap();
        *workspace.buffer_mut() =
            "fn add(a: i64, b: i64) -> i64 { a - b }\nfn run() -> i64 { add(1, 2) }\n".into();

        let impact = workspace.sync_buffer_to_graph().unwrap();

        assert_eq!(
            impact.nodes.get(&NodeId::from_path("crate::lib::add")),
            Some(&0)
        );
        assert_eq!(
            impact.nodes.get(&NodeId::from_path("crate::lib::run")),
            Some(&1)
        );
    }

    #[test]
    fn open_reconciles_persisted_metadata_with_fresh_source() {
        let project = TempProject::new("reconcile");
        project.write("src/lib.rs", "pub fn run() -> i64 { 2 }\n");
        let mut persisted = SemanticGraph::new();
        let mut run = Node::new(NodeKind::Function, "run", "crate::lib::run")
            .with_language("rust")
            .with_source("pub fn run() -> i64 { 1 }");
        run.file = Some("src/lib.rs".into());
        run.set_attr("summary", "durable metadata");
        persisted.upsert_node(run);
        persisted.save(project.0.join("project.aether")).unwrap();

        let workspace = ProjectWorkspace::open(&project.0).unwrap();
        let graph = workspace.graph().lock().unwrap();
        let run = graph.find_by_path("crate::lib::run").unwrap();

        assert_eq!(run.source, "pub fn run() -> i64 { 2 }");
        assert_eq!(run.attr("summary"), Some("durable metadata"));
        assert!(workspace.open_status().contains("reconciled durable graph"));
        assert!(workspace.open_status().contains("1 source update"));
    }

    #[test]
    fn corrupt_graph_is_rebuilt_and_replaced_on_save() {
        let project = TempProject::new("corrupt");
        project.write("src/lib.rs", "pub fn run() {}\n");
        project.write("project.aether", "broken");
        let mut workspace = ProjectWorkspace::open(&project.0).unwrap();

        assert!(workspace
            .open_status()
            .contains("persisted graph was invalid"));
        workspace.buffer_mut().push_str("// local\n");
        workspace.save().unwrap();

        let persisted = SemanticGraph::load(project.0.join("project.aether")).unwrap();
        assert!(persisted.find_by_path("crate::lib::run").is_some());
    }

    fn add_agent_function(workspace: &ProjectWorkspace, name: &str) {
        let (module, file) = workspace.agent_target();
        let module = module.to_string();
        let file = file.to_string();
        let mut graph = workspace.graph().lock().unwrap();
        let module_id = graph.upsert_node(
            Node::new(
                NodeKind::Module,
                module.rsplit("::").next().unwrap(),
                &module,
            )
            .with_language("rust"),
        );
        let path = format!("{module}::{name}");
        let mut function = Node::new(NodeKind::Function, name, &path)
            .with_language("rust")
            .with_source(format!("pub fn {name}() -> i64 {{ 42 }}"));
        function.file = Some(file);
        function.set_attr("authored_by", "Coder");
        function.set_attr("summary", "generated safely");
        let function_id = graph.upsert_node(function);
        graph
            .add_edge(module_id, function_id, Edge::new(EdgeKind::Contains))
            .unwrap();
    }

    fn validate_pending(workspace: &mut ProjectWorkspace) {
        let request = workspace.agent_validation_request().unwrap();
        let outcome = request.run(&Arc::new(AtomicBool::new(false))).unwrap();
        assert!(outcome.report.passed(), "{:?}", outcome.report.steps);
        workspace.accept_agent_validation(outcome).unwrap();
    }

    #[test]
    fn agent_changes_commit_source_and_graph_then_reopen_cleanly() {
        let project = TempProject::new("agent-commit");
        project.write("src/lib.rs", "pub fn existing() {}\n");
        let mut workspace = ProjectWorkspace::open(&project.0).unwrap();
        workspace.begin_agent_transaction().unwrap();
        add_agent_function(&workspace, "generated");

        assert!(workspace.finish_agent_transaction().unwrap() > 0);
        assert!(workspace.has_pending_agent_changes());
        validate_pending(&mut workspace);
        let projected = workspace.commit_agent_changes().unwrap();

        assert_eq!(projected, [PathBuf::from("src/forge.rs")]);
        assert!(!workspace.has_pending_agent_changes());
        assert!(workspace
            .files()
            .iter()
            .any(|file| file.relative() == "src/forge.rs"));
        assert!(std::fs::read_to_string(project.0.join("src/forge.rs"))
            .unwrap()
            .contains("pub fn generated()"));
        let graph = workspace.graph().lock().unwrap();
        let generated = graph.find_by_path("crate::forge::generated").unwrap();
        assert_eq!(generated.attr("summary"), Some("generated safely"));
    }

    #[test]
    fn agent_changes_can_be_rolled_back_without_touching_disk() {
        let project = TempProject::new("agent-rollback");
        project.write("src/lib.rs", "pub fn existing() {}\n");
        let mut workspace = ProjectWorkspace::open(&project.0).unwrap();
        workspace.begin_agent_transaction().unwrap();
        add_agent_function(&workspace, "discarded");
        workspace.finish_agent_transaction().unwrap();

        workspace.rollback_agent_changes().unwrap();

        assert!(!workspace.has_pending_agent_changes());
        assert!(workspace
            .graph()
            .lock()
            .unwrap()
            .find_by_path("crate::forge::discarded")
            .is_none());
        assert!(!project.0.join("src/forge.rs").exists());
        assert!(!project.0.join("project.aether").exists());
    }

    #[test]
    fn agent_commit_rejects_external_output_changes_and_keeps_checkpoint() {
        let project = TempProject::new("agent-conflict");
        project.write("src/lib.rs", "pub fn existing() {}\n");
        let mut workspace = ProjectWorkspace::open(&project.0).unwrap();
        workspace.begin_agent_transaction().unwrap();
        add_agent_function(&workspace, "generated");
        workspace.finish_agent_transaction().unwrap();
        validate_pending(&mut workspace);
        project.write("src/forge.rs", "pub fn external() {}\n");

        let error = workspace.commit_agent_changes().unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(project.0.join("src/forge.rs")).unwrap(),
            "pub fn external() {}\n"
        );
        assert!(workspace.has_pending_agent_changes());
        assert!(!project.0.join("project.aether").exists());
    }

    #[test]
    fn agent_commit_requires_validation() {
        let project = TempProject::new("agent-requires-validation");
        project.write("src/lib.rs", "pub fn existing() {}\n");
        let mut workspace = ProjectWorkspace::open(&project.0).unwrap();
        workspace.begin_agent_transaction().unwrap();
        add_agent_function(&workspace, "generated");
        workspace.finish_agent_transaction().unwrap();

        let error = workspace.commit_agent_changes().unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        assert!(workspace.has_pending_agent_changes());
        assert!(!project.0.join("src/forge.rs").exists());
    }

    #[test]
    fn agent_commit_rejects_graph_changes_after_validation() {
        let project = TempProject::new("agent-stale-validation");
        project.write("src/lib.rs", "pub fn existing() {}\n");
        let mut workspace = ProjectWorkspace::open(&project.0).unwrap();
        workspace.begin_agent_transaction().unwrap();
        add_agent_function(&workspace, "generated");
        workspace.finish_agent_transaction().unwrap();
        validate_pending(&mut workspace);
        workspace
            .graph()
            .lock()
            .unwrap()
            .get_mut(NodeId::from_path("crate::forge::generated"))
            .unwrap()
            .set_attr("changed_after_validation", "true");

        let error = workspace.commit_agent_changes().unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        assert!(workspace.has_pending_agent_changes());
        assert!(!project.0.join("src/forge.rs").exists());
    }

    #[test]
    fn agent_validation_rejects_config_changes() {
        let project = TempProject::new("agent-validation-config-conflict");
        project.write("src/lib.rs", "pub fn existing() {}\n");
        let mut workspace = ProjectWorkspace::open(&project.0).unwrap();
        workspace.begin_agent_transaction().unwrap();
        add_agent_function(&workspace, "generated");
        workspace.finish_agent_transaction().unwrap();
        project.write("girder.toml", "# changed externally\n");

        let request = workspace.agent_validation_request().unwrap();
        let error = request.run(&Arc::new(AtomicBool::new(false))).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(workspace.has_pending_agent_changes());
        assert!(!project.0.join("src/forge.rs").exists());
        assert!(!project.0.join("project.aether").exists());
    }

    #[test]
    fn agent_commit_rejects_config_changes_after_validation() {
        let project = TempProject::new("agent-commit-config-conflict");
        project.write("src/lib.rs", "pub fn existing() {}\n");
        let mut workspace = ProjectWorkspace::open(&project.0).unwrap();
        workspace.begin_agent_transaction().unwrap();
        add_agent_function(&workspace, "generated");
        workspace.finish_agent_transaction().unwrap();
        validate_pending(&mut workspace);
        project.write("girder.toml", "# changed externally\n");

        let error = workspace.commit_agent_changes().unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(workspace.has_pending_agent_changes());
        assert!(!project.0.join("src/forge.rs").exists());
        assert!(!project.0.join("project.aether").exists());
    }
}
