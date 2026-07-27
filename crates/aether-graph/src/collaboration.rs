//! Convergent, graph-native collaboration.
//!
//! [`GraphReplica`] is an operation-set CRDT over semantic nodes and typed
//! edges. Every local operation carries a per-actor dot plus the causal version
//! observed when it was created. Replicas exchange idempotent deltas and derive
//! the same [`SemanticGraph`] regardless of delivery order.
//!
//! Conflict policy is deliberately explicit:
//! - causally newer operations replace older operations;
//! - concurrent removals win over upserts;
//! - concurrent upserts use a deterministic `(counter, actor)` tie-break;
//! - removing and later recreating a node does not resurrect edges from the
//!   previous node generation.

use crate::{Edge, EdgeKind, GraphError, Node, NodeId, SemanticGraph};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

const COLLAB_MAGIC: &str = "BITCODE_COLLAB";
const COLLAB_VERSION: u32 = 2;
const LEGACY_COLLAB_VERSION: u32 = 1;
const BOOTSTRAP_ACTOR: &str = "bitcode.bootstrap";
static NEXT_TEMP_FILE: AtomicUsize = AtomicUsize::new(0);

/// Stable identity for one human, agent, or automation replica.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ActorId(String);

impl ActorId {
    /// Create a bounded portable actor id.
    pub fn new(value: impl Into<String>) -> Result<Self, GraphError> {
        let value = value.into();
        validate_actor(&value, false)?;
        Ok(Self(value))
    }

    fn bootstrap() -> Self {
        Self(BOOTSTRAP_ACTOR.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ActorId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn validate_actor(actor: &str, allow_bootstrap: bool) -> Result<(), GraphError> {
    if actor.is_empty() || actor.len() > 64 {
        return Err(GraphError::Collaboration(
            "actor id must contain 1..=64 bytes".into(),
        ));
    }
    if !actor
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(GraphError::Collaboration(format!(
            "invalid actor id '{actor}': use ASCII letters, digits, '.', '_' or '-'"
        )));
    }
    if !allow_bootstrap && actor == BOOTSTRAP_ACTOR {
        return Err(GraphError::Collaboration(format!(
            "'{BOOTSTRAP_ACTOR}' is reserved"
        )));
    }
    Ok(())
}

/// One unique event in an actor's monotonically increasing sequence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Dot {
    pub actor: ActorId,
    pub counter: u64,
}

impl Dot {
    fn lww_cmp(&self, other: &Self) -> Ordering {
        self.counter
            .cmp(&other.counter)
            .then_with(|| self.actor.cmp(&other.actor))
    }
}

/// Compact record of the highest contiguous event observed from each actor.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionVector {
    entries: BTreeMap<ActorId, u64>,
}

impl VersionVector {
    pub fn counter(&self, actor: &ActorId) -> u64 {
        self.entries.get(actor).copied().unwrap_or(0)
    }

    pub fn observes(&self, dot: &Dot) -> bool {
        self.counter(&dot.actor) >= dot.counter
    }

    pub fn dominates(&self, other: &Self) -> bool {
        other
            .entries
            .iter()
            .all(|(actor, counter)| self.counter(actor) >= *counter)
    }

    pub fn actors(&self) -> impl Iterator<Item = (&ActorId, u64)> {
        self.entries
            .iter()
            .map(|(actor, counter)| (actor, *counter))
    }

    fn observe(&mut self, dot: &Dot) {
        let counter = self.entries.entry(dot.actor.clone()).or_default();
        *counter = (*counter).max(dot.counter);
    }

    fn meet(&self, other: &Self) -> Self {
        let entries = self
            .entries
            .iter()
            .filter_map(|(actor, counter)| {
                let shared = (*counter).min(other.counter(actor));
                (shared != 0).then(|| (actor.clone(), shared))
            })
            .collect();
        Self { entries }
    }
}

/// A replicated semantic-graph mutation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GraphAction {
    UpsertNode(Node),
    RemoveNode(NodeId),
    UpsertEdge {
        from: NodeId,
        to: NodeId,
        edge: Edge,
    },
    RemoveEdge {
        from: NodeId,
        to: NodeId,
        kind: EdgeKind,
    },
}

/// An immutable operation plus the causal state seen by its author.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphOperation {
    pub dot: Dot,
    pub context: VersionVector,
    pub action: GraphAction,
}

/// Idempotent operations missing from a peer's version vector.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphDelta {
    pub operations: Vec<GraphOperation>,
}

impl GraphDelta {
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    pub fn len(&self) -> usize {
        self.operations.len()
    }
}

/// Outcome of merging a delta or another replica.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MergeReport {
    pub inserted: usize,
    pub already_present: usize,
}

/// Semantic mutations recorded while reconciling a replica with a graph snapshot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub nodes_upserted: usize,
    pub nodes_removed: usize,
    pub edges_upserted: usize,
    pub edges_removed: usize,
}

impl SyncReport {
    pub fn operation_count(self) -> usize {
        self.nodes_upserted + self.nodes_removed + self.edges_upserted + self.edges_removed
    }
}

/// Outcome of conservatively pruning causally superseded acknowledged history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionReport {
    pub operations_before: usize,
    pub operations_after: usize,
    pub removed_operations: usize,
    pub history_floor: VersionVector,
}

