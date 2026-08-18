//! The four resizable IDE panels, each a *projection* of shared state.

use crate::app::{AetherApp, AuthorMode, ExtensionPanelView, RightPanel};
use crate::graph_view::{all_edge_kinds, all_node_kinds, GraphScope, ViewEdge, ViewNode};
use crate::project::AuthorEvent;
use aether_extensions::{
    Capability, CommandAction, Contribution, ExtensionState, PanelLocation, PanelView,
};
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
        NodeKind::Extension => Color32::from_rgb(0xD7, 0xBA, 0x7D),
        NodeKind::ExtensionContribution => Color32::from_rgb(0xB5, 0xCE, 0xA8),
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
        EdgeKind::Contributes => Color32::from_rgb(0xD7, 0xBA, 0x7D),
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
        NodeKind::Extension => "Extension",
        NodeKind::ExtensionContribution => "Contribution",
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
        EdgeKind::Contributes => "Contributes",
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

    let editable = !app.workspace.has_pending_agent_changes()
        && app.swarm_rx.is_none()
        && !app.extension_busy();
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
    ui.horizontal(|ui| {
        ui.selectable_value(&mut app.right_panel, RightPanel::Agents, "Agents");
        ui.selectable_value(&mut app.right_panel, RightPanel::Extensions, "Extensions");
        ui.selectable_value(
            &mut app.right_panel,
            RightPanel::Collaboration,
            "Collaboration",
        );
        ui.selectable_value(&mut app.right_panel, RightPanel::Author, "Author");
    });
    ui.separator();

    match app.right_panel {
        RightPanel::Agents => agent_console(app, ui),
        RightPanel::Extensions => extensions_panel(app, ui),
        RightPanel::Collaboration => collaboration_panel(app, ui),
        RightPanel::Author => author_panel(app, ui),
    }
}

/// Renders one [`AuthorEvent`] as it would print on the CLI (see
/// `author::print_author_event`) — the panel and the CLI share the same
/// event model, they just render it into different places.
fn format_author_event(event: &AuthorEvent) -> String {
    match event {
        AuthorEvent::Loading { root } => format!("Loading {} ...", root.display()),
        AuthorEvent::Loaded { files, node_count } => {
            format!("  {files} file(s), {node_count} nodes")
        }
        AuthorEvent::NodesSelected {
            pinned,
            intent,
            nodes,
        } => {
            let header = if *pinned {
                format!("Using pinned nodes for \"{intent}\":")
            } else {
                format!("Selecting nodes for \"{intent}\":")
            };
            let mut lines = vec![header];
            for (path, score) in nodes {
                lines.push(match score {
                    Some(score) => format!("  {score:.2}  {path}"),
                    None => format!("  {path}"),
                });
            }
            lines.join("\n")
        }
        AuthorEvent::AttemptStarted {
            provider,
            attempt,
            max_attempts,
            node_paths,
        } => format!(
            "[{provider}] attempt {attempt}/{max_attempts} — nodes: {}",
            node_paths.join(", ")
        ),
        AuthorEvent::ProviderDeclined {
            diagnostic,
            elapsed_secs,
        } => format!("  {diagnostic} ({elapsed_secs}s elapsed)"),
        AuthorEvent::ProviderResponded {
            provider,
            elapsed_secs,
        } => format!("  {provider} responded in {elapsed_secs}s"),
        AuthorEvent::ModelResponseInvalid { diagnostic }
        | AuthorEvent::PlanWrapFailed { diagnostic } => {
            format!("  {diagnostic}")
        }
        AuthorEvent::AttemptFailed { diagnostic } => format!("  attempt failed:\n    {diagnostic}"),
    }
}

