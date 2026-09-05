use aether_builder::HlKind;
use aether_graph::{EdgeKind, NodeKind};
use egui::style::WidgetVisuals;
use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
};
use std::sync::Arc;

const MONO_FONT_NAME: &str = "Girder DejaVu Sans Mono";
const MONO_FONT_BYTES: &[u8] = include_bytes!("../../../../assets/fonts/DejaVuSansMono.ttf");

#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub background: Color32,
    pub surface: Color32,
    pub surface_raised: Color32,
    pub surface_hovered: Color32,
    pub surface_active: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub text_strong: Color32,
    pub accent: Color32,
    pub accent_hovered: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,
    pub impact: Color32,
    pub impact_secondary: Color32,
    pub syntax_keyword: Color32,
    pub syntax_type: Color32,
    pub syntax_function: Color32,
    pub syntax_string: Color32,
    pub syntax_number: Color32,
    pub syntax_comment: Color32,
    pub syntax_identifier: Color32,
    pub syntax_plain: Color32,
    pub graph_dependency: Color32,
    pub graph_extension: Color32,
}

pub(crate) const PALETTE: Palette = Palette {
    background: Color32::from_rgb(0x0E, 0x11, 0x16),
    surface: Color32::from_rgb(0x17, 0x1B, 0x22),
    surface_raised: Color32::from_rgb(0x20, 0x25, 0x2E),
    surface_hovered: Color32::from_rgb(0x2A, 0x31, 0x3C),
    surface_active: Color32::from_rgb(0x32, 0x3B, 0x48),
    border: Color32::from_rgb(0x35, 0x3D, 0x49),
    border_strong: Color32::from_rgb(0x6E, 0x7B, 0x8E),
    text: Color32::from_rgb(0xD4, 0xD8, 0xDF),
    text_muted: Color32::from_rgb(0x91, 0x9B, 0xA8),
    text_strong: Color32::from_rgb(0xF2, 0xF4, 0xF7),
    accent: Color32::from_rgb(0x56, 0x9C, 0xD6),
    accent_hovered: Color32::from_rgb(0x72, 0xB7, 0xEC),
    success: Color32::from_rgb(0x4E, 0xC9, 0xB0),
    warning: Color32::from_rgb(0xE5, 0xC0, 0x7B),
    error: Color32::from_rgb(0xF4, 0x87, 0x71),
    impact: Color32::from_rgb(0xFF, 0xA0, 0x30),
    impact_secondary: Color32::from_rgb(0xDC, 0x4A, 0x2A),
    syntax_keyword: Color32::from_rgb(0xC5, 0x86, 0xC0),
    syntax_type: Color32::from_rgb(0x4E, 0xC9, 0xB0),
    syntax_function: Color32::from_rgb(0xDC, 0xDC, 0xAA),
    syntax_string: Color32::from_rgb(0xCE, 0x91, 0x78),
    syntax_number: Color32::from_rgb(0xB5, 0xCE, 0xA8),
    syntax_comment: Color32::from_rgb(0x6A, 0x99, 0x55),
    syntax_identifier: Color32::from_rgb(0x9C, 0xDC, 0xFE),
    syntax_plain: Color32::from_rgb(0xD4, 0xD4, 0xD4),
    graph_dependency: Color32::from_rgb(0x80, 0x80, 0x80),
    graph_extension: Color32::from_rgb(0xD7, 0xBA, 0x7D),
};

#[derive(Clone, Copy)]
pub(crate) struct SpacingScale {
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
}

pub(crate) const SPACING: SpacingScale = SpacingScale {
    xs: 4.0,
    sm: 8.0,
    md: 12.0,
    lg: 16.0,
    xl: 24.0,
};

#[derive(Clone, Copy)]
pub(crate) struct RadiusScale {
    pub small: u8,
    pub medium: u8,
    pub large: u8,
}

pub(crate) const RADII: RadiusScale = RadiusScale {
    small: 2,
    medium: 4,
    large: 8,
};

#[derive(Clone, Copy)]
pub(crate) struct Typography {
    pub small: f32,
    pub body: f32,
    pub button: f32,
    pub heading: f32,
    pub editor: f32,
    pub editor_line_height: f32,
    pub graph_label: f32,
    pub graph_label_selected: f32,
}

pub(crate) const TYPOGRAPHY: Typography = Typography {
    small: 11.0,
    body: 13.0,
    button: 13.0,
    heading: 18.0,
    editor: 14.0,
    editor_line_height: 21.0,
    graph_label: 10.5,
    graph_label_selected: 12.0,
};

