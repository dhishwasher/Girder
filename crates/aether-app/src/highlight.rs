//! Map `aether-builder` highlight spans into an egui `LayoutJob` for the editor.

use crate::gui::theme;
use aether_builder::{spans, HlKind, Lang};
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};

/// Build a syntax-highlighted layout job for `source`.
pub fn layout(source: &str, language: Lang, font: FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    let spans = spans(source, language);

    let mut cursor = 0usize;
    for span in spans {
        // Skip spans that overlap already-emitted text (defensive).
        if span.start < cursor {
            continue;
        }
        // Gap before the token -> default color.
        if span.start > cursor {
            push(
                &mut job,
                &source[cursor..span.start],
                theme::syntax_color(HlKind::Plain),
                font.clone(),
            );
        }
        let end = span.end.min(source.len());
        if span.start < end {
            push(
                &mut job,
                &source[span.start..end],
                theme::syntax_color(span.kind),
                font.clone(),
            );
        }
        cursor = end;
    }
    if cursor < source.len() {
        push(
            &mut job,
            &source[cursor..],
            theme::syntax_color(HlKind::Plain),
            font.clone(),
        );
    }
    job
}

fn push(job: &mut LayoutJob, text: &str, col: Color32, font: FontId) {
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: font,
            line_height: Some(theme::TYPOGRAPHY.editor_line_height),
            color: col,
            ..Default::default()
        },
    );
}