/// Shared "are you sure" gate for a live (non-dry) Run / Run-authored click,
/// mirroring the remove-extension confirm/cancel row
/// (`extension_installed_panel`): a GUI button that silently writes to the
/// tree has no command line to review first, so a non-dry run always takes
/// a second, explicit click.
fn author_run_button(
    app: &mut AetherApp,
    ui: &mut egui::Ui,
    label: &str,
    run: impl FnOnce(&mut AetherApp),
) {
    if app.author_live_run_confirming {
        ui.horizontal(|ui| {
            ui.colored_label(
                Color32::from_rgb(0xF4, 0x87, 0x71),
                "This writes to the working tree. Run for real?",
            );
            if ui.button("Confirm").clicked() {
                run(app);
            }
            if ui.button("Cancel").clicked() {
                app.author_live_run_confirming = false;
            }
        });
    } else if ui
        .add_enabled(!app.author_write_busy(), egui::Button::new(label))
        .clicked()
    {
        run(app);
    }
}

fn author_panel(app: &mut AetherApp, ui: &mut egui::Ui) {
    ui.heading("Author");
    ui.label(
        "Author a plan against the graph — locally via the router's providers, or externally by pasting a plan back in from a model that isn't wired in.",
    );
    ui.add_space(6.0);

    ui.label("Intent");
    ui.text_edit_multiline(&mut app.author_intent);

    ui.horizontal(|ui| {
        // Read-only and always safe to re-click: never gated on
        // author_busy() (see `run_author_search`'s doc comment).
        if ui.button("Search").clicked() {
            app.run_author_search();
        }
        ui.label(format!("{} node(s) found", app.author_search_results.len()));
    });
    if !app.author_search_results.is_empty() {
        egui::ScrollArea::vertical()
            .id_salt("author_search_results")
            .max_height(140.0)
            .show(ui, |ui| {
                for hit in &mut app.author_search_results {
                    let label = match hit.score {
                        Some(score) => format!("{score:.2}  {}", hit.path),
                        None => hit.path.clone(),
                    };
                    ui.checkbox(&mut hit.selected, label);
                }
            });
    }

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.selectable_value(&mut app.author_mode, AuthorMode::Local, "Local model");
        ui.selectable_value(&mut app.author_mode, AuthorMode::External, "External model");
    });
    if ui.checkbox(&mut app.author_dry_run, "Dry run").changed() {
        app.author_live_run_confirming = false;
    }
    ui.separator();

    match app.author_mode {
        AuthorMode::Local => author_local_panel(app, ui),
        AuthorMode::External => author_external_panel(app, ui),
    }

    if let Some(error) = app.author_last_error.clone() {
        ui.add_space(6.0);
        ui.colored_label(Color32::from_rgb(0xF4, 0x87, 0x71), error);
    }
    if let Some(result) = app.author_last_result.clone() {
        ui.add_space(6.0);
        ui.separator();
        ui.strong("Result");
        egui::ScrollArea::vertical()
            .id_salt("author_result")
            .max_height(200.0)
            .show(ui, |ui| {
                ui.monospace(result);
            });
    }
}

fn author_local_panel(app: &mut AetherApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label("Max repairs");
        ui.add(egui::DragValue::new(&mut app.author_max_repairs).range(0..=10));
    });

    author_run_button(app, ui, "Run", AetherApp::run_author);

    if !app.author_log.is_empty() {
        ui.add_space(6.0);
        ui.strong("Log");
        egui::ScrollArea::vertical()
            .id_salt("author_log")
            .max_height(220.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for event in &app.author_log {
                    ui.monospace(format_author_event(event));
                }
            });
    }
}

fn author_external_panel(app: &mut AetherApp, ui: &mut egui::Ui) {
    // Read-only and always safe to re-click: never gated on author_busy()
    // (see `copy_author_context_json`'s doc comment).
    if ui
        .button("Copy context JSON")
        .on_hover_text("Puts exactly what `bitcode context --json` would print onto the clipboard.")
        .clicked()
    {
        app.copy_author_context_json();
    }

    ui.add_space(6.0);
    ui.label("Paste a plan back in");
    egui::ScrollArea::vertical()
        .id_salt("author_pasted_plan")
        .max_height(160.0)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut app.author_pasted_plan)
                    .desired_rows(8)
                    .code_editor(),
            );
        });

    ui.label("Authored by");
    ui.text_edit_singleline(&mut app.author_authored_by);

    author_run_button(app, ui, "Run authored", AetherApp::run_author_authored);
}

