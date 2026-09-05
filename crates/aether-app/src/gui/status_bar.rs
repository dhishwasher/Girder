use crate::gui::theme::{self, PALETTE, SPACING, TYPOGRAPHY};

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

pub(crate) fn show(
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
            ui.horizontal(|ui| {
                ui.monospace(open_path.unwrap_or("No file open"));
                ui.separator();
                ui.label(format!("Ln {}, Col {}", cursor.line, cursor.column));
                ui.separator();
                if indexing {
                    ui.spinner();
                    ui.colored_label(PALETTE.warning, "Indexing");
                } else {
                    ui.colored_label(PALETTE.success, "Index ready");
                }
                ui.separator();
                ui.label(format!("{node_count} graph nodes"));
                ui.add_space(theme::SPACING.md);
                ui.colored_label(PALETTE.text_muted, format!("{} pt mono", TYPOGRAPHY.editor));
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
