//! Small painting helpers shared by the widgets: key glyphs, the time ruler, icons.

use crate::theme::Theme;
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use metallic_anim::Interpolation;

/// Draws a key glyph whose shape tells the interpolation: diamond = Bezier, circle = linear,
/// square = constant, diamond with a dot = eased.
pub fn key_glyph(
    p: &Painter,
    theme: &Theme,
    center: Pos2,
    radius: f32,
    interp: Interpolation,
    selected: bool,
    hovered: bool,
) {
    let fill = if selected {
        theme.accent
    } else if hovered {
        theme.text
    } else {
        theme.silver
    };
    let outline = Stroke::new(1.0, theme.deep);
    let r = radius;
    match interp {
        Interpolation::Bezier | Interpolation::Eased(..) => {
            let pts = vec![
                pos2(center.x, center.y - r),
                pos2(center.x + r, center.y),
                pos2(center.x, center.y + r),
                pos2(center.x - r, center.y),
            ];
            p.add(Shape::convex_polygon(pts, fill, outline));
            if matches!(interp, Interpolation::Eased(..)) {
                p.circle_filled(center, r * 0.32, theme.deep);
            }
        }
        Interpolation::Linear => {
            p.circle(center, r * 0.85, fill, outline);
        }
        Interpolation::Constant => {
            let s = r * 0.78;
            let rect = Rect::from_center_size(center, vec2(2.0 * s, 2.0 * s));
            p.rect(rect, 1.0, fill, outline, StrokeKind::Middle);
        }
    }
}

/// The frame number at `time` (the Studio timeline counts frames, not seconds).
pub fn format_time(time: f32, fps: f32) -> String {
    format!("{}", (time * fps.max(1.0)).round() as i64)
}

/// Picks a ruler label step in frames (5, 10, 20, 50...) so labels are at least 40 px apart.
pub fn tick_step_frames(px_per_frame: f32, _fps: f32) -> i64 {
    for c in [5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000] {
        if c as f32 * px_per_frame >= 40.0 {
            return c;
        }
    }
    10_000
}

/// Paints the time ruler into `rect` (ticks, labels, clip end) and grid lines into `grid_rect`.
#[allow(clippy::too_many_arguments)]
pub fn ruler(
    p: &Painter,
    theme: &Theme,
    rect: Rect,
    grid_rect: Rect,
    view: &crate::state::TimeView,
    fps: f32,
    duration: f32,
) {
    p.rect_filled(rect, 0.0, theme.panel);
    p.hline(rect.x_range(), rect.bottom() - 0.5, theme.hairline());
    let left = rect.left();
    let px_per_frame = view.px_per_sec / fps.max(1.0);
    let step = tick_step_frames(px_per_frame, fps);
    let minor = (step / 5).max(1);
    let t0 = view.time(left, rect.left());
    let t1 = view.time(left, rect.right());
    let f0 = ((t0 * fps).floor() as i64 / minor - 1) * minor;
    let f1 = (t1 * fps).ceil() as i64 + minor;
    let per_sec = fps.round().max(1.0) as i64;
    let font = FontId::monospace(11.0);
    let mut f = f0.max(0);
    while f <= f1 {
        let t = f as f32 / fps;
        let x = view.x(left, t).round() + 0.5;
        if x >= rect.left() && x <= rect.right() {
            let major = f % step == 0;
            let h = if major { 9.0 } else { 4.0 };
            p.vline(
                x,
                (rect.bottom() - h)..=rect.bottom(),
                Stroke::new(1.0, if major { theme.text_meta } else { theme.line }),
            );
            if major {
                p.text(
                    pos2(x + 4.0, rect.top() + 4.0),
                    Align2::LEFT_TOP,
                    format_time(t, fps),
                    font.clone(),
                    theme.text_dim,
                );
                let strong = f % per_sec == 0;
                let c = if strong {
                    theme.line.gamma_multiply(0.9)
                } else {
                    theme.grid
                };
                p.vline(x, grid_rect.y_range(), Stroke::new(1.0, c));
            }
        }
        f += minor;
    }
    // Clip end.
    let xe = view.x(left, duration);
    if xe < grid_rect.right() {
        let shade = Rect::from_min_max(
            pos2(xe.max(grid_rect.left()), grid_rect.top()),
            grid_rect.max,
        );
        p.rect_filled(shade, 0.0, Color32::from_black_alpha(70));
        p.vline(
            xe,
            rect.top()..=grid_rect.bottom(),
            Stroke::new(1.0, theme.text_meta.gamma_multiply(0.7)),
        );
    }
    let xs = view.x(left, 0.0);
    if xs > grid_rect.left() {
        let shade = Rect::from_min_max(
            grid_rect.min,
            pos2(xs.min(grid_rect.right()), grid_rect.bottom()),
        );
        p.rect_filled(shade, 0.0, Color32::from_black_alpha(70));
    }
}

