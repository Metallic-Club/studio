//! Studio theme tokens (from `docs/research/2026-10-30-studio-ui-design.md`, section 5.13) and fonts.
//!
//! These mirror the values `metallic_studio::theme` will own once Task 9 lands; when it does, this module
//! should re-export from there instead of keeping a copy.

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke,
    TextStyle, Vec2, Visuals,
};
use std::sync::Arc;

/// Colour tokens.
pub mod color {
    use egui::Color32;
    /// Title bar, status bar, text inputs, segments.
    pub const BG_DEEP: Color32 = Color32::from_rgb(0x11, 0x10, 0x15);
    /// Window background.
    pub const BG_BASE: Color32 = Color32::from_rgb(0x16, 0x15, 0x1b);
    /// Docked panels.
    pub const BG_PANEL: Color32 = Color32::from_rgb(0x1c, 0x1b, 0x22);
    /// Buttons, popovers, cards.
    pub const BG_RAISED: Color32 = Color32::from_rgb(0x24, 0x22, 0x2b);
    /// Hover fill.
    pub const BG_HOVER: Color32 = Color32::from_rgb(0x2c, 0x2a, 0x34);
    /// Pressed, open, selected segment.
    pub const BG_ACTIVE: Color32 = Color32::from_rgb(0x35, 0x32, 0x3e);
    /// Selected rows.
    pub const SELECT: Color32 = Color32::from_rgb(0x3a, 0x33, 0x54);
    /// Rows related to the selection.
    pub const SELECT_WEAK: Color32 = Color32::from_rgb(0x2a, 0x26, 0x38);
    /// Separators and grid lines.
    pub const LINE: Color32 = Color32::from_rgb(0x2b, 0x29, 0x33);
    /// Popover borders.
    pub const LINE_STRONG: Color32 = Color32::from_rgb(0x3b, 0x38, 0x45);
    /// Primary text.
    pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe4, 0xea);
    /// Labels.
    pub const TEXT_2: Color32 = Color32::from_rgb(0xae, 0xaa, 0xb6);
    /// Hints and meta.
    pub const TEXT_3: Color32 = Color32::from_rgb(0x8e, 0x8a, 0x97);
    /// Icons (silver).
    pub const ICON: Color32 = Color32::from_rgb(0xbd, 0xba, 0xc4);
    /// The one accent (violet).
    pub const ACCENT: Color32 = Color32::from_rgb(0x9d, 0x8b, 0xe0);
    /// Accent hover.
    pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0xab, 0x9b, 0xe6);
    /// Text on accent fills.
    pub const ON_ACCENT: Color32 = Color32::from_rgb(0x11, 0x10, 0x15);
    /// Errors.
    pub const ERROR: Color32 = Color32::from_rgb(0xec, 0x7a, 0x72);
    /// Warnings.
    pub const WARNING: Color32 = Color32::from_rgb(0xe0, 0xad, 0x55);
    /// Play state.
    pub const SUCCESS: Color32 = Color32::from_rgb(0x62, 0xbf, 0x91);
    /// X axis letter.
    pub const AXIS_X: Color32 = Color32::from_rgb(0xe5, 0x67, 0x5e);
    /// Y axis letter.
    pub const AXIS_Y: Color32 = Color32::from_rgb(0x79, 0xc4, 0x6a);
    /// Z axis letter.
    pub const AXIS_Z: Color32 = Color32::from_rgb(0x5d, 0x95, 0xea);
}

/// Small radius: rows, fields, chips, keys.
pub const R_SM: u8 = 3;
/// Medium radius: buttons, segments.
pub const R_MD: u8 = 5;
/// Large radius: popovers, cards.
pub const R_LG: u8 = 8;
/// Property label column width.
pub const LABEL_W: f32 = 104.0;
/// Property row height.
pub const ROW_H: f32 = 26.0;

/// Family name of the semibold UI face.
pub const SEMIBOLD: &str = "semibold";

/// Font id for semibold text at `size`.
#[must_use]
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

/// Candidate system fonts: regular, semibold, mono. IBM Plex replaces these once the owner approves
/// OFL-1.1 (design doc section 8); until then nothing is redistributed.
const FONT_CANDIDATES: [(&str, &str, &str); 3] = [
    (
        "C:/Windows/Fonts/segoeui.ttf",
        "C:/Windows/Fonts/seguisb.ttf",
        "C:/Windows/Fonts/consola.ttf",
    ),
    (
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
        "/System/Library/Fonts/Menlo.ttc",
    ),
    (
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    ),
];