/// Operation-set CRDT for a semantic graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphReplica {
    actor: ActorId,
    clock: VersionVector,
    #[serde(default)]
    history_floor: VersionVector,
    #[serde(default)]
    acknowledgements: BTreeMap<ActorId, VersionVector>,
    operations: BTreeMap<Dot, GraphOperation>,
}

impl GraphReplica {
    pub fn new(actor: ActorId) -> Self {
        Self {
            actor,
            clock: VersionVector::default(),
            history_floor: VersionVector::default(),
            acknowledgements: BTreeMap::new(),
            operations: BTreeMap::new(),
        }
    }

    /// Turn a graph snapshot into a deterministic bootstrap history.
    ///
    /// Every replica initialized from identical graph bytes receives identical
    /// bootstrap operations, so subsequent deltas contain only real edits.
    pub fn from_graph(actor: ActorId, graph: &SemanticGraph) -> Self {
        let mut replica = Self::new(actor);
        let bootstrap = ActorId::bootstrap();

        let mut nodes: Vec<_> = graph.nodes().cloned().collect();
        nodes.sort_by_key(|node| node.id);
        for node in nodes {
            replica
                .record_as(bootstrap.clone(), GraphAction::UpsertNode(node))
                .expect("bootstrap counter cannot overflow");
        }

        let mut edges = graph.edge_records();
        edges.sort_by(|left, right| {
            (left.0, left.1, left.2.kind).cmp(&(right.0, right.1, right.2.kind))
        });
        for (from, to, edge) in edges {
            replica
                .record_as(
                    bootstrap.clone(),
                    GraphAction::UpsertEdge { from, to, edge },
                )
                .expect("bootstrap counter cannot overflow");
        }
        replica
    }

    pub fn actor(&self) -> &ActorId {
        &self.actor
    }

    pub fn version(&self) -> &VersionVector {
        &self.clock
    }

    pub fn operation_count(&self) -> usize {
        self.operations.len()
    }

    pub fn history_floor(&self) -> &VersionVector {
        &self.history_floor
    }

    pub fn acknowledgements(&self) -> impl Iterator<Item = (&ActorId, &VersionVector)> {
        self.acknowledgements.iter()
    }

    /// Copy this history for a new unique actor.
    pub fn fork(&self, actor: ActorId) -> Result<Self, GraphError> {
        if self.clock.counter(&actor) != 0 || self.acknowledgements.contains_key(&actor) {
            return Err(GraphError::Collaboration(format!(
                "actor '{actor}' already exists in this history"
            )));
        }
        let mut replica = self.clone();
        replica.actor = actor;
        Ok(replica)
    }

    pub fn upsert_node(&mut self, node: Node) -> Result<Dot, GraphError> {
        self.record(GraphAction::UpsertNode(node))
    }

    pub fn remove_node(&mut self, id: NodeId) -> Result<Dot, GraphError> {
        self.record(GraphAction::RemoveNode(id))
    }

    pub fn upsert_edge(&mut self, from: NodeId, to: NodeId, edge: Edge) -> Result<Dot, GraphError> {
        let graph = self.materialize()?;
        if !graph.contains(from) {
            return Err(GraphError::NodeNotFound(from));
        }
        if !graph.contains(to) {
            return Err(GraphError::NodeNotFound(to));
        }
        self.record(GraphAction::UpsertEdge { from, to, edge })
    }

    pub fn remove_edge(
        &mut self,
        from: NodeId,
        to: NodeId,
        kind: EdgeKind,
    ) -> Result<Dot, GraphError> {
        self.record(GraphAction::RemoveEdge { from, to, kind })
    }

    pub fn delta_since(&self, known: &VersionVector) -> Result<GraphDelta, GraphError> {
        if !known.dominates(&self.history_floor) {
            return Err(GraphError::Collaboration(
                "peer version predates compacted collaboration history; transfer a current bundle before exchanging deltas".into(),
            ));
        }
        Ok(GraphDelta {
            operations: self
                .operations
                .values()
                .filter(|operation| !known.observes(&operation.dot))
                .cloned()
                .collect(),
        })
    }

    pub fn merge(&mut self, other: &Self) -> Result<MergeReport, GraphError> {
        self.apply_delta(&other.delta_since(&self.clock)?)
    }