/// Paints the playhead line and its time badge.
pub fn playhead(p: &Painter, theme: &Theme, x: f32, ruler: Rect, bottom: f32, label: &str) {
    if x < ruler.left() - 1.0 || x > ruler.right() + 1.0 {
        return;
    }
    p.vline(x, ruler.bottom()..=bottom, Stroke::new(1.5, theme.accent));
    let font = FontId::monospace(10.5);
    let galley = p.layout_no_wrap(label.to_owned(), font, theme.on_accent);
    let w = galley.size().x + 10.0;
    let badge = Rect::from_center_size(pos2(x, ruler.center().y + 1.0), vec2(w, 16.0));
    p.rect_filled(badge, f32::from(crate::theme::R_SM), theme.accent);
    p.galley(
        badge.center() - galley.size() / 2.0,
        galley,
        theme.on_accent,
    );
    let tip = vec![
        pos2(x - 4.0, badge.bottom()),
        pos2(x + 4.0, badge.bottom()),
        pos2(x, badge.bottom() + 4.0),
    ];
    p.add(Shape::convex_polygon(tip, theme.accent, Stroke::NONE));
}

/// Transport and toolbar icons, drawn as shapes so they look the same on every platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// Jump to start.
    ToStart,
    /// Previous key.
    PrevKey,
    /// Play.
    Play,
    /// Pause.
    Pause,
    /// Next key.
    NextKey,
    /// Jump to end.
    ToEnd,
    /// Loop.
    Loop,
    /// Magnet (snapping).
    Snap,
    /// Frame all.
    FrameAll,
    /// Add key.
    AddKey,
}

