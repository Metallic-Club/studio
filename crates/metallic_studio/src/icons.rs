//! Toolbar and Explorer icons, drawn with egui shapes (no icon font, so nothing depends on which
//! glyphs a bundled font happens to contain). Silver on graphite, 16 px.
use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};

use crate::tokens::ui_tokens;

/// A toolbar icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// Arrow pointer.
    Select,
    /// Four-way arrows.
    Move,
    /// Circular arrow.
    Rotate,
    /// Diagonal double arrow with a box.
    Scale,
    /// A 3 x 3 grid.
    Snap,
    /// A globe (world/local axes).
    Space,
    /// Arrow turning left.
    Undo,
    /// Arrow turning right.
    Redo,
    /// Triangle.
    Play,
    /// Square.
    Stop,
    /// Two figures (Team Test and other multi-player modes).
    Players,
    /// Short text instead of a drawing.
    Text(&'static str),
}

fn arc(centre: Pos2, r: f32, from: f32, to: f32, n: usize) -> Vec<Pos2> {
    (0..=n)
        .map(|i| {
            let a = from + (to - from) * i as f32 / n as f32;
            centre + vec2(a.cos(), a.sin()) * r
        })
        .collect()
}

fn head(painter: &egui::Painter, tip: Pos2, dir: Vec2, c: Color32) {
    let d = dir.normalized();
    let side = vec2(-d.y, d.x);
    painter.add(Shape::convex_polygon(
        vec![
            tip + d * 1.5,
            tip - d * 3.5 + side * 3.0,
            tip - d * 3.5 - side * 3.0,
        ],
        c,
        Stroke::NONE,
    ));
}

/// Paints `icon` centred in `rect`.
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, c: Color32) {
    let s = Stroke::new(1.6, c);
    let o = rect.center();
    let r = 6.5;
    match icon {
        Icon::Select => {
            let pts = vec![
                o + vec2(-4.0, -7.0),
                o + vec2(5.0, 1.5),
                o + vec2(0.5, 2.0),
                o + vec2(-1.5, 7.0),
                o + vec2(-3.5, 6.0),
                o + vec2(-1.5, 1.5),
                o + vec2(-4.0, 3.5),
            ];
            painter.add(Shape::closed_line(pts, s));
        }
        Icon::Move => {
            for d in [
                vec2(1.0, 0.0),
                vec2(-1.0, 0.0),
                vec2(0.0, 1.0),
                vec2(0.0, -1.0),
            ] {
                painter.line_segment([o, o + d * (r - 1.0)], s);
                head(painter, o + d * (r + 0.5), d, c);
            }
        }
        Icon::Rotate => {
            let a0 = -0.6f32;
            let a1 = std::f32::consts::PI * 1.45;
            painter.add(Shape::line(arc(o, r - 1.0, a0, a1, 18), s));
            let end = o + vec2(a0.cos(), a0.sin()) * (r - 1.0);
            head(painter, end, vec2(a0.sin(), -a0.cos()) * -1.0, c);
        }
        Icon::Scale => {
            painter.rect_stroke(
                Rect::from_center_size(o + vec2(-2.0, 2.0), Vec2::splat(7.0)),
                0.0,
                s,
                egui::StrokeKind::Middle,
            );
            painter.line_segment([o + vec2(-1.0, 1.0), o + vec2(5.0, -5.0)], s);
            head(painter, o + vec2(6.0, -6.0), vec2(1.0, -1.0), c);
        }
        Icon::Snap => {
            for i in 0..4 {
                let t = -6.0 + i as f32 * 4.0;
                painter.line_segment([o + vec2(t, -6.0), o + vec2(t, 6.0)], Stroke::new(1.0, c));
                painter.line_segment([o + vec2(-6.0, t), o + vec2(6.0, t)], Stroke::new(1.0, c));
            }
        }
        Icon::Space => {
            painter.circle_stroke(o, r, s);
            painter.line_segment([o + vec2(-r, 0.0), o + vec2(r, 0.0)], Stroke::new(1.0, c));
            painter.add(Shape::closed_line(
                arc(o, 1.0, 0.0, std::f32::consts::TAU, 24)
                    .into_iter()
                    .map(|p| pos2(o.x + (p.x - o.x) * 3.0, o.y + (p.y - o.y) * r))
                    .collect(),
                Stroke::new(1.0, c),
            ));
        }
        Icon::Undo | Icon::Redo => {
            let flip = if icon == Icon::Undo { 1.0 } else { -1.0 };
            let centre = o + vec2(0.0, 1.5);
            let pts: Vec<Pos2> = arc(
                centre,
                5.0,
                std::f32::consts::PI,
                std::f32::consts::TAU + 0.6,
                16,
            )
            .into_iter()
            .map(|p| pos2(o.x + (p.x - o.x) * flip, p.y))
            .collect();
            let start = pts[0];
            painter.add(Shape::line(pts, s));
            head(painter, start + vec2(0.0, 1.5), vec2(0.0, 1.0), c);
        }
        Icon::Play => {
            painter.add(Shape::convex_polygon(
                vec![
                    o + vec2(-4.0, -6.0),
                    o + vec2(6.0, 0.0),
                    o + vec2(-4.0, 6.0),
                ],
                c,
                Stroke::NONE,
            ));
        }
        Icon::Stop => {
            painter.rect_filled(Rect::from_center_size(o, Vec2::splat(10.0)), 1.5, c);
        }
        Icon::Players => {
            for dx in [-3.5, 3.5] {
                painter.circle_stroke(o + vec2(dx, -3.0), 2.3, Stroke::new(1.3, c));
                painter.add(Shape::line(
                    arc(
                        o + vec2(dx, 5.5),
                        3.8,
                        std::f32::consts::PI,
                        std::f32::consts::TAU,
                        8,
                    ),
                    Stroke::new(1.3, c),
                ));
            }
        }
        Icon::Text(t) => {
            painter.text(o, egui::Align2::CENTER_CENTER, t, ui_tokens().body(), c);
        }
    }
}