fn collaboration_panel(app: &mut AetherApp, ui: &mut egui::Ui) {
    ui.heading("Graph-native collaboration");
    ui.label(
        "Exchange causal semantic-node and edge operations. Source projection stays explicit so remote changes are never written into files without review.",
    );
    ui.add_space(6.0);
    ui.label("Bundle");
    ui.text_edit_singleline(&mut app.collaboration_bundle_input);
    ui.label("Actor id (initialize or manage membership)");
    ui.text_edit_singleline(&mut app.collaboration_actor_input);

    let can_snapshot = !app.collaboration_busy()
        && !app.workspace.is_dirty()
        && !app.workspace.has_pending_agent_changes();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(can_snapshot, egui::Button::new("Initialize"))
            .clicked()
        {
            app.initialize_collaboration();
        }
        if ui
            .add_enabled(can_snapshot, egui::Button::new("Sync local graph"))
            .clicked()
        {
            app.sync_collaboration();
        }
        if ui.button("Inspect").clicked() {
            app.inspect_collaboration();
        }
        if ui
            .add_enabled(can_snapshot, egui::Button::new("Compact history"))
            .on_hover_text(
                "Prune only causally superseded operations acknowledged durably by every active member",
            )
            .clicked()
        {
            app.compact_collaboration();
        }
    });
    ui.horizontal(|ui| {
        if ui
            .add_enabled(can_snapshot, egui::Button::new("Add member"))
            .on_hover_text("Record a causal membership operation for the actor id above")
            .clicked()
        {
            app.modify_collaboration_member(true);
        }
        if ui
            .add_enabled(can_snapshot, egui::Button::new("Remove member"))
            .on_hover_text("Revoke the actor id above; the local actor cannot remove itself")
            .clicked()
        {
            app.modify_collaboration_member(false);
        }
    });
    if !can_snapshot && !app.collaboration_busy() {
        ui.small("Save or resolve pending agent changes before snapshotting the local graph.");
    }
    ui.small(
        "Membership is causal and convergent. Adding a member blocks compaction until that actor durably acknowledges; removing one retains a tombstone barrier.",
    );

    ui.separator();
    ui.strong("Reviewed source projection");
    ui.horizontal(|ui| {
        if ui
            .add_enabled(can_snapshot, egui::Button::new("Review remote source"))
            .clicked()
        {
            app.start_collaboration_projection_review();
        }
        if ui
            .add_enabled(
                can_snapshot && app.collaboration_projection_approved(),
                egui::Button::new("Apply reviewed projection"),
            )
            .on_hover_text(
                "Re-check the approved bundle and local baselines, validate in isolation, then journal-commit files and graph",
            )
            .clicked()
        {
            app.start_collaboration_projection_apply();
        }
    });
    ui.small(
        "Review rebuilds whole-file bytes back into a semantic graph and blocks inconsistent CRDT winners. Approval is digest-bound and invalidated by any bundle or project change.",
    );

    ui.separator();
    ui.strong("Live peer");
    ui.label("Loopback address");
    ui.text_edit_singleline(&mut app.collaboration_address_input);
    ui.label("Secret file (32+ bytes, mode 600)");
    ui.text_edit_singleline(&mut app.collaboration_secret_input);
    ui.label("Private local discovery directory");
    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut app.collaboration_discovery_input);
        if ui
            .add_enabled(!app.collaboration_busy(), egui::Button::new("Discover"))
            .on_hover_text("Verify bounded loopback host tickets with the group secret")
            .clicked()
        {
            app.scan_collaboration_peers();
        }
    });
    for peer in app.collaboration_discovered_peers.clone() {
        ui.horizontal(|ui| {
            ui.monospace(format!(
                "{} at {} (pid {})",
                peer.actor, peer.address, peer.process_id
            ));
            if ui.button("Use").clicked() {
                app.collaboration_address_input = peer.address.to_string();
                app.collaboration_status = format!(
                    "Selected discovered peer {} at {}.",
                    peer.actor, peer.address
                );
            }
        });
    }
    ui.small(
        "Discovery tickets are authenticated loopback hints from active roster members. The full live handshake remains authoritative.",
    );
    ui.add_space(4.0);
    ui.checkbox(
        &mut app.collaboration_identity_enabled,
        "Require pinned identities and operation provenance",
    )
    .on_hover_text(
        "Authenticate the active roster actor and verify durable per-operation Ed25519 signatures in addition to the group secret",
    );
    if app.collaboration_identity_enabled {
        ui.label("Private actor identity");
        ui.text_edit_singleline(&mut app.collaboration_identity_input);
        ui.label("Shareable public identity");
        ui.text_edit_singleline(&mut app.collaboration_identity_public_input);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!app.collaboration_busy(), egui::Button::new("Generate identity"))
                .on_hover_text(
                    "Create a new actor-bound private key and shareable public identity; existing files are never overwritten",
                )
                .clicked()
            {
                app.generate_actor_identity();
            }
        });

        ui.label("Pinned identity trust store");
        ui.text_edit_singleline(&mut app.collaboration_trust_input);
        ui.label("Peer public identity to review");
        ui.text_edit_singleline(&mut app.collaboration_peer_identity_input);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !app.collaboration_busy(),
                    egui::Button::new("Review peer identity"),
                )
                .clicked()
            {
                app.review_peer_identity();
            }
            if ui
                .add_enabled(
                    !app.collaboration_busy() && app.collaboration_identity_review.is_some(),
                    egui::Button::new("Trust reviewed identity"),
                )
                .on_hover_text(
                    "Re-read the public file and pin only the exact reviewed fingerprint",
                )
                .clicked()
            {
                app.trust_reviewed_identity();
            }
            if ui
                .add_enabled(!app.collaboration_busy(), egui::Button::new("Refresh pins"))
                .clicked()
            {
                app.refresh_trusted_identities();
            }
        });
        if let Some(identity) = &app.collaboration_identity_review {
            ui.monospace(format!(
                "Reviewed {} · SHA-256 {}",
                identity.actor, identity.fingerprint
            ));
        }
        for identity in &app.collaboration_trusted_identities {
            ui.monospace(format!(
                "Pinned {} · SHA-256 {}",
                identity.actor, identity.fingerprint
            ));
        }
        ui.small(
            "Verify fingerprints out of band. Strict sessions sign legacy local history and reject unsigned or forged actor operations before persistence. Both endpoints must enable identity mode; use the CLI to attest or audit offline and to record dual-signed rotation or remove a pin.",
        );
    } else {
        ui.small(
            "Legacy mode authenticates only group membership: any secret holder can claim any active actor, and incoming operation proofs are ignored. Enable pinned identities to bind endpoints and stored operation authorship to approved Ed25519 keys.",
        );
    }
    ui.label("Session presence (optional)");
    ui.text_edit_singleline(&mut app.collaboration_presence_input);
    ui.small(
        "Presence is explicit single-line status (up to 256 UTF-8 bytes), authenticated for this sync only, and never saved in the collaboration bundle.",
    );
    let joining = app.collaboration_busy();
    ui.horizontal(|ui| {
        if ui.button("Generate secret").clicked() {
            app.generate_collaboration_secret();
        }
        if ui
            .add_enabled(
                !joining,
                egui::Button::new(if joining {
                    "Joining..."
                } else {
                    "Join and sync"
                }),
            )
            .clicked()
        {
            app.start_collaboration_join();
        }
    });
    ui.small(
        "Sessions are mutually authenticated and integrity-protected at the selected identity level. They are loopback-only because payloads are not encrypted; use an SSH tunnel for a remote peer.",
    );
    ui.small(
        "Concurrent removals win; concurrent updates use a deterministic actor/counter tie-break. Joining updates the bundle, not project files.",
    );
    ui.small(
        "Successful live sessions require both actors in the active roster and record durable peer acknowledgements. Compaction is conservative and rejects missing member acknowledgements or stale peers that need discarded history.",
    );

    ui.separator();
    ui.strong("Status");
    egui::ScrollArea::vertical()
        .max_height(180.0)
        .show(ui, |ui| {
            ui.monospace(&app.collaboration_status);
        });
}

