//! Drawing the touch overlay with egui (spec 7: graphite fills, silver strokes, restrained violet
//! for the primary button and the held knob; no glow).
//!
//! Colours are the visual-language tokens (`design/tokens/metallic.tokens.json`); a test checks
//! them against the token file. Opacities: hint ring 35%, stick base while held 55%, knob 85%,
//! buttons 90% (pressed: 100% and 94% size).
use super::controls::TouchControls;
use super::layout::PlacedButton;
use super::{MENU_SLOT, TOUCH_STICK_DEADZONE, TouchIcon};
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect as ERect, Stroke, TextureId, pos2, vec2};

/// `g-750` (bg-raised): button and stick fill.
pub(crate) const FILL: [u8; 3] = [0x24, 0x22, 0x2b];
/// `v-800` (select): primary button fill.
pub(crate) const PRIMARY_FILL: [u8; 3] = [0x3a, 0x33, 0x54];
/// `g-300`: silver strokes and the idle knob.
pub(crate) const SILVER: [u8; 3] = [0xa3, 0xa0, 0xad];
/// `g-200`: the dead-zone ring.
pub(crate) const SILVER_LIGHT: [u8; 3] = [0xc3, 0xc0, 0xca];
/// `g-500`: the idle knob's rim.
pub(crate) const KNOB_RIM: [u8; 3] = [0x5f, 0x5b, 0x6b];
/// `v-300`: primary button stroke.
pub(crate) const VIOLET_LINE: [u8; 3] = [0xb4, 0xa6, 0xec];
/// `v-400` (accent): the held knob.
pub(crate) const VIOLET: [u8; 3] = [0x9d, 0x8b, 0xe0];
/// `v-700`: the held knob's rim.
pub(crate) const VIOLET_RIM: [u8; 3] = [0x58, 0x49, 0x93];
/// `g-100` (text-1): icons and labels.
pub(crate) const TEXT: [u8; 3] = [0xe6, 0xe4, 0xea];
/// `g-900` (bg-deep): the label pill behind text.
pub(crate) const PILL: [u8; 3] = [0x11, 0x10, 0x15];
/// `g-400`: the safe-area guide.
pub(crate) const GUIDE: [u8; 3] = [0x81, 0x7d, 0x8c];

/// Opacity of the stick's hint ring before the first touch.
pub const HINT_OPACITY: f32 = 0.35;
/// Opacity of the stick base while held.
pub const BASE_OPACITY: f32 = 0.55;
/// Opacity of the knob.
pub const KNOB_OPACITY: f32 = 0.85;
/// Opacity of buttons.
pub const BUTTON_OPACITY: f32 = 0.9;
/// A pressed button is drawn this much smaller.
pub const PRESSED_SCALE: f32 = 0.94;
/// Label text size in dp (the caption size, 11 px at 100%).
pub const LABEL_SIZE: f32 = 11.0;
/// Icon box on a secondary button and on the primary, in dp.
pub const ICON_SIZE: [f32; 2] = [24.0, 28.0];

/// How to draw the overlay this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayStyle {
    /// Where dp (0, 0) is, in egui points.
    pub origin: [f32; 2],
    /// egui points per dp.
    pub points_per_dp: f32,
    /// Fade, 0 (hidden) to 1 (shown).
    pub alpha: f32,
    /// The player's opacity setting (25% to 100%); scales everything.
    pub opacity: f32,
    /// Draw the safe area's edge (device emulation).
    pub show_safe_area: bool,
}

impl Default for OverlayStyle {
    fn default() -> Self {
        OverlayStyle {
            origin: [0.0; 2],
            points_per_dp: 1.0,
            alpha: 1.0,
            opacity: 1.0,
            show_safe_area: false,
        }
    }
}

struct Pen<'a> {
    painter: &'a Painter,
    style: OverlayStyle,
}

