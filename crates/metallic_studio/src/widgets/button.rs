//! Studio's buttons and keycaps, per research doc section 3.7.
//!
//! - **Primary**: the one violet action per view. 36 px tall, `accent` fill, the small cut, a
//!   2 px `on-accent` focus ring 3 to 5 px inside the edge (the cut would clip an outside ring).
//! - **Secondary**: a raised plate (`bg-raised`, highlight top, edge-low bottom, no outline).
//! - **Ghost**: no fill until hovered; `text-2` turning `text-1`.
//! - **Danger**: shaped like secondary, filled `danger`.
//!
//! All of them: hover and press fills fade over `t-fast`, a press moves the button 1 px down,
//! keyboard focus draws a 2 px ring (`focus`, 2 px outside, except on primary), a disabled button
//! is `bg-raised` with `text-off`, and Enter or Space activates the focused button
//! (`Response::clicked`).

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, Galley, Painter, Rect, Response, Sense, Stroke, Ui, Vec2, Widget,
    WidgetInfo, vec2,
};
use metallic_tokens::consts as tk;

use crate::theme::{self, fonts};
use crate::widgets::{cut, plate::Plate};

/// The four button roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The one action of a view (violet, cut).
    Primary,
    /// Everything else that is a button.
    Secondary,
    /// Low-emphasis actions in bars and lists.
    Ghost,
    /// Destructive actions.
    Danger,
}

/// A state to draw regardless of the pointer and focus (for the components sheet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShowAs {
    /// Under the pointer.
    Hovered,
    /// Held down.
    Pressed,
    /// Keyboard focus.
    Focused,
}

/// A Studio button. Add it with `ui.add(...)` (or `ui.add_enabled(false, ...)` to disable it).
#[must_use = "add the button with `ui.add`"]
pub struct Button<'a> {
    kind: Kind,
    text: &'a str,
    icon: Option<char>,
    keycap: Option<&'a str>,
    width: Option<f32>,
    compact: bool,
    show_as: Option<ShowAs>,
    alt: Option<&'a str>,
}

/// Fills and ink for one button state.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Palette {
    rest: Color32,
    hover: Color32,
    press: Color32,
    text: Color32,
    text_hover: Color32,
    icon: Color32,
    keycap_fill: Color32,
    keycap_text: Color32,
}

impl<'a> Button<'a> {
    fn new(kind: Kind, text: &'a str) -> Self {
        Self {
            kind,
            text,
            icon: None,
            keycap: None,
            width: None,
            compact: false,
            show_as: None,
            alt: None,
        }
    }

    /// The one violet action of a view.
    pub fn primary(text: &'a str) -> Self {
        Self::new(Kind::Primary, text)
    }

    /// A raised secondary button.
    pub fn secondary(text: &'a str) -> Self {
        Self::new(Kind::Secondary, text)
    }

    /// A ghost button (no fill at rest).
    pub fn ghost(text: &'a str) -> Self {
        Self::new(Kind::Ghost, text)
    }

    /// A destructive button.
    pub fn danger(text: &'a str) -> Self {
        Self::new(Kind::Danger, text)
    }

    /// A 16 px Phosphor icon before the text.
    pub fn icon(mut self, icon: char) -> Self {
        self.icon = Some(icon);
        self
    }

    /// A keycap with the button's shortcut, after the text (at the right edge when the button is
    /// wider than its content).
    pub fn keycap(mut self, keys: &'a str) -> Self {
        self.keycap = Some(keys);
        self
    }

    /// At least this wide; the content stays left-aligned.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// The accessible name (and tooltip) of an icon-only button.
    pub fn alt(mut self, name: &'a str) -> Self {
        self.alt = Some(name);
        self
    }

    /// 28 px tall (title bar and toolbars).
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// Draws `state` whatever the pointer and keyboard do (the components sheet uses this to show
    /// every state in one picture).
    pub fn show_as(mut self, state: ShowAs) -> Self {
        self.show_as = Some(state);
        self
    }