    pub fn apply_delta(&mut self, delta: &GraphDelta) -> Result<MergeReport, GraphError> {
        self.validate()?;
        let mut report = MergeReport::default();
        let mut staged: BTreeMap<Dot, GraphOperation> = BTreeMap::new();
        for operation in &delta.operations {
            validate_operation(operation)?;
            if let Some(previous) = staged.insert(operation.dot.clone(), operation.clone()) {
                if previous != *operation {
                    return Err(GraphError::Collaboration(format!(
                        "delta reuses dot {}:{} for different operations",
                        operation.dot.actor, operation.dot.counter
                    )));
                }
                report.already_present += 1;
            }
        }

        for operation in staged.values() {
            if let Some(existing) = self.operations.get(&operation.dot) {
                if existing != operation {
                    return Err(GraphError::Collaboration(format!(
                        "conflicting operations reuse dot {}:{}",
                        operation.dot.actor, operation.dot.counter
                    )));
                }
                report.already_present += 1;
            } else if self.clock.observes(&operation.dot) {
                report.already_present += 1;
            }
        }

        let mut available_clock = self.clock.clone();
        for operation in staged.values() {
            if available_clock.observes(&operation.dot) {
                continue;
            }
            let expected = available_clock
                .counter(&operation.dot.actor)
                .checked_add(1)
                .ok_or_else(|| {
                    GraphError::Collaboration(format!(
                        "actor '{}' counter overflow",
                        operation.dot.actor
                    ))
                })?;
            if operation.dot.counter != expected {
                return Err(GraphError::Collaboration(format!(
                    "delta has a gap for '{}': expected counter {expected}, received {}",
                    operation.dot.actor, operation.dot.counter
                )));
            }
            available_clock.observe(&operation.dot);
        }
        for operation in staged.values() {
            for (actor, counter) in operation.context.actors() {
                if available_clock.counter(actor) < counter {
                    return Err(GraphError::Collaboration(format!(
                        "operation {}:{} is missing causal history for {actor}:{counter}",
                        operation.dot.actor, operation.dot.counter
                    )));
                }
            }
        }

        for (dot, operation) in staged {
            if self.clock.observes(&dot) {
                continue;
            }
            self.operations.insert(dot.clone(), operation);
            self.clock.observe(&dot);
            report.inserted += 1;
        }
        Ok(report)
    }

    /// Record a peer's durable causal acknowledgement.
    ///
    /// Acknowledgements are monotonic and cannot claim history this replica has
    /// not observed. Live transport records them only after the peer persists.
    pub fn acknowledge(&mut self, peer: ActorId, version: VersionVector) -> Result<(), GraphError> {
        if peer == self.actor {
            return Err(GraphError::Collaboration(
                "a replica cannot acknowledge itself as a peer".into(),
            ));
        }
        if !self.clock.dominates(&version) {
            return Err(GraphError::Collaboration(format!(
                "peer '{peer}' acknowledges history not present in this replica"
            )));
        }
        if let Some(previous) = self.acknowledgements.get(&peer) {
            if !version.dominates(previous) {
                return Err(GraphError::Collaboration(format!(
                    "peer '{peer}' acknowledgement would move backwards"
                )));
            }
        }
        self.acknowledgements.insert(peer, version);
        Ok(())
    }

    /// Prune only acknowledged operations that are causally superseded.
    ///
    /// Maximal concurrent operations and node-removal generation barriers are
    /// retained. Peers older than the resulting history floor must receive a
    /// current bundle because their missing operations no longer exist.
    pub fn compact_acknowledged(&mut self) -> Result<CompactionReport, GraphError> {
        self.validate()?;
        if self.acknowledgements.is_empty() {
            return Err(GraphError::Collaboration(
                "history compaction requires at least one durable peer acknowledgement".into(),
            ));
        }
        let stable = self
            .acknowledgements
            .values()
            .fold(self.clock.clone(), |frontier, version| {
                frontier.meet(version)
            });
        let operations_before = self.operations.len();
        let mut by_key: BTreeMap<OperationKey, Vec<&GraphOperation>> = BTreeMap::new();
        for operation in self.operations.values() {
            by_key
                .entry(OperationKey::from_action(&operation.action))
                .or_default()
                .push(operation);
        }
        let removable: Vec<_> = self
            .operations
            .values()
            .filter(|candidate| {
                stable.observes(&candidate.dot)
                    && !matches!(candidate.action, GraphAction::RemoveNode(_))
                    && by_key[&OperationKey::from_action(&candidate.action)]
                        .iter()
                        .copied()
                        .any(|later| later.dot != candidate.dot && happens_before(candidate, later))
            })
            .map(|operation| operation.dot.clone())
            .collect();

        for dot in &removable {
            self.operations.remove(dot);
            self.history_floor.observe(dot);
        }
        self.validate()?;
        Ok(CompactionReport {
            operations_before,
            operations_after: self.operations.len(),
            removed_operations: removable.len(),
            history_floor: self.history_floor.clone(),
        })
    }

    /// Record the minimal semantic mutations needed to match a graph snapshot.
    pub fn sync_graph(&mut self, target: &SemanticGraph) -> Result<SyncReport, GraphError> {
        let current = self.materialize()?;
        let current_nodes: BTreeMap<_, _> = current.nodes().map(|node| (node.id, node)).collect();
        let target_nodes: BTreeMap<_, _> = target.nodes().map(|node| (node.id, node)).collect();
        let current_edges: BTreeMap<_, _> = current
            .edge_records()
            .into_iter()
            .map(|(from, to, edge)| (EdgeKey::new(from, to, edge.kind), edge))
            .collect();
        let target_edges: BTreeMap<_, _> = target
            .edge_records()
            .into_iter()
            .map(|(from, to, edge)| (EdgeKey::new(from, to, edge.kind), edge))
            .collect();
        let mut report = SyncReport::default();

        for (id, node) in &target_nodes {
            if current_nodes.get(id).copied() != Some(*node) {
                self.record(GraphAction::UpsertNode((*node).clone()))?;
                report.nodes_upserted += 1;
            }
        }
        for (key, edge) in &target_edges {
            if current_edges.get(key) != Some(edge) {
                self.record(GraphAction::UpsertEdge {
                    from: key.from,
                    to: key.to,
                    edge: (*edge).clone(),
                })?;
                report.edges_upserted += 1;
            }
        }
        for key in current_edges.keys() {
            if !target_edges.contains_key(key) {
                self.record(GraphAction::RemoveEdge {
                    from: key.from,
                    to: key.to,
                    kind: key.kind,
                })?;
                report.edges_removed += 1;
            }
        }
        for id in current_nodes.keys() {
            if !target_nodes.contains_key(id) {
                self.record(GraphAction::RemoveNode(*id))?;
                report.nodes_removed += 1;
            }
        }

        Ok(report)
    }

