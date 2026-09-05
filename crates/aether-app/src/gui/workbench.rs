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
