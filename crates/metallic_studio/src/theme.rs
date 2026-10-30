//! The Metallic Studio look: graphite panels with one restrained violet accent.
//!
//! Colours are sampled from `branding/placeholder-icon.png` (within a few units per channel) and
//! will be swapped when the final brand files arrive. No neon, no glow, no gradients.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

/// Window background.
pub const BASE: Color32 = Color32::from_rgb(0x1c, 0x19, 0x22);
/// Panels and cards.
pub const PANEL: Color32 = Color32::from_rgb(0x26, 0x22, 0x2e);
/// Deepest background (sunken areas, text fields).
pub const DEEP: Color32 = Color32::from_rgb(0x14, 0x12, 0x17);
/// The single accent colour (selection, primary button, links).
pub const ACCENT: Color32 = Color32::from_rgb(0x9b, 0x82, 0xd4);
/// Body text.
pub const TEXT: Color32 = Color32::from_rgb(0xd7, 0xd5, 0xdc);
/// Secondary text.
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x9a, 0x97, 0xa3);
/// Hairline borders between surfaces.
pub const BORDER: Color32 = Color32::from_rgb(0x3a, 0x35, 0x44);
/// Hovered surface.
pub const HOVER: Color32 = Color32::from_rgb(0x31, 0x2c, 0x3a);
/// Error text on dark surfaces.
pub const ERROR: Color32 = Color32::from_rgb(0xe0, 0x8a, 0x8a);

/// Installs the Metallic theme (colours, spacing, text sizes) into `ctx`.
pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.panel_fill = BASE;
    v.window_fill = PANEL;
    v.extreme_bg_color = DEEP;
    v.faint_bg_color = PANEL;
    v.code_bg_color = DEEP;
    v.hyperlink_color = ACCENT;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_shadow = egui::Shadow::NONE;
    v.popup_shadow = egui::Shadow::NONE;
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, ACCENT);

    let radius = CornerRadius::same(5);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.bg_fill = PANEL;
    w.inactive.weak_bg_fill = PANEL;
    w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.bg_fill = HOVER;
    w.hovered.weak_bg_fill = HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    w.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    w.active.bg_fill = HOVER;
    w.active.weak_bg_fill = HOVER;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.0, TEXT);
    for state in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        state.corner_radius = radius;
    }
    ctx.set_visuals(v);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.text_styles = [
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(14.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Heading,
                FontId::new(26.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
        ]
        .into();
    });
}

/// WCAG relative luminance of an sRGB colour.
fn luminance(c: Color32) -> f64 {
    let lin = |v: u8| {
        let s = f64::from(v) / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// WCAG contrast ratio between two colours (1.0 to 21.0).
pub fn contrast_ratio(a: Color32, b: Color32) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_text_contrast() {
        for text in [TEXT, TEXT_DIM, ACCENT, ERROR] {
            for surface in [BASE, PANEL, DEEP] {
                let ratio = contrast_ratio(text, surface);
                assert!(
                    ratio >= 4.5,
                    "{text:?} on {surface:?} has contrast {ratio:.2}"
                );
            }
        }
    }

    #[test]
    fn deep_colour_stays_near_the_icon_background() {
        // The icon's background is (20, 18, 25); DEEP must stay within 8 per channel of it.
        let icon_bg = [20i16, 18, 25];
        for (got, want) in [DEEP.r(), DEEP.g(), DEEP.b()].into_iter().zip(icon_bg) {
            assert!((i16::from(got) - want).abs() <= 8);
        }
    }
}
