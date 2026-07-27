use crate::project::config::ProjectConfig;
use crate::project::source::{
    commit_project_writes, graph_project_write, is_configured_source_path, load_reconciled_graph,
    read_project_bytes, ProjectWrite,
};
#[cfg(feature = "gui")]
use crate::project::validation::validate_candidate;
use aether_builder::{module_path_for, GraphBuilder, Lang};
use aether_graph::{EdgeKind, GraphDiff, GraphReplica, Node, NodeId, NodeKind, SemanticGraph};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
#[cfg(feature = "gui")]
use std::sync::atomic::AtomicBool;
#[cfg(feature = "gui")]
use std::sync::Arc;

const FILE_PROJECTION_VERSION: &str = "file-v1";
const MAX_PROJECTED_FILES: usize = 4096;
const MAX_PROJECTED_FILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_PROJECTION_CONFLICTS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CollaborationFileChangeKind {
    Added,
    Modified,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CollaborationFileChange {
    pub(crate) path: String,
    pub(crate) kind: CollaborationFileChangeKind,
}

pub(crate) struct CollaborationProjectionPlan {
    writes: Vec<ProjectWrite>,
    pub(crate) files: Vec<CollaborationFileChange>,
    pub(crate) semantic: GraphDiff,
    pub(crate) conflicts: Vec<String>,
}

#[cfg(feature = "gui")]
pub(crate) struct CollaborationProjectionReview {
    pub(crate) text: String,
    pub(crate) approval_digest: Option<String>,
}

impl CollaborationProjectionPlan {
    pub(crate) fn can_apply(&self) -> bool {
        self.conflicts.is_empty()
    }

    pub(crate) fn writes(&self) -> &[ProjectWrite] {
        &self.writes
    }

    pub(crate) fn approval_digest(&self) -> Option<String> {
        if !self.can_apply() {
            return None;
        }
        let mut digest = Sha256::new();
        digest.update(b"BITCODE_COLLABORATION_PROJECTION_V1");
        for write in &self.writes {
            digest_field(&mut digest, write.relative().to_string_lossy().as_bytes());
            digest_optional(&mut digest, write.expected());
            digest_optional(&mut digest, write.contents());
        }
        Some(format!("{:x}", digest.finalize()))
    }

    pub(crate) fn commit(self, root: &Path) -> std::io::Result<Vec<PathBuf>> {
        if !self.conflicts.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "collaboration projection has unresolved conflicts",
            ));
        }
        commit_project_writes(root, self.writes)?;
        Ok(self
            .files
            .into_iter()
            .map(|change| PathBuf::from(change.path))
            .collect())
    }
}

pub(crate) fn load_collaboration_projection(
    root: &Path,
    bundle: &Path,
) -> std::io::Result<CollaborationProjectionPlan> {
    let config = ProjectConfig::load(root)?;
    let (local, graph_expected, _) = load_reconciled_graph(root, &config)?;
    let remote = GraphReplica::load(bundle)
        .and_then(|replica| replica.materialize())
        .map_err(std::io::Error::other)?;
    plan_collaboration_projection(root, &config, &local, &remote, graph_expected)
}

#[cfg(feature = "gui")]
pub(crate) fn review_collaboration_projection(
    root: &Path,
    bundle: &Path,
) -> std::io::Result<CollaborationProjectionReview> {
    let plan = load_collaboration_projection(root, bundle)?;
    Ok(CollaborationProjectionReview {
        text: render_projection_review(&plan),
        approval_digest: plan.approval_digest(),
    })
}

#[cfg(feature = "gui")]
pub(crate) fn apply_reviewed_collaboration_projection(
    root: &Path,
    bundle: &Path,
    expected_digest: &str,
) -> std::io::Result<String> {
    let config = ProjectConfig::load(root)?;
    let plan = load_collaboration_projection(root, bundle)?;
    let actual_digest = plan.approval_digest().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "collaboration projection has unresolved conflicts",
        )
    })?;
    if actual_digest != expected_digest {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "collaboration bundle or project baseline changed after review; review it again",
        ));
    }
    let validation = validate_candidate(
        root,
        &config,
        plan.writes(),
        &Arc::new(AtomicBool::new(false)),
    )?;
    if !validation.passed() {
        return Err(std::io::Error::other(format!(
            "{}; project was not modified",
            validation.summary()
        )));
    }
    let files = plan.commit(root)?;
    Ok(format!(
        "Applied {} reviewed source projection(s) and committed the semantic graph. {}",
        files.len(),
        validation.summary()
    ))
}

