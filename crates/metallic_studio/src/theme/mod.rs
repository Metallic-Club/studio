//! The Metallic Studio look: machined graphite with one restrained violet accent.
//!
//! Every value comes from the shared design tokens (`design/tokens/metallic.tokens.json`, read
//! through [`metallic_tokens::consts`]); nothing here names a colour or a size of its own. The
//! widget states follow `docs/research/2026-10-30-ui-overhaul.md` section 3.7 as far as egui's
//! [`Visuals`] can express them; the raised-plate edges and the cut come with Studio's own widgets.

pub mod fonts;

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Visuals,
    style::{TextCursorStyle, WidgetVisuals},
};
use metallic_tokens::consts as tk;

const fn rgba(c: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied_const(c[0], c[1], c[2], c[3])
}

const fn shadow(s: metallic_tokens::Shadow) -> Shadow {
    Shadow {
        offset: [s.offset[0] as i8, s.offset[1] as i8],
        blur: s.blur as u8,
        spread: s.spread as u8,
        color: rgba(s.color),
    }
}

/// Title bar, text fields, viewport surround.
pub const BG_DEEP: Color32 = rgba(tk::BG_DEEP);
/// Window ground.
pub const BG_BASE: Color32 = rgba(tk::BG_BASE);
/// Docked panels, rail, cards at rest.
pub const BG_PANEL: Color32 = rgba(tk::BG_PANEL);
/// Secondary buttons, selected card, menus.
pub const BG_RAISED: Color32 = rgba(tk::BG_RAISED);
/// Hover fill, toasts.
pub const BG_HOVER: Color32 = rgba(tk::BG_HOVER);
/// Pressed fill, selected segment, keycaps.
pub const BG_ACTIVE: Color32 = rgba(tk::BG_ACTIVE);
/// Selected row.
pub const SELECT: Color32 = rgba(tk::SELECT);
/// Multi-selected row.
pub const SELECT_WEAK: Color32 = rgba(tk::SELECT_WEAK);
/// Dividers.
pub const LINE: Color32 = rgba(tk::LINE);
/// Panel edges.
pub const LINE_STRONG: Color32 = rgba(tk::LINE_STRONG);
/// Input borders (3:1 against their surroundings).
pub const LINE_CONTROL: Color32 = rgba(tk::LINE_CONTROL);
/// Body text.
pub const TEXT_1: Color32 = rgba(tk::TEXT_1);
/// Secondary text.
pub const TEXT_2: Color32 = rgba(tk::TEXT_2);
/// Hints.
pub const TEXT_3: Color32 = rgba(tk::TEXT_3);
/// Disabled text.
pub const TEXT_OFF: Color32 = rgba(tk::TEXT_OFF);
/// Default icon glyph.
pub const ICON: Color32 = rgba(tk::ICON);
/// The one action per view and the selection ring.
pub const ACCENT: Color32 = rgba(tk::ACCENT);
/// Accent under the pointer.
pub const ACCENT_HOVER: Color32 = rgba(tk::ACCENT_HOVER);
/// Accent while pressed.
pub const ACCENT_PRESS: Color32 = rgba(tk::ACCENT_PRESS);
/// Links.
pub const ACCENT_TEXT: Color32 = rgba(tk::ACCENT_TEXT);
/// Text on the accent.
pub const ON_ACCENT: Color32 = rgba(tk::ON_ACCENT);
/// Keyboard focus ring (2 px).
pub const FOCUS: Color32 = rgba(tk::FOCUS);
/// 1 px top edge of raised plates.
pub const HIGHLIGHT: Color32 = rgba(tk::HIGHLIGHT);
/// Error text and icons.
pub const ERROR: Color32 = rgba(tk::ERROR);
/// Error background.
pub const ERROR_BG: Color32 = rgba(tk::ERROR_BG);
/// Warning text and icons.
pub const WARNING: Color32 = rgba(tk::WARNING);
/// Warning background.
pub const WARNING_BG: Color32 = rgba(tk::WARNING_BG);
/// Success text and icons.
pub const SUCCESS: Color32 = rgba(tk::SUCCESS);
/// Success background.
pub const SUCCESS_BG: Color32 = rgba(tk::SUCCESS_BG);
/// Information text and icons.
pub const INFO: Color32 = rgba(tk::INFO);
/// Information background.
pub const INFO_BG: Color32 = rgba(tk::INFO_BG);
/// Destructive buttons.
pub const DANGER: Color32 = rgba(tk::DANGER);
/// Destructive buttons under the pointer.
pub const DANGER_HOVER: Color32 = rgba(tk::DANGER_HOVER);

