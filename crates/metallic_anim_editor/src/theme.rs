//! Colour tokens for the animation editor: a snapshot of the active Metallic palette.
//!
//! The palettes, fonts and egui `Visuals` live in `metallic_ui_theme`, shared with Studio and the
//! VFX editor; nothing here names a colour of its own. [`Theme::current`] reads the active palette
//! (Graphite, Titanium or Light), so build the widgets from it each frame and a theme switch shows
//! at once. [`apply`] installs the theme the creator saved in Studio Settings (Graphite when none).

use egui::{Color32, Stroke};
use metallic_ui_theme as shared;

pub use shared::{R_LG, R_MD, R_SM};

/// Editor colour tokens (names follow the design doc's token table).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// `bg-deep`: canvases, inputs, title bar.
    pub deep: Color32,
    /// `bg-base`: window background.
    pub base: Color32,
    /// `bg-panel`: docked panels, tab bars.
    pub panel: Color32,
    /// `bg-raised`: buttons, chips, popovers.
    pub raised: Color32,
    /// `bg-hover`.
    pub hover: Color32,
    /// `bg-active`: pressed, selected segment.
    pub active: Color32,
    /// `select`: selected rows.
    pub select: Color32,
    /// `select-weak`: rows related to the selection, drop zones.
    pub select_weak: Color32,
    /// `line`: separators and grid lines (decoration only).
    pub line: Color32,
    /// `line-strong`: popover borders, emphasised separators.
    pub line_strong: Color32,
    /// Grid lines fainter than `line`, for dense canvases (halfway from `bg-panel` to `line`).
    pub grid: Color32,
    /// `text-1`: primary text.
    pub text: Color32,
    /// `text-2`: labels, inactive tabs.
    pub text_dim: Color32,
    /// `text-3`: meta, hints (not on hover/active fills).
    pub text_meta: Color32,
    /// `icon`: silver icons, unselected keys, bones.
    pub silver: Color32,
    /// `accent`: selection, focus, primary button, playhead.
    pub accent: Color32,
    /// `accent-hover`.
    pub accent_hover: Color32,
    /// `on-accent`: text and icons on accent fills.
    pub on_accent: Color32,
    /// Accent at low opacity for box-select fills.
    pub accent_soft: Color32,
    /// `error`: errors and the Record keys state.
    pub error: Color32,
    /// `warning`: event markers.
    pub warning: Color32,
    /// X/Y/Z (or R/G/B) curves: `error`, `success`, `info`; always shown with the letter.
    pub axis: [Color32; 3],
}

impl Default for Theme {
    fn default() -> Self {
        Self::current()
    }
}

impl Theme {
    /// The colours of the active Metallic theme.
    pub fn current() -> Theme {
        Self::from_palette(shared::palette())
    }

    /// The colours of `palette`.
    pub fn from_palette(p: &shared::Palette) -> Theme {
        let accent = p.accent;
        Theme {
            deep: p.bg_deep,
            base: p.bg_base,
            panel: p.bg_panel,
            raised: p.bg_raised,
            hover: p.bg_hover,
            active: p.bg_active,
            select: p.select,
            select_weak: p.select_weak,
            line: p.line,
            line_strong: p.line_strong,
            grid: p.bg_panel.lerp_to_gamma(p.line, 0.5),
            text: p.text_1,
            text_dim: p.text_2,
            text_meta: p.text_3,
            silver: p.icon,
            accent,
            accent_hover: p.accent_hover,
            on_accent: p.on_accent,
            accent_soft: Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 51),
            error: p.error,
            warning: p.warning,
            axis: [p.error, p.success, p.info],
        }
    }

    /// A 1 px hairline in the `line` colour.
    pub fn hairline(&self) -> Stroke {
        Stroke::new(1.0, self.line)
    }
}

/// Installs the theme saved in Studio Settings (fonts, colours, spacing, text sizes) and returns
/// the editor tokens for it. Graphite when no setting is saved.
pub fn apply(ctx: &egui::Context) -> Theme {
    shared::apply_saved(ctx);
    Theme::current()
}

/// WCAG contrast ratio between two colours (1 to 21).
pub fn contrast(a: Color32, b: Color32) -> f32 {
    shared::contrast_ratio(a, b) as f32
}