impl Pen<'_> {
    fn p(&self, dp: [f32; 2]) -> Pos2 {
        pos2(
            self.style.origin[0] + dp[0] * self.style.points_per_dp,
            self.style.origin[1] + dp[1] * self.style.points_per_dp,
        )
    }

    fn len(&self, dp: f32) -> f32 {
        dp * self.style.points_per_dp
    }

    fn color(&self, rgb: [u8; 3], alpha: f32) -> Color32 {
        let a = (alpha * self.style.alpha * self.style.opacity).clamp(0.0, 1.0);
        Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], (a * 255.0).round() as u8)
    }

    fn circle(&self, c: [f32; 2], r: f32, fill: [u8; 3], fill_a: f32, line: [u8; 3], line_a: f32) {
        let stroke = Stroke::new(self.len(2.0).max(1.0), self.color(line, line_a));
        self.painter
            .circle(self.p(c), self.len(r), self.color(fill, fill_a), stroke);
    }
}

/// Draws `controls` with `painter`. Images for buttons with a `TouchImage` come from `image`
/// (by control slot); a button without one draws its role's icon or its label's first letter.
pub fn paint(
    painter: &Painter,
    controls: &TouchControls,
    style: &OverlayStyle,
    image: &dyn Fn(usize) -> Option<TextureId>,
) {
    if style.alpha <= 0.0 || style.opacity <= 0.0 || style.points_per_dp <= 0.0 {
        return;
    }
    let pen = Pen {
        painter,
        style: *style,
    };
    let layout = controls.layout();
    if style.show_safe_area {
        let s = layout.safe;
        let rect = ERect::from_min_max(pen.p([s.x, s.y]), pen.p([s.right(), s.bottom()]));
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, pen.color(GUIDE, 0.8)),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(rest) = layout.stick {
        match controls.held_stick() {
            Some(stick) => {
                pen.circle(
                    stick.base,
                    rest.radius,
                    FILL,
                    0.8 * BASE_OPACITY,
                    SILVER,
                    BASE_OPACITY,
                );
                let dz = rest.radius * TOUCH_STICK_DEADZONE;
                painter.circle_stroke(
                    pen.p(stick.base),
                    pen.len(dz),
                    Stroke::new(1.0, pen.color(SILVER_LIGHT, BASE_OPACITY)),
                );
                pen.circle(
                    stick.knob(),
                    rest.knob_radius,
                    VIOLET,
                    KNOB_OPACITY,
                    VIOLET_RIM,
                    KNOB_OPACITY,
                );
            }
            None => {
                let fixed = controls.profile().stick_mode == super::StickMode::Fixed;
                if fixed || !controls.touched_once() {
                    let a = HINT_OPACITY;
                    pen.circle(rest.center, rest.radius, FILL, 0.5 * a, SILVER, a);
                    pen.circle(rest.center, rest.knob_radius, SILVER, a, KNOB_RIM, a);
                }
            }
        }
    }
    let set = controls.set();
    for b in layout.buttons.iter().chain(layout.menu.iter()) {
        let def = set.button_in_slot(b.slot);
        let (label, icon) = match def {
            Some(d) => (d.label.as_str(), d.icon),
            None => ("", TouchIcon::Letter),
        };
        draw_button(
            &pen,
            b,
            controls.is_pressed(b.slot),
            label,
            icon,
            image(b.slot),
        );
    }
}