// The first theme's names, kept for the code that already uses them.
/// Window background (`bg-base`).
pub const BASE: Color32 = BG_BASE;
/// Panels and cards (`bg-panel`).
pub const PANEL: Color32 = BG_PANEL;
/// Deepest background (`bg-deep`).
pub const DEEP: Color32 = BG_DEEP;
/// Body text (`text-1`).
pub const TEXT: Color32 = TEXT_1;
/// Secondary text (`text-2`).
pub const TEXT_DIM: Color32 = TEXT_2;
/// Borders drawn around surfaces (`line-strong`).
pub const BORDER: Color32 = LINE_STRONG;
/// Hovered surface (`bg-hover`).
pub const HOVER: Color32 = BG_HOVER;

/// Keycaps, thumbnails.
pub const R_XS: u8 = tk::R_XS as u8;
/// Buttons, fields.
pub const R_SM: u8 = tk::R_SM as u8;
/// Cards, toasts, menus.
pub const R_MD: u8 = tk::R_MD as u8;
/// Stage, splash, dialogs.
pub const R_LG: u8 = tk::R_LG as u8;

/// Tree, property and list rows.
pub const ROW_HEIGHT: f32 = tk::H_ROW;
/// Secondary buttons and inputs.
pub const CONTROL_HEIGHT: f32 = tk::H_CONTROL;
/// Primary button, toolbar.
pub const CONTROL_HEIGHT_LG: f32 = tk::H_CONTROL_LG;

/// Floating surfaces: menus, popovers, toasts, tooltips (elevation 3).
pub const SHADOW_POP: Shadow = shadow(tk::SHADOW_POP);
/// Dialogs and the splash (elevation 4).
pub const SHADOW_MODAL: Shadow = shadow(tk::SHADOW_MODAL);

/// Archivo's line height (ascender minus descender, no line gap) as a share of the font size.
const ARCHIVO_LINE: f32 = 1.088;

fn state(fill: Color32, border: Stroke, text: Color32) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: border,
        corner_radius: CornerRadius::same(R_SM),
        fg_stroke: Stroke::new(1.0, text),
        expansion: 0.0,
    }
}

/// The Graphite visuals.
pub fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.override_text_color = None;
    v.weak_text_color = Some(TEXT_3);
    v.hyperlink_color = ACCENT_TEXT;
    v.panel_fill = BG_BASE;
    v.faint_bg_color = BG_PANEL;
    v.extreme_bg_color = BG_DEEP;
    v.text_edit_bg_color = Some(BG_DEEP);
    v.code_bg_color = BG_DEEP;
    v.warn_fg_color = WARNING;
    v.error_fg_color = ERROR;
    // Dialogs (level 4) and menus, popups, tooltips (level 3).
    v.window_fill = BG_PANEL;
    v.window_stroke = Stroke::new(1.0, HIGHLIGHT);
    v.window_shadow = SHADOW_MODAL;
    v.window_corner_radius = CornerRadius::same(R_LG);
    v.menu_corner_radius = CornerRadius::same(R_MD);
    v.popup_shadow = SHADOW_POP;
    // Selected rows and text: `select` with `text-1`; a focused input is outlined in `focus`.
    v.selection.bg_fill = SELECT;
    v.selection.stroke = Stroke::new(2.0, FOCUS);
    v.text_cursor = TextCursorStyle {
        stroke: Stroke::new(2.0, TEXT_1),
        ..v.text_cursor
    };

    // Secondary buttons and inputs share egui's widget states: raised fill (inputs keep
    // `bg-deep`), a `line-control` border at rest that brightens to `text-3` under the pointer.
    let w = &mut v.widgets;
    w.noninteractive = state(BG_PANEL, Stroke::new(1.0, LINE), TEXT_1);
    w.inactive = state(BG_RAISED, Stroke::new(1.0, LINE_CONTROL), TEXT_1);
    w.hovered = state(BG_HOVER, Stroke::new(1.0, TEXT_3), TEXT_1);
    w.active = state(BG_ACTIVE, Stroke::new(1.0, TEXT_3), TEXT_1);
    w.open = state(BG_ACTIVE, Stroke::new(1.0, TEXT_3), TEXT_1);
    v
}