fn agent_console(app: &mut AetherApp, ui: &mut egui::Ui) {
    ui.add(
        egui::TextEdit::multiline(&mut app.intent)
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .hint_text("Describe a change"),
    );
    let running = app.swarm_rx.is_some();
    let can_run = !running
        && !app.extension_busy()
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

#[derive(Debug)]
enum PendingExtensionAction {
    AskGraph(String),
    OpenFile(String, Option<u32>),
    RunValidation(String, String),
    SetEnabled(String, bool),
    Remove(String),
}

fn extensions_panel(app: &mut AetherApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.selectable_value(
            &mut app.extension_panel_view,
            ExtensionPanelView::Generate,
            "Generate",
        );
        ui.selectable_value(
            &mut app.extension_panel_view,
            ExtensionPanelView::Marketplace,
            "Marketplace",
        );
        ui.selectable_value(
            &mut app.extension_panel_view,
            ExtensionPanelView::Installed,
            "Installed",
        );
    });
    ui.separator();

    match app.extension_panel_view {
        ExtensionPanelView::Generate => extension_generate_panel(app, ui),
        ExtensionPanelView::Marketplace => extension_marketplace_panel(app, ui),
        ExtensionPanelView::Installed => extension_installed_panel(app, ui),
    }
}

fn extension_generate_panel(app: &mut AetherApp, ui: &mut egui::Ui) {
    let busy = app.extension_busy();
    ui.add_enabled(
        !busy,
        egui::TextEdit::multiline(&mut app.extension_intent)
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .hint_text("Describe an extension"),
    );
    let can_generate = !busy
        && !app.extension_intent.trim().is_empty()
        && !app.workspace.is_dirty()
        && !app.workspace.has_pending_agent_changes();
    if ui
        .add_enabled(can_generate, egui::Button::new("Generate recipe"))
        .clicked()
    {
        app.generate_extension();
    }
    if busy {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Extension operation in progress");
        });
    }

    if let Some(recipe) = app.extension_candidate.clone() {
        ui.separator();
        ui.strong("Approval required");
        if let Some(source) = app.extension_candidate_source.clone() {
            ui.label(format!("Adapted from {} ({})", source.name, source.id));
            ui.monospace(format!("Listing SHA-256 {}", source.listing_digest));
            ui.monospace(format!("Recipe SHA-256 {}", source.recipe_digest));
            ui.label(format!(
                "{} exact-digest approval(s)",
                source.approved_review_count()
            ));
            match source.capability_delta(&recipe) {
                Ok(delta) if delta.is_empty() => {
                    ui.colored_label(
                        Color32::from_rgb(0x4E, 0xC9, 0xB0),
                        "Capability scope unchanged from reviewed reference",
                    );
                }
                Ok(delta) => {
                    ui.strong("Capability changes from reviewed reference");
                    for capability in delta.added {
                        ui.colored_label(
                            Color32::from_rgb(0xF4, 0x87, 0x71),
                            format!("+ {}", extension_capability_label(&capability)),
                        );
                    }
                    for capability in delta.removed {
                        ui.label(format!("- {}", extension_capability_label(&capability)));
                    }
                }
                Err(error) => {
                    ui.colored_label(
                        Color32::from_rgb(0xF4, 0x87, 0x71),
                        format!("Invalid marketplace adaptation: {error}"),
                    );
                }
            }
            ui.separator();
        }
        ui.label(&recipe.name);
        ui.weak(&recipe.description);
        ui.monospace(&recipe.id);
        match recipe.digest() {
            Ok(digest) => ui.monospace(format!("SHA-256 {digest}")),
            Err(error) => ui.colored_label(
                Color32::from_rgb(0xF4, 0x87, 0x71),
                format!("Invalid recipe: {error}"),
            ),
        };
        ui.label("Requested capabilities");
        if recipe.capabilities.is_empty() {
            ui.weak("None");
        } else {
            for capability in &recipe.capabilities {
                ui.label(format!("• {}", extension_capability_label(capability)));
            }
        }
        ui.label(format!(
            "{} contribution(s), {} project projection(s)",
            recipe.contributions.len(),
            recipe.projections.len()
        ));
        ui.collapsing("Exact recipe JSON", |ui| {
            egui::ScrollArea::vertical()
                .id_salt("extension_candidate_json")
                .max_height(240.0)
                .show(ui, |ui| match recipe.to_json_pretty() {
                    Ok(json) => {
                        ui.monospace(json);
                    }
                    Err(error) => {
                        ui.colored_label(Color32::from_rgb(0xF4, 0x87, 0x71), error.to_string());
                    }
                });
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Approve and install"))
                .on_hover_text("Grant exactly these capabilities to this recipe digest")
                .clicked()
            {
                app.approve_extension();
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Dismiss"))
                .clicked()
            {
                app.extension_candidate = None;
                app.extension_candidate_source = None;
            }
        });
    }
}

