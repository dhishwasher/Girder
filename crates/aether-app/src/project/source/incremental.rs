//! Cached project loading using the same source scope and durable reconciliation
//! as cold analysis. A candidate update never changes the published instance.

use super::{
    build_from_dir_with_config, collect_sources_with_config, load_graph_snapshot,
    read_optional_bytes, ProjectConfig,
};
use aether_builder::{FileChange, FullRebuildReason, GraphBuilder, UpdateReport};
use aether_graph::SemanticGraph;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Clone)]
pub(crate) struct CachedProject {
    pub(crate) root: PathBuf,
    pub(crate) config: ProjectConfig,
    pub(crate) graph: SemanticGraph,
    pub(crate) builder: GraphBuilder,
    pub(crate) persisted_bytes: Option<Vec<u8>>,
    metadata: BTreeMap<String, Option<Vec<u8>>>,
}

fn metadata(root: &Path) -> std::io::Result<BTreeMap<String, Option<Vec<u8>>>> {
    ["girder.toml", "Cargo.toml", "go.mod"]
        .into_iter()
        .map(|name| Ok((name.to_string(), read_optional_bytes(&root.join(name))?)))
        .collect()
}

fn normalize_event(path: &str) -> std::io::Result<String> {
    let path = path.replace('\\', "/");
    let parts: Vec<_> = path
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if path.starts_with('/') || path.contains(':') || path.contains('\0') || parts.contains(&"..") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "event path must stay inside the project",
        ));
    }
    Ok(parts.join("/"))
}

