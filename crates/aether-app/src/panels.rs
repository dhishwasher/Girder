//! The four resizable IDE panels, each a *projection* of shared state.

use crate::app::AetherApp;
use crate::graph_view::{all_edge_kinds, all_node_kinds, GraphScope, ViewEdge, ViewNode};
use aether_graph::{EdgeKind, NodeKind};
use egui::{Align2, Color32, FontId, Rect, Sense, Stroke};

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

/// Left panel: project navigation plus a view of the live semantic graph.
pub fn workspace_panel(app: &mut AetherApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.strong("Explorer");
        ui.label(format!("{} files", app.workspace.files().len()));
    });
    ui.add(
        egui::TextEdit::singleline(&mut app.file_filter)
            .desired_width(f32::INFINITY)
            .hint_text("Filter files"),
    );

    let filter = app.file_filter.trim().to_ascii_lowercase();
    let active = app.workspace.active_file().map(str::to_string);
    let files: Vec<(String, &'static str)> = app
        .workspace
        .files()
        .iter()
        .filter(|file| filter.is_empty() || file.relative().to_ascii_lowercase().contains(&filter))
        .map(|file| {
            let language = match file.language() {
                aether_builder::Lang::Rust => "RS",
                aether_builder::Lang::Python => "PY",
            };
            (file.relative().to_string(), language)
        })
        .collect();

    let explorer_height = if app.graph_view.selected.is_some() {
        100.0
    } else {
        (ui.available_height() * 0.30).clamp(100.0, 220.0)
    };
    egui::ScrollArea::vertical()
        .id_salt("project_files")
        .max_height(explorer_height)
        .show(ui, |ui| {
            for (relative, language) in files {
                ui.horizontal(|ui| {
                    ui.monospace(language);
                    if ui
                        .selectable_label(active.as_deref() == Some(relative.as_str()), &relative)
                        .clicked()
                    {
                        app.select_file(&relative);
                    }
                });
            }
        });

    ui.separator();
    graph_panel(app, ui);
}