#[cfg(feature = "gui")]
pub(crate) fn render_projection_review(plan: &CollaborationProjectionPlan) -> String {
    let mut lines = vec![
        "Collaboration source-projection review".to_string(),
        format!(
            "semantic nodes: +{} ~{} -{}; edges: +{} -{}",
            plan.semantic.added.len(),
            plan.semantic.modified.len(),
            plan.semantic.removed.len(),
            plan.semantic.added_edges.len(),
            plan.semantic.removed_edges.len()
        ),
        format!("source files ({}):", plan.files.len()),
    ];
    if plan.files.is_empty() {
        lines.push("  none".into());
    } else {
        for change in &plan.files {
            let marker = match change.kind {
                CollaborationFileChangeKind::Added => "+",
                CollaborationFileChangeKind::Modified => "~",
                CollaborationFileChangeKind::Removed => "-",
            };
            lines.push(format!("  {marker} {}", change.path));
        }
    }
    if plan.conflicts.is_empty() {
        lines.push("conflicts: none".into());
    } else {
        lines.push(format!("conflicts ({}):", plan.conflicts.len()));
        for conflict in &plan.conflicts {
            lines.push(format!("  ! {conflict}"));
        }
    }
    lines.join("\n")
}

pub(crate) fn plan_collaboration_projection(
    root: &Path,
    config: &ProjectConfig,
    local: &SemanticGraph,
    remote: &SemanticGraph,
    graph_expected: Option<Vec<u8>>,
) -> std::io::Result<CollaborationProjectionPlan> {
    let semantic = remote.diff_from(local);
    let mut conflicts = Vec::new();
    let local_modules = file_modules(local, "local graph", &mut conflicts);
    let remote_modules = file_modules(remote, "collaboration bundle", &mut conflicts);
    let all_files: BTreeSet<_> = local_modules
        .keys()
        .chain(remote_modules.keys())
        .cloned()
        .collect();
    if all_files.len() > MAX_PROJECTED_FILES {
        conflicts.push(format!(
            "projection contains {} files; limit is {MAX_PROJECTED_FILES}",
            all_files.len()
        ));
    }

    let mut files = Vec::new();
    let mut writes = Vec::new();
    for file in &all_files {
        let local_module = local_modules.get(file).copied();
        let remote_module = remote_modules.get(file).copied();
        validate_module(file, local_module, config, "local graph", &mut conflicts)?;
        validate_module(
            file,
            remote_module,
            config,
            "collaboration bundle",
            &mut conflicts,
        )?;

        let current = read_project_bytes(root, file)?;
        if let Some(module) = local_module {
            if current.as_deref() != Some(module.source.as_bytes()) {
                conflicts.push(format!(
                    "{file} no longer matches the local semantic-graph baseline"
                ));
            }
        } else if current.is_some() {
            conflicts.push(format!(
                "{file} exists on disk but has no local module baseline"
            ));
        }

        match (local_module, remote_module) {
            (None, Some(remote)) => {
                files.push(CollaborationFileChange {
                    path: file.clone(),
                    kind: CollaborationFileChangeKind::Added,
                });
                writes.push(ProjectWrite::text(file, current, remote.source.clone()));
            }
            (Some(_), None) => {
                files.push(CollaborationFileChange {
                    path: file.clone(),
                    kind: CollaborationFileChangeKind::Removed,
                });
                if let Some(expected) = current {
                    writes.push(ProjectWrite::delete(file, expected));
                }
            }
            (Some(local), Some(remote)) if local.source != remote.source => {
                files.push(CollaborationFileChange {
                    path: file.clone(),
                    kind: CollaborationFileChangeKind::Modified,
                });
                writes.push(ProjectWrite::text(file, current, remote.source.clone()));
            }
            _ => {}
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));

    let projected = rebuild_projected_graph(&remote_modules, &mut conflicts);
    compare_projected_model(remote, &projected, &remote_modules, &mut conflicts);
    if conflicts.len() > MAX_PROJECTION_CONFLICTS {
        let omitted = conflicts.len() - MAX_PROJECTION_CONFLICTS;
        conflicts.truncate(MAX_PROJECTION_CONFLICTS);
        conflicts.push(format!("{omitted} additional conflict(s) omitted"));
    }
    if conflicts.is_empty() {
        let (reconciled, _) = SemanticGraph::reconcile_persisted(projected, remote);
        writes.push(graph_project_write(
            root,
            config,
            &reconciled,
            graph_expected,
        )?);
    } else {
        writes.clear();
    }

    Ok(CollaborationProjectionPlan {
        writes,
        files,
        semantic,
        conflicts,
    })
}

