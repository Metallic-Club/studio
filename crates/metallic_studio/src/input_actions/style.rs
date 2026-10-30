//! Visual tokens (from `design/tokens/metallic.tokens.json`, checked by a test) and the few
//! painted widgets the panel needs: icons, keycaps and pad glyphs, chips, toggles, segmented
//! controls and focus rings. Everything is drawn here, so the panel has no default-egui look.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Response, Sense, Shape,
    Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use metallic_runtime::input::{BindingDef, Control};

use super::model::control_label;
use super::preview::GlyphStyle;

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

// Role tokens.
pub const G950: Color32 = hex(0x0c0b0f);
pub const BG_DEEP: Color32 = hex(0x111015);
pub const BG_BASE: Color32 = hex(0x16151b);
pub const BG_PANEL: Color32 = hex(0x1c1b22);
pub const BG_RAISED: Color32 = hex(0x24222b);
pub const BG_HOVER: Color32 = hex(0x2c2a34);
pub const BG_ACTIVE: Color32 = hex(0x35323e);
pub const SELECT: Color32 = hex(0x3a3354);
pub const SELECT_WEAK: Color32 = hex(0x2a2638);
pub const LINE: Color32 = hex(0x2b2933);
pub const LINE_STRONG: Color32 = hex(0x43404d);
pub const LINE_CONTROL: Color32 = hex(0x6e6a79);
pub const TEXT_1: Color32 = hex(0xe6e4ea);
pub const TEXT_2: Color32 = hex(0xaeaab6);
pub const TEXT_3: Color32 = hex(0x8e8a97);
pub const TEXT_OFF: Color32 = hex(0x5f5b6b);
pub const ICON: Color32 = hex(0xbdbac4);
pub const ACCENT: Color32 = hex(0x9d8be0);
pub const ACCENT_HOVER: Color32 = hex(0xb4a6ec);
pub const ACCENT_TEXT: Color32 = hex(0xb4a6ec);
pub const ON_ACCENT: Color32 = hex(0x111015);
pub const FOCUS: Color32 = hex(0xb4a6ec);
pub const ERROR: Color32 = hex(0xec7a72);
pub const ERROR_BG: Color32 = hex(0x3a2226);
pub const WARNING: Color32 = hex(0xe0ad55);
pub const WARNING_BG: Color32 = hex(0x352c1d);
pub const SUCCESS: Color32 = hex(0x62bf91);
pub const SUCCESS_BG: Color32 = hex(0x1d3329);
pub const INFO: Color32 = hex(0x7fb0e6);
pub const INFO_BG: Color32 = hex(0x1c2a3a);
/// Keycap fill (`g-650`).
pub const KEYCAP: Color32 = hex(0x35323e);
/// Silver strokes of the touch overlay (`g-300`).
pub const SILVER: Color32 = hex(0xa3a0ad);
/// Raised top-edge highlight (`highlight`, white at 6 %).
pub const HIGHLIGHT: Color32 = Color32::from_rgba_premultiplied(15, 15, 15, 15);

// Space, radius and type tokens.
pub const S2: f32 = 4.0;
pub const S3: f32 = 6.0;
pub const S4: f32 = 8.0;
pub const S5: f32 = 12.0;
pub const S6: f32 = 16.0;
pub const R_XS: u8 = 3;
pub const R_SM: u8 = 4;
pub const R_MD: u8 = 6;
pub const R_LG: u8 = 10;
pub const ROW: f32 = 24.0;

pub fn caption() -> FontId {
    FontId::proportional(11.0)
}
pub fn small() -> FontId {
    FontId::proportional(12.0)
}
pub fn body() -> FontId {
    FontId::proportional(13.0)
}
pub fn heading() -> FontId {
    FontId::proportional(15.0)
}
pub fn mono() -> FontId {
    FontId::monospace(12.0)
}

