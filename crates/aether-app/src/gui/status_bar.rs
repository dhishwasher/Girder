use crate::app::AetherApp;
use crate::gui::theme::{PALETTE, SPACING};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CursorPosition {
    line: usize,
    column: usize,
}

impl Default for CursorPosition {
    fn default() -> Self {
        Self { line: 1, column: 1 }
    }
}

impl CursorPosition {
    pub(crate) fn update(&mut self, source: &str, char_offset: usize) {
        self.line = 1;
        self.column = 1;
        for character in source.chars().take(char_offset) {
            if character == '\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
        }
    }
}

pub(crate) fn panel(app: &AetherApp, ctx: &egui::Context) {
    let node_count = app.workspace.graph().lock().unwrap().node_count();
    egui::TopBottomPanel::bottom("status_bar")
        .frame(egui::Frame::NONE)
        .exact_height(SPACING.xl + SPACING.sm)
        .show(ctx, |ui| {
            show(
                ui,
                app.editor_tabs.active_path(),
                app.editor_cursor,
                app.file_tree.is_indexing(),
                node_count,
            );
        });
}

fn show(
    ui: &mut egui::Ui,
    open_path: Option<&str>,
    cursor: CursorPosition,
    indexing: bool,
    node_count: usize,
) {
    egui::Frame::new()
        .fill(PALETTE.background)
        .inner_margin(egui::Margin::symmetric(SPACING.sm as i8, SPACING.xs as i8))
        .show(ui, |ui| {
            ui.style_mut().override_text_style = Some(egui::TextStyle::Small);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("{node_count} graph nodes"));
                ui.separator();
                ui.label(format!("Ln {}, Col {}", cursor.line, cursor.column));
                ui.separator();
                if indexing {
                    ui.add(egui::Spinner::new().size(SPACING.lg));
                    ui.colored_label(PALETTE.warning, "Indexing");
                } else {
                    ui.colored_label(PALETTE.success, "Index ready");
                }
                ui.separator();
                ui.add_space(SPACING.md);
                let path = open_path.unwrap_or("No file open");
                ui.add_sized(
                    [ui.available_width(), SPACING.xl],
                    egui::Label::new(egui::RichText::new(path).color(PALETTE.text_muted))
                        .truncate(),
                )
                .on_hover_text(path);
            });
        });
}

#[cfg(test)]
mod tests {
    use super::CursorPosition;

    #[test]
    fn cursor_position_is_one_based_and_unicode_aware() {
        let mut cursor = CursorPosition::default();
        cursor.update("zero\nλx", 7);
        assert_eq!(cursor.line, 2);
        assert_eq!(cursor.column, 3);
    }
}