impl CachedProject {
    pub(crate) fn open(root: &Path) -> std::io::Result<Self> {
        let root = root.canonicalize()?;
        let config = ProjectConfig::load(&root)?;
        let metadata = metadata(&root)?;
        let (source, builder, _) = build_from_dir_with_config(&root, &config)?;
        let persisted = load_graph_snapshot(&root, &config)?;
        if let Some(error) = persisted.error {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, error));
        }
        let graph = match persisted.graph {
            Some(durable) => SemanticGraph::reconcile_persisted(source, &durable).0,
            None => source,
        };
        Ok(Self {
            root,
            config,
            graph,
            builder,
            persisted_bytes: persisted.bytes,
            metadata,
        })
    }

    /// Re-read ownership/configuration and build a complete candidate generation.
    /// The caller publishes it only after any required filesystem validation and
    /// persistence. Unexpected inventory changes force full source reconciliation.
    pub(crate) fn updated(
        &self,
        dirty_paths: &[String],
        structural_reasons: &[FullRebuildReason],
    ) -> std::io::Result<(Self, UpdateReport)> {
        let started = Instant::now();
        let events: BTreeSet<_> = dirty_paths
            .iter()
            .map(|p| normalize_event(p))
            .collect::<Result<_, _>>()?;
        let config = ProjectConfig::load(&self.root)?;
        let metadata = metadata(&self.root)?;
        let sources = collect_sources_with_config(&self.root, &config)?;
        let before: BTreeSet<_> = self.builder.source_files().into_iter().collect();
        let after: BTreeSet<_> = sources.iter().map(|(_, p)| p.clone()).collect();
        let dirty: BTreeSet<_> = events
            .iter()
            .filter(|p| before.contains(*p) || after.contains(*p))
            .cloned()
            .collect();
        let mut reasons: BTreeSet<_> = structural_reasons.iter().copied().collect();
        for event in &events {
            if !before.contains(event)
                && !after.contains(event)
                && super::is_configured_source_path(&config, event)?
            {
                reasons.insert(FullRebuildReason::UncertainEventMapping);
            }
        }
        if self.metadata != metadata {
            reasons.insert(FullRebuildReason::ConfigurationChanged);
        }
        if before
            .symmetric_difference(&after)
            .any(|path| !dirty.contains(path))
        {
            reasons.insert(FullRebuildReason::OwnershipChanged);
        }
        if !self.builder.cache_is_consistent() {
            reasons.insert(FullRebuildReason::CacheInconsistent);
        }
        if dirty.len().saturating_mul(2) > before.len().max(after.len()) {
            reasons.insert(FullRebuildReason::DirtySetExceedsHalf);
        }
        let persisted = load_graph_snapshot(&self.root, &config)?;
        if let Some(error) = persisted.error {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, error));
        }
        if persisted.bytes != self.persisted_bytes {
            // Another graph writer may have changed graph-owned metadata. Cold
            // reconciliation starts from clean source, never stale merged data.
            reasons.insert(FullRebuildReason::CacheInconsistent);
        }
        let collect_seconds = started.elapsed().as_secs_f64();
        if !reasons.is_empty() {
            let rebuild_started = Instant::now();
            let (source, builder, _) = build_from_dir_with_config(&self.root, &config)?;
            let rebuild_seconds = rebuild_started.elapsed().as_secs_f64();
            let reconcile_started = Instant::now();
            let graph = match persisted.graph {
                Some(durable) => SemanticGraph::reconcile_persisted(source, &durable).0,
                None => source,
            };
            let report = UpdateReport {
                dirty_files: dirty.into_iter().collect(),
                parsed_files: after.iter().cloned().collect(),
                reused_files: Vec::new(),
                invalidation_scope: before.union(&after).cloned().collect(),
                invalidated_source_nodes: self.graph.nodes().filter(|n| n.file.is_some()).count(),
                invalidated_incident_edges: self
                    .graph
                    .edge_records()
                    .iter()
                    .filter(|(from, to, _)| {
                        self.graph.get(*from).is_some_and(|n| n.file.is_some())
                            || self.graph.get(*to).is_some_and(|n| n.file.is_some())
                    })
                    .count(),
                full_rebuild_reasons: reasons.into_iter().collect(),
                collect_seconds,
                // Cold loader combines extraction, reconstruction and resolution.
                parse_seconds: 0.0,
                reconstruct_seconds: 0.0,
                resolve_seconds: 0.0,
                full_rebuild_seconds: rebuild_seconds,
                reconcile_seconds: reconcile_started.elapsed().as_secs_f64(),
                elapsed_seconds: started.elapsed().as_secs_f64(),
            };
            return Ok((
                Self {
                    root: self.root.clone(),
                    config,
                    graph,
                    builder,
                    persisted_bytes: persisted.bytes,
                    metadata,
                },
                report,
            ));
        }
        let mut changes = Vec::new();
        for path in &dirty {
            changes.push(if after.contains(path) {
                FileChange::replace(path, std::fs::read_to_string(self.root.join(path))?)
            } else {
                FileChange::delete(path)
            });
        }
        let mut next = self.clone();
        let mut report = next
            .builder
            .update_files(&mut next.graph, &changes, &[])
            .map_err(std::io::Error::other)?;
        next.config = config;
        next.metadata = metadata;
        next.persisted_bytes = persisted.bytes;
        report.collect_seconds += collect_seconds;
        report.elapsed_seconds = started.elapsed().as_secs_f64();
        Ok((next, report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_graph::{Edge, EdgeKind, Node, NodeId, NodeKind};
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct FixtureRoot(PathBuf);

    impl FixtureRoot {
        fn new(files: &Value) -> Self {
            let path = std::env::temp_dir().join(format!(
                "girder-incremental-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            for (relative, source) in files.as_object().unwrap() {
                let file = path.join(relative);
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(file, source.as_str().unwrap()).unwrap();
            }
            Self(path)
        }

        fn mutate(&self, step: &Value) -> Vec<String> {
            step["changes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|change| {
                    let relative = change["path"].as_str().unwrap();
                    let path = self.0.join(relative);
                    if let Some(source) = change["source"].as_str() {
                        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                        std::fs::write(path, source).unwrap();
                    } else {
                        std::fs::remove_file(path).unwrap();
                    }
                    relative.to_string()
                })
                .collect()
        }
    }

    impl Drop for FixtureRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn corpus() -> Value {
        serde_json::from_str(include_str!(
            "../../../../../docs/incremental-mutation-corpus.json"
        ))
        .unwrap()
    }

    fn canonical(graph: &SemanticGraph) -> Value {
        let mut nodes: Vec<_> = graph.nodes().cloned().collect();
        nodes.sort_by_key(|node| node.id);
        let mut edges = graph.edge_records();
        edges.sort_by_key(|(from, to, edge)| (*from, *to, edge.kind));
        json!({"nodes":nodes,"edges":edges})
    }

    fn assert_cold(candidate: &CachedProject) {
        let (cold, _, _) =
            super::super::load_reconciled_graph(&candidate.root, &candidate.config).unwrap();
        assert_eq!(canonical(&candidate.graph), canonical(&cold));
    }

    #[test]
    fn incremental_project_configuration_and_ownership_match_cold() {
        let corpus = corpus();
        for fixture in corpus["project_cases"].as_array().unwrap().iter().take(3) {
            let root = FixtureRoot::new(&fixture["files"]);
            let mut cached = CachedProject::open(&root.0).unwrap();
            for step in fixture["mutations"].as_array().unwrap() {
                let paths = root.mutate(step);
                let (next, report) = cached.updated(&paths, &[]).unwrap();
                assert!(
                    report
                        .full_rebuild_reasons
                        .contains(&FullRebuildReason::ConfigurationChanged),
                    "{}",
                    fixture["id"]
                );
                assert!(report.reused_files.is_empty());
                assert_eq!(report.parsed_files, next.builder.source_files());
                assert_cold(&next);
                cached = next;
            }
        }
    }

    #[test]
    fn incremental_project_facade_reuses_two_files_and_keeps_old_generation_private() {
        let corpus = corpus();
        let fixture = &corpus["fixtures"][0];
        let root = FixtureRoot::new(&fixture["files"]);
        let cached = CachedProject::open(&root.0).unwrap();
        let previous = canonical(&cached.graph);
        let paths = root.mutate(&fixture["mutations"][0]);
        let (next, report) = cached.updated(&paths, &[]).unwrap();
        assert_eq!(report.parsed_files, ["src/math.rs"]);
        assert_eq!(report.reused_files.len(), 2);
        assert!(report.full_rebuild_reasons.is_empty());
        assert_eq!(canonical(&cached.graph), previous);
        assert_cold(&next);
    }

    #[test]
    fn incremental_project_preserves_exact_durable_reconciliation() {
        let corpus = corpus();
        let fixture = corpus["project_cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == "persisted-reconciliation")
            .unwrap();
        let root = FixtureRoot::new(&fixture["files"]);
        let mut initial = CachedProject::open(&root.0).unwrap();
        let setup = &fixture["durable_setup"];
        let app = NodeId::from_path(setup["metadata_on"].as_str().unwrap());
        initial
            .graph
            .get_mut(app)
            .unwrap()
            .set_attr("intent", setup["intent"].as_str().unwrap());
        let note = Node::new(
            NodeKind::Concept,
            "manual_note",
            setup["graph_owned_node"].as_str().unwrap(),
        );
        let note_id = note.id;
        initial.graph.upsert_node(note);
        initial
            .graph
            .add_edge(note_id, app, Edge::with_weight(EdgeKind::Impacts, 0.7))
            .unwrap();
        super::super::save_graph(&root.0, &initial.config, &initial.graph).unwrap();
        let cached = CachedProject::open(&root.0).unwrap();
        let paths = root.mutate(&fixture["mutations"][0]);
        let (next, _) = cached.updated(&paths, &[]).unwrap();
        assert_cold(&next);
        assert_eq!(
            next.graph.get(app).unwrap().attr("intent"),
            Some("durable user intent")
        );
        assert!(next.graph.contains(note_id));
        assert!(!next.graph.contains(NodeId::from_path("crate::math::add")));
        assert!(next.graph.edge_records().contains(&(
            note_id,
            app,
            Edge::with_weight(EdgeKind::Impacts, 0.7)
        )));
    }

    #[test]
    fn incremental_project_reconciles_unreported_inventory_and_external_graph_changes() {
        let corpus = corpus();
        let fixture = &corpus["fixtures"][0];
        let root = FixtureRoot::new(&fixture["files"]);
        let cached = CachedProject::open(&root.0).unwrap();
        std::fs::write(root.0.join("src/new.rs"), "pub fn new_function() {}\n").unwrap();
        let (next, report) = cached.updated(&[], &[]).unwrap();
        assert!(report
            .full_rebuild_reasons
            .contains(&FullRebuildReason::OwnershipChanged));
        assert_cold(&next);
        let mut durable = next.graph.clone();
        durable
            .get_mut(NodeId::from_path("crate::app::run"))
            .unwrap()
            .set_attr("intent", "external edit");
        super::super::save_graph(&root.0, &next.config, &durable).unwrap();
        let (reconciled, report) = next.updated(&[], &[]).unwrap();
        assert!(report
            .full_rebuild_reasons
            .contains(&FullRebuildReason::CacheInconsistent));
        assert_cold(&reconciled);
    }
}