fn file_modules<'a>(
    graph: &'a SemanticGraph,
    label: &str,
    conflicts: &mut Vec<String>,
) -> BTreeMap<String, &'a Node> {
    let mut modules = BTreeMap::new();
    for node in graph.query_by_kind(NodeKind::Module) {
        let Some(file) = &node.file else {
            continue;
        };
        if modules.insert(file.clone(), node).is_some() {
            conflicts.push(format!("{label} contains multiple modules for {file}"));
        }
    }
    modules
}

fn validate_module(
    file: &str,
    module: Option<&Node>,
    config: &ProjectConfig,
    label: &str,
    conflicts: &mut Vec<String>,
) -> std::io::Result<()> {
    let Some(module) = module else {
        return Ok(());
    };
    if !is_configured_source_path(config, file)? {
        conflicts.push(format!(
            "{label} projects unsupported or out-of-scope path {file}"
        ));
    }
    if module.attr("source_projection") != Some(FILE_PROJECTION_VERSION) {
        conflicts.push(format!(
            "{label} module {} lacks {FILE_PROJECTION_VERSION} whole-file source",
            module.path
        ));
    }
    if module.path != module_path_for(file) {
        conflicts.push(format!(
            "{label} module path {} does not match file {file}",
            module.path
        ));
    }
    let language = Lang::from_path(file).map(Lang::name);
    if language != Some(module.language.as_str()) {
        conflicts.push(format!(
            "{label} module {} has language '{}' inconsistent with {file}",
            module.path, module.language
        ));
    }
    if module.source.len() > MAX_PROJECTED_FILE_BYTES {
        conflicts.push(format!(
            "{label} source for {file} is {} bytes; limit is {MAX_PROJECTED_FILE_BYTES}",
            module.source.len()
        ));
    }
    Ok(())
}

fn rebuild_projected_graph(
    modules: &BTreeMap<String, &Node>,
    conflicts: &mut Vec<String>,
) -> SemanticGraph {
    let mut graph = SemanticGraph::new();
    let mut builder = GraphBuilder::new();
    for (file, module) in modules {
        if module.attr("source_projection") == Some(FILE_PROJECTION_VERSION)
            && module.source.len() <= MAX_PROJECTED_FILE_BYTES
            && Lang::from_path(file).is_some()
        {
            builder.load_file(&mut graph, file, &module.source);
        } else {
            conflicts.push(format!("cannot rebuild invalid projected module {file}"));
        }
    }
    graph
}

fn compare_projected_model(
    remote: &SemanticGraph,
    projected: &SemanticGraph,
    modules: &BTreeMap<String, &Node>,
    conflicts: &mut Vec<String>,
) {
    let files: BTreeSet<_> = modules.keys().map(String::as_str).collect();
    for node in remote.nodes().filter(|node| is_source_kind(node.kind)) {
        if let Some(file) = node.file.as_deref() {
            if !files.contains(file) {
                conflicts.push(format!(
                    "remote source node {} has no whole-file module for {file}",
                    node.path
                ));
            }
        }
    }
    let remote_nodes: BTreeMap<_, _> = remote
        .nodes()
        .filter(|node| {
            node.file
                .as_deref()
                .is_some_and(|file| files.contains(file))
        })
        .filter(|node| is_source_kind(node.kind))
        .map(|node| (node.id, node))
        .collect();
    let projected_nodes: BTreeMap<_, _> = projected
        .nodes()
        .filter(|node| is_source_kind(node.kind))
        .map(|node| (node.id, node))
        .collect();

    for (id, remote_node) in &remote_nodes {
        match projected_nodes.get(id) {
            Some(projected_node) if same_projection(remote_node, projected_node) => {}
            Some(_) => conflicts.push(format!(
                "whole-file source does not reproduce remote node {}",
                remote_node.path
            )),
            None => conflicts.push(format!(
                "whole-file source is missing remote node {}",
                remote_node.path
            )),
        }
    }
    for (id, projected_node) in &projected_nodes {
        if !remote_nodes.contains_key(id) {
            conflicts.push(format!(
                "whole-file source produces unrecorded remote node {}",
                projected_node.path
            ));
        }
    }

    let remote_edges = projection_edges(remote, &remote_nodes);
    let projected_edges = projection_edges(projected, &projected_nodes);
    for edge in remote_edges.difference(&projected_edges) {
        conflicts.push(format!("whole-file source is missing remote edge {edge:?}"));
    }
    for edge in projected_edges.difference(&remote_edges) {
        conflicts.push(format!(
            "whole-file source produces unrecorded edge {edge:?}"
        ));
    }
}

