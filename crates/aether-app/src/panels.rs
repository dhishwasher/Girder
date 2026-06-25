//! The four resizable IDE panels, each a *projection* of shared state.

use crate::app::AetherApp;
use aether_graph::{EdgeKind, NodeId, NodeKind};
use egui::{Align2, Color32, FontId, Sense, Stroke};
use std::collections::HashMap;

fn node_color(kind: NodeKind) -> Color32 {
    match kind {
        NodeKind::Module => Color32::from_rgb(0x56, 0x9C, 0xD6),
        NodeKind::Function => Color32::from_rgb(0xDC, 0xDC, 0xAA),
        NodeKind::Type => Color32::from_rgb(0x4E, 0xC9, 0xB0),
        NodeKind::Field => Color32::from_rgb(0x9C, 0xDC, 0xFE),
        NodeKind::Concept => Color32::from_rgb(0xC5, 0x86, 0xC0),
        NodeKind::Dependency => Color32::from_rgb(0x80, 0x80, 0x80),
    }
}

fn edge_color(kind: EdgeKind) -> Color32 {
    match kind {
        EdgeKind::Calls => Color32::from_rgb(0xDC, 0xDC, 0xAA),
        EdgeKind::Inherits => Color32::from_rgb(0x4E, 0xC9, 0xB0),
        EdgeKind::DataFlow => Color32::from_rgb(0x56, 0x9C, 0xD6),
        EdgeKind::Contains => Color32::from_rgb(0x55, 0x55, 0x55),
        EdgeKind::SemanticSimilar => Color32::from_rgb(0xC5, 0x86, 0xC0),
        EdgeKind::Impacts => Color32::from_rgb(0xCE, 0x91, 0x78),
    }
}

/// Left panel: a force-directed-style view of the live semantic graph.
pub fn graph_panel(app: &mut AetherApp, ui: &mut egui::Ui) {
    ui.heading("Semantic Graph");
    ui.label("the source of truth — code is projected from this");
    ui.separator();

    let (nodes, edges) = {
        let g = app.graph.lock().unwrap();
        let nodes: Vec<(NodeId, String, NodeKind)> =
            g.nodes().map(|n| (n.id, n.name.clone(), n.kind)).collect();
        (nodes, g.edges())
    };

    let size = egui::vec2(ui.available_width(), ui.available_height().max(180.0));
    let (resp, painter) = ui.allocate_painter(size, Sense::hover());
    let rect = resp.rect;
    let center = rect.center();
    let radius = (rect.width().min(rect.height()) * 0.5 - 28.0).max(24.0);

    // Deterministic radial layout (a real build would run a force simulation).
    let n = nodes.len().max(1);
    let mut pos: HashMap<NodeId, egui::Pos2> = HashMap::new();
    for (i, (id, _, _)) in nodes.iter().enumerate() {
        let theta = std::f32::consts::TAU * (i as f32) / (n as f32);
        pos.insert(*id, center + radius * egui::vec2(theta.cos(), theta.sin()));
    }

    for (a, b, kind) in &edges {
        if let (Some(pa), Some(pb)) = (pos.get(a), pos.get(b)) {
            painter.line_segment([*pa, *pb], Stroke::new(1.0, edge_color(*kind)));
        }
    }
    for (id, name, kind) in &nodes {
        if let Some(p) = pos.get(id) {
            painter.circle_filled(*p, 6.0, node_color(*kind));
            painter.text(
                *p + egui::vec2(9.0, -9.0),
                Align2::LEFT_BOTTOM,
                name,
                FontId::proportional(11.0),
                ui.visuals().text_color(),
            );
        }
    }
}

/// Center panel: the code editor projection with tree-sitter highlighting.
/// Edits are folded straight back into the graph (bidirectional sync).
pub fn editor_panel(app: &mut AetherApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.heading("Code Projection");
        ui.label(format!("· {}", app.file));
    });
    ui.separator();

    let mut layouter = |ui: &egui::Ui, text: &str, wrap_width: f32| {
        let mut job = crate::highlight::layout(text, FontId::monospace(13.0));
        job.wrap.max_width = wrap_width;
        ui.fonts(|f| f.layout_job(job))
    };

    let resp = egui::ScrollArea::vertical()
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut app.code)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .layouter(&mut layouter),
            )
        })
        .inner;

    if resp.changed() {
        // Editor → graph: re-parse and diff the edit into the source of truth.
        app.sync_code_to_graph();
    }
}

/// Right panel: the agent swarm console.
pub fn agents_panel(app: &mut AetherApp, ui: &mut egui::Ui) {
    ui.heading("Agent Swarm");
    ui.label("Planner · Coder · Tester · Documenter");
    ui.label("Refactorer · Optimizer · SecurityAuditor");
    ui.separator();

    ui.label("Natural-language intent:");
    ui.add(
        egui::TextEdit::multiline(&mut app.intent)
            .desired_rows(2)
            .desired_width(f32::INFINITY),
    );
    if ui.button("▶  Dispatch swarm").clicked() {
        app.run_swarm();
    }

    ui.separator();
    ui.label("Conversation:");
    egui::ScrollArea::vertical()
        .max_height(160.0)
        .show(ui, |ui| {
            for (who, body) in &app.transcript {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(format!("[{who}]"));
                    ui.label(body);
                });
            }
        });

    ui.separator();
    // Inspect the freshly-authored node's agent annotations, if present.
    let g = app.graph.lock().unwrap();
    if let Some(node) = g.find_by_path("crate::math::multiply") {
        ui.collapsing("multiply (agent-authored)", |ui| {
            ui.monospace(node.source.clone());
            for (k, v) in &node.attributes {
                ui.label(format!("{k}: {v}"));
            }
        });
    }
}

/// Bottom panel: the time-travel debugger timeline + branch controls.
pub fn debugger_panel(app: &mut AetherApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.heading("Time-Travel Debugger");
        if ui.button("⑂ what-if: area = 12").clicked() {
            let id = app
                .timeline
                .fork_what_if(0, 2, "area", 12, "what-if area=12");
            app.selected_branch = id;
        }
        if app.timeline.branches().len() > 1 {
            if let Some(div) = app.timeline.first_divergence(0, app.selected_branch) {
                ui.label(format!("· first divergence at step {div}"));
            }
        }
    });

    ui.horizontal(|ui| {
        ui.label("Branch:");
        let labels: Vec<(usize, String)> = app
            .timeline
            .branches()
            .iter()
            .map(|b| (b.id, format!("{}: {}", b.id, b.label)))
            .collect();
        for (id, label) in labels {
            ui.selectable_value(&mut app.selected_branch, id, label);
        }
    });
    ui.separator();

    if let Some(branch) = app.timeline.branch(app.selected_branch) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            for step in &branch.trace.steps {
                let text = format!("step {}: {}", step.seq, step.description);
                if step.intervened {
                    ui.colored_label(Color32::from_rgb(0xDC, 0xDC, 0xAA), text);
                } else {
                    ui.label(text);
                }
            }
        });
    }
}