/// Paints the Explorer icon of `class` centred in `rect`.
pub fn paint_class(painter: &egui::Painter, rect: Rect, class: &str, c: Color32) {
    let o = rect.center();
    let s = Stroke::new(1.3, c);
    match class {
        "Workspace" => paint(painter, rect, Icon::Space, c),
        "Folder" => {
            let body = Rect::from_center_size(o + vec2(0.0, 1.0), vec2(12.0, 8.5));
            painter.rect_stroke(body, 1.0, s, egui::StrokeKind::Middle);
            painter.line_segment(
                [
                    body.left_top() + vec2(0.5, -2.0),
                    body.left_top() + vec2(5.0, -2.0),
                ],
                s,
            );
        }
        "Model" => {
            let b = Rect::from_center_size(o, Vec2::splat(11.0));
            for (p, d) in [
                (b.left_top(), vec2(1.0, 1.0)),
                (b.right_top(), vec2(-1.0, 1.0)),
                (b.left_bottom(), vec2(1.0, -1.0)),
                (b.right_bottom(), vec2(-1.0, -1.0)),
            ] {
                painter.line_segment([p, p + vec2(d.x * 4.0, 0.0)], s);
                painter.line_segment([p, p + vec2(0.0, d.y * 4.0)], s);
            }
            painter.rect_filled(Rect::from_center_size(o, Vec2::splat(4.0)), 0.5, c);
        }
        "Part" | "SpawnLocation" | "Checkpoint" => {
            painter.rect_filled(Rect::from_center_size(o, Vec2::splat(10.0)), 1.5, c);
            if class != "Part" {
                painter.circle_filled(o, 2.0, ui_tokens().icon_hole);
            }
        }
        "Script" | "LocalScript" => {
            let page = Rect::from_center_size(o, vec2(10.0, 12.0));
            painter.rect_stroke(page, 1.0, s, egui::StrokeKind::Middle);
            for i in 0..3 {
                let y = page.top() + 3.5 + i as f32 * 2.7;
                painter.line_segment(
                    [pos2(page.left() + 2.5, y), pos2(page.right() - 2.5, y)],
                    Stroke::new(1.0, c),
                );
            }
        }
        "Players" | "Player" => paint(painter, rect, Icon::Players, c),
        "RemoteEvent" => {
            painter.line_segment([o + vec2(-6.0, -2.0), o + vec2(5.0, -2.0)], s);
            head(painter, o + vec2(6.0, -2.0), vec2(1.0, 0.0), c);
            painter.line_segment([o + vec2(6.0, 3.0), o + vec2(-5.0, 3.0)], s);
            head(painter, o + vec2(-6.0, 3.0), vec2(-1.0, 0.0), c);
        }
        "ScreenGui" | "Frame" | "TextLabel" | "TextButton" | "ImageButton" => {
            painter.rect_stroke(
                Rect::from_center_size(o, vec2(13.0, 9.0)),
                1.5,
                s,
                egui::StrokeKind::Middle,
            );
        }
        _ => {
            painter.rect_stroke(
                Rect::from_center_size(o, Vec2::splat(10.0)),
                3.0,
                s,
                egui::StrokeKind::Middle,
            );
        }
    }
}