    /// Derive the converged semantic graph from the operation set.
    pub fn materialize(&self) -> Result<SemanticGraph, GraphError> {
        self.validate()?;

        let mut node_operations: BTreeMap<NodeId, Vec<&GraphOperation>> = BTreeMap::new();
        let mut edge_operations: BTreeMap<EdgeKey, Vec<&GraphOperation>> = BTreeMap::new();
        for operation in self.operations.values() {
            match &operation.action {
                GraphAction::UpsertNode(node) => {
                    node_operations.entry(node.id).or_default().push(operation);
                }
                GraphAction::RemoveNode(id) => {
                    node_operations.entry(*id).or_default().push(operation);
                }
                GraphAction::UpsertEdge { from, to, edge } => {
                    edge_operations
                        .entry(EdgeKey::new(*from, *to, edge.kind))
                        .or_default()
                        .push(operation);
                }
                GraphAction::RemoveEdge { from, to, kind } => {
                    edge_operations
                        .entry(EdgeKey::new(*from, *to, *kind))
                        .or_default()
                        .push(operation);
                }
            }
        }

        let mut active_nodes: BTreeMap<NodeId, (&Node, &GraphOperation)> = BTreeMap::new();
        for (id, operations) in &node_operations {
            if let Some(winner) = winning_upsert(operations, |action| {
                matches!(action, GraphAction::RemoveNode(_))
            }) {
                if let GraphAction::UpsertNode(node) = &winner.action {
                    active_nodes.insert(*id, (node, winner));
                }
            }
        }

        let mut graph = SemanticGraph::new();
        for (node, _) in active_nodes.values() {
            graph.upsert_node((*node).clone());
        }

        for (key, operations) in edge_operations {
            let Some(winner) = winning_upsert(&operations, |action| {
                matches!(action, GraphAction::RemoveEdge { .. })
            }) else {
                continue;
            };
            let GraphAction::UpsertEdge { edge, .. } = &winner.action else {
                continue;
            };
            let (Some((_, from_generation)), Some((_, to_generation))) =
                (active_nodes.get(&key.from), active_nodes.get(&key.to))
            else {
                continue;
            };
            if !edge_belongs_to_active_generation(
                winner,
                from_generation,
                node_operations
                    .get(&key.from)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
            ) || !edge_belongs_to_active_generation(
                winner,
                to_generation,
                node_operations
                    .get(&key.to)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
            ) {
                continue;
            }
            graph.add_edge(key.from, key.to, edge.clone())?;
        }
        Ok(graph)
    }

