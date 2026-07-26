//! Retained state and layout logic for the semantic graph explorer.

use aether_graph::{EdgeKind, NodeId, NodeKind};
use egui::{Pos2, Rect, Vec2};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

const MIN_ZOOM: f32 = 0.18;
const MAX_ZOOM: f32 = 4.0;
const SETTLE_STEPS: u16 = 180;

#[derive(Debug, Clone)]
pub(crate) struct ViewNode {
    pub(crate) id: NodeId,
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) kind: NodeKind,
    pub(crate) language: String,
    pub(crate) file: Option<String>,
    pub(crate) row: usize,
    pub(crate) attributes: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewEdge {
    pub(crate) from: NodeId,
    pub(crate) to: NodeId,
    pub(crate) kind: EdgeKind,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DisplayEdge {
    pub(crate) from: NodeId,
    pub(crate) to: NodeId,
    pub(crate) kind: EdgeKind,
    pub(crate) count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GraphScope {
    All,
    OneHop,
    TwoHops,
}

impl GraphScope {
    fn hops(self) -> Option<usize> {
        match self {
            Self::All => None,
            Self::OneHop => Some(1),
            Self::TwoHops => Some(2),
        }
    }
}

pub(crate) struct GraphViewState {
    pub(crate) search: String,
    pub(crate) scope: GraphScope,
    pub(crate) selected: Option<NodeId>,
    node_kinds: HashSet<NodeKind>,
    edge_kinds: HashSet<EdgeKind>,
    positions: HashMap<NodeId, Vec2>,
    velocities: HashMap<NodeId, Vec2>,
    pan: Vec2,
    zoom: f32,
    settle_steps: u16,
    fit_requested: bool,
}

impl Default for GraphViewState {
    fn default() -> Self {
        Self {
            search: String::new(),
            scope: GraphScope::All,
            selected: None,
            node_kinds: all_node_kinds().into_iter().collect(),
            edge_kinds: [EdgeKind::Calls, EdgeKind::Inherits].into_iter().collect(),
            positions: HashMap::new(),
            velocities: HashMap::new(),
            pan: Vec2::ZERO,
            zoom: 1.0,
            settle_steps: 0,
            fit_requested: true,
        }
    }
}

impl GraphViewState {
    pub(crate) fn reset_for_project(&mut self) {
        self.selected = None;
        self.positions.clear();
        self.velocities.clear();
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;
        self.settle_steps = 0;
        self.fit_requested = true;
    }

    pub(crate) fn request_fit(&mut self) {
        self.fit_requested = true;
    }

    pub(crate) fn selected_node<'a>(&self, nodes: &'a [ViewNode]) -> Option<&'a ViewNode> {
        let selected = self.selected?;
        nodes.iter().find(|node| node.id == selected)
    }

    pub(crate) fn node_kind_enabled(&self, kind: NodeKind) -> bool {
        self.node_kinds.contains(&kind)
    }

    pub(crate) fn toggle_node_kind(&mut self, kind: NodeKind) {
        if !self.node_kinds.remove(&kind) {
            self.node_kinds.insert(kind);
        }
        self.fit_requested = true;
    }

    pub(crate) fn edge_kind_enabled(&self, kind: EdgeKind) -> bool {
        self.edge_kinds.contains(&kind)
    }

    pub(crate) fn toggle_edge_kind(&mut self, kind: EdgeKind) {
        if !self.edge_kinds.remove(&kind) {
            self.edge_kinds.insert(kind);
        }
    }

    pub(crate) fn synchronize(&mut self, nodes: &[ViewNode]) {
        let ids: HashSet<NodeId> = nodes.iter().map(|node| node.id).collect();
        let before = self.positions.len();
        self.positions.retain(|id, _| ids.contains(id));
        self.velocities.retain(|id, _| ids.contains(id));
        if self.selected.is_some_and(|id| !ids.contains(&id)) {
            self.selected = None;
        }

        let mut added = false;
        for (index, node) in nodes.iter().enumerate() {
            self.positions.entry(node.id).or_insert_with(|| {
                added = true;
                seeded_position(node.id, index)
            });
            self.velocities.entry(node.id).or_insert(Vec2::ZERO);
        }
        if added || before != self.positions.len() {
            self.settle_steps = SETTLE_STEPS;
            self.fit_requested = true;
        }
    }

    pub(crate) fn visible_node_ids(
        &self,
        nodes: &[ViewNode],
        edges: &[ViewEdge],
    ) -> HashSet<NodeId> {
        let query = self.search.trim().to_ascii_lowercase();
        let neighborhood = self.scope.hops().and_then(|hops| {
            self.selected
                .map(|selected| neighborhood(selected, hops, edges))
        });

        nodes
            .iter()
            .filter(|node| {
                let selected = self.selected == Some(node.id);
                let kind_matches = self.node_kinds.contains(&node.kind);
                let query_matches = query.is_empty()
                    || node.name.to_ascii_lowercase().contains(&query)
                    || node.path.to_ascii_lowercase().contains(&query);
                let scope_matches = neighborhood
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&node.id));
                selected || (kind_matches && query_matches && scope_matches)
            })
            .map(|node| node.id)
            .collect()
    }

    pub(crate) fn display_node_ids(
        &self,
        nodes: &[ViewNode],
        visible: &HashSet<NodeId>,
    ) -> HashSet<NodeId> {
        if visible.len() <= 160
            || !self.search.trim().is_empty()
            || self.scope != GraphScope::All
            || self.zoom >= 0.52
        {
            return visible.clone();
        }

        let show_types = self.zoom >= 0.34;
        let mut display: HashSet<NodeId> = nodes
            .iter()
            .filter(|node| {
                visible.contains(&node.id)
                    && (matches!(
                        node.kind,
                        NodeKind::Module | NodeKind::Concept | NodeKind::Dependency
                    ) || (show_types && node.kind == NodeKind::Type))
            })
            .map(|node| node.id)
            .collect();
        if let Some(selected) = self.selected {
            if visible.contains(&selected) {
                display.insert(selected);
            }
        }
        if display.is_empty() {
            display.extend(visible.iter().copied().take(160));
        }
        display
    }

    pub(crate) fn display_edges(
        &self,
        nodes: &[ViewNode],
        edges: &[ViewEdge],
        visible: &HashSet<NodeId>,
        display: &HashSet<NodeId>,
    ) -> Vec<DisplayEdge> {
        let modules_by_file: HashMap<&str, NodeId> = nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Module && display.contains(&node.id))
            .filter_map(|node| node.file.as_deref().map(|file| (file, node.id)))
            .collect();
        let display_modules: Vec<&ViewNode> = nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Module && display.contains(&node.id))
            .collect();
        let representatives: HashMap<NodeId, NodeId> = nodes
            .iter()
            .filter(|node| visible.contains(&node.id))
            .filter_map(|node| {
                if display.contains(&node.id) {
                    return Some((node.id, node.id));
                }
                let module = node
                    .file
                    .as_deref()
                    .and_then(|file| modules_by_file.get(file).copied())
                    .or_else(|| {
                        display_modules
                            .iter()
                            .filter(|module| {
                                node.path == module.path
                                    || node
                                        .path
                                        .strip_prefix(&module.path)
                                        .is_some_and(|suffix| suffix.starts_with("::"))
                            })
                            .max_by_key(|module| module.path.len())
                            .map(|module| module.id)
                    });
                module.map(|module| (node.id, module))
            })
            .collect();

        let mut aggregated = BTreeMap::new();
        for edge in edges {
            if !self.edge_kinds.contains(&edge.kind)
                || !visible.contains(&edge.from)
                || !visible.contains(&edge.to)
            {
                continue;
            }
            let (Some(&from), Some(&to)) = (
                representatives.get(&edge.from),
                representatives.get(&edge.to),
            ) else {
                continue;
            };
            if from != to {
                *aggregated.entry((from, to, edge.kind)).or_insert(0) += 1;
            }
        }
        let mut display_edges: Vec<DisplayEdge> = aggregated
            .into_iter()
            .map(|((from, to, kind), count)| DisplayEdge {
                from,
                to,
                kind,
                count,
            })
            .collect();
        if display.len() < visible.len() {
            display_edges.sort_by(|left, right| {
                let left_selected =
                    self.selected == Some(left.from) || self.selected == Some(left.to);
                let right_selected =
                    self.selected == Some(right.from) || self.selected == Some(right.to);
                right_selected
                    .cmp(&left_selected)
                    .then_with(|| right.count.cmp(&left.count))
                    .then_with(|| {
                        (left.from, left.to, left.kind).cmp(&(right.from, right.to, right.kind))
                    })
            });
            display_edges.truncate((display.len() * 2).max(40));
        }
        display_edges
    }

    pub(crate) fn simulate(&mut self, nodes: &[ViewNode], edges: &[ViewEdge]) -> bool {
        if self.settle_steps == 0 || nodes.len() < 2 {
            return false;
        }

        const CELL: f32 = 105.0;
        const REPULSION: f32 = 1_800.0;
        const SPRING: f32 = 0.014;
        const REST: f32 = 95.0;
        const GRAVITY: f32 = 0.0018;
        const DAMPING: f32 = 0.78;
        const MAX_SPEED: f32 = 13.0;

        let mut forces: HashMap<NodeId, Vec2> =
            nodes.iter().map(|node| (node.id, Vec2::ZERO)).collect();
        let mut grid: HashMap<(i32, i32), Vec<NodeId>> = HashMap::new();
        for node in nodes {
            let p = self.positions[&node.id];
            grid.entry(((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32))
                .or_default()
                .push(node.id);
        }

        for node in nodes {
            let a = node.id;
            let pa = self.positions[&a];
            let cell = ((pa.x / CELL).floor() as i32, (pa.y / CELL).floor() as i32);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let Some(others) = grid.get(&(cell.0 + dx, cell.1 + dy)) else {
                        continue;
                    };
                    for &b in others {
                        if b <= a {
                            continue;
                        }
                        let pb = self.positions[&b];
                        let delta = pa - pb;
                        let distance_sq = delta.length_sq().max(36.0);
                        let direction = if delta.length_sq() < 0.01 {
                            seeded_direction(a, b)
                        } else {
                            delta.normalized()
                        };
                        let force = direction * (REPULSION / distance_sq);
                        *forces.get_mut(&a).expect("force slot") += force;
                        *forces.get_mut(&b).expect("force slot") -= force;
                    }
                }
            }
        }

        for edge in edges {
            let (Some(&from), Some(&to)) =
                (self.positions.get(&edge.from), self.positions.get(&edge.to))
            else {
                continue;
            };
            let delta = to - from;
            let distance = delta.length().max(0.01);
            let rest = if edge.kind == EdgeKind::Contains {
                REST * 0.72
            } else {
                REST
            };
            let force = delta / distance * ((distance - rest) * SPRING);
            if let Some(slot) = forces.get_mut(&edge.from) {
                *slot += force;
            }
            if let Some(slot) = forces.get_mut(&edge.to) {
                *slot -= force;
            }
        }

        for node in nodes {
            let id = node.id;
            let position = self.positions[&id];
            let force = forces[&id] - position * GRAVITY;
            let velocity = self.velocities.entry(id).or_default();
            *velocity = (*velocity + force) * DAMPING;
            if velocity.length() > MAX_SPEED {
                *velocity = velocity.normalized() * MAX_SPEED;
            }
            *self.positions.get_mut(&id).expect("position slot") += *velocity;
        }

        self.settle_steps -= 1;
        true
    }

    pub(crate) fn screen_position(&self, id: NodeId, rect: Rect) -> Option<Pos2> {
        self.positions
            .get(&id)
            .map(|world| rect.center() + self.pan + *world * self.zoom)
    }

    pub(crate) fn zoom(&self) -> f32 {
        self.zoom
    }

    pub(crate) fn pan_by(&mut self, delta: Vec2) {
        self.pan += delta;
    }

    pub(crate) fn zoom_at(&mut self, pointer: Pos2, rect: Rect, factor: f32) {
        let old_zoom = self.zoom;
        let new_zoom = (old_zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        if (new_zoom - old_zoom).abs() < f32::EPSILON {
            return;
        }
        let world = (pointer - rect.center() - self.pan) / old_zoom;
        self.zoom = new_zoom;
        self.pan = pointer - rect.center() - world * new_zoom;
    }

    pub(crate) fn fit_if_requested(&mut self, rect: Rect, visible: &HashSet<NodeId>) {
        if !self.fit_requested || visible.is_empty() {
            return;
        }
        let positions: Vec<Vec2> = visible
            .iter()
            .filter_map(|id| self.positions.get(id).copied())
            .collect();
        if positions.is_empty() {
            return;
        }
        let min = positions.iter().fold(positions[0], |acc, p| {
            egui::vec2(acc.x.min(p.x), acc.y.min(p.y))
        });
        let max = positions.iter().fold(positions[0], |acc, p| {
            egui::vec2(acc.x.max(p.x), acc.y.max(p.y))
        });
        let extent = (max - min) + egui::vec2(90.0, 90.0);
        self.zoom = (rect.width() / extent.x.max(1.0))
            .min(rect.height() / extent.y.max(1.0))
            .clamp(MIN_ZOOM, 1.5);
        self.pan = -((min + max) * 0.5) * self.zoom;
        self.fit_requested = false;
    }

    pub(crate) fn hit_test(
        &self,
        pointer: Pos2,
        rect: Rect,
        visible: &HashSet<NodeId>,
    ) -> Option<NodeId> {
        visible
            .iter()
            .filter_map(|id| {
                let position = self.screen_position(*id, rect)?;
                let distance = position.distance(pointer);
                (distance <= 11.0).then_some((*id, distance))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }

    pub(crate) fn node_is_on_screen(&self, id: NodeId, rect: Rect) -> bool {
        self.screen_position(id, rect)
            .is_some_and(|position| rect.expand(70.0).contains(position))
    }
}

pub(crate) fn all_node_kinds() -> [NodeKind; 8] {
    [
        NodeKind::Module,
        NodeKind::Function,
        NodeKind::Type,
        NodeKind::Field,
        NodeKind::Concept,
        NodeKind::Dependency,
        NodeKind::Extension,
        NodeKind::ExtensionContribution,
    ]
}

pub(crate) fn all_edge_kinds() -> [EdgeKind; 7] {
    [
        EdgeKind::Calls,
        EdgeKind::Inherits,
        EdgeKind::DataFlow,
        EdgeKind::Contains,
        EdgeKind::SemanticSimilar,
        EdgeKind::Impacts,
        EdgeKind::Contributes,
    ]
}

fn neighborhood(origin: NodeId, hops: usize, edges: &[ViewEdge]) -> HashSet<NodeId> {
    let mut visited = HashSet::from([origin]);
    let mut queue = VecDeque::from([(origin, 0)]);
    while let Some((current, depth)) = queue.pop_front() {
        if depth == hops {
            continue;
        }
        for edge in edges {
            let next = if edge.from == current {
                Some(edge.to)
            } else if edge.to == current {
                Some(edge.from)
            } else {
                None
            };
            if let Some(next) = next {
                if visited.insert(next) {
                    queue.push_back((next, depth + 1));
                }
            }
        }
    }
    visited
}

fn seeded_position(id: NodeId, index: usize) -> Vec2 {
    let angle_bits = id.0 ^ id.0.rotate_left(23);
    let angle = (angle_bits as f64 / u64::MAX as f64) as f32 * std::f32::consts::TAU;
    let radius = 45.0 + (index as f32).sqrt() * 58.0;
    egui::vec2(angle.cos(), angle.sin()) * radius
}

fn seeded_direction(a: NodeId, b: NodeId) -> Vec2 {
    let bits = a.0 ^ b.0.rotate_left(17);
    let angle = (bits as f64 / u64::MAX as f64) as f32 * std::f32::consts::TAU;
    egui::vec2(angle.cos(), angle.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(path: &str, kind: NodeKind) -> ViewNode {
        ViewNode {
            id: NodeId::from_path(path),
            name: path.rsplit("::").next().unwrap().to_string(),
            path: path.to_string(),
            kind,
            language: "rust".to_string(),
            file: Some("src/lib.rs".to_string()),
            row: 0,
            attributes: Vec::new(),
        }
    }

    #[test]
    fn search_and_kind_filters_keep_only_matching_nodes() {
        let nodes = vec![
            node("crate::math", NodeKind::Module),
            node("crate::math::add", NodeKind::Function),
            node("crate::math::subtract", NodeKind::Function),
        ];
        let mut state = GraphViewState::default();
        state.toggle_node_kind(NodeKind::Module);

        let visible = state.visible_node_ids(&nodes, &[]);
        assert_eq!(visible.len(), 2);

        state.search = "add".to_string();
        let visible = state.visible_node_ids(&nodes, &[]);
        assert_eq!(
            visible,
            HashSet::from([NodeId::from_path("crate::math::add")])
        );
    }

    #[test]
    fn neighborhood_scope_traverses_edges_in_both_directions() {
        let nodes = vec![
            node("crate::a", NodeKind::Function),
            node("crate::b", NodeKind::Function),
            node("crate::c", NodeKind::Function),
            node("crate::d", NodeKind::Function),
        ];
        let edges = vec![
            ViewEdge {
                from: nodes[0].id,
                to: nodes[1].id,
                kind: EdgeKind::Calls,
            },
            ViewEdge {
                from: nodes[2].id,
                to: nodes[1].id,
                kind: EdgeKind::Calls,
            },
            ViewEdge {
                from: nodes[2].id,
                to: nodes[3].id,
                kind: EdgeKind::Calls,
            },
        ];
        let mut state = GraphViewState {
            selected: Some(nodes[1].id),
            scope: GraphScope::OneHop,
            ..Default::default()
        };
        assert_eq!(
            state.visible_node_ids(&nodes, &edges),
            HashSet::from([nodes[0].id, nodes[1].id, nodes[2].id])
        );

        state.scope = GraphScope::TwoHops;
        assert_eq!(state.visible_node_ids(&nodes, &edges).len(), 4);
    }

    #[test]
    fn synchronization_retains_positions_and_prunes_removed_nodes() {
        let first = vec![
            node("crate::a", NodeKind::Function),
            node("crate::b", NodeKind::Function),
        ];
        let mut state = GraphViewState::default();
        state.synchronize(&first);
        let position = state.positions[&first[0].id];

        let second = vec![first[0].clone(), node("crate::c", NodeKind::Function)];
        state.synchronize(&second);

        assert_eq!(state.positions[&first[0].id], position);
        assert!(!state.positions.contains_key(&first[1].id));
        assert!(state.positions.contains_key(&second[1].id));
    }

    #[test]
    fn fit_places_visible_positions_inside_viewport() {
        let nodes = vec![
            node("crate::a", NodeKind::Function),
            node("crate::b", NodeKind::Function),
        ];
        let mut state = GraphViewState::default();
        state.synchronize(&nodes);
        state
            .positions
            .insert(nodes[0].id, egui::vec2(-500.0, -200.0));
        state
            .positions
            .insert(nodes[1].id, egui::vec2(500.0, 200.0));
        let visible = nodes.iter().map(|node| node.id).collect();
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 240.0));

        state.fit_if_requested(rect, &visible);

        assert!(nodes.iter().all(|node| {
            rect.contains(
                state
                    .screen_position(node.id, rect)
                    .expect("screen position"),
            )
        }));
    }

    #[test]
    fn zoom_keeps_world_point_under_pointer() {
        let mut state = GraphViewState::default();
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 240.0));
        let pointer = egui::pos2(130.0, 80.0);
        let world_before = (pointer - rect.center() - state.pan) / state.zoom;

        state.zoom_at(pointer, rect, 1.8);

        let screen_after = rect.center() + state.pan + world_before * state.zoom;
        assert!(screen_after.distance(pointer) < 0.001);
    }

    #[test]
    fn overview_collapses_nodes_and_aggregates_edges_by_module() {
        let mut left_module = node("crate::left", NodeKind::Module);
        left_module.file = Some("src/left.rs".to_string());
        let mut right_module = node("crate::right", NodeKind::Module);
        right_module.file = Some("src/right.rs".to_string());
        let mut nodes = vec![left_module, right_module];
        for index in 0..100 {
            let mut left = node(&format!("crate::left::f{index}"), NodeKind::Function);
            left.file = Some("src/left.rs".to_string());
            nodes.push(left);
            let mut right = node(&format!("crate::right::f{index}"), NodeKind::Function);
            right.file = Some("src/right.rs".to_string());
            nodes.push(right);
        }
        let edges = vec![
            ViewEdge {
                from: nodes[2].id,
                to: nodes[3].id,
                kind: EdgeKind::Calls,
            },
            ViewEdge {
                from: nodes[4].id,
                to: nodes[5].id,
                kind: EdgeKind::Calls,
            },
        ];
        let state = GraphViewState {
            zoom: 0.25,
            ..Default::default()
        };
        let visible: HashSet<NodeId> = nodes.iter().map(|node| node.id).collect();

        let display = state.display_node_ids(&nodes, &visible);
        let display_edges = state.display_edges(&nodes, &edges, &visible, &display);

        assert_eq!(display, HashSet::from([nodes[0].id, nodes[1].id]));
        assert_eq!(display_edges.len(), 1);
        assert_eq!(display_edges[0].from, nodes[0].id);
        assert_eq!(display_edges[0].to, nodes[1].id);
        assert_eq!(display_edges[0].count, 2);
    }

    #[test]
    fn overview_defaults_to_architectural_relationships() {
        let state = GraphViewState::default();
        assert!(state.edge_kind_enabled(EdgeKind::Calls));
        assert!(state.edge_kind_enabled(EdgeKind::Inherits));
        assert!(!state.edge_kind_enabled(EdgeKind::Impacts));
        assert!(!state.edge_kind_enabled(EdgeKind::Contains));
        assert!(!state.edge_kind_enabled(EdgeKind::DataFlow));
        assert!(!state.edge_kind_enabled(EdgeKind::SemanticSimilar));
    }

    #[test]
    fn simulation_keeps_positions_finite_for_large_sparse_graph() {
        let nodes: Vec<ViewNode> = (0..600)
            .map(|index| node(&format!("crate::n{index}"), NodeKind::Function))
            .collect();
        let edges: Vec<ViewEdge> = nodes
            .windows(2)
            .map(|pair| ViewEdge {
                from: pair[0].id,
                to: pair[1].id,
                kind: EdgeKind::Calls,
            })
            .collect();
        let mut state = GraphViewState::default();
        state.synchronize(&nodes);

        for _ in 0..8 {
            assert!(state.simulate(&nodes, &edges));
        }

        assert!(state
            .positions
            .values()
            .all(|position| position.x.is_finite() && position.y.is_finite()));
    }
}