fn extension_marketplace_panel(app: &mut AetherApp, ui: &mut egui::Ui) {
    let busy = app.extension_busy();
    ui.add_enabled(
        !busy,
        egui::TextEdit::singleline(&mut app.marketplace_query)
            .desired_width(f32::INFINITY)
            .hint_text("Search extensions"),
    );
    let catalog_digest = app.marketplace_catalog.digest();
    ui.horizontal_wrapped(|ui| {
        ui.weak(format!(
            "{} · {} listing(s)",
            app.marketplace_catalog.name,
            app.marketplace_catalog.listings.len()
        ));
        if let Ok(digest) = &catalog_digest {
            ui.monospace(format!("SHA-256 {}", &digest[..12]));
        }
    });
    if let Err(error) = catalog_digest {
        ui.colored_label(
            Color32::from_rgb(0xF4, 0x87, 0x71),
            format!("Invalid catalog: {error}"),
        );
        return;
    }

    let records = match app.workspace.extension_records() {
        Ok(records) => records,
        Err(error) => {
            ui.colored_label(
                Color32::from_rgb(0xF4, 0x87, 0x71),
                format!("Could not read installed extensions: {error}"),
            );
            return;
        }
    };
    let mut installed = std::collections::BTreeSet::new();
    for record in records {
        match record {
            Ok(record) => {
                installed.insert(record.recipe.id);
            }
            Err(error) => {
                ui.colored_label(
                    Color32::from_rgb(0xF4, 0x87, 0x71),
                    format!("Corrupt extension record: {error}"),
                );
                return;
            }
        }
    }
    let listings = app
        .marketplace_catalog
        .search(&app.marketplace_query)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    if listings.is_empty() {
        ui.weak("No matching extensions");
        return;
    }

    let mut adapt = None;
    egui::ScrollArea::vertical()
        .id_salt("marketplace_listings")
        .show(ui, |ui| {
            for listing in listings {
                ui.separator();
                ui.strong(&listing.name);
                ui.weak(&listing.summary);
                ui.monospace(&listing.id);
                ui.horizontal_wrapped(|ui| {
                    for tag in &listing.tags {
                        ui.label(tag);
                    }
                });
                ui.label(format!(
                    "{} exact-digest approval(s)",
                    listing.approved_review_count()
                ));
                ui.collapsing("Review and reference recipe", |ui| {
                    ui.monospace(format!("Listing SHA-256 {}", listing.listing_digest));
                    ui.monospace(format!("SHA-256 {}", listing.recipe_digest));
                    for review in &listing.reviews {
                        ui.label(format!(
                            "{:?} by {}: {}",
                            review.decision, review.reviewer, review.evidence
                        ));
                    }
                    ui.separator();
                    match listing.reference_recipe.to_json_pretty() {
                        Ok(json) => {
                            ui.monospace(json);
                        }
                        Err(error) => {
                            ui.colored_label(
                                Color32::from_rgb(0xF4, 0x87, 0x71),
                                error.to_string(),
                            );
                        }
                    }
                });
                let is_installed = installed.contains(&listing.id);
                if ui
                    .add_enabled(
                        !busy && !is_installed,
                        egui::Button::new(if is_installed {
                            "Installed"
                        } else {
                            "Adapt to project"
                        }),
                    )
                    .on_hover_text("Regenerate this reviewed intent for the current semantic graph")
                    .clicked()
                {
                    adapt = Some(listing.id.clone());
                }
            }
        });
    if let Some(listing_id) = adapt {
        app.adapt_marketplace_listing(&listing_id);
    }
}