/// Sets the panel's own widget visuals inside `ui` (egui widgets that remain: text fields,
/// drag values, combo boxes, scroll bars).
pub fn scope_visuals(ui: &mut Ui) {
    let v = ui.visuals_mut();
    v.override_text_color = None;
    v.extreme_bg_color = BG_DEEP;
    v.text_edit_bg_color = Some(BG_DEEP);
    v.faint_bg_color = BG_RAISED;
    v.selection.bg_fill = SELECT;
    v.selection.stroke = Stroke::new(1.0, ACCENT_TEXT);
    v.window_fill = BG_PANEL;
    v.window_stroke = Stroke::new(1.0, LINE_STRONG);
    v.menu_corner_radius = CornerRadius::same(R_MD);
    v.weak_text_color = Some(TEXT_3);
    v.warn_fg_color = WARNING;
    v.error_fg_color = ERROR;
    v.text_cursor.stroke = Stroke::new(2.0, ACCENT_TEXT);
    let w = &mut v.widgets;
    for (state, fill) in [
        (&mut w.noninteractive, BG_PANEL),
        (&mut w.inactive, BG_RAISED),
        (&mut w.hovered, BG_HOVER),
        (&mut w.active, BG_ACTIVE),
        (&mut w.open, BG_ACTIVE),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::NONE;
        state.fg_stroke = Stroke::new(1.0, TEXT_1);
        state.corner_radius = CornerRadius::same(R_SM);
        state.expansion = 0.0;
    }
    w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_2);
    let s = ui.spacing_mut();
    s.item_spacing = vec2(S4, S3);
    s.button_padding = vec2(S5, 5.0);
    s.interact_size = vec2(40.0, ROW);
    s.indent = S6;
}

/// The 2 px focus ring, 2 px outside `rect`, when `response` has keyboard focus.
pub fn focus_ring(ui: &Ui, response: &Response, rect: Rect, radius: u8) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(radius + 2),
            Stroke::new(2.0, FOCUS),
            StrokeKind::Outside,
        );
    }
}

/// Painted icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Eye,
    EyeOff,
    Plus,
    Trash,
    Undo,
    Redo,
    Listen,
    Error,
    Warning,
    Info,
    Check,
    ChevronRight,
    ChevronDown,
    Keyboard,
    Gamepad,
    Phone,
    Tablet,
    Context,
    Action,
    Binding,
    Reset,
    Layers,
}

