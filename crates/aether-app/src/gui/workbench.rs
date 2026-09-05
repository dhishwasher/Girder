use crate::app::AetherApp;
use crate::gui::status_bar;
use crate::gui::theme::PALETTE;
use crate::panels;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum WorkbenchMode {
    #[default]
    Editor,
    Graph,
}

pub(crate) fn mode_switcher(ui: &mut egui::Ui, mode: &mut WorkbenchMode) {
    ui.selectable_value(mode, WorkbenchMode::Editor, "Editor")
        .on_hover_text("Edit the active source projection");
    ui.selectable_value(mode, WorkbenchMode::Graph, "Graph")
        .on_hover_text("Explore the semantic graph");
}

pub(crate) fn show_panels(app: &mut AetherApp, ctx: &egui::Context) {
    egui::TopBottomPanel::top("title").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading("Girder");
            ui.separator();
            let (n, e) = {
                let g = app.workspace.graph().lock().unwrap();
                (g.node_count(), g.edge_count())
            };
            ui.label(format!("semantic graph: {n} nodes · {e} edges"));
            ui.separator();
            mode_switcher(ui, &mut app.workbench_mode);
            if app.workspace.is_dirty() {
                ui.colored_label(PALETTE.warning, "modified");
            }
            if app.workspace.has_pending_agent_changes() {
                ui.colored_label(PALETTE.success, "agent changes pending");
            }
        });
        ui.horizontal(|ui| {
            ui.label("Project");
            let extension_busy = app.extension_busy();
            ui.add_enabled(
                !extension_busy,
                egui::TextEdit::singleline(&mut app.project_path_input).desired_width(320.0),
            );
            if ui
                .add_enabled(!extension_busy, egui::Button::new("Open"))
                .on_hover_text("Open project directory")
                .clicked()
            {
                app.open_project();
            }
            let has_file = app.workspace.active_file().is_some();
            let pending_agents = app.workspace.has_pending_agent_changes();
            if ui
                .add_enabled(
                    has_file && app.workspace.is_dirty() && !pending_agents && !extension_busy,
                    egui::Button::new("Save"),
                )
                .on_hover_text("Save source and semantic graph")
                .clicked()
            {
                app.save_active_file();
            }
            if ui
                .add_enabled(
                    has_file && app.workspace.is_dirty() && !pending_agents && !extension_busy,
                    egui::Button::new("Discard"),
                )
                .on_hover_text("Discard unsaved editor changes")
                .clicked()
            {
                app.discard_active_changes();
            }
            if ui
                .add_enabled(
                    !app.workspace.is_dirty()
                        && !pending_agents
                        && app.swarm_rx.is_none()
                        && !extension_busy,
                    egui::Button::new("Reload"),
                )
                .on_hover_text("Re-index project from disk")
                .clicked()
            {
                app.reload_project();
            }
            ui.separator();
            if app.workspace_status_is_error {
                ui.colored_label(PALETTE.error, &app.workspace_status);
            } else {
                ui.label(&app.workspace_status);
            }
        });
    });

    status_bar::panel(app, ctx);

    let available_width = ctx.available_rect().width();
    let workspace_max = (available_width * 0.40).clamp(300.0, 520.0);
    let agents_max = (available_width * 0.32).clamp(240.0, 440.0);

    egui::SidePanel::left("workspace")
        .resizable(true)
        .default_width(360.0)
        .min_width(240.0)
        .max_width(workspace_max)
        .show(ctx, |ui| panels::workspace_panel(app, ui));

    egui::SidePanel::right("agents")
        .resizable(true)
        .default_width(320.0)
        .min_width(240.0)
        .max_width(agents_max)
        .show(ctx, |ui| panels::agents_panel(app, ui));

    egui::TopBottomPanel::bottom("debugger")
        .resizable(true)
        .default_height(200.0)
        .show(ctx, |ui| panels::debugger_panel(app, ui));

    egui::CentralPanel::default().show(ctx, |ui| match app.workbench_mode {
        WorkbenchMode::Editor => panels::editor_panel(app, ui),
        WorkbenchMode::Graph => panels::graph_panel(app, ui),
    });
}