fn graph_panel(app: &mut AetherApp, ui: &mut egui::Ui) {
    let (nodes, edges) = {
        let graph = app.workspace.graph().lock().unwrap();
        let nodes: Vec<ViewNode> = graph
            .nodes()
            .map(|node| ViewNode {
                id: node.id,
                name: node.name.clone(),
                path: node.path.clone(),
                kind: node.kind,
                language: node.language.clone(),
                file: node.file.clone(),
                row: node.span.start_row,
                attributes: node.attributes.clone(),
            })
            .collect();
        let edges: Vec<ViewEdge> = graph
            .edges()
            .into_iter()
            .map(|(from, to, kind)| ViewEdge { from, to, kind })
            .collect();
        (nodes, edges)
    };

    app.graph_view.synchronize(&nodes);
    if app.graph_view.simulate(&nodes, &edges) {
        ui.ctx().request_repaint();
    }

    ui.horizontal(|ui| {
        ui.strong("Semantic Graph");
        let visible = app.graph_view.visible_node_ids(&nodes, &edges).len();
        let display = app
            .graph_view
            .display_node_ids(&nodes, &app.graph_view.visible_node_ids(&nodes, &edges))
            .len();
        if display < visible {
            ui.weak(format!("{display} shown / {visible}"));
        } else {
            ui.weak(format!("{visible}/{}", nodes.len()));
        }
        if ui
            .small_button("Fit")
            .on_hover_text("Fit visible nodes in the viewport")
            .clicked()
        {
            app.graph_view.request_fit();
        }
    });
    ui.horizontal(|ui| {
        let search_changed = ui
            .add(
                egui::TextEdit::singleline(&mut app.graph_view.search)
                    .desired_width((ui.available_width() - 100.0).max(60.0))
                    .hint_text("Find nodes"),
            )
            .changed();
        if search_changed {
            app.graph_view.request_fit();
        }
        ui.menu_button("Types", |ui| {
            for kind in all_node_kinds() {
                let mut enabled = app.graph_view.node_kind_enabled(kind);
                if ui.checkbox(&mut enabled, node_kind_label(kind)).changed() {
                    app.graph_view.toggle_node_kind(kind);
                }
            }
        });
        ui.menu_button("Edges", |ui| {
            for kind in all_edge_kinds() {
                let mut enabled = app.graph_view.edge_kind_enabled(kind);
                if ui.checkbox(&mut enabled, edge_kind_label(kind)).changed() {
                    app.graph_view.toggle_edge_kind(kind);
                }
            }
        });
    });
    ui.horizontal(|ui| {
        ui.label("Scope");
        let mut scope_changed = ui
            .selectable_value(&mut app.graph_view.scope, GraphScope::All, "All")
            .changed();
        ui.add_enabled_ui(app.graph_view.selected.is_some(), |ui| {
            scope_changed |= ui
                .selectable_value(&mut app.graph_view.scope, GraphScope::OneHop, "1 hop")
                .changed();
            scope_changed |= ui
                .selectable_value(&mut app.graph_view.scope, GraphScope::TwoHops, "2 hops")
                .changed();
        });
        if scope_changed {
            app.graph_view.request_fit();
        }
    });

    let visible = app.graph_view.visible_node_ids(&nodes, &edges);
    let inspector_height = if app.graph_view.selected.is_some() {
        126.0
    } else {
        0.0
    };
    let size = egui::vec2(
        ui.available_width(),
        (ui.available_height() - inspector_height).max(120.0),
    );
    let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    let rect = response.rect;
    painter.rect_filled(rect, 2.0, Color32::from_rgb(0x17, 0x19, 0x1D));
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0_f32, Color32::from_rgb(0x35, 0x38, 0x40)),
        egui::StrokeKind::Inside,
    );
    app.graph_view.fit_if_requested(rect, &visible);
    let display = app.graph_view.display_node_ids(&nodes, &visible);

    if response.hovered() {
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll.abs() > f32::EPSILON {
            if let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) {
                app.graph_view
                    .zoom_at(pointer, rect, (scroll * 0.002).exp());
            }
        }
    }
    if response.dragged_by(egui::PointerButton::Primary)
        || response.dragged_by(egui::PointerButton::Middle)
    {
        app.graph_view
            .pan_by(ui.input(|input| input.pointer.delta()));
    }

    const RIPPLE_SECS: f32 = 2.5;
    let ripple_t = app
        .ripple_start
        .map(|start| (start.elapsed().as_secs_f32() / RIPPLE_SECS).min(1.0))
        .unwrap_or(1.0);

    for edge in app
        .graph_view
        .display_edges(&nodes, &edges, &visible, &display)
    {
        if let (Some(from), Some(to)) = (
            app.graph_view.screen_position(edge.from, rect),
            app.graph_view.screen_position(edge.to, rect),
        ) {
            if !Rect::from_two_pos(from, to).expand(2.0).intersects(rect) {
                continue;
            }
            let in_blast = app.impact_nodes.contains_key(&edge.from)
                || app.impact_nodes.contains_key(&edge.to);
            let selected_edge = app
                .graph_view
                .selected
                .is_some_and(|selected| selected == edge.from || selected == edge.to);
            let color = if in_blast && ripple_t < 1.0 {
                let alpha = ((1.0 - ripple_t) * 180.0) as u8;
                Color32::from_rgba_unmultiplied(0xFF, 0xA0, 0x30, alpha)
            } else if selected_edge {
                edge_color(edge.kind).gamma_multiply(1.3)
            } else {
                edge_color(edge.kind).gamma_multiply(if app.graph_view.zoom() < 0.5 {
                    0.45
                } else {
                    0.75
                })
            };
            painter.line_segment(
                [from, to],
                Stroke::new(
                    if in_blast && ripple_t < 1.0 {
                        2.0_f32
                    } else if selected_edge {
                        1.8_f32 + (edge.count as f32).log2().min(3.0) * 0.25
                    } else {
                        1.0_f32 + (edge.count as f32).log2().min(3.0) * 0.18
                    },
                    color,
                ),
            );
        }
    }

    let pointer = ui.input(|input| input.pointer.hover_pos());
    let hovered = pointer.and_then(|pointer| app.graph_view.hit_test(pointer, rect, &display));
    let mut occupied_labels = Vec::new();
    let mut draw_nodes: Vec<&ViewNode> = nodes
        .iter()
        .filter(|node| {
            display.contains(&node.id) && app.graph_view.node_is_on_screen(node.id, rect)
        })
        .collect();
    draw_nodes.sort_by_key(|node| {
        (
            app.graph_view.selected == Some(node.id),
            hovered == Some(node.id),
            node.id,
        )
    });

    for node in draw_nodes {
        if let Some(position) = app.graph_view.screen_position(node.id, rect) {
            if ripple_t < 1.0 {
                if let Some(&distance) = app.impact_nodes.get(&node.id) {
                    let distance_factor = 1.0 / (1.0 + distance as f32 * 0.6);
                    let intensity = (1.0 - ripple_t) * distance_factor;
                    let (red, green, blue) = if distance == 0 {
                        (0xFF, 0x8C, 0x00u8)
                    } else {
                        (0xDC, 0x4A, 0x2A)
                    };
                    let alpha = (intensity * 255.0).clamp(0.0, 255.0) as u8;
                    let ring = Color32::from_rgba_unmultiplied(red, green, blue, alpha);
                    painter.circle_stroke(
                        position,
                        6.0 + intensity * 12.0,
                        Stroke::new(2.0_f32, ring),
                    );
                }
            }

            let selected = app.graph_view.selected == Some(node.id);
            let is_hovered = hovered == Some(node.id);
            if selected {
                painter.circle_stroke(
                    position,
                    10.0,
                    Stroke::new(2.0_f32, Color32::from_rgb(0xF2, 0xF2, 0xF2)),
                );
            }
            painter.circle_filled(
                position,
                if selected || is_hovered { 7.0 } else { 5.0 },
                node_color(node.kind),
            );

            let show_label = selected
                || is_hovered
                || !app.graph_view.search.trim().is_empty()
                || app.graph_view.zoom() >= 0.68;
            if show_label {
                let font = FontId::proportional(if selected { 12.0 } else { 10.5 });
                let text_color = if selected {
                    Color32::WHITE
                } else {
                    ui.visuals().text_color()
                };
                let galley = painter.layout_no_wrap(node.name.clone(), font, text_color);
                let label_rect = Rect::from_min_size(
                    position + egui::vec2(9.0, -galley.size().y * 0.5),
                    galley.size(),
                )
                .expand(2.0);
                let collides = occupied_labels
                    .iter()
                    .any(|occupied: &Rect| occupied.intersects(label_rect));
                if selected || is_hovered || !collides {
                    painter.galley(label_rect.min + egui::vec2(2.0, 2.0), galley, text_color);
                    occupied_labels.push(label_rect);
                }
            }
        }
    }

    if let Some(node) = hovered.and_then(|id| nodes.iter().find(|node| node.id == id)) {
        response.clone().on_hover_ui_at_pointer(|ui| {
            ui.strong(&node.path);
            ui.label(format!(
                "{}{}",
                node_kind_label(node.kind),
                node.file
                    .as_ref()
                    .map(|file| format!(" - {file}:{}", node.row + 1))
                    .unwrap_or_default()
            ));
        });
    }

    let mut navigate = None;
    if response.clicked() {
        app.graph_view.selected =
            pointer.and_then(|pointer| app.graph_view.hit_test(pointer, rect, &display));
    }
    if response.double_clicked() {
        navigate = pointer.and_then(|pointer| app.graph_view.hit_test(pointer, rect, &display));
    }

    if visible.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No matching graph nodes",
            FontId::proportional(12.0),
            ui.visuals().weak_text_color(),
        );
    }

    ui.horizontal(|ui| {
        ui.weak(format!("{:.0}%", app.graph_view.zoom() * 100.0));
        if hovered.is_some() {
            ui.weak("Double-click to open source");
        }
    });

    if let Some(node) = app.graph_view.selected_node(&nodes).cloned() {
        ui.separator();
        ui.horizontal(|ui| {
            ui.colored_label(node_color(node.kind), node_kind_label(node.kind));
            ui.strong(&node.name);
        });
        ui.monospace(&node.path);
        if let Some(file) = &node.file {
            ui.label(format!("{file}:{} - {}", node.row + 1, node.language));
        } else {
            ui.weak("Graph-owned node");
        }
        if let Some((_, summary)) = node
            .attributes
            .iter()
            .find(|(key, _)| key == "summary")
            .or_else(|| node.attributes.first())
        {
            ui.label(summary);
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(node.file.is_some(), egui::Button::new("Open source"))
                .clicked()
            {
                navigate = Some(node.id);
            }
            if ui.button("Focus").clicked() {
                app.graph_view.scope = GraphScope::OneHop;
                app.graph_view.request_fit();
            }
            if ui.button("Clear").clicked() {
                app.graph_view.selected = None;
                app.graph_view.scope = GraphScope::All;
                app.graph_view.request_fit();
            }
        });
    }

    if let Some(id) = navigate {
        app.navigate_to_graph_node(id);
    }
}

