//! Editor widgets: property rows, painted icons, the curve and gradient editors, the node canvas and the
//! timeline. Curve, node canvas and timeline are written to be shared (Studio design doc 5.11); see each
//! module for what is VFX-specific.

pub mod curve;
pub mod gradient;
pub mod node_canvas;
pub mod timeline;

use crate::theme::{self, color};
use egui::{Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};
use metallic_vfx::{Range, Rgb, Vec3};

/// Painted icons (Phosphor is planned; until its font is added these are drawn as vectors in silver).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// Triangle.
    Play,
    /// Two bars.
    Pause,
    /// Circular arrow.
    Restart,
    /// Bar plus triangle.
    StepForward,
    /// Plus sign.
    Plus,
    /// Three dots.
    More,
    /// Down chevron.
    ChevronDown,
    /// Right chevron.
    ChevronRight,
    /// Loop arrows.
    Loop,
    /// Grid.
    Grid,
}

/// Paints `icon` centred in `rect`.
pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let s = rect.width().min(rect.height()) * 0.5;
    let stroke = Stroke::new(1.5, color);
    match icon {
        Icon::Play => {
            let pts = vec![
                c + Vec2::new(-s * 0.45, -s * 0.6),
                c + Vec2::new(s * 0.6, 0.0),
                c + Vec2::new(-s * 0.45, s * 0.6),
            ];
            painter.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
        }
        Icon::Pause => {
            for dx in [-0.3, 0.3] {
                let r = Rect::from_center_size(
                    c + Vec2::new(s * dx, 0.0),
                    Vec2::new(s * 0.28, s * 1.2),
                );
                painter.rect_filled(r, 1.0, color);
            }
        }
        Icon::Restart => {
            let n = 20;
            let pts: Vec<Pos2> = (0..=n)
                .map(|i| {
                    let a = -0.6 + i as f32 / n as f32 * 5.0;
                    c + Vec2::new(a.cos(), a.sin()) * (s * 0.6)
                })
                .collect();
            let tip = pts[0];
            painter.add(egui::Shape::line(pts, stroke));
            painter.add(egui::Shape::convex_polygon(
                vec![
                    tip + Vec2::new(-s * 0.05, -s * 0.45),
                    tip + Vec2::new(s * 0.4, -s * 0.05),
                    tip + Vec2::new(-s * 0.2, s * 0.25),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::StepForward => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    c + Vec2::new(-s * 0.5, -s * 0.55),
                    c + Vec2::new(s * 0.3, 0.0),
                    c + Vec2::new(-s * 0.5, s * 0.55),
                ],
                color,
                Stroke::NONE,
            ));
            painter.rect_filled(
                Rect::from_center_size(c + Vec2::new(s * 0.5, 0.0), Vec2::new(s * 0.22, s * 1.1)),
                1.0,
                color,
            );
        }
        Icon::Plus => {
            painter.line_segment(
                [c - Vec2::new(s * 0.6, 0.0), c + Vec2::new(s * 0.6, 0.0)],
                stroke,
            );
            painter.line_segment(
                [c - Vec2::new(0.0, s * 0.6), c + Vec2::new(0.0, s * 0.6)],
                stroke,
            );
        }
        Icon::More => {
            for dx in [-0.5, 0.0, 0.5] {
                painter.circle_filled(c + Vec2::new(s * dx, 0.0), 1.6, color);
            }
        }
        Icon::ChevronDown => {
            painter.add(egui::Shape::line(
                vec![
                    c + Vec2::new(-s * 0.4, -s * 0.2),
                    c + Vec2::new(0.0, s * 0.2),
                    c + Vec2::new(s * 0.4, -s * 0.2),
                ],
                stroke,
            ));
        }
        Icon::ChevronRight => {
            painter.add(egui::Shape::line(
                vec![
                    c + Vec2::new(-s * 0.2, -s * 0.4),
                    c + Vec2::new(s * 0.2, 0.0),
                    c + Vec2::new(-s * 0.2, s * 0.4),
                ],
                stroke,
            ));
        }
        Icon::Loop => {
            let r = Rect::from_center_size(c, Vec2::new(s * 1.3, s * 0.9));
            painter.rect_stroke(r, CornerRadius::same(4), stroke, StrokeKind::Middle);
            let tip = Pos2::new(r.max.x, r.center().y);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    tip + Vec2::new(-s * 0.25, -s * 0.05),
                    tip + Vec2::new(s * 0.25, -s * 0.05),
                    tip + Vec2::new(0.0, s * 0.3),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Grid => {
            let r = Rect::from_center_size(c, Vec2::splat(s * 1.3));
            painter.rect_stroke(r, CornerRadius::same(2), stroke, StrokeKind::Middle);
            painter.line_segment([Pos2::new(c.x, r.min.y), Pos2::new(c.x, r.max.y)], stroke);
            painter.line_segment([Pos2::new(r.min.x, c.y), Pos2::new(r.max.x, c.y)], stroke);
        }
    }
}

