//! Batched parsing reuse with complete project-wide resolution.

use super::{extract, FileState, GraphBuilder, IncrementalParser, Lang};
use aether_graph::{NodeId, SemanticGraph};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::Instant;

/// A create or replacement contains source; a deletion contains `None`.
/// Represent moves with a deletion and creation in the same batch.
#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: String,
    pub source: Option<String>,
}

impl FileChange {
    pub fn replace(path: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            source: Some(source.into()),
        }
    }

    pub fn delete(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            source: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FullRebuildReason {
    OwnershipChanged,
    ConfigurationChanged,
    UncertainEventMapping,
    CacheInconsistent,
    DirtySetExceedsHalf,
}

impl FullRebuildReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OwnershipChanged => "ownership_changed",
            Self::ConfigurationChanged => "configuration_changed",
            Self::UncertainEventMapping => "uncertain_event_mapping",
            Self::CacheInconsistent => "cache_inconsistent",
            Self::DirtySetExceedsHalf => "dirty_set_exceeds_half",
        }
    }
}

#[derive(Debug, Clone)]
pub struct UpdateReport {
    pub dirty_files: Vec<String>,
    pub parsed_files: Vec<String>,
    pub reused_files: Vec<String>,
    /// Every previous and current owned source file is invalidated for resolution.
    pub invalidation_scope: Vec<String>,
    pub invalidated_source_nodes: usize,
    pub invalidated_incident_edges: usize,
    pub full_rebuild_reasons: Vec<FullRebuildReason>,
    pub collect_seconds: f64,
    pub parse_seconds: f64,
    pub reconstruct_seconds: f64,
    pub resolve_seconds: f64,
    pub reconcile_seconds: f64,
    /// Combined cold-loader duration when individual fallback phases are unavailable.
    pub full_rebuild_seconds: f64,
    pub elapsed_seconds: f64,
}

#[derive(Debug, thiserror::Error)]
#[error("expected a supported project-relative source path, got {0:?}")]
pub struct UpdateError(String);

/// Normalize both platform separators. Reject parent traversal even if it
/// could be collapsed to an in-root path, and reject Windows prefixes on Unix.
pub fn normalize_source_path(path: &str) -> Result<String, UpdateError> {
    let normalized = path.replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains(':') || normalized.contains('\0') {
        return Err(UpdateError(path.into()));
    }
    let parts: Vec<_> = normalized
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if parts.is_empty() || parts.contains(&"..") {
        return Err(UpdateError(path.into()));
    }
    let normalized = parts.join("/");
    if Lang::from_path(&normalized).is_none() {
        return Err(UpdateError(path.into()));
    }
    Ok(normalized)
}

impl GraphBuilder {
    pub fn source_files(&self) -> Vec<String> {
        let mut paths: Vec<_> = self.files.keys().cloned().collect();
        paths.sort();
        paths
    }

    pub fn cache_is_consistent(&self) -> bool {
        self.files.iter().all(|(file, state)| {
            normalize_source_path(file).is_ok_and(|normalized| normalized == *file)
                && state.source_fingerprint == NodeId::from_path(&state.source)
                && state.owned == state.extraction.node_ids().into_iter().collect()
                && state.paths
                    == state
                        .extraction
                        .nodes
                        .iter()
                        .map(|node| node.path.clone())
                        .collect::<Vec<_>>()
        })
    }