/// Paints an icon in a square `rect`.
pub fn paint_icon(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let s = rect.width().min(rect.height());
    let u = s / 16.0;
    let st = Stroke::new(1.5_f32.max(u * 1.3), color);
    let pt = |x: f32, y: f32| pos2(c.x + x * u, c.y + y * u);
    match icon {
        Icon::Eye | Icon::EyeOff => {
            let mut pts = Vec::new();
            for i in 0..=16 {
                let t = i as f32 / 16.0 * std::f32::consts::PI;
                pts.push(pt(-6.5 * t.cos(), -4.0 * t.sin()));
            }
            for i in 0..=16 {
                let t = i as f32 / 16.0 * std::f32::consts::PI;
                pts.push(pt(6.5 * t.cos(), 4.0 * t.sin()));
            }
            p.add(Shape::closed_line(pts, st));
            p.circle_filled(c, 1.8 * u, color);
            if icon == Icon::EyeOff {
                p.line_segment([pt(-6.0, 6.0), pt(6.0, -6.0)], st);
            }
        }
        Icon::Plus => {
            p.line_segment([pt(-5.0, 0.0), pt(5.0, 0.0)], st);
            p.line_segment([pt(0.0, -5.0), pt(0.0, 5.0)], st);
        }
        Icon::Trash => {
            p.line_segment([pt(-6.0, -4.0), pt(6.0, -4.0)], st);
            p.line_segment([pt(-2.0, -6.0), pt(2.0, -6.0)], st);
            p.add(Shape::closed_line(
                vec![pt(-4.5, -4.0), pt(-3.5, 6.0), pt(3.5, 6.0), pt(4.5, -4.0)],
                st,
            ));
        }
        Icon::Undo | Icon::Redo => {
            let flip = if icon == Icon::Undo { 1.0 } else { -1.0 };
            let mut pts = Vec::new();
            for i in 0..=12 {
                let t = -std::f32::consts::FRAC_PI_2 + i as f32 / 12.0 * std::f32::consts::PI;
                pts.push(pt(flip * 5.0 * t.cos(), 5.0 * t.sin() + 1.0));
            }
            pts.push(pt(-flip * 5.0, 6.0));
            p.add(Shape::line(pts, st));
            p.add(Shape::line(
                vec![
                    pt(-flip * 2.0, -6.5),
                    pt(-flip * 5.5, -4.0),
                    pt(-flip * 2.0, -1.5),
                ],
                st,
            ));
        }
        Icon::Listen => {
            p.circle_stroke(c, 6.0 * u, st);
            p.circle_filled(c, 3.0 * u, color);
        }
        Icon::Error => {
            p.circle_filled(c, 7.0 * u, color);
            let st = Stroke::new(st.width, BG_PANEL);
            p.line_segment([pt(0.0, -3.5), pt(0.0, 1.0)], st);
            p.circle_filled(pt(0.0, 3.6), 1.0 * u, BG_PANEL);
        }
        Icon::Warning => {
            p.add(Shape::convex_polygon(
                vec![pt(0.0, -7.0), pt(7.5, 6.0), pt(-7.5, 6.0)],
                color,
                Stroke::NONE,
            ));
            let st = Stroke::new(st.width, BG_PANEL);
            p.line_segment([pt(0.0, -2.5), pt(0.0, 1.5)], st);
            p.circle_filled(pt(0.0, 3.8), 0.9 * u, BG_PANEL);
        }
        Icon::Info => {
            p.circle_stroke(c, 6.5 * u, st);
            p.line_segment([pt(0.0, -0.5), pt(0.0, 3.5)], st);
            p.circle_filled(pt(0.0, -3.2), 1.0 * u, color);
        }
        Icon::Check => {
            p.add(Shape::line(
                vec![pt(-5.0, 0.5), pt(-1.5, 4.0), pt(5.5, -3.5)],
                st,
            ));
        }
        Icon::ChevronRight => {
            p.add(Shape::line(
                vec![pt(-1.5, -4.0), pt(2.5, 0.0), pt(-1.5, 4.0)],
                st,
            ));
        }
        Icon::ChevronDown => {
            p.add(Shape::line(
                vec![pt(-4.0, -1.5), pt(0.0, 2.5), pt(4.0, -1.5)],
                st,
            ));
        }
        Icon::Keyboard => {
            p.rect_stroke(
                Rect::from_center_size(c, vec2(14.0 * u, 9.0 * u)),
                CornerRadius::same(2),
                st,
                StrokeKind::Middle,
            );
            for (x, y) in [(-4.0, -1.5), (-1.3, -1.5), (1.3, -1.5), (4.0, -1.5)] {
                p.circle_filled(pt(x, y), 0.8 * u, color);
            }
            p.line_segment([pt(-3.5, 2.0), pt(3.5, 2.0)], st);
        }
        Icon::Gamepad => {
            p.rect_stroke(
                Rect::from_center_size(c, vec2(15.0 * u, 9.0 * u)),
                CornerRadius::same((4.0 * u) as u8),
                st,
                StrokeKind::Middle,
            );
            p.line_segment([pt(-5.5, 0.0), pt(-2.5, 0.0)], st);
            p.line_segment([pt(-4.0, -1.5), pt(-4.0, 1.5)], st);
            p.circle_filled(pt(3.0, 1.0), 0.9 * u, color);
            p.circle_filled(pt(5.0, -1.0), 0.9 * u, color);
        }
        Icon::Phone | Icon::Tablet => {
            let size = if icon == Icon::Phone {
                vec2(8.0, 13.0)
            } else {
                vec2(11.0, 13.0)
            };
            p.rect_stroke(
                Rect::from_center_size(c, size * u),
                CornerRadius::same((2.0 * u) as u8),
                st,
                StrokeKind::Middle,
            );
            p.line_segment([pt(-1.2, 4.5), pt(1.2, 4.5)], st);
        }
        Icon::Context => {
            for (dy, alpha) in [(-3.0, 1.0), (0.5, 0.7), (4.0, 0.45)] {
                p.rect_stroke(
                    Rect::from_center_size(pt(0.0, dy), vec2(12.0 * u, 2.6 * u)),
                    CornerRadius::same(1),
                    Stroke::new(st.width, color.gamma_multiply(alpha)),
                    StrokeKind::Middle,
                );
            }
        }
        Icon::Action => {
            p.add(Shape::convex_polygon(
                vec![pt(1.5, -7.0), pt(-4.5, 1.0), pt(0.5, 1.0)],
                color,
                Stroke::NONE,
            ));
            p.add(Shape::convex_polygon(
                vec![pt(-0.5, 0.0), pt(4.5, 0.0), pt(-1.5, 7.0)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Binding => {
            p.rect_stroke(
                Rect::from_center_size(c, vec2(11.0 * u, 9.0 * u)),
                CornerRadius::same(2),
                st,
                StrokeKind::Middle,
            );
            p.line_segment([pt(-2.0, 1.5), pt(2.0, 1.5)], st);
        }
        Icon::Reset => {
            let mut pts = Vec::new();
            for i in 0..=14 {
                let t = 0.6 + i as f32 / 14.0 * 5.0;
                pts.push(pt(5.5 * t.cos(), 5.5 * t.sin()));
            }
            p.add(Shape::line(pts, st));
            p.add(Shape::line(
                vec![pt(5.5, -5.5), pt(4.5, -1.5), pt(0.8, -2.5)],
                st,
            ));
        }
        Icon::Layers => {
            p.add(Shape::closed_line(
                vec![pt(0.0, -6.0), pt(7.0, -2.5), pt(0.0, 1.0), pt(-7.0, -2.5)],
                st,
            ));
            p.add(Shape::line(
                vec![pt(-7.0, 1.5), pt(0.0, 5.0), pt(7.0, 1.5)],
                st,
            ));
        }
    }
}

/// A 24 x 24 icon button with hover and pressed fills and a tooltip.
pub fn icon_button(ui: &mut Ui, icon: Icon, tip: &str, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(ROW), Sense::click());
    let resp = if enabled {
        resp
    } else {
        resp.on_disabled_hover_text(tip)
    };
    let fill = if !enabled {
        Color32::TRANSPARENT
    } else if resp.is_pointer_button_down_on() {
        BG_ACTIVE
    } else if resp.hovered() || resp.has_focus() {
        BG_HOVER
    } else {
        Color32::TRANSPARENT
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(R_SM), fill);
    let color = if enabled { ICON } else { TEXT_OFF };
    paint_icon(ui.painter(), rect.shrink(4.0), icon, color);
    focus_ring(ui, &resp, rect, R_SM);
    let resp = resp.on_hover_text(tip);
    if enabled {
        resp
    } else {
        resp.with_new_rect(rect)
    }
}

/// Button kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// Violet fill: the one main action of a view.
    Primary,
    /// Raised graphite.
    Secondary,
    /// No fill until hovered.
    Ghost,
}

/// A text button with an optional leading icon, 28 px tall.
pub fn button(ui: &mut Ui, kind: ButtonKind, icon: Option<Icon>, text: &str) -> Response {
    let font = body();
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, TEXT_1);
    let icon_w = if icon.is_some() { 16.0 + S3 } else { 0.0 };
    let size = vec2(galley.size().x + icon_w + 2.0 * S5, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let down = resp.is_pointer_button_down_on();
    let hot = resp.hovered() || resp.has_focus();
    let (fill, fg) = match kind {
        ButtonKind::Primary => (
            if down {
                hex(0x8673cf)
            } else if hot {
                ACCENT_HOVER
            } else {
                ACCENT
            },
            ON_ACCENT,
        ),
        ButtonKind::Secondary => (
            if down {
                BG_ACTIVE
            } else if hot {
                BG_HOVER
            } else {
                BG_RAISED
            },
            TEXT_1,
        ),
        ButtonKind::Ghost => (
            if down {
                BG_ACTIVE
            } else if hot {
                BG_HOVER
            } else {
                Color32::TRANSPARENT
            },
            TEXT_2,
        ),
    };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(R_SM), fill);
    if kind == ButtonKind::Secondary {
        p.hline(
            rect.x_range().shrink(R_SM as f32),
            rect.top() + 0.5,
            Stroke::new(1.0, HIGHLIGHT),
        );
    }
    let mut x = rect.left() + S5;
    if let Some(i) = icon {
        paint_icon(
            p,
            Rect::from_min_size(pos2(x, rect.center().y - 8.0), Vec2::splat(16.0)),
            i,
            fg,
        );
        x += icon_w;
    }
    p.text(
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        body(),
        fg,
    );
    focus_ring(ui, &resp, rect, R_SM);
    resp
}

/// A small coloured label.
pub fn chip(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), caption(), fg);
    let size = vec2(galley.size().x + 2.0 * S3, 18.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(R_XS), bg);
    ui.painter().galley(
        pos2(rect.left() + S3, rect.center().y - galley.size().y / 2.0),
        galley,
        fg,
    );
    resp
}