/// Loads system UI fonts. Returns `false` when none were found (text will not render).
pub fn install_fonts(ctx: &egui::Context) -> bool {
    let read = |p: &str| std::fs::read(p).ok();
    for (regular, bold, mono) in FONT_CANDIDATES {
        let Some(reg) = read(regular) else {
            continue;
        };
        let bold = read(bold).unwrap_or_else(|| reg.clone());
        let mono = read(mono).unwrap_or_else(|| reg.clone());
        let mut fonts = FontDefinitions::empty();
        fonts
            .font_data
            .insert("ui".into(), Arc::new(FontData::from_owned(reg)));
        fonts
            .font_data
            .insert("ui-semibold".into(), Arc::new(FontData::from_owned(bold)));
        fonts
            .font_data
            .insert("ui-mono".into(), Arc::new(FontData::from_owned(mono)));
        fonts
            .families
            .insert(FontFamily::Proportional, vec!["ui".into()]);
        fonts
            .families
            .insert(FontFamily::Monospace, vec!["ui-mono".into(), "ui".into()]);
        fonts.families.insert(
            FontFamily::Name(SEMIBOLD.into()),
            vec!["ui-semibold".into(), "ui".into()],
        );
        ctx.set_fonts(fonts);
        return true;
    }
    false
}

/// Applies the Studio theme (dark) to a context.
pub fn apply(ctx: &egui::Context) {
    use color::*;
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.animation_time = 0.14;
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Body, FontId::proportional(13.0)),
            (TextStyle::Button, FontId::proportional(13.0)),
            (TextStyle::Monospace, FontId::monospace(12.0)),
            (TextStyle::Heading, semibold(15.0)),
        ]
        .into();
        let s = &mut style.spacing;
        s.item_spacing = Vec2::new(8.0, 4.0);
        s.button_padding = Vec2::new(10.0, 4.0);
        s.interact_size = Vec2::new(40.0, 22.0);
        s.indent = 18.0;
        s.menu_margin = Margin::same(6);
        s.window_margin = Margin::same(12);
        s.combo_width = 120.0;
        s.slider_width = 120.0;

        let mut v = Visuals::dark();
        v.dark_mode = true;
        v.override_text_color = None;
        v.panel_fill = BG_PANEL;
        v.window_fill = BG_RAISED;
        v.extreme_bg_color = BG_DEEP;
        v.faint_bg_color = BG_RAISED;
        v.code_bg_color = BG_DEEP;
        v.window_stroke = Stroke::new(1.0, LINE_STRONG);
        v.window_corner_radius = CornerRadius::same(R_LG);
        v.menu_corner_radius = CornerRadius::same(R_MD);
        let shadow = Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(115),
        };
        v.window_shadow = shadow;
        v.popup_shadow = shadow;
        v.hyperlink_color = Color32::from_rgb(0xb4, 0xa6, 0xec);
        v.weak_text_color = Some(TEXT_3);
        v.error_fg_color = ERROR;
        v.warn_fg_color = WARNING;
        v.selection.bg_fill = SELECT;
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.text_cursor.stroke = Stroke::new(2.0, ACCENT);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = BG_PANEL;
        w.noninteractive.weak_bg_fill = BG_PANEL;
        w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_2);
        w.noninteractive.corner_radius = CornerRadius::same(R_SM);
        w.inactive.bg_fill = BG_RAISED;
        w.inactive.weak_bg_fill = BG_RAISED;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.inactive.corner_radius = CornerRadius::same(R_SM);
        w.hovered.bg_fill = BG_HOVER;
        w.hovered.weak_bg_fill = BG_HOVER;
        w.hovered.bg_stroke = Stroke::NONE;
        w.hovered.fg_stroke = Stroke::new(1.0, TEXT);
        w.hovered.corner_radius = CornerRadius::same(R_SM);
        w.hovered.expansion = 0.0;
        w.active.bg_fill = BG_ACTIVE;
        w.active.weak_bg_fill = BG_ACTIVE;
        w.active.bg_stroke = Stroke::new(1.0, ACCENT);
        w.active.fg_stroke = Stroke::new(1.0, TEXT);
        w.active.corner_radius = CornerRadius::same(R_SM);
        w.active.expansion = 0.0;
        w.open.bg_fill = BG_ACTIVE;
        w.open.weak_bg_fill = BG_ACTIVE;
        w.open.fg_stroke = Stroke::new(1.0, TEXT);
        w.open.corner_radius = CornerRadius::same(R_SM);
        style.visuals = v;
    });
}

/// A frame for docked panels (no radius, no shadow).
pub fn panel_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(color::BG_PANEL)
        .inner_margin(Margin::same(0))
}