    fn height(&self) -> f32 {
        match (self.kind, self.compact) {
            (_, true) => tk::H_CONTROL - tk::S2,
            (Kind::Primary, false) => theme::CONTROL_HEIGHT_LG,
            _ => theme::CONTROL_HEIGHT,
        }
    }

    fn pad_x(&self) -> f32 {
        match self.kind {
            Kind::Primary => tk::S6,
            Kind::Secondary | Kind::Danger => tk::S5,
            Kind::Ghost => tk::S4,
        }
    }

    fn radius(&self) -> CornerRadius {
        CornerRadius::same(theme::R_SM)
    }

    fn cut(&self) -> f32 {
        if self.kind == Kind::Primary {
            theme::CUT_SM
        } else {
            0.0
        }
    }

    fn palette(&self, enabled: bool) -> Palette {
        if !enabled {
            let fill = if self.kind == Kind::Ghost {
                Color32::TRANSPARENT
            } else {
                theme::bg_raised()
            };
            return Palette {
                rest: fill,
                hover: fill,
                press: fill,
                text: theme::text_off(),
                text_hover: theme::text_off(),
                icon: theme::text_off(),
                keycap_fill: theme::bg_active().gamma_multiply(0.5),
                keycap_text: theme::text_off(),
            };
        }
        match self.kind {
            Kind::Primary => Palette {
                rest: theme::accent(),
                hover: theme::accent_hover(),
                press: theme::accent_press(),
                text: theme::on_accent(),
                text_hover: theme::on_accent(),
                icon: theme::on_accent(),
                keycap_fill: theme::keycap_on_accent(),
                keycap_text: theme::on_accent(),
            },
            Kind::Secondary => Palette {
                rest: theme::bg_raised(),
                hover: theme::bg_hover(),
                press: theme::bg_active(),
                text: theme::text_1(),
                text_hover: theme::text_1(),
                icon: theme::icon(),
                keycap_fill: theme::bg_active(),
                keycap_text: theme::text_2(),
            },
            Kind::Ghost => Palette {
                rest: Color32::TRANSPARENT,
                hover: theme::bg_hover(),
                press: theme::bg_active(),
                text: theme::text_2(),
                text_hover: theme::text_1(),
                icon: theme::text_2(),
                keycap_fill: theme::bg_active(),
                keycap_text: theme::text_2(),
            },
            Kind::Danger => Palette {
                rest: theme::danger(),
                hover: theme::danger_hover(),
                press: theme::danger(),
                text: theme::on_danger(),
                text_hover: theme::on_danger(),
                icon: theme::on_danger(),
                keycap_fill: theme::keycap_on_accent(),
                keycap_text: theme::on_danger(),
            },
        }
    }
}