fn node_kind_label(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Module => "Module",
        NodeKind::Function => "Function",
        NodeKind::Type => "Type",
        NodeKind::Field => "Field",
        NodeKind::Concept => "Concept",
        NodeKind::Dependency => "Dependency",
    }
}

fn edge_kind_label(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Calls => "Calls",
        EdgeKind::Inherits => "Inherits",
        EdgeKind::DataFlow => "Data flow",
        EdgeKind::Contains => "Contains",
        EdgeKind::SemanticSimilar => "Similar",
        EdgeKind::Impacts => "Impacts",
    }
}

/// Center panel: the code editor projection with tree-sitter highlighting.
/// Edits are folded straight back into the graph (bidirectional sync).
pub fn editor_panel(app: &mut AetherApp, ui: &mut egui::Ui) {
    let Some(file) = app.workspace.active_file().map(str::to_string) else {
        ui.heading("Code");
        ui.separator();
        ui.label("No supported source files were found in this project.");
        return;
    };
    let Some(language) = app.workspace.active_language() else {
        return;
    };

    ui.horizontal(|ui| {
        ui.heading(&file);
        if app.workspace.is_dirty() {
            ui.colored_label(Color32::from_rgb(0xE5, 0xC0, 0x7B), "modified");
        }
    });
    ui.separator();

    let mut layouter = |ui: &egui::Ui, text: &str, wrap_width: f32| {
        let mut job = crate::highlight::layout(text, language, FontId::monospace(13.0));
        job.wrap.max_width = wrap_width;
        ui.fonts(|f| f.layout_job(job))
    };

    let editable = !app.workspace.has_pending_agent_changes() && app.swarm_rx.is_none();
    let editor_jump = app.editor_jump.take();
    let output = egui::ScrollArea::vertical()
        .show(ui, |ui| {
            ui.add_enabled_ui(editable, |ui| {
                let mut output = egui::TextEdit::multiline(app.workspace.buffer_mut())
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .layouter(&mut layouter)
                    .show(ui);
                if let Some(byte_offset) = editor_jump {
                    let source = app.workspace.buffer_mut();
                    let mut byte_offset = byte_offset.min(source.len());
                    while !source.is_char_boundary(byte_offset) {
                        byte_offset -= 1;
                    }
                    let char_offset = source[..byte_offset].chars().count();
                    let cursor = egui::text::CCursor::new(char_offset);
                    output
                        .state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::one(cursor)));
                    output.state.clone().store(ui.ctx(), output.response.id);
                    output.response.request_focus();
                    let cursor_rect = output
                        .galley
                        .pos_from_ccursor(cursor)
                        .translate(output.galley_pos.to_vec2());
                    ui.scroll_to_rect(cursor_rect, Some(egui::Align::Center));
                }
                output
            })
            .inner
        })
        .inner;

    if output.response.changed() {
        // Editor → graph: re-parse and diff the edit into the source of truth.
        app.sync_code_to_graph();
    }
}