    pub fn to_ron(&self) -> Result<String, GraphError> {
        self.validate()?;
        let file = CollaborationFile {
            magic: COLLAB_MAGIC.to_string(),
            version: COLLAB_VERSION,
            replica: self.clone(),
        };
        ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default())
            .map_err(|error| GraphError::Serialize(error.to_string()))
    }

    pub fn from_ron(text: &str) -> Result<Self, GraphError> {
        let file: CollaborationFile =
            ron::from_str(text).map_err(|error| GraphError::Deserialize(error.to_string()))?;
        validate_file(&file)?;
        file.replica.validate()?;
        Ok(file.replica)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, GraphError> {
        self.validate()?;
        let file = CollaborationFile {
            magic: COLLAB_MAGIC.to_string(),
            version: COLLAB_VERSION,
            replica: self.clone(),
        };
        bincode::serialize(&file).map_err(|error| GraphError::Serialize(error.to_string()))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, GraphError> {
        if let Ok(file) = bincode::deserialize::<CollaborationFile>(bytes) {
            validate_file(&file)?;
            file.replica.validate()?;
            return Ok(file.replica);
        }
        let legacy: LegacyCollaborationFile = bincode::deserialize(bytes)
            .map_err(|error| GraphError::Deserialize(error.to_string()))?;
        if legacy.magic != COLLAB_MAGIC || legacy.version != LEGACY_COLLAB_VERSION {
            return Err(GraphError::Deserialize(
                "unsupported collaboration bundle format".into(),
            ));
        }
        let replica = GraphReplica {
            actor: legacy.replica.actor,
            clock: legacy.replica.clock,
            history_floor: VersionVector::default(),
            acknowledgements: BTreeMap::new(),
            operations: legacy.replica.operations,
        };
        replica.validate()?;
        Ok(replica)
    }

    /// Save a collaboration bundle. `.aethercb` selects compact bincode;
    /// every other extension uses reviewable RON.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), GraphError> {
        let path = path.as_ref();
        let bytes = if path.extension().and_then(|extension| extension.to_str()) == Some("aethercb")
        {
            self.to_bytes()?
        } else {
            self.to_ron()?.into_bytes()
        };
        atomic_write(path, &bytes)?;
        Ok(())
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, GraphError> {
        let path = path.as_ref();
        if path.extension().and_then(|extension| extension.to_str()) == Some("aethercb") {
            Self::from_bytes(&std::fs::read(path)?)
        } else {
            Self::from_ron(&std::fs::read_to_string(path)?)
        }
    }

    fn record(&mut self, action: GraphAction) -> Result<Dot, GraphError> {
        self.record_as(self.actor.clone(), action)
    }

    fn record_as(&mut self, actor: ActorId, action: GraphAction) -> Result<Dot, GraphError> {
        let context = self.clock.clone();
        let counter = self.clock.counter(&actor).checked_add(1).ok_or_else(|| {
            GraphError::Collaboration(format!("actor '{actor}' counter overflow"))
        })?;
        let dot = Dot { actor, counter };
        let operation = GraphOperation {
            dot: dot.clone(),
            context,
            action,
        };
        self.operations.insert(dot.clone(), operation);
        self.clock.observe(&dot);
        Ok(dot)
    }

    fn validate(&self) -> Result<(), GraphError> {
        validate_actor(self.actor.as_str(), false)?;
        validate_version(&self.clock)?;
        validate_version(&self.history_floor)?;
        if !self.clock.dominates(&self.history_floor) {
            return Err(GraphError::Collaboration(
                "history floor exceeds the replica clock".into(),
            ));
        }
        for (dot, operation) in &self.operations {
            if dot != &operation.dot {
                return Err(GraphError::Collaboration(
                    "operation map key does not match operation dot".into(),
                ));
            }
            validate_operation(operation)?;
            if !self.clock.observes(dot) {
                return Err(GraphError::Collaboration(format!(
                    "operation {}:{} exceeds the replica clock",
                    dot.actor, dot.counter
                )));
            }
            if !self.clock.dominates(&operation.context) {
                return Err(GraphError::Collaboration(format!(
                    "operation {}:{} references unseen causal history",
                    dot.actor, dot.counter
                )));
            }
        }
        for (actor, maximum) in self.clock.actors() {
            let floor = self.history_floor.counter(actor);
            if floor == maximum {
                continue;
            }
            for counter in (floor + 1)..=maximum {
                let dot = Dot {
                    actor: actor.clone(),
                    counter,
                };
                if !self.operations.contains_key(&dot) {
                    return Err(GraphError::Collaboration(format!(
                        "operation history for '{actor}' has a gap above compacted counter {floor}: missing {counter}"
                    )));
                }
            }
        }
        for (peer, version) in &self.acknowledgements {
            validate_actor(peer.as_str(), false)?;
            if peer == &self.actor {
                return Err(GraphError::Collaboration(
                    "replica acknowledgement table contains its own actor".into(),
                ));
            }
            validate_version(version)?;
            if !self.clock.dominates(version) {
                return Err(GraphError::Collaboration(format!(
                    "peer '{peer}' acknowledgement exceeds the replica clock"
                )));
            }
        }
        Ok(())
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("collaboration");
    let temporary = parent.join(format!(
        ".{file_name}.bitcode-collab-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP_FILE.fetch_add(1, AtomicOrdering::Relaxed)
    ));
    let permissions = std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let result: std::io::Result<()> = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct EdgeKey {
    from: NodeId,
    to: NodeId,
    kind: EdgeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum OperationKey {
    Node(NodeId),
    Edge(EdgeKey),
}

impl OperationKey {
    fn from_action(action: &GraphAction) -> Self {
        match action {
            GraphAction::UpsertNode(node) => Self::Node(node.id),
            GraphAction::RemoveNode(id) => Self::Node(*id),
            GraphAction::UpsertEdge { from, to, edge } => {
                Self::Edge(EdgeKey::new(*from, *to, edge.kind))
            }
            GraphAction::RemoveEdge { from, to, kind } => {
                Self::Edge(EdgeKey::new(*from, *to, *kind))
            }
        }
    }
}

impl EdgeKey {
    fn new(from: NodeId, to: NodeId, kind: EdgeKind) -> Self {
        Self { from, to, kind }
    }
}

#[derive(Serialize, Deserialize)]
struct CollaborationFile {
    magic: String,
    version: u32,
    replica: GraphReplica,
}

#[derive(Serialize, Deserialize)]
struct LegacyCollaborationFile {
    magic: String,
    version: u32,
    replica: LegacyGraphReplica,
}

#[derive(Serialize, Deserialize)]
struct LegacyGraphReplica {
    actor: ActorId,
    clock: VersionVector,
    operations: BTreeMap<Dot, GraphOperation>,
}

fn validate_file(file: &CollaborationFile) -> Result<(), GraphError> {
    if file.magic != COLLAB_MAGIC {
        return Err(GraphError::Deserialize(
            "bad collaboration bundle magic".into(),
        ));
    }
    if !matches!(file.version, LEGACY_COLLAB_VERSION | COLLAB_VERSION) {
        return Err(GraphError::Deserialize(format!(
            "unsupported collaboration version {}; expected {LEGACY_COLLAB_VERSION} or {COLLAB_VERSION}",
            file.version
        )));
    }
    Ok(())
}

fn validate_version(version: &VersionVector) -> Result<(), GraphError> {
    for (actor, counter) in version.actors() {
        validate_actor(actor.as_str(), true)?;
        if counter == 0 {
            return Err(GraphError::Collaboration(format!(
                "version vector contains zero counter for '{actor}'"
            )));
        }
    }
    Ok(())
}

fn validate_operation(operation: &GraphOperation) -> Result<(), GraphError> {
    validate_actor(operation.dot.actor.as_str(), true)?;
    if operation.dot.counter == 0 {
        return Err(GraphError::Collaboration(
            "operation counters start at one".into(),
        ));
    }
    if operation.context.observes(&operation.dot) {
        return Err(GraphError::Collaboration(format!(
            "operation {}:{} causally observes itself",
            operation.dot.actor, operation.dot.counter
        )));
    }
    if operation
        .context
        .counter(&operation.dot.actor)
        .checked_add(1)
        != Some(operation.dot.counter)
    {
        return Err(GraphError::Collaboration(format!(
            "operation {}:{} has a non-contiguous actor context",
            operation.dot.actor, operation.dot.counter
        )));
    }
    validate_version(&operation.context)?;
    if let GraphAction::UpsertNode(node) = &operation.action {
        if node.id != NodeId::from_path(&node.path) {
            return Err(GraphError::Collaboration(format!(
                "node {:?} does not match semantic path '{}'",
                node.id, node.path
            )));
        }
    }
    if let GraphAction::UpsertEdge { edge, .. } = &operation.action {
        if !edge.weight.is_finite() || !(0.0..=1.0).contains(&edge.weight) {
            return Err(GraphError::Collaboration(
                "edge weight must be finite and within [0, 1]".into(),
            ));
        }
    }
    Ok(())
}

fn happens_before(left: &GraphOperation, right: &GraphOperation) -> bool {
    right.context.observes(&left.dot)
}

fn maximal_operations<'a>(operations: &[&'a GraphOperation]) -> Vec<&'a GraphOperation> {
    operations
        .iter()
        .copied()
        .filter(|candidate| {
            !operations
                .iter()
                .copied()
                .any(|other| candidate.dot != other.dot && happens_before(candidate, other))
        })
        .collect()
}

fn winning_upsert<'a>(
    operations: &[&'a GraphOperation],
    is_remove: impl Fn(&GraphAction) -> bool,
) -> Option<&'a GraphOperation> {
    let maximal = maximal_operations(operations);
    if maximal.iter().any(|operation| is_remove(&operation.action)) {
        return None;
    }
    maximal
        .into_iter()
        .max_by(|left, right| left.dot.lww_cmp(&right.dot))
}