fn extension_installed_panel(app: &mut AetherApp, ui: &mut egui::Ui) {
    let busy = app.extension_busy();
    let records = match app.workspace.extension_records() {
        Ok(records) => records,
        Err(error) => {
            ui.colored_label(
                Color32::from_rgb(0xF4, 0x87, 0x71),
                format!("Could not read extensions: {error}"),
            );
            return;
        }
    };
    if records.is_empty() {
        ui.weak("No extensions installed");
    }

    let mut pending = None;
    for record in records {
        let record = match record {
            Ok(record) => record,
            Err(error) => {
                ui.colored_label(
                    Color32::from_rgb(0xF4, 0x87, 0x71),
                    format!("Corrupt extension record: {error}"),
                );
                continue;
            }
        };
        let extension_id = record.recipe.id.clone();
        let enabled = record.state == ExtensionState::Enabled;
        ui.collapsing(
            format!("{}  ·  {}", record.recipe.name, extension_id),
            |ui| {
                ui.label(&record.recipe.description);
                ui.monospace(format!("SHA-256 {}", record.grant.recipe_digest));
                let mut next_enabled = enabled;
                if ui
                    .add_enabled(!busy, egui::Checkbox::new(&mut next_enabled, "Enabled"))
                    .changed()
                {
                    pending = Some(PendingExtensionAction::SetEnabled(
                        extension_id.clone(),
                        next_enabled,
                    ));
                }

                if enabled {
                    render_extension_contributions(
                        app,
                        ui,
                        &extension_id,
                        &record.recipe.contributions,
                        &mut pending,
                    );
                } else {
                    ui.weak("Contributions are disabled");
                }

                let confirming =
                    app.extension_remove_confirmation.as_deref() == Some(extension_id.as_str());
                if confirming {
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            Color32::from_rgb(0xF4, 0x87, 0x71),
                            "Restore projections and remove?",
                        );
                        if ui.add_enabled(!busy, egui::Button::new("Remove")).clicked() {
                            pending = Some(PendingExtensionAction::Remove(extension_id.clone()));
                        }
                        if ui.button("Cancel").clicked() {
                            app.extension_remove_confirmation = None;
                        }
                    });
                } else if ui
                    .add_enabled(!busy, egui::Button::new("Remove extension"))
                    .clicked()
                {
                    app.extension_remove_confirmation = Some(extension_id.clone());
                }
            },
        );
    }

    if !app.extension_action_output.is_empty() {
        ui.separator();
        ui.strong("Output");
        egui::ScrollArea::vertical()
            .id_salt("extension_action_output")
            .max_height(220.0)
            .show(ui, |ui| {
                ui.monospace(&app.extension_action_output);
            });
    }

    match pending {
        Some(PendingExtensionAction::AskGraph(question)) => {
            app.ask_extension_graph(&question);
        }
        Some(PendingExtensionAction::OpenFile(path, line)) => {
            app.open_extension_file(&path, line);
        }
        Some(PendingExtensionAction::RunValidation(extension_id, contribution_id)) => {
            app.run_extension_command(extension_id, contribution_id);
        }
        Some(PendingExtensionAction::SetEnabled(extension_id, enabled)) => {
            app.set_extension_enabled(extension_id, enabled);
        }
        Some(PendingExtensionAction::Remove(extension_id)) => {
            app.remove_extension(extension_id);
        }
        None => {}
    }
}