/// Right panel: the agent swarm console.
pub fn agents_panel(app: &mut AetherApp, ui: &mut egui::Ui) {
    ui.heading("Agents");
    ui.separator();

    ui.add(
        egui::TextEdit::multiline(&mut app.intent)
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .hint_text("Describe a change"),
    );
    let running = app.swarm_rx.is_some();
    let can_run = !running
        && !app.workspace.is_dirty()
        && !app.workspace.has_pending_agent_changes()
        && app.workspace.active_file().is_some()
        && !app.intent.trim().is_empty();
    let label = if running { "Running..." } else { "Run agents" };
    if ui.add_enabled(can_run, egui::Button::new(label)).clicked() {
        app.run_swarm();
    }
    if app.workspace.has_pending_agent_changes() {
        let validating = app.agent_validation_running();
        let validation_passed = app
            .workspace
            .agent_validation_report()
            .is_some_and(|report| report.passed());
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    validation_passed && !validating,
                    egui::Button::new("Commit"),
                )
                .on_hover_text("Project agent-authored graph changes to source and persist")
                .clicked()
            {
                app.commit_agent_changes();
            }
            if ui
                .add_enabled(!validating, egui::Button::new("Validate"))
                .on_hover_text("Rebuild and test the candidate in an isolated workspace")
                .clicked()
            {
                app.start_agent_validation();
            }
            if ui
                .button("Roll back")
                .on_hover_text("Restore the graph checkpoint from before this agent run")
                .clicked()
            {
                app.rollback_agent_changes();
            }
        });
        if validating {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Validating candidate");
            });
        }
        if let Some(report) = app.workspace.agent_validation_report() {
            ui.collapsing("Validation details", |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("agent_validation")
                    .max_height(180.0)
                    .show(ui, |ui| {
                        for step in &report.steps {
                            let color = if matches!(
                                step.status,
                                crate::project::ValidationStatus::Passed
                                    | crate::project::ValidationStatus::Skipped
                            ) {
                                Color32::from_rgb(0x4E, 0xC9, 0xB0)
                            } else {
                                Color32::from_rgb(0xF4, 0x87, 0x71)
                            };
                            ui.horizontal_wrapped(|ui| {
                                ui.colored_label(color, step.status.label());
                                ui.strong(&step.label);
                                ui.weak(format!("{:.2}s", step.duration.as_secs_f32()));
                            });
                            if let Some(command) = &step.command {
                                ui.monospace(command);
                            }
                            if !step.output.trim().is_empty()
                                && step.status != crate::project::ValidationStatus::Passed
                            {
                                ui.monospace(&step.output);
                            }
                        }
                    });
            });
        }
    }

    ui.separator();
    ui.label("Conversation:");
    egui::ScrollArea::vertical()
        .id_salt("swarm_transcript")
        .max_height(160.0)
        .show(ui, |ui| {
            for (who, body) in &app.transcript {
                let is_spec = who == "Planner" && body.starts_with("spec:");
                let is_built = who == "Coder" && body.starts_with("built");
                ui.horizontal_wrapped(|ui| {
                    ui.strong(format!("[{who}]"));
                    if is_spec || is_built {
                        ui.colored_label(Color32::from_rgb(0x4E, 0xC9, 0xB0), body);
                    } else {
                        ui.label(body);
                    }
                });
            }
        });

    ui.separator();
    // Show all agent-authored nodes in the forge module.
    let g = app.workspace.graph().lock().unwrap();
    let authored: Vec<_> = g
        .nodes()
        .filter(|n| n.attr("authored_by").is_some())
        .map(|n| (n.name.clone(), n.source.clone(), n.attributes.clone()))
        .collect();
    if !authored.is_empty() {
        ui.label(format!("Agent-authored ({}):", authored.len()));
        for (name, source, attrs) in authored {
            ui.collapsing(&name, |ui| {
                ui.monospace(&source);
                for (k, v) in &attrs {
                    if k != "authored_by" {
                        ui.label(format!("{k}: {v}"));
                    }
                }
            });
        }
    }
}