/// Paints `icon` centred in `rect`.
pub fn icon(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let s = rect.height().min(rect.width()) * 0.32;
    let tri = |dir: f32, cx: f32| -> Shape {
        let pts = if dir > 0.0 {
            vec![
                pos2(cx - s * 0.6, c.y - s),
                pos2(cx + s * 0.75, c.y),
                pos2(cx - s * 0.6, c.y + s),
            ]
        } else {
            vec![
                pos2(cx + s * 0.6, c.y - s),
                pos2(cx + s * 0.6, c.y + s),
                pos2(cx - s * 0.75, c.y),
            ]
        };
        Shape::convex_polygon(pts, color, Stroke::NONE)
    };
    let bar = |x: f32, w: f32| Rect::from_center_size(pos2(x, c.y), vec2(w, s * 2.0));
    match icon {
        Icon::Play => {
            p.add(tri(1.0, c.x + s * 0.1));
        }
        Icon::Pause => {
            p.rect_filled(bar(c.x - s * 0.45, s * 0.5), 1.0, color);
            p.rect_filled(bar(c.x + s * 0.45, s * 0.5), 1.0, color);
        }
        Icon::ToStart => {
            p.rect_filled(bar(c.x - s * 0.8, s * 0.28), 0.5, color);
            p.add(tri(-1.0, c.x + s * 0.15));
        }
        Icon::ToEnd => {
            p.rect_filled(bar(c.x + s * 0.8, s * 0.28), 0.5, color);
            p.add(tri(1.0, c.x - s * 0.15));
        }
        Icon::PrevKey | Icon::NextKey => {
            let dir = if icon == Icon::NextKey { 1.0 } else { -1.0 };
            let d = s * 0.55;
            let k = vec![
                pos2(c.x - dir * s * 0.55, c.y - d),
                pos2(c.x - dir * s * 0.55 + d, c.y),
                pos2(c.x - dir * s * 0.55, c.y + d),
                pos2(c.x - dir * s * 0.55 - d, c.y),
            ];
            p.add(Shape::convex_polygon(k, color, Stroke::NONE));
            let ax = c.x + dir * s * 0.55;
            let pts = vec![
                pos2(ax - dir * s * 0.35, c.y - s * 0.55),
                pos2(ax + dir * s * 0.45, c.y),
                pos2(ax - dir * s * 0.35, c.y + s * 0.55),
            ];
            p.add(Shape::line(pts, Stroke::new(1.5, color)));
        }
        Icon::Loop => {
            let r = Rect::from_center_size(c, vec2(s * 2.2, s * 1.4));
            p.rect_stroke(r, s * 0.7, Stroke::new(1.5, color), StrokeKind::Middle);
            let a = pos2(r.right(), c.y - s * 0.1);
            p.add(Shape::convex_polygon(
                vec![
                    pos2(a.x - s * 0.4, a.y - s * 0.1),
                    pos2(a.x + s * 0.4, a.y - s * 0.1),
                    pos2(a.x, a.y + s * 0.4),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Snap => {
            // A magnet: U shape with tips.
            let w = s * 0.9;
            let pts: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let a = std::f32::consts::PI * i as f32 / 12.0;
                    pos2(c.x - w * a.cos(), c.y + s * 0.15 + w * a.sin())
                })
                .collect();
            let mut path = vec![pos2(c.x - w, c.y - s)];
            path.extend(pts);
            path.push(pos2(c.x + w, c.y - s));
            p.add(Shape::line(path, Stroke::new(2.2, color)));
            p.hline(
                (c.x - w - 1.1)..=(c.x - w + 1.1),
                c.y - s * 0.55,
                Stroke::new(2.0, Color32::from_black_alpha(120)),
            );
        }
        Icon::FrameAll => {
            let r = Rect::from_center_size(c, vec2(s * 2.0, s * 2.0));
            let l = s * 0.7;
            for (corner, dx, dy) in [
                (r.left_top(), 1.0, 1.0),
                (r.right_top(), -1.0, 1.0),
                (r.left_bottom(), 1.0, -1.0),
                (r.right_bottom(), -1.0, -1.0),
            ] {
                p.line_segment(
                    [corner, corner + vec2(dx * l, 0.0)],
                    Stroke::new(1.5, color),
                );
                p.line_segment(
                    [corner, corner + vec2(0.0, dy * l)],
                    Stroke::new(1.5, color),
                );
            }
        }
        Icon::AddKey => {
            let d = s * 0.8;
            let k = vec![
                pos2(c.x - s * 0.3, c.y - d),
                pos2(c.x - s * 0.3 + d, c.y),
                pos2(c.x - s * 0.3, c.y + d),
                pos2(c.x - s * 0.3 - d, c.y),
            ];
            p.add(Shape::convex_polygon(
                k,
                Color32::TRANSPARENT,
                Stroke::new(1.5, color),
            ));
            let px = c.x + s * 0.85;
            let py = c.y - s * 0.55;
            p.hline((px - s * 0.4)..=(px + s * 0.4), py, Stroke::new(1.5, color));
            p.vline(px, (py - s * 0.4)..=(py + s * 0.4), Stroke::new(1.5, color));
        }
    }
}

/// A flat icon button. `active` shows the accent background (for toggles).
pub fn icon_button(
    ui: &mut egui::Ui,
    theme: &Theme,
    ic: Icon,
    active: bool,
    tooltip: &str,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 24.0), egui::Sense::click());
    let p = ui.painter();
    let bg = if active {
        theme.active
    } else if resp.hovered() {
        theme.hover
    } else {
        Color32::TRANSPARENT
    };
    p.rect_filled(rect, f32::from(crate::theme::R_MD), bg);
    let col = if active || resp.hovered() {
        theme.text
    } else {
        theme.silver
    };
    icon(p, rect, ic, col);
    resp.on_hover_text(tooltip)
}