fn projection_edges(
    graph: &SemanticGraph,
    nodes: &BTreeMap<NodeId, &Node>,
) -> BTreeSet<(NodeId, NodeId, EdgeKind)> {
    graph
        .edge_records()
        .into_iter()
        .filter(|(from, to, edge)| {
            nodes.contains_key(from) && nodes.contains_key(to) && edge.kind.is_projection_derived()
        })
        .map(|(from, to, edge)| (from, to, edge.kind))
        .collect()
}

fn is_source_kind(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Module | NodeKind::Function | NodeKind::Type | NodeKind::Field
    )
}

fn same_projection(left: &Node, right: &Node) -> bool {
    left.id == right.id
        && left.kind == right.kind
        && left.path == right.path
        && left.language == right.language
        && left.file == right.file
        && left.span == right.span
        && left.source == right.source
}

fn digest_field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(bytes);
}

fn digest_optional(digest: &mut Sha256, bytes: Option<&[u8]>) {
    match bytes {
        Some(bytes) => {
            digest.update([1]);
            digest_field(digest, bytes);
        }
        None => digest.update([0]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_graph::Node;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "bitcode-collaboration-projection-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, relative: &str, source: &str) {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, source).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn graph(files: &[(&str, &str)]) -> SemanticGraph {
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        for (path, source) in files {
            builder.load_file(&mut graph, path, source);
        }
        graph
    }

    #[test]
    fn whole_file_projection_adds_modifies_deletes_and_preserves_metadata() {
        let root = TempDir::new();
        root.write("src/lib.rs", "pub fn value() -> i64 { 1 }\n");
        root.write("src/removed.rs", "pub fn removed() {}\n");
        let local = graph(&[
            ("src/lib.rs", "pub fn value() -> i64 { 1 }\n"),
            ("src/removed.rs", "pub fn removed() {}\n"),
        ]);
        let mut remote = graph(&[
            ("src/lib.rs", "pub fn value() -> i64 { 2 }\n"),
            ("src/added.rs", "pub fn added() {}\n"),
        ]);
        remote.upsert_node(Node::new(
            NodeKind::Concept,
            "reviewed",
            "concept::reviewed",
        ));

        let plan = plan_collaboration_projection(
            &root.0,
            &ProjectConfig::default(),
            &local,
            &remote,
            None,
        )
        .unwrap();
        assert!(plan.can_apply(), "{:?}", plan.conflicts);
        assert_eq!(plan.files.len(), 3);
        plan.commit(&root.0).unwrap();

        assert_eq!(
            std::fs::read_to_string(root.0.join("src/lib.rs")).unwrap(),
            "pub fn value() -> i64 { 2 }\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.0.join("src/added.rs")).unwrap(),
            "pub fn added() {}\n"
        );
        assert!(!root.0.join("src/removed.rs").exists());
        let persisted = SemanticGraph::load(root.0.join("project.aether")).unwrap();
        assert!(persisted.find_by_path("concept::reviewed").is_some());
    }

    #[test]
    fn inconsistent_remote_file_and_semantic_nodes_are_conflicts() {
        let root = TempDir::new();
        let source = "pub fn value() -> i64 { 1 }\n";
        root.write("src/lib.rs", source);
        let local = graph(&[("src/lib.rs", source)]);
        let mut remote = graph(&[("src/lib.rs", "pub fn value() -> i64 { 2 }\n")]);
        let mut node = remote.find_by_path("crate::lib::value").cloned().unwrap();
        node.source = "pub fn value() -> i64 { 999 }".into();
        remote.upsert_node(node);

        let plan = plan_collaboration_projection(
            &root.0,
            &ProjectConfig::default(),
            &local,
            &remote,
            None,
        )
        .unwrap();
        assert!(!plan.can_apply());
        assert!(plan
            .conflicts
            .iter()
            .any(|conflict| conflict.contains("does not reproduce remote node")));
        assert!(plan.commit(&root.0).is_err());
        assert_eq!(
            std::fs::read_to_string(root.0.join("src/lib.rs")).unwrap(),
            source
        );
    }

    #[test]
    fn stale_local_files_block_the_entire_projection_transaction() {
        let root = TempDir::new();
        root.write("src/a.rs", "pub fn a() -> i64 { 1 }\n");
        root.write("src/b.rs", "pub fn b() -> i64 { 1 }\n");
        let local = graph(&[
            ("src/a.rs", "pub fn a() -> i64 { 1 }\n"),
            ("src/b.rs", "pub fn b() -> i64 { 1 }\n"),
        ]);
        let remote = graph(&[
            ("src/a.rs", "pub fn a() -> i64 { 2 }\n"),
            ("src/b.rs", "pub fn b() -> i64 { 2 }\n"),
        ]);
        let plan = plan_collaboration_projection(
            &root.0,
            &ProjectConfig::default(),
            &local,
            &remote,
            None,
        )
        .unwrap();
        assert!(plan.can_apply());

        root.write("src/b.rs", "pub fn b() -> i64 { 3 }\n");
        assert_eq!(
            plan.commit(&root.0).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(
            std::fs::read_to_string(root.0.join("src/a.rs")).unwrap(),
            "pub fn a() -> i64 { 1 }\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.0.join("src/b.rs")).unwrap(),
            "pub fn b() -> i64 { 3 }\n"
        );
    }

    #[test]
    fn approval_digest_binds_remote_bytes_and_local_baselines() {
        let root = TempDir::new();
        root.write("src/lib.rs", "pub fn value() -> i64 { 1 }\n");
        let local = graph(&[("src/lib.rs", "pub fn value() -> i64 { 1 }\n")]);
        let remote_two = graph(&[("src/lib.rs", "pub fn value() -> i64 { 2 }\n")]);
        let remote_three = graph(&[("src/lib.rs", "pub fn value() -> i64 { 3 }\n")]);
        let first = plan_collaboration_projection(
            &root.0,
            &ProjectConfig::default(),
            &local,
            &remote_two,
            None,
        )
        .unwrap()
        .approval_digest()
        .unwrap();
        let second = plan_collaboration_projection(
            &root.0,
            &ProjectConfig::default(),
            &local,
            &remote_three,
            None,
        )
        .unwrap()
        .approval_digest()
        .unwrap();
        assert_ne!(first, second);

        root.write("src/lib.rs", "pub fn value() -> i64 { 4 }\n");
        let stale = plan_collaboration_projection(
            &root.0,
            &ProjectConfig::default(),
            &local,
            &remote_two,
            None,
        )
        .unwrap();
        assert!(!stale.can_apply());
        assert_eq!(stale.approval_digest(), None);
    }

    #[test]
    fn remote_source_nodes_without_file_modules_are_conflicts() {
        let root = TempDir::new();
        root.write("src/lib.rs", "pub fn value() -> i64 { 1 }\n");
        let local = graph(&[("src/lib.rs", "pub fn value() -> i64 { 1 }\n")]);
        let mut remote = local.clone();
        remote.remove_node(NodeId::from_path("crate::lib"));
        let plan = plan_collaboration_projection(
            &root.0,
            &ProjectConfig::default(),
            &local,
            &remote,
            None,
        )
        .unwrap();
        assert!(!plan.can_apply());
        assert!(plan
            .conflicts
            .iter()
            .any(|conflict| conflict.contains("has no whole-file module")));
    }

    #[test]
    fn path_escape_in_remote_module_is_rejected() {
        let root = TempDir::new();
        let local = SemanticGraph::new();
        let mut remote = SemanticGraph::new();
        let mut module = Node::new(NodeKind::Module, "escape", "crate::escape")
            .with_language("rust")
            .with_source("pub fn escape() {}\n");
        module.file = Some("../escape.rs".into());
        module.set_attr("source_projection", FILE_PROJECTION_VERSION);
        remote.upsert_node(module);

        assert!(plan_collaboration_projection(
            &root.0,
            &ProjectConfig::default(),
            &local,
            &remote,
            None,
        )
        .is_err());
    }
}
