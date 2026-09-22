//! Evidence-bearing call claims. Legacy edge weights are never certificates.
//!
//! Evidence lives in parser-owned node metadata so old RON/bincode graphs remain
//! readable without changing their wire layout. Missing, stale or invalid
//! evidence is an explicit Unknown boundary, not an empty call inventory.

use crate::{GraphError, Node, NodeId, NodeKind, SemanticGraph, Span};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

pub const CALL_EVIDENCE_ATTRIBUTE: &str = "call_evidence_v1";

/// Strength of evidence for a call target, not probability of execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallClass {
    Must,
    May,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallClaim {
    /// File-relative source position. Coverage gaps use the enclosing span.
    pub site: Span,
    pub class: CallClass,
    /// Proven viable targets only. Unknown boundaries do not invent targets.
    pub targets: Vec<NodeId>,
    pub reason: String,
    /// Distinguishes a call site from an extractor/file coverage boundary.
    pub coverage_gap: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallEvidence {
    pub version: u32,
    pub source_fingerprint: NodeId,
    pub assumptions: Vec<String>,
    pub calls: Vec<CallClaim>,
}

impl CallEvidence {
    pub fn new(node: &Node, calls: Vec<CallClaim>, assumptions: Vec<String>) -> Self {
        Self {
            version: 1,
            source_fingerprint: NodeId::from_path(&node.source),
            assumptions,
            calls,
        }
    }

    /// Attach to precisely the source snapshot that was examined.
    pub fn attach(&self, node: &mut Node) -> Result<(), GraphError> {
        if self.source_fingerprint != NodeId::from_path(&node.source) {
            return Err(GraphError::Serialize("stale call evidence".into()));
        }
        let value = ron::to_string(self).map_err(|err| GraphError::Serialize(err.to_string()))?;
        node.set_attr(CALL_EVIDENCE_ATTRIBUTE, value);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnknownBoundary {
    pub caller: String,
    pub file: Option<String>,
    pub site: Span,
    pub reason: String,
    pub coverage_gap: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassifiedImpact {
    pub must: Vec<NodeId>,
    pub may: Vec<NodeId>,
    pub unknown: Vec<NodeId>,
    pub boundaries: Vec<UnknownBoundary>,
}

impl ClassifiedImpact {
    /// Test classification retains boundaries even if no tests were extracted.
    pub fn tests(&self, graph: &SemanticGraph) -> Self {
        let select = |ids: &[NodeId]| {
            ids.iter()
                .copied()
                .filter(|id| {
                    graph
                        .get(*id)
                        .is_some_and(|node| node.attr("is_test").is_some())
                })
                .collect()
        };
        Self {
            must: select(&self.must),
            may: select(&self.may),
            unknown: select(&self.unknown),
            boundaries: self.boundaries.clone(),
        }
    }
}

impl SemanticGraph {
    /// Read checked evidence; a reason is returned instead of silently ignoring
    /// malformed metadata or accepting its numeric confidence as certainty.
    pub fn call_evidence(&self, id: NodeId) -> Result<CallEvidence, &'static str> {
        let node = self.get(id).ok_or("missing-node")?;
        let raw = node
            .attr(CALL_EVIDENCE_ATTRIBUTE)
            .ok_or("missing-call-evidence")?;
        let evidence: CallEvidence = ron::from_str(raw).map_err(|_| "invalid-call-evidence")?;
        if evidence.version != 1 {
            return Err("unsupported-call-evidence-version");
        }
        if evidence.source_fingerprint != NodeId::from_path(&node.source) {
            return Err("stale-call-evidence");
        }
        for call in &evidence.calls {
            let unique: HashSet<_> = call.targets.iter().collect();
            if call.reason.is_empty()
                || unique.len() != call.targets.len()
                || call.site.start_byte > call.site.end_byte
                || call.targets.iter().any(|target| !self.contains(*target))
                || match call.class {
                    CallClass::Must => call.targets.len() != 1 || call.coverage_gap,
                    CallClass::May => call.targets.is_empty() || call.coverage_gap,
                    CallClass::Unknown => !call.targets.is_empty(),
                }
            {
                return Err("invalid-call-claim");
            }
        }
        Ok(evidence)
    }

    /// Full, uncapped reverse reachability over independently certified calls.
    ///
    /// A site with unbounded targets might connect otherwise disconnected code.
    /// Until its scope is proved, every remaining callable node is Unknown, also
    /// across languages. Known positive paths retain their strongest class;
    /// their unresolved boundaries remain independently visible.
    pub fn classified_impact(&self, origins: &[NodeId]) -> ClassifiedImpact {
        if origins.is_empty() {
            return ClassifiedImpact::default();
        }
        let mut incoming: HashMap<NodeId, Vec<(NodeId, CallClass)>> = HashMap::new();
        let mut represented = HashSet::new();
        let mut boundaries = Vec::new();
        for node in self
            .nodes()
            .filter(|node| node.kind == NodeKind::Function || node.kind == NodeKind::Module)
        {
            match self.call_evidence(node.id) {
                Ok(evidence) => {
                    for call in evidence.calls {
                        if call.class == CallClass::Unknown {
                            boundaries.push(boundary(
                                node,
                                call.site,
                                &call.reason,
                                call.coverage_gap,
                            ));
                        } else {
                            for target in call.targets {
                                represented.insert((node.id, target));
                                incoming
                                    .entry(target)
                                    .or_default()
                                    .push((node.id, call.class));
                            }
                        }
                    }
                }
                Err(reason) => boundaries.push(boundary(node, node.span, reason, true)),
            }
        }
        // Legacy/generated/inferred edges can contribute uncertain paths but
        // cannot acquire Must merely by having weight 1.0. Also expose them as
        // boundaries when no call-site evidence supports that relationship.
        for (from, to, edge) in self.edge_records() {
            if edge.kind.propagates_impact() && !represented.contains(&(from, to)) {
                incoming
                    .entry(to)
                    .or_default()
                    .push((from, CallClass::Unknown));
                if let Some(node) = self.get(from) {
                    boundaries.push(boundary(
                        node,
                        node.span,
                        &format!("uncertified-{:?}-edge", edge.kind),
                        true,
                    ));
                }
            }
        }
        let mut best = HashMap::new();
        let mut queue = VecDeque::new();
        for &origin in origins {
            if self.contains(origin) {
                best.insert(origin, CallClass::Must);
                queue.push_back((origin, CallClass::Must));
            } else {
                boundaries.push(UnknownBoundary {
                    caller: format!("missing-node:{:016x}", origin.0),
                    file: None,
                    site: Span::default(),
                    reason: "missing-origin".into(),
                    coverage_gap: true,
                });
            }
        }
        while let Some((target, path_class)) = queue.pop_front() {
            for &(caller, edge_class) in incoming.get(&target).into_iter().flatten() {
                let class = path_class.max(edge_class);
                if best.get(&caller).is_none_or(|existing| class < *existing) {
                    best.insert(caller, class);
                    queue.push_back((caller, class));
                }
            }
        }
        if !boundaries.is_empty() {
            for node in self.nodes().filter(|node| node.kind == NodeKind::Function) {
                best.entry(node.id).or_insert(CallClass::Unknown);
            }
        }
        let mut result = ClassifiedImpact {
            boundaries,
            ..Default::default()
        };
        for (id, class) in best {
            match class {
                CallClass::Must => result.must.push(id),
                CallClass::May => result.may.push(id),
                CallClass::Unknown => result.unknown.push(id),
            }
        }
        for ids in [&mut result.must, &mut result.may, &mut result.unknown] {
            ids.sort_by(|a, b| self.get(*a).unwrap().path.cmp(&self.get(*b).unwrap().path));
        }
        result.boundaries.sort_by(|a, b| {
            (
                &a.file,
                a.site.start_byte,
                a.site.end_byte,
                &a.caller,
                &a.reason,
                a.coverage_gap,
            )
                .cmp(&(
                    &b.file,
                    b.site.start_byte,
                    b.site.end_byte,
                    &b.caller,
                    &b.reason,
                    b.coverage_gap,
                ))
        });
        result.boundaries.dedup();
        result
    }
}

fn boundary(node: &Node, site: Span, reason: &str, coverage_gap: bool) -> UnknownBoundary {
    UnknownBoundary {
        caller: node.path.clone(),
        file: node.file.clone(),
        site,
        reason: reason.into(),
        coverage_gap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, EdgeKind};

    fn add(graph: &mut SemanticGraph, path: &str, is_test: bool) -> NodeId {
        let mut node = Node::new(NodeKind::Function, path, path).with_source("body");
        if is_test {
            node.set_attr("is_test", "true");
        }
        CallEvidence::new(&node, vec![], vec![])
            .attach(&mut node)
            .unwrap();
        graph.upsert_node(node)
    }

    fn claims(graph: &mut SemanticGraph, from: NodeId, targets: &[(NodeId, CallClass)]) {
        let node = graph.get_mut(from).unwrap();
        let calls = targets
            .iter()
            .map(|(target, class)| CallClaim {
                site: Span::default(),
                class: *class,
                targets: vec![*target],
                reason: "test-proof".into(),
                coverage_gap: false,
            })
            .collect();
        CallEvidence::new(node, calls, vec![]).attach(node).unwrap();
    }

    #[test]
    fn weakest_segment_and_strongest_supported_path_are_distinct() {
        let mut graph = SemanticGraph::new();
        let target = add(&mut graph, "target", false);
        let middle = add(&mut graph, "middle", false);
        let test = add(&mut graph, "test", true);
        claims(&mut graph, middle, &[(target, CallClass::May)]);
        claims(&mut graph, test, &[(middle, CallClass::Must)]);
        let result = graph.classified_impact(&[target]).tests(&graph);
        assert_eq!(result.may, vec![test]);
        assert!(result.must.is_empty());
        claims(
            &mut graph,
            test,
            &[(middle, CallClass::Must), (target, CallClass::Must)],
        );
        let result = graph.classified_impact(&[target]).tests(&graph);
        assert_eq!(result.must, vec![test]);
        assert!(result.may.is_empty());
    }

    #[test]
    fn unknown_target_exposes_disconnected_tests_without_inventing_edges() {
        let mut graph = SemanticGraph::new();
        let target = add(&mut graph, "target", false);
        let test = add(&mut graph, "test", true);
        let node = graph.get_mut(test).unwrap();
        CallEvidence::new(
            node,
            vec![CallClaim {
                site: Span::default(),
                class: CallClass::Unknown,
                targets: vec![],
                reason: "getattr".into(),
                coverage_gap: false,
            }],
            vec![],
        )
        .attach(node)
        .unwrap();
        let result = graph.classified_impact(&[target]).tests(&graph);
        assert_eq!(result.unknown, vec![test]);
        assert_eq!(result.boundaries.len(), 1);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn legacy_edge_weight_one_never_becomes_must() {
        let mut graph = SemanticGraph::new();
        let target = add(&mut graph, "target", false);
        let test = add(&mut graph, "test", true);
        graph
            .add_edge(test, target, Edge::new(EdgeKind::Calls))
            .unwrap();
        let result = graph.classified_impact(&[target]).tests(&graph);
        assert_eq!(result.unknown, vec![test]);
        assert!(result.must.is_empty());
        assert_eq!(result.boundaries[0].reason, "uncertified-Calls-edge");
    }

    #[test]
    fn stale_or_malformed_evidence_fails_to_unknown() {
        let mut graph = SemanticGraph::new();
        let target = add(&mut graph, "target", false);
        let test = add(&mut graph, "test", true);
        claims(&mut graph, test, &[(target, CallClass::Must)]);
        graph.get_mut(test).unwrap().source = "changed".into();
        assert_eq!(
            graph.call_evidence(test).unwrap_err(),
            "stale-call-evidence"
        );
        assert_eq!(
            graph.classified_impact(&[target]).tests(&graph).unknown,
            vec![test]
        );
        graph
            .get_mut(test)
            .unwrap()
            .set_attr(CALL_EVIDENCE_ATTRIBUTE, "malformed");
        assert_eq!(
            graph.call_evidence(test).unwrap_err(),
            "invalid-call-evidence"
        );
    }

    #[test]
    fn invalid_must_target_sets_are_rejected() {
        let mut graph = SemanticGraph::new();
        let target = add(&mut graph, "target", false);
        let test = add(&mut graph, "test", true);
        let node = graph.get_mut(test).unwrap();
        CallEvidence::new(
            node,
            vec![CallClaim {
                site: Span::default(),
                class: CallClass::Must,
                targets: vec![target, target],
                reason: "not-unique".into(),
                coverage_gap: false,
            }],
            vec![],
        )
        .attach(node)
        .unwrap();
        assert_eq!(graph.call_evidence(test).unwrap_err(), "invalid-call-claim");
    }

    #[test]
    fn persistence_preserves_evidence_but_reparse_does_not_reuse_it() {
        let mut graph = SemanticGraph::new();
        let target = add(&mut graph, "target", false);
        let test = add(&mut graph, "test", true);
        claims(&mut graph, test, &[(target, CallClass::Must)]);
        let expected = graph.classified_impact(&[target]);
        for restored in [
            SemanticGraph::from_ron(&graph.to_ron().unwrap()).unwrap(),
            SemanticGraph::from_bytes(&graph.to_bytes().unwrap()).unwrap(),
        ] {
            assert_eq!(restored.classified_impact(&[target]), expected);
        }
        graph.upsert_projection_node(
            Node::new(NodeKind::Function, "test", "test").with_source("body"),
        );
        assert_eq!(
            graph.call_evidence(test).unwrap_err(),
            "missing-call-evidence"
        );
    }

    #[test]
    fn boundaries_survive_empty_test_inventory_and_cycles_terminate() {
        let mut graph = SemanticGraph::new();
        let a = add(&mut graph, "a", false);
        let b = add(&mut graph, "b", false);
        claims(&mut graph, a, &[(b, CallClass::Must)]);
        claims(&mut graph, b, &[(a, CallClass::Must)]);
        assert_eq!(graph.classified_impact(&[a]).must.len(), 2);
        graph.get_mut(b).unwrap().attributes.clear();
        let result = graph.classified_impact(&[a]).tests(&graph);
        assert!(result.must.is_empty());
        assert!(!result.boundaries.is_empty());
    }
}