impl Widget for Button<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let enabled = ui.is_enabled();
        let colors = self.palette(enabled);
        let painter = ui.painter();
        let text = layout(painter, self.text, fonts::strong(), colors.text);
        let icon = self
            .icon
            .map(|c| layout(painter, &c.to_string(), fonts::icon(16.0), colors.icon));
        let keycap = self
            .keycap
            .map(|k| layout(painter, k, fonts::caption(), colors.keycap_text));

        let gap = tk::S4;
        let pad = self.pad_x();
        let mut content = text.size().x;
        if let Some(icon) = &icon {
            content += icon.size().x;
            if !self.text.is_empty() {
                content += gap;
            }
        }
        if let Some(k) = &keycap {
            content += keycap_size(k).x + gap;
        }
        let width = (content + 2.0 * pad).max(self.width.unwrap_or(0.0)).ceil();
        let (rect, response) = ui.allocate_exact_size(vec2(width, self.height()), Sense::click());
        let name = self.alt.unwrap_or(self.text);
        response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, enabled, name));
        let response = match self.alt {
            Some(alt) if self.text.is_empty() => response.on_hover_text(alt),
            _ => response,
        };
        if !ui.is_rect_visible(rect) {
            return response;
        }

        let ctx = ui.ctx();
        let forced = |s| enabled && self.show_as == Some(s);
        let hovered = forced(ShowAs::Hovered) || (enabled && response.hovered());
        let pressed = forced(ShowAs::Pressed) || (enabled && response.is_pointer_button_down_on());
        let focused = forced(ShowAs::Focused) || (enabled && response.has_focus());
        let hover = ctx.animate_bool_with_time(response.id.with("hover"), hovered, theme::t_fast());
        let press = ctx.animate_bool_with_time(response.id.with("press"), pressed, theme::t_fast());
        let rect = if pressed {
            rect.translate(vec2(0.0, 1.0))
        } else {
            rect
        };
        let fill = colors
            .rest
            .lerp_to_gamma(colors.hover, hover)
            .lerp_to_gamma(colors.press, press);
        let painter = ui.painter();
        let mut plate = Plate::raised()
            .fill(fill)
            .radius(self.radius())
            .cut(self.cut());
        // Primary is flat violet; ghost has no plate at rest; disabled is flat.
        plate.highlight = enabled && matches!(self.kind, Kind::Secondary | Kind::Danger);
        plate.edge_low = plate.highlight;
        if fill.a() > 0 {
            plate.paint(painter, rect);
        }

        let mut x = rect.left() + pad;
        if let Some(icon) = icon {
            let pos = egui::pos2(x, rect.center().y - icon.size().y / 2.0);
            let tint = if self.kind == Kind::Ghost {
                colors.icon.lerp_to_gamma(colors.text_hover, hover)
            } else {
                colors.icon
            };
            x += icon.size().x + gap;
            painter.galley(pos, icon, tint);
        }
        let ink = colors.text.lerp_to_gamma(colors.text_hover, hover);
        let pos = egui::pos2(x, rect.center().y - text.size().y / 2.0);
        painter.galley_with_override_text_color(pos, text, ink);
        if let Some(k) = keycap {
            let size = keycap_size(&k);
            let at = Rect::from_min_size(
                egui::pos2(rect.right() - pad - size.x, rect.center().y - size.y / 2.0),
                size,
            );
            // On a pressed secondary or ghost button the fill is `bg-active` too: lift the cap.
            let cap = if self.kind == Kind::Primary {
                colors.keycap_fill
            } else {
                colors.keycap_fill.lerp_to_gamma(theme::bg_hover(), press)
            };
            paint_keycap(painter, at, k, cap);
        }

        if focused {
            if self.kind == Kind::Primary {
                let inset = 3.0;
                cut::frame(
                    painter,
                    rect.shrink(inset),
                    cut::inset_radius(self.radius(), inset),
                    cut::inset_cut(self.cut(), inset),
                    Stroke::new(2.0, theme::on_accent()),
                );
            } else {
                focus_ring(painter, rect, self.radius());
            }
        }
        response
    }
}

/// The standard focus ring: 2 px `focus`, 2 px outside `rect`.
pub fn focus_ring(painter: &Painter, rect: Rect, radius: CornerRadius) {
    let out = 4.0;
    cut::frame(
        painter,
        rect.expand(out),
        CornerRadius {
            nw: radius.nw.saturating_add(out as u8),
            ne: radius.ne.saturating_add(out as u8),
            sw: radius.sw.saturating_add(out as u8),
            se: radius.se.saturating_add(out as u8),
        },
        0.0,
        Stroke::new(2.0, theme::focus()),
    );
}

fn layout(painter: &Painter, text: &str, font: egui::FontId, color: Color32) -> Arc<Galley> {
    painter.layout_no_wrap(text.to_owned(), font, color)
}

/// Keycap size for a laid-out label: 16 px tall, 5 px side padding, at least 18 px wide.
fn keycap_size(galley: &Galley) -> Vec2 {
    vec2((galley.size().x + 2.0 * 5.0).max(18.0).ceil(), tk::S6)
}