/// A section title in the caption style.
pub fn section_title(ui: &mut Ui, text: &str) {
    ui.add_space(S2);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    ui.painter().text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        text.to_uppercase(),
        caption(),
        TEXT_3,
    );
}

/// A 28 x 16 switch. Returns true when it was flipped.
pub fn toggle(ui: &mut Ui, on: &mut bool, enabled: bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(28.0, 16.0), Sense::click());
    if enabled && resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.09);
    let track = if !enabled {
        BG_RAISED
    } else if *on {
        ACCENT
    } else if resp.hovered() {
        LINE_CONTROL
    } else {
        LINE_STRONG
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), track);
    let x = egui::lerp(rect.left() + 8.0..=rect.right() - 8.0, t);
    let knob = if *on && enabled { ON_ACCENT } else { TEXT_1 };
    ui.painter()
        .circle_filled(pos2(x, rect.center().y), 5.5, knob);
    focus_ring(ui, &resp, rect, 8);
    resp
}

/// A segmented control. Returns true when the selection changed.
pub fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    value: &mut T,
    options: &[(T, Option<Icon>, &str)],
) -> bool {
    let mut changed = false;
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, icon, text)| {
            let g = ui
                .painter()
                .layout_no_wrap(text.to_string(), small(), TEXT_1);
            g.size().x + if icon.is_some() { 20.0 } else { 0.0 } + 2.0 * S4
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 4.0;
    let (outer, _) = ui.allocate_exact_size(vec2(total, 26.0), Sense::hover());
    ui.painter()
        .rect_filled(outer, CornerRadius::same(R_MD), BG_DEEP);
    let mut x = outer.left() + 2.0;
    for ((v, icon, text), w) in options.iter().zip(widths) {
        let r = Rect::from_min_size(pos2(x, outer.top() + 2.0), vec2(w, 22.0));
        let id = ui.scope_id().with(("seg", text));
        let resp = ui.interact(r, id, Sense::click());
        let selected = *value == *v;
        if resp.clicked() && !selected {
            *value = *v;
            changed = true;
        }
        let fill = if selected {
            BG_ACTIVE
        } else if resp.hovered() {
            BG_HOVER
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(r, CornerRadius::same(R_SM), fill);
        let fg = if selected { TEXT_1 } else { TEXT_2 };
        let mut tx = r.left() + S4;
        if let Some(i) = icon {
            paint_icon(
                ui.painter(),
                Rect::from_min_size(pos2(tx, r.center().y - 7.0), Vec2::splat(14.0)),
                *i,
                fg,
            );
            tx += 20.0;
        }
        ui.painter().text(
            pos2(tx, r.center().y),
            Align2::LEFT_CENTER,
            *text,
            small(),
            fg,
        );
        focus_ring(ui, &resp, r, R_SM);
        resp.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, *text)
        });
        x += w;
    }
    changed
}

