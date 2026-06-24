//! Map `aether-builder` highlight spans into an egui `LayoutJob` for the editor.

use aether_builder::{spans, HlKind, Lang};
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};

fn color(kind: HlKind) -> Color32 {
    match kind {
        HlKind::Keyword => Color32::from_rgb(0xC5, 0x86, 0xC0), // purple
        HlKind::Type => Color32::from_rgb(0x4E, 0xC9, 0xB0),    // teal
        HlKind::Function => Color32::from_rgb(0xDC, 0xDC, 0xAA), // yellow
        HlKind::Str => Color32::from_rgb(0xCE, 0x91, 0x78),     // orange
        HlKind::Number => Color32::from_rgb(0xB5, 0xCE, 0xA8),  // green
        HlKind::Comment => Color32::from_rgb(0x6A, 0x99, 0x55), // dim green
        HlKind::Ident => Color32::from_rgb(0x9C, 0xDC, 0xFE),   // light blue
        HlKind::Punct => Color32::from_rgb(0xD4, 0xD4, 0xD4),   // light gray
        HlKind::Plain => Color32::from_rgb(0xD4, 0xD4, 0xD4),
    }
}

/// Build a syntax-highlighted layout job for `source`.
pub fn layout(source: &str, font: FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    let spans = spans(source, Lang::Rust);

    let mut cursor = 0usize;
    for span in spans {
        // Skip spans that overlap already-emitted text (defensive).
        if span.start < cursor {
            continue;
        }
        // Gap before the token -> default color.
        if span.start > cursor {
            push(&mut job, &source[cursor..span.start], color(HlKind::Plain), font.clone());
        }
        let end = span.end.min(source.len());
        if span.start < end {
            push(&mut job, &source[span.start..end], color(span.kind), font.clone());
        }
        cursor = end;
    }
    if cursor < source.len() {
        push(&mut job, &source[cursor..], color(HlKind::Plain), font.clone());
    }
    job
}

fn push(job: &mut LayoutJob, text: &str, col: Color32, font: FontId) {
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: font,
            color: col,
            ..Default::default()
        },
    );
}