/// Bottom panel: the time-travel debugger timeline + branch controls.
pub fn debugger_panel(app: &mut AetherApp, ui: &mut egui::Ui) {
    // ── Real Python tracer ────────────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.strong("Python Tracer");
        ui.add(
            egui::TextEdit::singleline(&mut app.py_file)
                .desired_width(200.0)
                .hint_text("path/to/script.py"),
        );
        let tracing = app.py_trace_rx.is_some();
        let btn = if tracing { "⏳" } else { "▶ Trace" };
        if ui
            .add_enabled(!tracing && !app.py_file.is_empty(), egui::Button::new(btn))
            .clicked()
        {
            app.trace_python_file();
        }
        if !app.py_steps.is_empty() {
            ui.label(format!("· {} steps", app.py_steps.len()));
        }
    });

    if !app.py_steps.is_empty() {
        egui::ScrollArea::vertical()
            .id_salt("py_trace_scroll")
            .max_height(80.0)
            .show(ui, |ui| {
                for (seq, desc, intervened) in &app.py_steps {
                    let text = format!("step {seq:4}: {desc}");
                    if *intervened {
                        ui.colored_label(Color32::from_rgb(0xFF, 0xA0, 0x30), text);
                    } else {
                        ui.monospace(text);
                    }
                }
            });
    }

    ui.separator();

    // ── Toy time-travel debugger ──────────────────────────────────────────────
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