// ---------------------------------------------------------------------------------------------
// Glyphs (spec 9.2: our own drawings; monochrome; no console marks).

/// One drawn part of a binding glyph.
#[derive(Debug, Clone, PartialEq)]
enum Part {
    Key(String),
    Mouse(u8),
    Wheel,
    MouseMove,
    Face(Control),
    Shoulder(&'static str),
    Stick(u8),
    DPad(Option<Control>),
    Touch(&'static str),
    Plus,
    Unbound,
}

fn part_of(c: Control) -> Part {
    use Control as C;
    match c {
        C::MouseButton1 => Part::Mouse(1),
        C::MouseButton2 => Part::Mouse(2),
        C::MouseButton3 => Part::Mouse(3),
        C::MouseWheel => Part::Wheel,
        C::MouseDelta | C::MousePosition => Part::MouseMove,
        C::ButtonA | C::ButtonB | C::ButtonX | C::ButtonY => Part::Face(c),
        C::ButtonL1
        | C::ButtonR1
        | C::ButtonL2
        | C::ButtonR2
        | C::ButtonL3
        | C::ButtonR3
        | C::ButtonSelect
        | C::ButtonStart => Part::Shoulder(control_label(c)),
        C::Thumbstick1 => Part::Stick(1),
        C::Thumbstick2 => Part::Stick(2),
        C::DPadUp | C::DPadDown | C::DPadLeft | C::DPadRight => Part::DPad(Some(c)),
        C::TouchStick | C::TouchLook | C::TouchButton => Part::Touch(control_label(c)),
        other => Part::Key(control_label(other).to_string()),
    }
}

fn parts(b: &BindingDef) -> Vec<Part> {
    let mut out = Vec::new();
    for m in b.modifiers.iter().flatten() {
        out.push(part_of(*m));
        out.push(Part::Plus);
    }
    if let Some(k) = b.key {
        out.push(part_of(k));
    } else {
        let keys: Vec<Control> = b.composite.iter().flatten().copied().collect();
        if keys
            == [
                Control::DPadUp,
                Control::DPadDown,
                Control::DPadLeft,
                Control::DPadRight,
            ]
        {
            out.push(Part::DPad(None));
        } else {
            for i in [0usize, 2, 1, 3, 4, 5] {
                if let Some(c) = b.composite[i] {
                    out.push(part_of(c));
                }
            }
        }
    }
    if out.is_empty() {
        out.push(Part::Unbound);
    }
    out
}

const GLYPH_H: f32 = 20.0;

fn part_width(p: &Painter, part: &Part) -> f32 {
    match part {
        Part::Key(t) => {
            (p.layout_no_wrap(t.clone(), caption(), TEXT_1).size().x + 10.0).max(GLYPH_H)
        }
        Part::Shoulder(t) => p.layout_no_wrap(t.to_string(), caption(), TEXT_1).size().x + 12.0,
        Part::Touch(t) => p.layout_no_wrap(t.to_string(), caption(), TEXT_1).size().x + 24.0,
        Part::Unbound => {
            p.layout_no_wrap("Unbound".into(), caption(), TEXT_1)
                .size()
                .x
                + 4.0
        }
        Part::Plus => 8.0,
        Part::Mouse(_) | Part::Wheel | Part::MouseMove => 16.0,
        _ => GLYPH_H,
    }
}

/// Width of a binding glyph.
pub fn glyph_width(p: &Painter, b: &BindingDef) -> f32 {
    let ps = parts(b);
    ps.iter().map(|x| part_width(p, x)).sum::<f32>() + (ps.len().saturating_sub(1)) as f32 * 3.0
}

fn paint_face(p: &Painter, c: Pos2, control: Control, style: GlyphStyle, fg: Color32) {
    let r = GLYPH_H / 2.0;
    p.circle_filled(c, r, KEYCAP);
    p.circle_stroke(c, r - 0.5, Stroke::new(1.0, LINE_CONTROL));
    match style {
        GlyphStyle::Letters => {
            p.text(
                c,
                Align2::CENTER_CENTER,
                control_label(control),
                caption(),
                fg,
            );
        }
        GlyphStyle::Shapes => {
            let st = Stroke::new(1.4, fg);
            let s = 4.2;
            match control {
                // South, East, West, North: cross, circle, square, triangle (drawn by us).
                Control::ButtonA => {
                    p.line_segment([c + vec2(-s, -s), c + vec2(s, s)], st);
                    p.line_segment([c + vec2(-s, s), c + vec2(s, -s)], st);
                }
                Control::ButtonB => {
                    p.circle_stroke(c, s, st);
                }
                Control::ButtonX => {
                    p.rect_stroke(
                        Rect::from_center_size(c, Vec2::splat(2.0 * s - 0.5)),
                        CornerRadius::ZERO,
                        st,
                        StrokeKind::Middle,
                    );
                }
                _ => {
                    p.add(Shape::closed_line(
                        vec![
                            c + vec2(0.0, -s),
                            c + vec2(s, s * 0.8),
                            c + vec2(-s, s * 0.8),
                        ],
                        st,
                    ));
                }
            }
        }
    }
}

/// Paints a binding glyph with its left edge at `at` (vertically centred). Returns the width.
pub fn paint_glyph(p: &Painter, at: Pos2, b: &BindingDef, style: GlyphStyle, dim: bool) -> f32 {
    let fg = if dim { TEXT_3 } else { TEXT_1 };
    let mut x = at.x;
    let y = at.y;
    for part in parts(b) {
        let w = part_width(p, &part);
        let rect = Rect::from_min_size(pos2(x, y - GLYPH_H / 2.0), vec2(w, GLYPH_H));
        let c = rect.center();
        match &part {
            Part::Key(t) => {
                p.rect_filled(rect, CornerRadius::same(R_XS), KEYCAP);
                p.hline(
                    rect.x_range().shrink(2.0),
                    rect.bottom() - 1.0,
                    Stroke::new(1.0, BG_DEEP),
                );
                p.text(c, Align2::CENTER_CENTER, t, caption(), fg);
            }
            Part::Shoulder(t) => {
                p.rect_filled(
                    rect,
                    CornerRadius {
                        nw: 7,
                        ne: 7,
                        sw: 3,
                        se: 3,
                    },
                    KEYCAP,
                );
                p.text(c, Align2::CENTER_CENTER, *t, caption(), fg);
            }
            Part::Mouse(n) => {
                let body = Rect::from_center_size(c, vec2(12.0, GLYPH_H - 2.0));
                p.rect_stroke(
                    body,
                    CornerRadius::same(6),
                    Stroke::new(1.2, fg),
                    StrokeKind::Middle,
                );
                p.vline(c.x, body.top()..=body.top() + 7.0, Stroke::new(1.0, fg));
                p.hline(body.x_range(), body.top() + 7.0, Stroke::new(1.0, fg));
                let half = match n {
                    1 => Some(Rect::from_min_max(
                        body.left_top() + vec2(1.5, 1.5),
                        pos2(c.x - 1.0, body.top() + 6.0),
                    )),
                    2 => Some(Rect::from_min_max(
                        pos2(c.x + 1.0, body.top() + 1.5),
                        pos2(body.right() - 1.5, body.top() + 6.0),
                    )),
                    _ => None,
                };
                match half {
                    Some(h) => {
                        p.rect_filled(
                            h,
                            CornerRadius {
                                nw: if *n == 1 { 4 } else { 0 },
                                ne: if *n == 2 { 4 } else { 0 },
                                sw: 0,
                                se: 0,
                            },
                            ACCENT,
                        );
                    }
                    None => {
                        p.rect_filled(
                            Rect::from_center_size(pos2(c.x, body.top() + 4.0), vec2(2.5, 4.5)),
                            CornerRadius::same(1),
                            ACCENT,
                        );
                    }
                }
            }
            Part::Wheel => {
                let body = Rect::from_center_size(c, vec2(12.0, GLYPH_H - 2.0));
                p.rect_stroke(
                    body,
                    CornerRadius::same(6),
                    Stroke::new(1.2, fg),
                    StrokeKind::Middle,
                );
                p.rect_filled(
                    Rect::from_center_size(pos2(c.x, body.top() + 5.0), vec2(2.5, 5.0)),
                    CornerRadius::same(1),
                    ACCENT,
                );
                p.add(Shape::line(
                    vec![
                        pos2(c.x - 2.5, c.y + 3.0),
                        pos2(c.x, c.y + 5.5),
                        pos2(c.x + 2.5, c.y + 3.0),
                    ],
                    Stroke::new(1.0, fg),
                ));
            }
            Part::MouseMove => {
                let body = Rect::from_center_size(c, vec2(12.0, GLYPH_H - 2.0));
                p.rect_stroke(
                    body,
                    CornerRadius::same(6),
                    Stroke::new(1.2, fg),
                    StrokeKind::Middle,
                );
                for d in [
                    vec2(-1.0, 0.0),
                    vec2(1.0, 0.0),
                    vec2(0.0, -1.0),
                    vec2(0.0, 1.0),
                ] {
                    p.circle_filled(c + vec2(0.0, 2.0) + d * 3.0, 0.9, fg);
                }
            }
            Part::Face(control) => paint_face(p, c, *control, style, fg),
            Part::Stick(n) => {
                let r = GLYPH_H / 2.0;
                p.circle_stroke(c, r - 0.5, Stroke::new(1.2, fg));
                p.circle_filled(c, r - 4.0, KEYCAP);
                p.text(
                    c,
                    Align2::CENTER_CENTER,
                    if *n == 1 { "L" } else { "R" },
                    caption(),
                    fg,
                );
            }
            Part::DPad(dir) => {
                let arm = 3.2;
                let len = GLYPH_H / 2.0 - 1.0;
                let h = Rect::from_center_size(c, vec2(2.0 * len, 2.0 * arm));
                let v = Rect::from_center_size(c, vec2(2.0 * arm, 2.0 * len));
                p.rect_filled(h, CornerRadius::same(1), KEYCAP);
                p.rect_filled(v, CornerRadius::same(1), KEYCAP);
                let lit = |r: Rect| p.rect_filled(r, CornerRadius::same(1), fg);
                match dir {
                    Some(Control::DPadUp) => {
                        lit(Rect::from_min_max(v.left_top(), pos2(v.right(), c.y - arm)));
                    }
                    Some(Control::DPadDown) => {
                        lit(Rect::from_min_max(
                            pos2(v.left(), c.y + arm),
                            v.right_bottom(),
                        ));
                    }
                    Some(Control::DPadLeft) => {
                        lit(Rect::from_min_max(
                            h.left_top(),
                            pos2(c.x - arm, h.bottom()),
                        ));
                    }
                    Some(Control::DPadRight) => {
                        lit(Rect::from_min_max(
                            pos2(c.x + arm, h.top()),
                            h.right_bottom(),
                        ));
                    }
                    _ => {
                        p.rect_stroke(
                            h,
                            CornerRadius::same(1),
                            Stroke::new(1.0, LINE_CONTROL),
                            StrokeKind::Inside,
                        );
                        p.rect_stroke(
                            v,
                            CornerRadius::same(1),
                            Stroke::new(1.0, LINE_CONTROL),
                            StrokeKind::Inside,
                        );
                    }
                }
            }
            Part::Touch(t) => {
                p.rect_filled(rect, CornerRadius::same(10), SELECT_WEAK);
                p.circle_stroke(
                    pos2(rect.left() + 10.0, c.y),
                    5.0,
                    Stroke::new(1.2, ACCENT_TEXT),
                );
                p.text(
                    pos2(rect.left() + 19.0, c.y),
                    Align2::LEFT_CENTER,
                    *t,
                    caption(),
                    fg,
                );
            }
            Part::Plus => {
                p.text(c, Align2::CENTER_CENTER, "+", caption(), TEXT_3);
            }
            Part::Unbound => {
                p.text(
                    rect.left_center(),
                    Align2::LEFT_CENTER,
                    "Unbound",
                    caption(),
                    WARNING,
                );
            }
        }
        x += w + 3.0;
    }
    x - at.x - 3.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_match_the_token_file() {
        let set = metallic_tokens::parse(metallic_tokens::TOKENS_JSON).unwrap();
        for (name, c) in [
            ("g-950", G950),
            ("bg-deep", BG_DEEP),
            ("bg-base", BG_BASE),
            ("bg-panel", BG_PANEL),
            ("bg-raised", BG_RAISED),
            ("bg-hover", BG_HOVER),
            ("bg-active", BG_ACTIVE),
            ("select", SELECT),
            ("select-weak", SELECT_WEAK),
            ("line", LINE),
            ("line-strong", LINE_STRONG),
            ("line-control", LINE_CONTROL),
            ("text-1", TEXT_1),
            ("text-2", TEXT_2),
            ("text-3", TEXT_3),
            ("text-off", TEXT_OFF),
            ("icon", ICON),
            ("accent", ACCENT),
            ("accent-hover", ACCENT_HOVER),
            ("accent-text", ACCENT_TEXT),
            ("on-accent", ON_ACCENT),
            ("focus", FOCUS),
            ("error", ERROR),
            ("error-bg", ERROR_BG),
            ("warning", WARNING),
            ("warning-bg", WARNING_BG),
            ("success", SUCCESS),
            ("success-bg", SUCCESS_BG),
            ("info", INFO),
            ("info-bg", INFO_BG),
            ("g-650", KEYCAP),
            ("g-300", SILVER),
        ] {
            let [r, g, b, _] =
                metallic_tokens::color(&set, name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!([c.r(), c.g(), c.b()], [r, g, b], "{name}");
        }
    }

    #[test]
    fn text_is_readable_on_panel_surfaces() {
        for fg in [TEXT_1, TEXT_2, ACCENT_TEXT, ERROR, WARNING, INFO, SUCCESS] {
            for bg in [BG_PANEL, BG_RAISED, BG_DEEP] {
                assert!(
                    crate::theme::contrast_ratio(fg, bg) >= 4.5,
                    "{fg:?} on {bg:?}"
                );
            }
        }
    }
}
