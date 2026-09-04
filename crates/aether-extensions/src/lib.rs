//! Declarative, permission-bound extensions for Girder.
//!
//! Extension recipes are data, not dynamically loaded code. A model may propose
//! a recipe, but Girder validates every field and requires an exact,
//! digest-bound capability grant before the recipe can enter the semantic graph.

use aether_graph::{Edge, EdgeKind, Node, NodeId, NodeKind, SemanticGraph};
use globset::GlobBuilder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Component, Path};

mod marketplace;

pub use marketplace::{
    adaptation_system_prompt, builtin_catalog, marketplace_project_context, CapabilityDelta,
    MarketplaceCatalog, MarketplaceListing, MarketplaceReview, ReviewDecision, CATALOG_VERSION,
    MAX_CATALOG_BYTES,
};

pub const RECIPE_VERSION: u32 = 1;
pub const RECORD_VERSION: u32 = 1;
const MAX_RECIPE_BYTES: usize = 256 * 1024;
pub const MAX_PROJECTION_BYTES: usize = 1024 * 1024;
pub const MAX_TOTAL_PROJECTION_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionRecipe {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub description: String,
    pub intent: String,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub contributions: Vec<Contribution>,
    #[serde(default)]
    pub projections: Vec<ProjectProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Capability {
    ReadGraph,
    WriteGraph { namespaces: Vec<String> },
    ReadProject { paths: Vec<String> },
    WriteProject { paths: Vec<String> },
    RunValidation { programs: Vec<String> },
    Network { hosts: Vec<String> },
    ContributeUi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    ReadGraph,
    WriteGraph,
    ReadProject,
    WriteProject,
    RunValidation,
    Network,
    ContributeUi,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Contribution {
    Panel {
        id: String,
        title: String,
        location: PanelLocation,
        view: PanelView,
    },
    Command {
        id: String,
        title: String,
        action: CommandAction,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelLocation {
    Left,
    Right,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PanelView {
    Markdown { content: String },
    GraphQuery { question: String },
    File { path: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandAction {
    AskGraph { question: String },
    OpenFile { path: String, line: Option<u32> },
    RunValidation { argv: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectProjection {
    pub path: String,
    pub contents: String,
    pub mode: ProjectionMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionMode {
    Create,
    Replace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionGrant {
    pub extension_id: String,
    pub recipe_digest: String,
    pub capabilities: Vec<CapabilityKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionState {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionRecord {
    pub version: u32,
    pub recipe: ExtensionRecipe,
    pub grant: ExtensionGrant,
    pub state: ExtensionState,
    #[serde(default)]
    pub projection_receipts: Vec<ProjectionReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionReceipt {
    pub path: String,
    pub previous: Option<String>,
    pub applied_digest: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    #[error("invalid extension recipe: {0}")]
    Invalid(String),
    #[error("extension authorization failed: {0}")]
    Authorization(String),
    #[error("extension record is corrupt: {0}")]
    Corrupt(String),
    #[error("extension JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("semantic graph update failed: {0}")]
    Graph(#[from] aether_graph::GraphError),
}

impl Capability {
    pub fn kind(&self) -> CapabilityKind {
        match self {
            Self::ReadGraph => CapabilityKind::ReadGraph,
            Self::WriteGraph { .. } => CapabilityKind::WriteGraph,
            Self::ReadProject { .. } => CapabilityKind::ReadProject,
            Self::WriteProject { .. } => CapabilityKind::WriteProject,
            Self::RunValidation { .. } => CapabilityKind::RunValidation,
            Self::Network { .. } => CapabilityKind::Network,
            Self::ContributeUi => CapabilityKind::ContributeUi,
        }
    }
}

impl Contribution {
    pub fn id(&self) -> &str {
        match self {
            Self::Panel { id, .. } | Self::Command { id, .. } => id,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            Self::Panel { title, .. } | Self::Command { title, .. } => title,
        }
    }

    fn kind_label(&self) -> &'static str {
        match self {
            Self::Panel { .. } => "panel",
            Self::Command { .. } => "command",
        }
    }
}

impl ExtensionRecipe {
    pub fn from_json(json: &str) -> Result<Self, ExtensionError> {
        if json.len() > MAX_RECIPE_BYTES {
            return Err(ExtensionError::Invalid(format!(
                "recipe exceeds {MAX_RECIPE_BYTES} bytes"
            )));
        }
        let json = strip_json_fence(json)?;
        let recipe: Self = serde_json::from_str(json)?;
        recipe.validate()?;
        Ok(recipe)
    }

    pub fn to_json_pretty(&self) -> Result<String, ExtensionError> {
        self.validate()?;
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn digest(&self) -> Result<String, ExtensionError> {
        self.validate()?;
        let canonical = serde_json::to_vec(self)?;
        let digest = Sha256::digest(canonical);
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }

    pub fn validate(&self) -> Result<(), ExtensionError> {
        if self.version != RECIPE_VERSION {
            return invalid(format!(
                "unsupported recipe version {}; expected {RECIPE_VERSION}",
                self.version
            ));
        }
        validate_extension_id(&self.id)?;
        validate_text("name", &self.name, 1, 80)?;
        validate_text("description", &self.description, 1, 500)?;
        validate_text("intent", &self.intent, 1, 2_000)?;
        if self.capabilities.len() > 32 {
            return invalid("at most 32 capabilities are allowed");
        }
        if self.contributions.len() > 64 {
            return invalid("at most 64 contributions are allowed");
        }
        if self.projections.len() > 64 {
            return invalid("at most 64 projections are allowed");
        }

        let mut capability_kinds = BTreeSet::new();
        for capability in &self.capabilities {
            if !capability_kinds.insert(capability.kind()) {
                return invalid(format!(
                    "capability {:?} is declared more than once",
                    capability.kind()
                ));
            }
            validate_capability(capability)?;
        }

        let mut contribution_ids = BTreeSet::new();
        for contribution in &self.contributions {
            validate_slug("contribution id", contribution.id(), 64)?;
            validate_text("contribution title", contribution.title(), 1, 80)?;
            if !contribution_ids.insert(contribution.id()) {
                return invalid(format!("duplicate contribution id {}", contribution.id()));
            }
            require_capability(self, CapabilityKind::ContributeUi)?;
            validate_contribution(self, contribution)?;
        }

        let mut projection_paths = BTreeSet::new();
        let mut total_projection_bytes = 0_usize;
        for projection in &self.projections {
            validate_relative_path("projection path", &projection.path)?;
            if !projection_paths.insert(projection.path.as_str()) {
                return invalid(format!("duplicate projection path {}", projection.path));
            }
            if projection.contents.len() > MAX_PROJECTION_BYTES {
                return invalid(format!(
                    "projection {} exceeds {MAX_PROJECTION_BYTES} bytes",
                    projection.path
                ));
            }
            total_projection_bytes = total_projection_bytes
                .checked_add(projection.contents.len())
                .ok_or_else(|| ExtensionError::Invalid("projection size overflow".into()))?;
            if total_projection_bytes > MAX_TOTAL_PROJECTION_BYTES {
                return invalid(format!(
                    "all projections exceed {MAX_TOTAL_PROJECTION_BYTES} bytes"
                ));
            }
            if !project_path_allowed(self, CapabilityKind::WriteProject, &projection.path)? {
                return invalid(format!(
                    "projection {} is outside the requested write_project paths",
                    projection.path
                ));
            }
        }

        Ok(())
    }

    pub fn capability_kinds(&self) -> Vec<CapabilityKind> {
        let mut kinds = self
            .capabilities
            .iter()
            .map(Capability::kind)
            .collect::<Vec<_>>();
        kinds.sort();
        kinds.dedup();
        kinds
    }
}

impl ExtensionGrant {
    pub fn approve(recipe: &ExtensionRecipe) -> Result<Self, ExtensionError> {
        recipe.validate()?;
        Ok(Self {
            extension_id: recipe.id.clone(),
            recipe_digest: recipe.digest()?,
            capabilities: recipe.capability_kinds(),
        })
    }

    pub fn authorize(&self, recipe: &ExtensionRecipe) -> Result<(), ExtensionError> {
        recipe.validate()?;
        if self.extension_id != recipe.id {
            return Err(ExtensionError::Authorization(format!(
                "grant belongs to {}, not {}",
                self.extension_id, recipe.id
            )));
        }
        if self.recipe_digest != recipe.digest()? {
            return Err(ExtensionError::Authorization(
                "grant digest does not match the exact recipe".into(),
            ));
        }
        let mut granted = self.capabilities.clone();
        granted.sort();
        if granted.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ExtensionError::Authorization(
                "grant contains duplicate capabilities".into(),
            ));
        }
        if granted != recipe.capability_kinds() {
            return Err(ExtensionError::Authorization(
                "grant must exactly match the recipe's requested capabilities".into(),
            ));
        }
        Ok(())
    }
}

impl ExtensionRecord {
    pub fn new(recipe: ExtensionRecipe, grant: ExtensionGrant) -> Result<Self, ExtensionError> {
        grant.authorize(&recipe)?;
        Ok(Self {
            version: RECORD_VERSION,
            recipe,
            grant,
            state: ExtensionState::Enabled,
            projection_receipts: Vec::new(),
        })
    }

    pub fn validate(&self) -> Result<(), ExtensionError> {
        if self.version != RECORD_VERSION {
            return Err(ExtensionError::Corrupt(format!(
                "unsupported record version {}; expected {RECORD_VERSION}",
                self.version
            )));
        }
        self.grant
            .authorize(&self.recipe)
            .map_err(|error| ExtensionError::Corrupt(error.to_string()))?;
        if self.projection_receipts.len() != self.recipe.projections.len() {
            return Err(ExtensionError::Corrupt(
                "projection receipts do not match the installed recipe".into(),
            ));
        }
        let mut total_baseline_bytes = 0_usize;
        for (projection, receipt) in self
            .recipe
            .projections
            .iter()
            .zip(&self.projection_receipts)
        {
            if projection.path != receipt.path {
                return Err(ExtensionError::Corrupt(format!(
                    "projection receipt {} does not match {}",
                    receipt.path, projection.path
                )));
            }
            if receipt.applied_digest != content_digest(projection.contents.as_bytes()) {
                return Err(ExtensionError::Corrupt(format!(
                    "projection receipt digest does not match {}",
                    projection.path
                )));
            }
            if receipt
                .previous
                .as_ref()
                .is_some_and(|previous| previous.len() > MAX_PROJECTION_BYTES)
            {
                return Err(ExtensionError::Corrupt(format!(
                    "projection baseline for {} exceeds {MAX_PROJECTION_BYTES} bytes",
                    projection.path
                )));
            }
            if let Some(previous) = &receipt.previous {
                total_baseline_bytes = total_baseline_bytes
                    .checked_add(previous.len())
                    .ok_or_else(|| {
                        ExtensionError::Corrupt("projection baseline size overflow".into())
                    })?;
                if total_baseline_bytes > MAX_TOTAL_PROJECTION_BYTES {
                    return Err(ExtensionError::Corrupt(format!(
                        "all projection baselines exceed {MAX_TOTAL_PROJECTION_BYTES} bytes"
                    )));
                }
            }
        }
        Ok(())
    }
}

pub fn install(
    graph: &mut SemanticGraph,
    recipe: ExtensionRecipe,
    grant: ExtensionGrant,
) -> Result<ExtensionRecord, ExtensionError> {
    let record = ExtensionRecord::new(recipe, grant)?;
    if !record.recipe.projections.is_empty() {
        return invalid("projection receipts are required to install project projections");
    }
    install_record(graph, record)
}

pub fn install_with_receipts(
    graph: &mut SemanticGraph,
    recipe: ExtensionRecipe,
    grant: ExtensionGrant,
    projection_receipts: Vec<ProjectionReceipt>,
) -> Result<ExtensionRecord, ExtensionError> {
    let mut record = ExtensionRecord::new(recipe, grant)?;
    record.projection_receipts = projection_receipts;
    record.validate()?;
    install_record(graph, record)
}

fn install_record(
    graph: &mut SemanticGraph,
    record: ExtensionRecord,
) -> Result<ExtensionRecord, ExtensionError> {
    let mut staged = graph.clone();
    let extension_path = extension_path(&record.recipe.id);
    let extension_id = NodeId::from_path(&extension_path);

    if let Some(existing) = staged.get(extension_id) {
        if existing.kind != NodeKind::Extension {
            return invalid(format!(
                "semantic path {extension_path} is owned by a non-extension node"
            ));
        }
        remove_contribution_nodes(&mut staged, extension_id);
    }

    let mut extension = Node::new(NodeKind::Extension, &record.recipe.name, &extension_path)
        .with_language("bitcode-extension")
        .with_source(serde_json::to_string_pretty(&record)?);
    extension.set_attr("extension_id", &record.recipe.id);
    extension.set_attr("recipe_digest", &record.grant.recipe_digest);
    extension.set_attr("state", state_label(record.state));
    staged.upsert_node(extension);

    for contribution in &record.recipe.contributions {
        let path = format!("{extension_path}::contribution::{}", contribution.id());
        let mut node = Node::new(NodeKind::ExtensionContribution, contribution.title(), &path)
            .with_language("bitcode-extension")
            .with_source(serde_json::to_string_pretty(contribution)?);
        node.set_attr("extension_id", &record.recipe.id);
        node.set_attr("contribution_id", contribution.id());
        node.set_attr("contribution_kind", contribution.kind_label());
        let node_id = staged.upsert_node(node);
        staged.add_edge(extension_id, node_id, Edge::new(EdgeKind::Contributes))?;
    }

    for (index, projection) in record.recipe.projections.iter().enumerate() {
        let path = format!("{extension_path}::projection::{index}");
        let mut node = Node::new(NodeKind::ExtensionContribution, &projection.path, &path)
            .with_language("bitcode-extension")
            .with_source(serde_json::to_string_pretty(projection)?);
        node.set_attr("extension_id", &record.recipe.id);
        node.set_attr("projection_path", &projection.path);
        node.set_attr("contribution_kind", "projection");
        let node_id = staged.upsert_node(node);
        staged.add_edge(extension_id, node_id, Edge::new(EdgeKind::Contributes))?;
    }

    *graph = staged;
    Ok(record)
}

pub fn uninstall(graph: &mut SemanticGraph, extension_id: &str) -> Result<bool, ExtensionError> {
    validate_extension_id(extension_id)?;
    let id = NodeId::from_path(&extension_path(extension_id));
    let Some(node) = graph.get(id) else {
        return Ok(false);
    };
    if node.kind != NodeKind::Extension {
        return Err(ExtensionError::Corrupt(format!(
            "{} is not an extension node",
            node.path
        )));
    }
    let mut staged = graph.clone();
    remove_contribution_nodes(&mut staged, id);
    staged.remove_node(id);
    *graph = staged;
    Ok(true)
}

pub fn set_enabled(
    graph: &mut SemanticGraph,
    extension_id: &str,
    enabled: bool,
) -> Result<ExtensionRecord, ExtensionError> {
    let mut record = find_record(graph, extension_id)?.ok_or_else(|| {
        ExtensionError::Invalid(format!("extension {extension_id} is not installed"))
    })?;
    record.state = if enabled {
        ExtensionState::Enabled
    } else {
        ExtensionState::Disabled
    };
    record.validate()?;

    let id = NodeId::from_path(&extension_path(extension_id));
    let node = graph.get_mut(id).ok_or_else(|| {
        ExtensionError::Corrupt(format!("extension node {extension_id} disappeared"))
    })?;
    node.source = serde_json::to_string_pretty(&record)?;
    node.set_attr("state", state_label(record.state));
    Ok(record)
}

pub fn records(graph: &SemanticGraph) -> Vec<Result<ExtensionRecord, ExtensionError>> {
    let mut nodes = graph
        .query_by_kind(NodeKind::Extension)
        .into_iter()
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.path.cmp(&right.path));
    nodes.into_iter().map(decode_record).collect::<Vec<_>>()
}

pub fn find_record(
    graph: &SemanticGraph,
    extension_id: &str,
) -> Result<Option<ExtensionRecord>, ExtensionError> {
    validate_extension_id(extension_id)?;
    graph
        .find_by_path(&extension_path(extension_id))
        .map(decode_record)
        .transpose()
}

pub fn generation_system_prompt() -> &'static str {
    r#"Return exactly one JSON object and no prose or markdown. Build a declarative Girder extension recipe with version 1. Extensions are data, never executable plugin code. Use a lowercase reverse-domain id. Request only capabilities that are necessary. Supported capability kinds: read_graph; write_graph with namespaces; read_project with paths; write_project with paths; run_validation with programs; network with hosts; contribute_ui. Supported contributions are panel and command. Panel views are markdown, graph_query, or file. Command actions are ask_graph, open_file, or run_validation. Project projections require an explicitly matching write_project path. Keep the recipe minimal and bounded."#
}

pub fn content_digest(contents: &[u8]) -> String {
    let digest = Sha256::digest(contents);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_record(node: &Node) -> Result<ExtensionRecord, ExtensionError> {
    if node.kind != NodeKind::Extension {
        return Err(ExtensionError::Corrupt(format!(
            "{} is not an extension node",
            node.path
        )));
    }
    let record: ExtensionRecord = serde_json::from_str(&node.source)
        .map_err(|error| ExtensionError::Corrupt(error.to_string()))?;
    record.validate()?;
    if node.path != extension_path(&record.recipe.id) {
        return Err(ExtensionError::Corrupt(format!(
            "record id {} does not match graph path {}",
            record.recipe.id, node.path
        )));
    }
    Ok(record)
}

fn remove_contribution_nodes(graph: &mut SemanticGraph, extension_id: NodeId) {
    let contribution_ids = graph
        .neighbors(extension_id, Some(EdgeKind::Contributes))
        .into_iter()
        .filter_map(|neighbor| {
            graph
                .get(neighbor.id)
                .is_some_and(|node| node.kind == NodeKind::ExtensionContribution)
                .then_some(neighbor.id)
        })
        .collect::<Vec<_>>();
    for id in contribution_ids {
        graph.remove_node(id);
    }
}

fn extension_path(id: &str) -> String {
    format!("extension::{id}")
}

fn state_label(state: ExtensionState) -> &'static str {
    match state {
        ExtensionState::Enabled => "enabled",
        ExtensionState::Disabled => "disabled",
    }
}

fn validate_capability(capability: &Capability) -> Result<(), ExtensionError> {
    match capability {
        Capability::ReadGraph | Capability::ContributeUi => Ok(()),
        Capability::WriteGraph { namespaces } => {
            validate_nonempty_list("write_graph.namespaces", namespaces, |namespace| {
                validate_namespace(namespace)
            })
        }
        Capability::ReadProject { paths } => {
            validate_nonempty_list("read_project.paths", paths, |path| {
                validate_relative_pattern("read_project path", path)
            })
        }
        Capability::WriteProject { paths } => {
            validate_nonempty_list("write_project.paths", paths, |path| {
                validate_relative_pattern("write_project path", path)
            })
        }
        Capability::RunValidation { programs } => {
            validate_nonempty_list("run_validation.programs", programs, |program| {
                validate_program(program)
            })
        }
        Capability::Network { hosts } => {
            validate_nonempty_list("network.hosts", hosts, |host| validate_host(host))
        }
    }
}

fn validate_contribution(
    recipe: &ExtensionRecipe,
    contribution: &Contribution,
) -> Result<(), ExtensionError> {
    match contribution {
        Contribution::Panel { view, .. } => match view {
            PanelView::Markdown { content } => {
                validate_text("panel markdown", content, 1, 32 * 1024)
            }
            PanelView::GraphQuery { question } => {
                require_capability(recipe, CapabilityKind::ReadGraph)?;
                validate_text("panel graph question", question, 1, 1_000)
            }
            PanelView::File { path } => {
                validate_relative_path("panel file path", path)?;
                if project_path_allowed(recipe, CapabilityKind::ReadProject, path)? {
                    Ok(())
                } else {
                    invalid(format!(
                        "panel file {path} is outside the requested read_project paths"
                    ))
                }
            }
        },
        Contribution::Command { action, .. } => match action {
            CommandAction::AskGraph { question } => {
                require_capability(recipe, CapabilityKind::ReadGraph)?;
                validate_text("command graph question", question, 1, 1_000)
            }
            CommandAction::OpenFile { path, line } => {
                validate_relative_path("command file path", path)?;
                if line.is_some_and(|line| line == 0 || line > 10_000_000) {
                    return invalid("command file line must be between 1 and 10000000");
                }
                if project_path_allowed(recipe, CapabilityKind::ReadProject, path)? {
                    Ok(())
                } else {
                    invalid(format!(
                        "command file {path} is outside the requested read_project paths"
                    ))
                }
            }
            CommandAction::RunValidation { argv } => {
                let (program, args) = argv.split_first().ok_or_else(|| {
                    ExtensionError::Invalid("run_validation argv is empty".into())
                })?;
                validate_program(program)?;
                if argv.len() > 64 {
                    return invalid("run_validation argv has more than 64 entries");
                }
                for argument in args {
                    validate_text("run_validation argument", argument, 0, 4_096)?;
                }
                let allowed = recipe.capabilities.iter().any(|capability| {
                    matches!(
                        capability,
                        Capability::RunValidation { programs }
                            if programs.iter().any(|allowed| allowed == program)
                    )
                });
                if allowed {
                    Ok(())
                } else {
                    invalid(format!(
                        "program {program} is outside the requested run_validation programs"
                    ))
                }
            }
        },
    }
}

fn require_capability(
    recipe: &ExtensionRecipe,
    required: CapabilityKind,
) -> Result<(), ExtensionError> {
    if recipe
        .capabilities
        .iter()
        .any(|capability| capability.kind() == required)
    {
        Ok(())
    } else {
        invalid(format!("contribution requires capability {required:?}"))
    }
}

fn project_path_allowed(
    recipe: &ExtensionRecipe,
    kind: CapabilityKind,
    path: &str,
) -> Result<bool, ExtensionError> {
    for capability in &recipe.capabilities {
        let patterns = match (kind, capability) {
            (CapabilityKind::ReadProject, Capability::ReadProject { paths })
            | (CapabilityKind::WriteProject, Capability::WriteProject { paths }) => paths,
            _ => continue,
        };
        for pattern in patterns {
            let glob = GlobBuilder::new(pattern)
                .literal_separator(true)
                .backslash_escape(false)
                .build()
                .map_err(|error| {
                    ExtensionError::Invalid(format!("invalid path pattern {pattern:?}: {error}"))
                })?
                .compile_matcher();
            if glob.is_match(path) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn validate_extension_id(id: &str) -> Result<(), ExtensionError> {
    if id.len() > 128 {
        return invalid("extension id exceeds 128 characters");
    }
    let segments = id.split('.').collect::<Vec<_>>();
    if segments.len() < 2 {
        return invalid("extension id must use reverse-domain segments");
    }
    for segment in segments {
        validate_slug("extension id segment", segment, 63)?;
    }
    Ok(())
}

fn validate_slug(label: &str, value: &str, max: usize) -> Result<(), ExtensionError> {
    if value.is_empty() || value.len() > max {
        return invalid(format!("{label} must contain 1 to {max} characters"));
    }
    let bytes = value.as_bytes();
    if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit()
        || !bytes[bytes.len() - 1].is_ascii_lowercase() && !bytes[bytes.len() - 1].is_ascii_digit()
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
    {
        return invalid(format!(
            "{label} must use lowercase letters, digits, and internal hyphens"
        ));
    }
    Ok(())
}

fn validate_text(label: &str, value: &str, min: usize, max: usize) -> Result<(), ExtensionError> {
    if value.len() < min || value.len() > max {
        return invalid(format!("{label} must contain {min} to {max} bytes"));
    }
    if value
        .chars()
        .any(|character| character == '\0' || (character.is_control() && character != '\n'))
    {
        return invalid(format!("{label} contains unsupported control characters"));
    }
    Ok(())
}

fn validate_relative_path(label: &str, value: &str) -> Result<(), ExtensionError> {
    validate_text(label, value, 1, 512)?;
    let path = Path::new(value);
    if path.is_absolute()
        || value.contains('\\')
        || value.contains("//")
        || value.ends_with('/')
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return invalid(format!(
            "{label} must be a normalized project-relative path"
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir))
    {
        return invalid(format!(
            "{label} must not contain current-directory segments"
        ));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::Normal(value)
                if value == ".git" || value == ".girder" || value == ".bitcode"
        )
    }) || path
        .file_name()
        .is_some_and(|name| name == "girder.toml" || name == "bitcode.toml")
        || matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("aether" | "aetherb")
        )
    {
        return invalid(format!(
            "{label} must not target source-control or Girder metadata"
        ));
    }
    Ok(())
}

fn validate_relative_pattern(label: &str, value: &str) -> Result<(), ExtensionError> {
    validate_relative_path(label, value)?;
    GlobBuilder::new(value)
        .literal_separator(true)
        .backslash_escape(false)
        .build()
        .map_err(|error| ExtensionError::Invalid(format!("{label} is invalid: {error}")))?;
    Ok(())
}

fn validate_namespace(namespace: &str) -> Result<(), ExtensionError> {
    validate_text("graph namespace", namespace, 1, 256)?;
    if namespace.starts_with("crate::") || namespace.starts_with("extension::") {
        Ok(())
    } else {
        invalid("graph namespaces must start with crate:: or extension::")
    }
}

fn validate_program(program: &str) -> Result<(), ExtensionError> {
    validate_text("program", program, 1, 128)?;
    if program
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'+' | b'.'))
    {
        Ok(())
    } else {
        invalid("program must be a bare executable name without a path")
    }
}

fn validate_host(host: &str) -> Result<(), ExtensionError> {
    validate_text("network host", host, 1, 253)?;
    if host.starts_with('.')
        || host.ends_with('.')
        || host.contains("..")
        || !host.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
    {
        invalid("network host must be a lowercase DNS name without a scheme or path")
    } else {
        Ok(())
    }
}

fn validate_nonempty_list<T: Ord>(
    label: &str,
    values: &[T],
    mut validate: impl FnMut(&T) -> Result<(), ExtensionError>,
) -> Result<(), ExtensionError> {
    if values.is_empty() || values.len() > 64 {
        return invalid(format!("{label} must contain 1 to 64 entries"));
    }
    let mut unique = BTreeSet::new();
    for value in values {
        if !unique.insert(value) {
            return invalid(format!("{label} contains a duplicate entry"));
        }
        validate(value)?;
    }
    Ok(())
}

fn strip_json_fence(value: &str) -> Result<&str, ExtensionError> {
    let trimmed = value.trim();
    if !trimmed.starts_with("```") {
        return Ok(trimmed);
    }
    let body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```JSON"))
        .ok_or_else(|| {
            ExtensionError::Invalid("only a fenced JSON object may wrap a recipe".into())
        })?;
    body.strip_suffix("```")
        .map(str::trim)
        .ok_or_else(|| ExtensionError::Invalid("unterminated JSON fence".into()))
}

fn invalid<T>(message: impl Into<String>) -> Result<T, ExtensionError> {
    Err(ExtensionError::Invalid(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe() -> ExtensionRecipe {
        ExtensionRecipe {
            version: RECIPE_VERSION,
            id: "dev.bitcode.impact-view".into(),
            name: "Impact View".into(),
            description: "Shows semantic impact and writes a bounded report.".into(),
            intent: "add an impact panel".into(),
            capabilities: vec![
                Capability::ReadGraph,
                Capability::ContributeUi,
                Capability::WriteProject {
                    paths: vec!["generated/**".into()],
                },
            ],
            contributions: vec![Contribution::Panel {
                id: "impact".into(),
                title: "Impact".into(),
                location: PanelLocation::Right,
                view: PanelView::GraphQuery {
                    question: "what is impacted by the selected node?".into(),
                },
            }],
            projections: vec![ProjectProjection {
                path: "generated/impact.md".into(),
                contents: "# Impact\n".into(),
                mode: ProjectionMode::Create,
            }],
        }
    }

    fn projection_receipts(recipe: &ExtensionRecipe) -> Vec<ProjectionReceipt> {
        recipe
            .projections
            .iter()
            .map(|projection| ProjectionReceipt {
                path: projection.path.clone(),
                previous: None,
                applied_digest: content_digest(projection.contents.as_bytes()),
            })
            .collect()
    }

    #[test]
    fn recipe_round_trip_and_digest_are_deterministic() {
        let recipe = recipe();
        let json = recipe.to_json_pretty().unwrap();
        let decoded = ExtensionRecipe::from_json(&json).unwrap();

        assert_eq!(decoded, recipe);
        assert_eq!(decoded.digest().unwrap(), recipe.digest().unwrap());
        assert_eq!(recipe.digest().unwrap().len(), 64);
    }

    #[test]
    fn project_traversal_is_rejected() {
        let mut recipe = recipe();
        recipe.projections[0].path = "../outside.md".into();

        assert!(recipe
            .validate()
            .unwrap_err()
            .to_string()
            .contains("project-relative"));
    }

    #[test]
    fn project_metadata_paths_are_rejected() {
        for path in [
            ".git/hooks/pre-commit",
            ".girder/transactions/state",
            "girder.toml",
            ".bitcode/transactions/state",
            "bitcode.toml",
            "project.aether",
        ] {
            let mut recipe = recipe();
            recipe.capabilities = vec![Capability::WriteProject {
                paths: vec![path.into()],
            }];
            recipe.contributions.clear();
            recipe.projections[0].path = path.into();

            assert!(
                recipe
                    .validate()
                    .unwrap_err()
                    .to_string()
                    .contains("metadata"),
                "{path} should be reserved"
            );
        }
    }

    #[test]
    fn contribution_requires_its_capability() {
        let mut recipe = recipe();
        recipe
            .capabilities
            .retain(|capability| capability.kind() != CapabilityKind::ReadGraph);

        assert!(recipe
            .validate()
            .unwrap_err()
            .to_string()
            .contains("ReadGraph"));
    }

    #[test]
    fn validation_command_must_use_an_approved_program() {
        let mut recipe = recipe();
        recipe.capabilities.push(Capability::RunValidation {
            programs: vec!["cargo".into()],
        });
        recipe.contributions.push(Contribution::Command {
            id: "check".into(),
            title: "Check".into(),
            action: CommandAction::RunValidation {
                argv: vec!["sh".into(), "-c".into(), "touch /tmp/escape".into()],
            },
        });

        assert!(recipe
            .validate()
            .unwrap_err()
            .to_string()
            .contains("outside the requested"));
    }

    #[test]
    fn capability_scope_entries_must_be_unique() {
        let mut recipe = recipe();
        recipe.capabilities.push(Capability::ReadProject {
            paths: vec!["src/**".into(), "src/**".into()],
        });

        assert!(recipe
            .validate()
            .unwrap_err()
            .to_string()
            .contains("duplicate entry"));
    }

    #[test]
    fn grant_is_bound_to_the_exact_recipe() {
        let recipe = recipe();
        let grant = ExtensionGrant::approve(&recipe).unwrap();
        let mut changed = recipe.clone();
        changed.description.push_str(" changed");

        assert!(grant.authorize(&recipe).is_ok());
        assert!(grant
            .authorize(&changed)
            .unwrap_err()
            .to_string()
            .contains("digest"));
    }

    #[test]
    fn install_update_disable_and_uninstall_are_graph_native() {
        let mut graph = SemanticGraph::new();
        let recipe = recipe();
        let grant = ExtensionGrant::approve(&recipe).unwrap();
        install_with_receipts(
            &mut graph,
            recipe.clone(),
            grant,
            projection_receipts(&recipe),
        )
        .unwrap();

        assert_eq!(graph.query_by_kind(NodeKind::Extension).len(), 1);
        assert_eq!(
            graph.query_by_kind(NodeKind::ExtensionContribution).len(),
            2
        );
        assert_eq!(
            graph
                .edges()
                .into_iter()
                .filter(|(_, _, kind)| *kind == EdgeKind::Contributes)
                .count(),
            2
        );
        assert_eq!(records(&graph).len(), 1);

        let record = set_enabled(&mut graph, &recipe.id, false).unwrap();
        assert_eq!(record.state, ExtensionState::Disabled);
        assert_eq!(
            find_record(&graph, &recipe.id).unwrap().unwrap().state,
            ExtensionState::Disabled
        );

        let mut updated = recipe.clone();
        updated.contributions.clear();
        updated.projections.clear();
        updated.capabilities = Vec::new();
        let grant = ExtensionGrant::approve(&updated).unwrap();
        install(&mut graph, updated, grant).unwrap();
        assert!(graph
            .query_by_kind(NodeKind::ExtensionContribution)
            .is_empty());

        assert!(uninstall(&mut graph, &recipe.id).unwrap());
        assert!(!uninstall(&mut graph, &recipe.id).unwrap());
        assert!(graph.query_by_kind(NodeKind::Extension).is_empty());
    }

    #[test]
    fn extension_records_survive_serialization_and_source_reconciliation() {
        let mut persisted = SemanticGraph::new();
        let recipe = recipe();
        let grant = ExtensionGrant::approve(&recipe).unwrap();
        install_with_receipts(
            &mut persisted,
            recipe.clone(),
            grant,
            projection_receipts(&recipe),
        )
        .unwrap();

        let bytes = persisted.to_bytes().unwrap();
        let decoded = SemanticGraph::from_bytes(&bytes).unwrap();
        let (reconciled, report) =
            SemanticGraph::reconcile_persisted(SemanticGraph::new(), &decoded);

        assert!(find_record(&reconciled, &recipe.id).unwrap().is_some());
        assert_eq!(report.graph_owned_nodes, 3);
        assert_eq!(report.graph_owned_edges, 2);
    }

    #[test]
    fn projection_receipt_baselines_are_bounded_in_aggregate() {
        let mut recipe = recipe();
        recipe.projections = (0..5)
            .map(|index| ProjectProjection {
                path: format!("generated/{index}.md"),
                contents: "new".into(),
                mode: ProjectionMode::Replace,
            })
            .collect();
        let receipts = recipe
            .projections
            .iter()
            .map(|projection| ProjectionReceipt {
                path: projection.path.clone(),
                previous: Some("x".repeat(MAX_PROJECTION_BYTES)),
                applied_digest: content_digest(projection.contents.as_bytes()),
            })
            .collect();
        let grant = ExtensionGrant::approve(&recipe).unwrap();

        let error =
            install_with_receipts(&mut SemanticGraph::new(), recipe, grant, receipts).unwrap_err();
        assert!(error.to_string().contains("baselines exceed"));
    }

    #[test]
    fn unknown_recipe_fields_are_rejected() {
        let mut value = serde_json::to_value(recipe()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("executable".into(), serde_json::json!("/tmp/plugin"));

        assert!(ExtensionRecipe::from_json(&value.to_string()).is_err());
    }
}