/// A square icon button (26 px). `active` shows the pressed fill.
pub fn icon_button(ui: &mut Ui, icon: Icon, active: bool, tooltip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    let fill = if active {
        color::BG_ACTIVE
    } else if response.hovered() {
        color::BG_HOVER
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, theme::R_MD, fill);
    paint_icon(ui.painter(), rect.shrink(6.0), icon, color::ICON);
    response.on_hover_text(tooltip)
}

/// A segmented control; returns the clicked index.
pub fn segmented(ui: &mut Ui, items: &[&str], selected: usize) -> Option<usize> {
    let mut clicked = None;
    let font = egui::TextStyle::Body.resolve(ui.style());
    let widths: Vec<f32> = items
        .iter()
        .map(|s| {
            ui.fonts_mut(|f| f.layout_no_wrap((*s).to_string(), font.clone(), color::TEXT))
                .size()
                .x
                + 20.0
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 4.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total, 26.0), Sense::hover());
    ui.painter().rect_filled(rect, theme::R_MD, color::BG_DEEP);
    let mut x = rect.min.x + 2.0;
    for (i, (label, w)) in items.iter().zip(&widths).enumerate() {
        let r = Rect::from_min_size(Pos2::new(x, rect.min.y + 2.0), Vec2::new(*w, 22.0));
        let resp = ui.interact(r, ui.scope_id().with(("seg", label, i)), Sense::click());
        let fill = if i == selected {
            color::BG_ACTIVE
        } else if resp.hovered() {
            color::BG_HOVER
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(r, theme::R_SM, fill);
        let text_color = if i == selected {
            color::TEXT
        } else {
            color::TEXT_2
        };
        ui.painter().text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            font.clone(),
            text_color,
        );
        if resp.clicked() {
            clicked = Some(i);
        }
        x += w;
    }
    clicked
}

/// A silver checkbox without a label (design doc: checkboxes in `icon` silver with dark ticks).
pub fn check(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let r = rect.shrink(1.5);
    if *on {
        ui.painter().rect_filled(r, theme::R_SM, color::ICON);
        let p = |x: f32, y: f32| Pos2::new(r.min.x + r.width() * x, r.min.y + r.height() * y);
        ui.painter().add(egui::Shape::line(
            vec![p(0.22, 0.52), p(0.42, 0.72), p(0.78, 0.3)],
            Stroke::new(1.8, color::ON_ACCENT),
        ));
    } else {
        let stroke = if response.hovered() {
            color::ICON
        } else {
            color::TEXT_3
        };
        ui.painter()
            .rect_stroke(r, theme::R_SM, Stroke::new(1.0, stroke), StrokeKind::Inside);
    }
    response
}

/// One property row: a label column and the value widget.
pub fn row<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.set_min_height(theme::ROW_H - 4.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(theme::LABEL_W, 20.0), Sense::hover());
        ui.painter().text(
            Pos2::new(rect.min.x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::TextStyle::Body.resolve(ui.style()),
            color::TEXT_2,
        );
        add(ui)
    })
    .inner
}

/// A compact drag field.
pub fn num(ui: &mut Ui, v: &mut f32, speed: f64, range: std::ops::RangeInclusive<f32>) -> Response {
    ui.add(
        egui::DragValue::new(v)
            .speed(speed)
            .range(range)
            .max_decimals(3)
            .min_decimals(1),
    )
}

/// `min – max` fields for a [`Range`].
pub fn range(
    ui: &mut Ui,
    r: &mut Range,
    speed: f64,
    limits: std::ops::RangeInclusive<f32>,
) -> bool {
    let a = num(ui, &mut r.min, speed, limits.clone()).changed();
    ui.label(egui::RichText::new("to").color(color::TEXT_3));
    let b = num(ui, &mut r.max, speed, limits).changed();
    a || b
}

/// X/Y/Z fields with coloured axis letters.
pub fn vec3(ui: &mut Ui, v: &mut Vec3, speed: f64) -> bool {
    let mut changed = false;
    for (letter, c, value) in [
        ("X", color::AXIS_X, &mut v.x),
        ("Y", color::AXIS_Y, &mut v.y),
        ("Z", color::AXIS_Z, &mut v.z),
    ] {
        ui.label(
            egui::RichText::new(letter)
                .color(c)
                .font(theme::semibold(11.0)),
        );
        changed |= ui
            .add(
                egui::DragValue::new(value)
                    .speed(speed)
                    .max_decimals(2)
                    .min_decimals(1),
            )
            .changed();
    }
    changed
}

/// A colour swatch with the egui colour picker.
pub fn color_button(ui: &mut Ui, c: &mut Rgb) -> bool {
    let [r, g, b] = c.to_u8();
    let mut c32 = Color32::from_rgb(r, g, b);
    let changed = egui::color_picker::color_edit_button_srgba(
        ui,
        &mut c32,
        egui::color_picker::Alpha::Opaque,
    )
    .changed();
    if changed {
        *c = Rgb::from_u8(c32.r(), c32.g(), c32.b());
    }
    changed
}

/// Converts a VFX colour to egui.
#[must_use]
pub fn to_color32(c: Rgb) -> Color32 {
    let [r, g, b] = c.to_u8();
    Color32::from_rgb(r, g, b)
}