pub(crate) fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        MONO_FONT_NAME.to_owned(),
        Arc::new(FontData::from_static(MONO_FONT_BYTES)),
    );
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, MONO_FONT_NAME.to_owned());
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(TYPOGRAPHY.small));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(TYPOGRAPHY.body));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(TYPOGRAPHY.button));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(TYPOGRAPHY.heading));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(TYPOGRAPHY.editor));

    style.spacing.item_spacing = egui::vec2(SPACING.sm, SPACING.xs);
    style.spacing.window_margin = egui::Margin::same(SPACING.sm as i8);
    style.spacing.menu_margin = egui::Margin::same(SPACING.sm as i8);
    style.spacing.button_padding = egui::vec2(SPACING.sm, SPACING.xs);
    style.spacing.indent = SPACING.lg;
    style.spacing.interact_size.y = SPACING.xl;
    style.spacing.icon_spacing = SPACING.xs;
    style.spacing.menu_spacing = SPACING.xs;

    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(PALETTE.text);
    visuals.selection.bg_fill = PALETTE.accent.gamma_multiply(0.45);
    visuals.selection.stroke = Stroke::new(1.0_f32, PALETTE.accent_hovered);
    visuals.hyperlink_color = PALETTE.accent_hovered;
    visuals.faint_bg_color = PALETTE.surface_raised;
    visuals.extreme_bg_color = PALETTE.background;
    visuals.code_bg_color = PALETTE.surface_raised;
    visuals.warn_fg_color = PALETTE.warning;
    visuals.error_fg_color = PALETTE.error;
    visuals.window_corner_radius = CornerRadius::same(RADII.large);
    visuals.menu_corner_radius = CornerRadius::same(RADII.medium);
    visuals.window_fill = PALETTE.surface;
    visuals.window_stroke = Stroke::new(1.0_f32, PALETTE.border);
    visuals.panel_fill = PALETTE.surface;
    visuals.widgets.noninteractive = widget(
        PALETTE.surface,
        PALETTE.surface,
        PALETTE.border,
        PALETTE.text,
        RADII.small,
        0.0,
    );
    visuals.widgets.inactive = widget(
        PALETTE.surface_raised,
        PALETTE.surface_raised,
        PALETTE.border,
        PALETTE.text,
        RADII.medium,
        0.0,
    );
    visuals.widgets.hovered = widget(
        PALETTE.surface_hovered,
        PALETTE.surface_hovered,
        PALETTE.accent,
        PALETTE.text_strong,
        RADII.medium,
        1.0,
    );
    visuals.widgets.active = widget(
        PALETTE.surface_active,
        PALETTE.surface_active,
        PALETTE.accent_hovered,
        PALETTE.text_strong,
        RADII.medium,
        1.0,
    );
    visuals.widgets.open = widget(
        PALETTE.surface_raised,
        PALETTE.surface,
        PALETTE.border_strong,
        PALETTE.text_strong,
        RADII.medium,
        0.0,
    );
    ctx.set_style(style);
    ctx.set_visuals(visuals);
}

fn widget(
    weak_bg_fill: Color32,
    bg_fill: Color32,
    border: Color32,
    foreground: Color32,
    radius: u8,
    expansion: f32,
) -> WidgetVisuals {
    WidgetVisuals {
        weak_bg_fill,
        bg_fill,
        bg_stroke: Stroke::new(1.0_f32, border),
        corner_radius: CornerRadius::same(radius),
        fg_stroke: Stroke::new(1.0_f32, foreground),
        expansion,
    }
}

pub(crate) fn editor_font() -> FontId {
    FontId::monospace(TYPOGRAPHY.editor)
}

pub(crate) fn graph_label_font(selected: bool) -> FontId {
    FontId::proportional(if selected {
        TYPOGRAPHY.graph_label_selected
    } else {
        TYPOGRAPHY.graph_label
    })
}

pub(crate) fn syntax_color(kind: HlKind) -> Color32 {
    match kind {
        HlKind::Keyword => PALETTE.syntax_keyword,
        HlKind::Type => PALETTE.syntax_type,
        HlKind::Function => PALETTE.syntax_function,
        HlKind::Str => PALETTE.syntax_string,
        HlKind::Number => PALETTE.syntax_number,
        HlKind::Comment => PALETTE.syntax_comment,
        HlKind::Ident => PALETTE.syntax_identifier,
        HlKind::Punct | HlKind::Plain => PALETTE.syntax_plain,
    }
}

pub(crate) fn node_color(kind: NodeKind) -> Color32 {
    match kind {
        NodeKind::Module => PALETTE.accent,
        NodeKind::Function => PALETTE.syntax_function,
        NodeKind::Type => PALETTE.syntax_type,
        NodeKind::Field => PALETTE.syntax_identifier,
        NodeKind::Concept => PALETTE.syntax_keyword,
        NodeKind::Dependency => PALETTE.graph_dependency,
        NodeKind::Extension => PALETTE.graph_extension,
        NodeKind::ExtensionContribution => PALETTE.syntax_number,
    }
}

pub(crate) fn edge_color(kind: EdgeKind) -> Color32 {
    match kind {
        EdgeKind::Calls => PALETTE.syntax_function,
        EdgeKind::Inherits => PALETTE.syntax_type,
        EdgeKind::DataFlow => PALETTE.accent,
        EdgeKind::Contains => PALETTE.border_strong,
        EdgeKind::SemanticSimilar => PALETTE.syntax_keyword,
        EdgeKind::Impacts => PALETTE.syntax_string,
        EdgeKind::Contributes => PALETTE.graph_extension,
    }
}

pub(crate) fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}