    /// Apply a complete batch privately, then replace `graph` after resolution.
    /// Callers supplying structural reasons must also supply any newly read
    /// source contents: this builder has no filesystem access. Project loading
    /// performs that full reconciliation before calling this interface.
    pub fn update_files(
        &mut self,
        graph: &mut SemanticGraph,
        changes: &[FileChange],
        structural_reasons: &[FullRebuildReason],
    ) -> Result<UpdateReport, UpdateError> {
        let started = Instant::now();

        // Stage 1: validate the entire batch before touching the cache or graph.
        let mut dirty = BTreeMap::new();
        for change in changes {
            dirty.insert(
                normalize_source_path(&change.path)?,
                change.source.as_deref(),
            );
        }
        let before: BTreeSet<_> = self.files.keys().cloned().collect();
        let mut after = before.clone();
        for (path, source) in &dirty {
            if source.is_some() {
                after.insert(path.clone());
            } else {
                after.remove(path);
            }
        }
        let mut reasons: BTreeSet<_> = structural_reasons.iter().copied().collect();
        if self.configuration_changed {
            reasons.insert(FullRebuildReason::ConfigurationChanged);
        }
        if !self.cache_is_consistent() {
            reasons.insert(FullRebuildReason::CacheInconsistent);
        }
        if dirty.len().saturating_mul(2) > before.len().max(after.len()) {
            reasons.insert(FullRebuildReason::DirtySetExceedsHalf);
        }

        // Stage 2: all source-owned nodes and their incident edges are invalid.
        // Reverse-edge evidence alone is insufficient: a clean facade or a new
        // same-named candidate can change another clean file's call resolution.
        let owned: HashSet<_> = self
            .files
            .values()
            .flat_map(|state| state.owned.iter().copied())
            .collect();
        let mut report = UpdateReport {
            dirty_files: dirty.keys().cloned().collect(),
            parsed_files: Vec::new(),
            reused_files: Vec::new(),
            invalidation_scope: before.union(&after).cloned().collect(),
            invalidated_source_nodes: owned.len(),
            invalidated_incident_edges: graph
                .edge_records()
                .iter()
                .filter(|(from, to, _)| owned.contains(from) || owned.contains(to))
                .count(),
            full_rebuild_reasons: reasons.into_iter().collect(),
            collect_seconds: started.elapsed().as_secs_f64(),
            parse_seconds: 0.0,
            reconstruct_seconds: 0.0,
            resolve_seconds: 0.0,
            reconcile_seconds: 0.0,
            full_rebuild_seconds: 0.0,
            elapsed_seconds: 0.0,
        };

        // Stage 3: parse changed files, or every file for a full parsing rebuild.
        // Extract changed files from a fresh syntax tree, exactly as cold loading
        // does. Clean files retain their complete extraction, including evidence
        // not represented by graph edges (imports, receiver hints, candidates).
        let parse_started = Instant::now();
        self.files.retain(|path, _| after.contains(path));
        for path in &after {
            if report.full_rebuild_reasons.is_empty() && !dirty.contains_key(path) {
                report.reused_files.push(path.clone());
                continue;
            }
            let source = dirty
                .get(path)
                .and_then(|source| *source)
                .or_else(|| self.files.get(path).map(|state| state.source.as_str()))
                .expect("every current source has cached or supplied contents");
            let lang = Lang::from_path(path).expect("source paths were validated");
            let mut parser = IncrementalParser::new(lang);
            let tree = parser.parse(source);
            let extraction = extract(&tree, source, path, lang);
            self.files.insert(
                path.clone(),
                FileState {
                    source: source.to_owned(),
                    source_fingerprint: NodeId::from_path(source),
                    owned: extraction.node_ids().into_iter().collect(),
                    paths: extraction
                        .nodes
                        .iter()
                        .map(|node| node.path.clone())
                        .collect(),
                    extraction,
                },
            );
            report.parsed_files.push(path.clone());
        }
        report.parse_seconds = parse_started.elapsed().as_secs_f64();

        let reconstruct_started = Instant::now();
        let mut source_graph = SemanticGraph::new();
        for path in &after {
            self.apply(
                &mut source_graph,
                path,
                &self.files[path].extraction,
                &HashSet::new(),
            );
        }
        report.reconstruct_seconds = reconstruct_started.elapsed().as_secs_f64();
        let resolve_started = Instant::now();
        self.resolve_calls(&mut source_graph);
        report.resolve_seconds = resolve_started.elapsed().as_secs_f64();

        let reconcile_started = Instant::now();
        let (complete, _) = SemanticGraph::reconcile_persisted(source_graph, graph);
        report.reconcile_seconds = reconcile_started.elapsed().as_secs_f64();
        *graph = complete;
        self.configuration_changed = false;
        report.elapsed_seconds = started.elapsed().as_secs_f64();
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inconsistent_extraction_cache_forces_full_parsing() {
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(
            &mut graph,
            [
                ("a.rs", "fn a() {}"),
                ("b.rs", "fn b() {}"),
                ("c.rs", "fn c() {}"),
            ],
        );
        let previous_nodes: BTreeSet<_> = graph.nodes().map(|node| node.id).collect();
        builder
            .files
            .get_mut("a.rs")
            .unwrap()
            .extraction
            .nodes
            .clear();
        assert!(!builder.cache_is_consistent());
        let report = builder.update_files(&mut graph, &[], &[]).unwrap();
        assert_eq!(
            report.full_rebuild_reasons,
            [FullRebuildReason::CacheInconsistent]
        );
        assert_eq!(report.parsed_files.len(), 3);
        assert!(report.reused_files.is_empty());
        assert!(builder.cache_is_consistent());
        assert_eq!(
            graph.nodes().map(|node| node.id).collect::<BTreeSet<_>>(),
            previous_nodes
        );
    }

    #[test]
    fn metadata_setters_invalidate_resolution_and_force_full_parsing() {
        let mut graph = SemanticGraph::new();
        let mut builder = GraphBuilder::new();
        builder.load_files(
            &mut graph,
            [
                ("a.go", "package probe\nfunc A() {}"),
                ("b.go", "package probe\nfunc B() {}"),
            ],
        );
        builder.set_go_module_path(Some("example.com/probe".into()));
        let report = builder.update_files(&mut graph, &[], &[]).unwrap();
        assert_eq!(
            report.full_rebuild_reasons,
            [FullRebuildReason::ConfigurationChanged]
        );
        assert_eq!(report.parsed_files.len(), 2);
        let clean = builder.update_files(&mut graph, &[], &[]).unwrap();
        assert!(clean.full_rebuild_reasons.is_empty());
        assert_eq!(clean.reused_files.len(), 2);
    }
}
