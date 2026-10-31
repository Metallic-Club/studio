//! Theme for the VFX editor: the shared Metallic palette, fonts and `Visuals` from
//! `metallic_ui_theme`, plus the few sizes only this editor uses.
//!
//! Nothing here names a colour of its own. The [`color`] functions read the active theme
//! (Graphite, Titanium or Light) every time they are called, so a theme switch shows at once.
//! [`apply`] installs the theme the creator saved in Studio Settings (Graphite when none).

use egui::{FontId, Margin};
use metallic_ui_theme::fonts;

pub use metallic_ui_theme::{R_LG, R_MD, R_SM};

/// Colour tokens of the active theme.
pub mod color {
    use egui::Color32;
    use metallic_ui_theme as shared;

    /// Title bar, status bar, text inputs, segments (`bg-deep`).
    pub fn bg_deep() -> Color32 {
        shared::bg_deep()
    }
    /// Window background (`bg-base`).
    pub fn bg_base() -> Color32 {
        shared::bg_base()
    }
    /// Docked panels (`bg-panel`).
    pub fn bg_panel() -> Color32 {
        shared::bg_panel()
    }
    /// Buttons, popovers, cards (`bg-raised`).
    pub fn bg_raised() -> Color32 {
        shared::bg_raised()
    }
    /// Hover fill (`bg-hover`).
    pub fn bg_hover() -> Color32 {
        shared::bg_hover()
    }
    /// Pressed, open, selected segment (`bg-active`).
    pub fn bg_active() -> Color32 {
        shared::bg_active()
    }
    /// Selected rows.
    pub fn select() -> Color32 {
        shared::select()
    }
    /// Rows related to the selection.
    pub fn select_weak() -> Color32 {
        shared::select_weak()
    }
    /// Separators and grid lines.
    pub fn line() -> Color32 {
        shared::line()
    }
    /// Popover borders.
    pub fn line_strong() -> Color32 {
        shared::line_strong()
    }
    /// Primary text (`text-1`).
    pub fn text() -> Color32 {
        shared::text_1()
    }
    /// Labels (`text-2`).
    pub fn text_2() -> Color32 {
        shared::text_2()
    }
    /// Hints and meta (`text-3`).
    pub fn text_3() -> Color32 {
        shared::text_3()
    }
    /// Icons (silver).
    pub fn icon() -> Color32 {
        shared::icon()
    }
    /// The one accent.
    pub fn accent() -> Color32 {
        shared::accent()
    }
    /// Text on accent fills.
    pub fn on_accent() -> Color32 {
        shared::on_accent()
    }
    /// Errors.
    pub fn error() -> Color32 {
        shared::error()
    }
    /// X axis letter (the `error` colour: red).
    pub fn axis_x() -> Color32 {
        shared::error()
    }
    /// Y axis letter (the `success` colour: green).
    pub fn axis_y() -> Color32 {
        shared::success()
    }
    /// Z axis letter (the `info` colour: blue).
    pub fn axis_z() -> Color32 {
        shared::info()
    }
}

/// Property label column width.
pub const LABEL_W: f32 = 104.0;
/// Property row height.
pub const ROW_H: f32 = 26.0;

/// Font id for semibold text at `size` (the shared strong voice).
#[must_use]
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name(fonts::UI_STRONG.into()))
}

/// Installs the theme saved in Studio Settings: bundled fonts, colours, spacing, text sizes.
/// Graphite when no setting is saved.
pub fn apply(ctx: &egui::Context) {
    metallic_ui_theme::apply_saved(ctx);
}

/// A frame for docked panels (no radius, no shadow).
pub fn panel_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(color::bg_panel())
        .inner_margin(Margin::same(0))
}