fn edge_belongs_to_active_generation(
    edge: &GraphOperation,
    active_upsert: &GraphOperation,
    node_operations: &[&GraphOperation],
) -> bool {
    node_operations.iter().copied().all(|operation| {
        if !matches!(operation.action, GraphAction::RemoveNode(_))
            || !happens_before(operation, active_upsert)
        {
            return true;
        }
        happens_before(operation, edge)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeKind;

    fn actor(name: &str) -> ActorId {
        ActorId::new(name).unwrap()
    }

    fn fixture() -> (SemanticGraph, NodeId, NodeId) {
        let mut graph = SemanticGraph::new();
        let module = graph.upsert_node(Node::new(NodeKind::Module, "app", "crate::app"));
        let run = graph.upsert_node(
            Node::new(NodeKind::Function, "run", "crate::app::run")
                .with_language("rust")
                .with_source("fn run() {}"),
        );
        graph
            .add_edge(module, run, Edge::new(EdgeKind::Contains))
            .unwrap();
        (graph, module, run)
    }

    fn edited_run(source: &str) -> Node {
        Node::new(NodeKind::Function, "run", "crate::app::run")
            .with_language("rust")
            .with_source(source)
    }

    #[test]
    fn concurrent_updates_converge_independent_of_merge_order() {
        let (graph, _, run) = fixture();
        let mut alice = GraphReplica::from_graph(actor("alice"), &graph);
        let mut bob = GraphReplica::from_graph(actor("bob"), &graph);
        alice
            .upsert_node(edited_run("fn run() { alice(); }"))
            .unwrap();
        bob.upsert_node(edited_run("fn run() { bob(); }")).unwrap();

        let alice_snapshot = alice.clone();
        let bob_snapshot = bob.clone();
        alice.merge(&bob_snapshot).unwrap();
        bob.merge(&alice_snapshot).unwrap();

        assert_eq!(
            alice.materialize().unwrap().to_ron().unwrap(),
            bob.materialize().unwrap().to_ron().unwrap()
        );
        assert_eq!(
            alice.materialize().unwrap().get(run).unwrap().source,
            "fn run() { bob(); }"
        );
    }

    #[test]
    fn concurrent_remove_wins_over_update() {
        let (graph, _, run) = fixture();
        let mut alice = GraphReplica::from_graph(actor("alice"), &graph);
        let mut bob = GraphReplica::from_graph(actor("bob"), &graph);
        alice.remove_node(run).unwrap();
        bob.upsert_node(edited_run("fn run() { changed(); }"))
            .unwrap();

        alice.merge(&bob).unwrap();
        assert!(!alice.materialize().unwrap().contains(run));
    }

    #[test]
    fn causally_later_recreation_wins_and_old_edges_stay_deleted() {
        let (graph, module, run) = fixture();
        let mut alice = GraphReplica::from_graph(actor("alice"), &graph);
        let mut bob = GraphReplica::from_graph(actor("bob"), &graph);
        alice.remove_node(run).unwrap();
        bob.merge(&alice).unwrap();
        bob.upsert_node(edited_run("fn run() { recreated(); }"))
            .unwrap();
        alice.merge(&bob).unwrap();

        let materialized = alice.materialize().unwrap();
        assert_eq!(
            materialized.get(run).unwrap().source,
            "fn run() { recreated(); }"
        );
        assert!(
            materialized
                .neighbors(module, Some(EdgeKind::Contains))
                .is_empty(),
            "an edge from the removed node generation must not resurrect"
        );
    }

    #[test]
    fn concurrent_edge_remove_wins_over_update() {
        let (graph, module, run) = fixture();
        let mut alice = GraphReplica::from_graph(actor("alice"), &graph);
        let mut bob = GraphReplica::from_graph(actor("bob"), &graph);
        alice.remove_edge(module, run, EdgeKind::Contains).unwrap();
        bob.upsert_edge(module, run, Edge::with_weight(EdgeKind::Contains, 0.5))
            .unwrap();
        alice.merge(&bob).unwrap();

        assert!(alice
            .materialize()
            .unwrap()
            .neighbors(module, Some(EdgeKind::Contains))
            .is_empty());
    }

    #[test]
    fn deltas_are_minimal_idempotent_and_convergent() {
        let (graph, _, run) = fixture();
        let mut alice = GraphReplica::from_graph(actor("alice"), &graph);
        let mut bob = GraphReplica::from_graph(actor("bob"), &graph);
        alice
            .upsert_node(edited_run("fn run() { synced(); }"))
            .unwrap();

        let delta = alice.delta_since(bob.version()).unwrap();
        assert_eq!(delta.len(), 1);
        assert_eq!(bob.apply_delta(&delta).unwrap().inserted, 1);
        assert_eq!(bob.apply_delta(&delta).unwrap().already_present, 1);
        assert!(alice.delta_since(bob.version()).unwrap().is_empty());
        assert_eq!(
            alice.materialize().unwrap().to_ron().unwrap(),
            bob.materialize().unwrap().to_ron().unwrap()
        );
        assert_eq!(
            bob.materialize().unwrap().get(run).unwrap().source,
            "fn run() { synced(); }"
        );
    }

    #[test]
    fn collaboration_bundle_round_trips_and_rejects_reserved_actor() {
        let (graph, _, _) = fixture();
        let replica = GraphReplica::from_graph(actor("alice"), &graph);
        let ron = replica.to_ron().unwrap();
        let decoded = GraphReplica::from_ron(&ron).unwrap();
        assert_eq!(decoded.operation_count(), replica.operation_count());
        let materialized = decoded.materialize().unwrap();
        assert_eq!(materialized.node_count(), graph.node_count());
        assert_eq!(materialized.edge_count(), graph.edge_count());
        for node in graph.nodes() {
            assert_eq!(materialized.get(node.id), Some(node));
        }

        let bytes = replica.to_bytes().unwrap();
        assert_eq!(
            GraphReplica::from_bytes(&bytes).unwrap().operation_count(),
            replica.operation_count()
        );
        assert!(ActorId::new(BOOTSTRAP_ACTOR).is_err());
    }

    #[test]
    fn acknowledged_compaction_preserves_state_and_rejects_stale_peers() {
        let (graph, _, run) = fixture();
        let mut alice = GraphReplica::from_graph(actor("alice"), &graph);
        let mut bob = alice.fork(actor("bob")).unwrap();
        alice
            .upsert_node(edited_run("fn run() { first(); }"))
            .unwrap();
        alice
            .upsert_node(edited_run("fn run() { second(); }"))
            .unwrap();
        bob.apply_delta(&alice.delta_since(bob.version()).unwrap())
            .unwrap();
        let expected = alice.materialize().unwrap().to_ron().unwrap();
        alice
            .acknowledge(actor("bob"), bob.version().clone())
            .unwrap();

        let report = alice.compact_acknowledged().unwrap();
        assert!(report.removed_operations >= 2);
        assert_eq!(alice.materialize().unwrap().to_ron().unwrap(), expected);
        assert_eq!(
            alice.materialize().unwrap().get(run).unwrap().source,
            "fn run() { second(); }"
        );
        assert!(alice.delta_since(&VersionVector::default()).is_err());
        assert!(alice.delta_since(bob.version()).unwrap().is_empty());

        let decoded = GraphReplica::from_ron(&alice.to_ron().unwrap()).unwrap();
        assert_eq!(decoded.history_floor(), alice.history_floor());
        assert_eq!(decoded.materialize().unwrap().to_ron().unwrap(), expected);
    }

    #[test]
    fn compaction_requires_monotonic_known_peer_acknowledgements() {
        let (graph, _, _) = fixture();
        let mut replica = GraphReplica::from_graph(actor("alice"), &graph);
        assert!(replica.compact_acknowledged().is_err());

        let empty = VersionVector::default();
        replica.acknowledge(actor("bob"), empty.clone()).unwrap();
        let future = VersionVector {
            entries: BTreeMap::from([(actor("mallory"), 1)]),
        };
        assert!(replica.acknowledge(actor("mallory"), future).is_err());

        let current = replica.version().clone();
        replica.acknowledge(actor("bob"), current).unwrap();
        assert!(replica.acknowledge(actor("bob"), empty).is_err());
        assert!(replica
            .acknowledge(actor("alice"), replica.version().clone())
            .is_err());
    }

    #[test]
    fn compaction_retains_node_generation_barriers() {
        let (graph, module, run) = fixture();
        let mut alice = GraphReplica::from_graph(actor("alice"), &graph);
        let mut bob = alice.fork(actor("bob")).unwrap();
        alice.remove_node(run).unwrap();
        alice
            .upsert_node(edited_run("fn run() { recreated(); }"))
            .unwrap();
        bob.apply_delta(&alice.delta_since(bob.version()).unwrap())
            .unwrap();
        alice
            .acknowledge(actor("bob"), bob.version().clone())
            .unwrap();

        alice.compact_acknowledged().unwrap();
        let materialized = alice.materialize().unwrap();
        assert!(materialized.contains(run));
        assert!(
            materialized
                .neighbors(module, Some(EdgeKind::Contains))
                .is_empty(),
            "an edge from the deleted node generation must not reappear"
        );
        assert!(alice.operations.values().any(|operation| {
            matches!(operation.action, GraphAction::RemoveNode(id) if id == run)
        }));
    }

    #[test]
    fn legacy_binary_collaboration_bundles_migrate_to_current_state() {
        let (graph, _, _) = fixture();
        let replica = GraphReplica::from_graph(actor("alice"), &graph);
        let legacy = LegacyCollaborationFile {
            magic: COLLAB_MAGIC.into(),
            version: LEGACY_COLLAB_VERSION,
            replica: LegacyGraphReplica {
                actor: replica.actor.clone(),
                clock: replica.clock.clone(),
                operations: replica.operations.clone(),
            },
        };
        let bytes = bincode::serialize(&legacy).unwrap();
        let migrated = GraphReplica::from_bytes(&bytes).unwrap();
        let materialized = migrated.materialize().unwrap();
        assert_eq!(materialized.node_count(), graph.node_count());
        assert_eq!(materialized.edge_count(), graph.edge_count());
        for node in graph.nodes() {
            assert_eq!(materialized.get(node.id), Some(node));
        }
        for (from, to, edge) in graph.edge_records() {
            assert!(materialized.edge_records().contains(&(from, to, edge)));
        }
        assert_eq!(migrated.history_floor(), &VersionVector::default());
        assert_eq!(migrated.acknowledgements().count(), 0);
    }

    #[test]
    fn sync_graph_records_changes_and_reversed_deltas_apply_atomically() {
        let (graph, module, run) = fixture();
        let mut alice = GraphReplica::from_graph(actor("alice"), &graph);
        let mut target = graph.clone();
        target.upsert_node(edited_run("fn run() { synchronized(); }"));
        let extra = target.upsert_node(
            Node::new(NodeKind::Function, "extra", "crate::app::extra")
                .with_language("rust")
                .with_source("fn extra() {}"),
        );
        target
            .add_edge(module, extra, Edge::new(EdgeKind::Contains))
            .unwrap();

        let report = alice.sync_graph(&target).unwrap();
        assert_eq!(report.nodes_upserted, 2);
        assert_eq!(report.edges_upserted, 1);
        assert_eq!(report.operation_count(), 3);
        assert_eq!(
            alice.materialize().unwrap().get(run).unwrap().source,
            "fn run() { synchronized(); }"
        );

        let mut bob = GraphReplica::from_graph(actor("bob"), &graph);
        let mut delta = alice.delta_since(bob.version()).unwrap();
        delta.operations.reverse();
        assert_eq!(bob.apply_delta(&delta).unwrap().inserted, 3);
        assert_eq!(
            alice.materialize().unwrap().to_ron().unwrap(),
            bob.materialize().unwrap().to_ron().unwrap()
        );

        let previous_count = bob.operation_count();
        let missing_history = GraphDelta {
            operations: vec![GraphOperation {
                dot: Dot {
                    actor: actor("mallory"),
                    counter: 2,
                },
                context: VersionVector {
                    entries: BTreeMap::from([(actor("mallory"), 1)]),
                },
                action: GraphAction::RemoveNode(run),
            }],
        };
        assert!(bob.apply_delta(&missing_history).is_err());
        assert_eq!(bob.operation_count(), previous_count);
    }

    #[test]
    fn conflicting_dot_reuse_is_rejected() {
        let (graph, _, run) = fixture();
        let mut replica = GraphReplica::from_graph(actor("alice"), &graph);
        let mut other = replica.fork(actor("bob")).unwrap();
        let dot = other
            .upsert_node(edited_run("fn run() { first(); }"))
            .unwrap();
        let mut delta = other.delta_since(replica.version()).unwrap();
        delta.operations[0].action = GraphAction::RemoveNode(run);
        assert_eq!(delta.operations[0].dot, dot);
        replica.apply_delta(&delta).unwrap();

        let original = other.delta_since(&VersionVector::default()).unwrap();
        let conflicting = GraphDelta {
            operations: original
                .operations
                .into_iter()
                .filter(|operation| operation.dot == dot)
                .collect(),
        };
        assert!(replica.apply_delta(&conflicting).is_err());
    }
}