/// Installs the Metallic theme (fonts, colours, spacing, text sizes) into `ctx`.
pub fn apply(ctx: &egui::Context) {
    fonts::install(ctx);
    ctx.set_visuals(visuals());
    ctx.all_styles_mut(|style| {
        let s = &mut style.spacing;
        s.item_spacing = egui::vec2(tk::S4, tk::S4);
        // Text plus this padding makes a control `h-control` tall.
        let pad_y = ((CONTROL_HEIGHT - tk::T_BODY_SIZE * ARCHIVO_LINE) / 2.0).round();
        s.button_padding = egui::vec2(tk::S5, pad_y);
        s.interact_size.y = ROW_HEIGHT;
        s.window_margin = Margin::same(tk::S5 as i8);
        s.menu_margin = Margin::same(tk::S3 as i8);
        s.icon_width = tk::S6;
        s.icon_width_inner = tk::S4;
        s.icon_spacing = tk::S4;
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(tk::T_SMALL_SIZE)),
            (TextStyle::Body, FontId::proportional(tk::T_BODY_SIZE)),
            (TextStyle::Button, fonts::strong()),
            (TextStyle::Heading, fonts::title()),
            (
                TextStyle::Monospace,
                FontId::new(tk::T_MONO_SIZE, FontFamily::Monospace),
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
        for text in [
            TEXT_1,
            TEXT_2,
            ACCENT,
            ACCENT_TEXT,
            ERROR,
            WARNING,
            SUCCESS,
            INFO,
        ] {
            for surface in [BG_BASE, BG_PANEL, BG_DEEP, BG_RAISED] {
                let ratio = contrast_ratio(text, surface);
                assert!(
                    ratio >= 4.5,
                    "{text:?} on {surface:?} has contrast {ratio:.2}"
                );
            }
        }
        assert!(contrast_ratio(TEXT_3, BG_RAISED) >= 4.5);
        assert!(contrast_ratio(ON_ACCENT, ACCENT) >= 4.5);
        assert!(contrast_ratio(TEXT_1, SELECT) >= 4.5);
    }

    #[test]
    fn focus_ring_and_input_borders_are_visible() {
        // WCAG 1.4.11: focus indicators and control borders need 3:1 against what surrounds them.
        for surface in [BG_BASE, BG_PANEL, BG_DEEP, BG_HOVER] {
            assert!(contrast_ratio(FOCUS, surface) >= 3.0, "{surface:?}");
        }
        for surface in [BG_PANEL, BG_DEEP] {
            assert!(contrast_ratio(LINE_CONTROL, surface) >= 3.0, "{surface:?}");
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

    #[test]
    fn shadows_and_radii_come_from_the_tokens() {
        assert_eq!(SHADOW_POP.offset, [0, 8]);
        assert_eq!(SHADOW_POP.blur, 24);
        assert_eq!(SHADOW_MODAL.blur, 48);
        assert_eq!((R_XS, R_SM, R_MD, R_LG), (3, 4, 6, 10));
        let v = visuals();
        assert_eq!(v.panel_fill, BG_BASE);
        assert_eq!(v.widgets.inactive.corner_radius, CornerRadius::same(R_SM));
    }
}