fn paint_keycap(painter: &Painter, rect: Rect, galley: Arc<Galley>, fill: Color32) {
    painter.rect_filled(rect, CornerRadius::same(theme::R_XS), fill);
    let pos = rect.center() - galley.size() / 2.0;
    painter.galley(pos, galley, theme::text_2());
}

/// An inline keycap (`F5`, `Ctrl K`), for help text: `bg-active` with `text-2`.
pub fn keycap(ui: &mut Ui, keys: &str) -> Response {
    let galley = layout(ui.painter(), keys, fonts::caption(), theme::text_2());
    let size = keycap_size(&galley);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_keycap(ui.painter(), rect, galley, theme::bg_active());
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one pass with the theme installed, adding the button from `make` at a fixed place.
    fn pass(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        enabled: bool,
        make: impl Fn() -> Button<'static>,
    ) -> Response {
        let mut out = None;
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        // No renderer here: drop the font atlas upload.
        ctx.run_ui(input, |ui| {
            out = Some(ui.add_enabled(enabled, make()));
        })
        .textures_delta
        .clear();
        out.expect("the pass ran")
    }

    fn themed() -> egui::Context {
        let ctx = egui::Context::default();
        theme::apply(&ctx, theme::ThemeId::Graphite);
        ctx
    }

    #[test]
    fn sizes_follow_the_component_table() {
        let ctx = themed();
        let r = pass(&ctx, vec![], true, || Button::primary("Create game"));
        assert_eq!(r.rect.height(), 36.0);
        let r = pass(&ctx, vec![], true, || Button::secondary("Browse"));
        assert_eq!(r.rect.height(), 32.0);
        let r = pass(&ctx, vec![], true, || Button::ghost("Help").compact());
        assert_eq!(r.rect.height(), 28.0);
        let r = pass(&ctx, vec![], true, || {
            Button::secondary("Open").width(256.0)
        });
        assert_eq!(r.rect.width(), 256.0);
    }

    #[test]
    fn enter_and_space_activate_the_focused_button() {
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        for k in [egui::Key::Enter, egui::Key::Space] {
            let ctx = themed();
            let r = pass(&ctx, vec![], true, || Button::primary("Create game"));
            assert!(!r.clicked());
            r.request_focus();
            let r = pass(&ctx, vec![], true, || Button::primary("Create game"));
            assert!(r.has_focus());
            let r = pass(&ctx, vec![key(k)], true, || Button::primary("Create game"));
            assert!(r.clicked(), "{k:?}");
        }
    }

    #[test]
    fn a_disabled_button_ignores_the_keyboard_and_turns_flat() {
        let ctx = themed();
        let r = pass(&ctx, vec![], false, || Button::primary("Create game"));
        r.request_focus();
        let enter = egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let r = pass(&ctx, vec![enter], false, || Button::primary("Create game"));
        assert!(!r.clicked());
        let p = Button::primary("x").palette(false);
        assert_eq!((p.rest, p.text), (theme::bg_raised(), theme::text_off()));
        let ghost = Button::ghost("x").palette(false);
        assert_eq!(ghost.rest, Color32::TRANSPARENT);
    }

    #[test]
    fn every_kind_has_distinct_hover_and_press_fills() {
        for b in [
            Button::primary("x"),
            Button::secondary("x"),
            Button::ghost("x"),
        ] {
            let p = b.palette(true);
            assert_ne!(p.rest, p.hover, "{:?}", b.kind);
            assert_ne!(p.hover, p.press, "{:?}", b.kind);
        }
        assert_ne!(
            Button::danger("x").palette(true).rest,
            Button::danger("x").palette(true).hover
        );
    }

    #[test]
    fn button_text_meets_contrast_on_its_fill() {
        for b in [
            Button::primary("x"),
            Button::secondary("x"),
            Button::danger("x"),
        ] {
            let p = b.palette(true);
            for fill in [p.rest, p.hover] {
                assert!(theme::contrast_ratio(p.text, fill) >= 4.5, "{:?}", b.kind);
            }
        }
    }
}