fn render_extension_contributions(
    app: &AetherApp,
    ui: &mut egui::Ui,
    extension_id: &str,
    contributions: &[Contribution],
    pending: &mut Option<PendingExtensionAction>,
) {
    for contribution in contributions {
        match contribution {
            Contribution::Panel {
                title,
                location,
                view,
                ..
            } => {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.strong(title);
                    ui.weak(panel_location_label(*location));
                });
                match view {
                    PanelView::Markdown { content } => {
                        ui.label(content);
                    }
                    PanelView::GraphQuery { question } => {
                        let query = aether_graph::parse_query(question);
                        let answer = app
                            .workspace
                            .graph()
                            .lock()
                            .unwrap()
                            .answer_query(&query)
                            .display();
                        ui.monospace(answer);
                    }
                    PanelView::File { path } => {
                        match app.workspace.read_extension_file(path) {
                            Ok(contents) => ui.monospace(contents),
                            Err(error) => ui.colored_label(
                                Color32::from_rgb(0xF4, 0x87, 0x71),
                                error.to_string(),
                            ),
                        };
                    }
                };
            }
            Contribution::Command {
                id, title, action, ..
            } => {
                if ui
                    .add_enabled(!app.extension_busy(), egui::Button::new(title))
                    .clicked()
                {
                    *pending = Some(match action {
                        CommandAction::AskGraph { question } => {
                            PendingExtensionAction::AskGraph(question.clone())
                        }
                        CommandAction::OpenFile { path, line } => {
                            PendingExtensionAction::OpenFile(path.clone(), *line)
                        }
                        CommandAction::RunValidation { .. } => {
                            PendingExtensionAction::RunValidation(
                                extension_id.to_string(),
                                id.clone(),
                            )
                        }
                    });
                }
            }
        }
    }
}

fn extension_capability_label(capability: &Capability) -> String {
    match capability {
        Capability::ReadGraph => "Read semantic graph".into(),
        Capability::WriteGraph { namespaces } => {
            format!("Write graph: {}", namespaces.join(", "))
        }
        Capability::ReadProject { paths } => {
            format!("Read project: {}", paths.join(", "))
        }
        Capability::WriteProject { paths } => {
            format!("Write project: {}", paths.join(", "))
        }
        Capability::RunValidation { programs } => {
            format!("Run validation: {}", programs.join(", "))
        }
        Capability::Network { hosts } => {
            format!("Network: {}", hosts.join(", "))
        }
        Capability::ContributeUi => "Contribute UI".into(),
    }
}

fn panel_location_label(location: PanelLocation) -> &'static str {
    match location {
        PanelLocation::Left => "left",
        PanelLocation::Right => "right",
        PanelLocation::Bottom => "bottom",
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