fn draw_button(
    pen: &Pen<'_>,
    b: &PlacedButton,
    pressed: bool,
    label: &str,
    icon: TouchIcon,
    image: Option<TextureId>,
) {
    let a = if pressed { 1.0 } else { BUTTON_OPACITY };
    let r = if pressed {
        b.radius * PRESSED_SCALE
    } else {
        b.radius
    };
    let (fill, fill_a, line) = if b.primary {
        (PRIMARY_FILL, 0.88 * a, VIOLET_LINE)
    } else {
        (FILL, 0.78 * a, SILVER)
    };
    pen.circle(b.center, r, fill, fill_a, line, a);
    let size = ICON_SIZE[usize::from(b.primary)] * if pressed { PRESSED_SCALE } else { 1.0 };
    let ink = pen.color(TEXT, a);
    if let Some(tex) = image {
        let half = pen.len(size / 2.0);
        let rect = ERect::from_center_size(pen.p(b.center), vec2(2.0 * half, 2.0 * half));
        let uv = ERect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        pen.painter
            .image(tex, rect, uv, pen.color([255, 255, 255], a));
    } else {
        draw_icon(pen, b.center, size, icon, label, ink);
    }
    if b.slot != MENU_SLOT && !label.is_empty() {
        let font = FontId::proportional(pen.len(LABEL_SIZE));
        let galley = pen.painter.layout_no_wrap(label.to_owned(), font, ink);
        let top = pen.p([b.center[0], b.center[1] + b.radius + 4.0]);
        let size = galley.size();
        let pad = vec2(pen.len(4.0), pen.len(1.0));
        let rect = ERect::from_min_size(pos2(top.x - size.x / 2.0, top.y), size).expand2(pad);
        pen.painter
            .rect_filled(rect, pen.len(3.0), pen.color(PILL, 0.78));
        pen.painter
            .galley(pos2(top.x - size.x / 2.0, top.y), galley, ink);
    }
}

/// The icons, drawn as strokes in a 24-unit box (our own drawings).
fn draw_icon(pen: &Pen<'_>, c: [f32; 2], size: f32, icon: TouchIcon, label: &str, ink: Color32) {
    let k = size / 24.0;
    let at = |x: f32, y: f32| pen.p([c[0] + (x - 12.0) * k, c[1] + (y - 12.0) * k]);
    let stroke = Stroke::new(pen.len(2.0 * k).max(1.0), ink);
    let line = |a: (f32, f32), b: (f32, f32)| {
        pen.painter
            .line_segment([at(a.0, a.1), at(b.0, b.1)], stroke);
    };
    match icon {
        TouchIcon::Jump => {
            for y in [5.0, 11.0] {
                line((12.0, y), (5.0, y + 8.0));
                line((12.0, y), (19.0, y + 8.0));
            }
        }
        TouchIcon::Run => {
            for x in [6.0, 12.0] {
                line((x, 6.0), (x + 7.0, 12.0));
                line((x + 7.0, 12.0), (x, 18.0));
            }
        }
        TouchIcon::Interact => {
            pen.painter
                .circle_stroke(at(12.0, 12.0), pen.len(3.2 * k), stroke);
            pen.painter
                .circle_stroke(at(12.0, 12.0), pen.len(8.0 * k), stroke);
        }
        TouchIcon::Primary => {
            pen.painter
                .circle_stroke(at(12.0, 12.0), pen.len(6.5 * k), stroke);
            line((12.0, 3.0), (12.0, 8.0));
            line((12.0, 16.0), (12.0, 21.0));
            line((3.0, 12.0), (8.0, 12.0));
            line((16.0, 12.0), (21.0, 12.0));
        }
        TouchIcon::Secondary => {
            line((12.0, 4.0), (20.0, 12.0));
            line((20.0, 12.0), (12.0, 20.0));
            line((12.0, 20.0), (4.0, 12.0));
            line((4.0, 12.0), (12.0, 4.0));
        }
        TouchIcon::Menu => {
            line((9.0, 6.0), (9.0, 18.0));
            line((15.0, 6.0), (15.0, 18.0));
        }
        TouchIcon::Letter => {
            let mut buf = [0u8; 4];
            let letter = label
                .chars()
                .next()
                .map(|ch| &*ch.to_uppercase().next().unwrap_or(ch).encode_utf8(&mut buf))
                .unwrap_or("");
            if !letter.is_empty() {
                pen.painter.text(
                    pen.p(c),
                    Align2::CENTER_CENTER,
                    letter,
                    FontId::proportional(pen.len(size * 0.8)),
                    ink,
                );
            }
        }
    }
}
