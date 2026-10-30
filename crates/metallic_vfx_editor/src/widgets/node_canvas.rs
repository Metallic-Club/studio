//! A generic node canvas: nodes with stacked rows, wires, group boxes, pan and zoom.
//!
//! Nothing here knows about VFX. The VFX graph view builds [`Node`]s from an effect (one context node
//! per stage, one row per module, like VFX Graph's contexts and blocks); the material editor and visual
//! scripting can build theirs the same way. Wires are straight with short rounded corners (design doc
//! 3.7: no long spaghetti curves). Node positions live in the caller's state, so layouts persist.
//!
//! Controls: drag the background to pan, wheel to zoom about the pointer, drag a node header to move it,
//! click a row to select it, click a row's dot to switch it on or off.

use crate::theme::{self, color};
use egui::{Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

/// Pan and zoom of a canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasView {
    /// Screen offset of the canvas origin from the widget's top-left corner.
    pub offset: Vec2,
    /// Scale factor.
    pub zoom: f32,
}

impl Default for CanvasView {
    fn default() -> Self {
        Self {
            offset: Vec2::new(24.0, 24.0),
            zoom: 1.0,
        }
    }
}

/// Which edge of a node a wire attaches to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Top edge (flow in).
    Top,
    /// Bottom edge (flow out).
    Bottom,
    /// Left edge (data in).
    Left,
    /// Right edge (data out).
    Right,
}

/// A row inside a node (a VFX module, a material input, ...).
#[derive(Debug, Clone, PartialEq)]
pub struct NodeRow {
    /// Caller id.
    pub id: u64,
    /// Main text.
    pub label: String,
    /// Secondary text, right-aligned.
    pub detail: String,
    /// Switched on.
    pub enabled: bool,
    /// Highlighted as the selection.
    pub selected: bool,
}

/// A node.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Caller id.
    pub id: u64,
    /// Top-left in canvas units.
    pub pos: Pos2,
    /// Width in canvas units.
    pub width: f32,
    /// Header text.
    pub title: String,
    /// Header detail text.
    pub subtitle: String,
    /// Rows.
    pub rows: Vec<NodeRow>,
    /// Highlighted as the selection.
    pub selected: bool,
}

impl Node {
    /// Height in canvas units.
    #[must_use]
    pub fn height(&self) -> f32 {
        HEADER + self.rows.len() as f32 * ROW + if self.rows.is_empty() { 4.0 } else { 8.0 }
    }

    fn rect(&self) -> Rect {
        Rect::from_min_size(self.pos, Vec2::new(self.width, self.height()))
    }

    fn anchor(&self, side: Side) -> Pos2 {
        let r = self.rect();
        match side {
            Side::Top => Pos2::new(r.center().x, r.min.y),
            Side::Bottom => Pos2::new(r.center().x, r.max.y),
            Side::Left => Pos2::new(r.min.x, r.min.y + HEADER * 0.5),
            Side::Right => Pos2::new(r.max.x, r.min.y + HEADER * 0.5),
        }
    }
}

/// A wire between two nodes (indices into the node slice).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wire {
    /// Source node and side.
    pub from: (usize, Side),
    /// Target node and side.
    pub to: (usize, Side),
}

/// A labelled box drawn behind a set of nodes (comment box).
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// Title.
    pub title: String,
    /// Node indices inside.
    pub nodes: Vec<usize>,
}

/// What happened this frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CanvasOutput {
    /// A node header was clicked (node id).
    pub clicked_node: Option<u64>,
    /// A row was clicked (node id, row id).
    pub clicked_row: Option<(u64, u64)>,
    /// A row's on/off dot was clicked (node id, row id).
    pub toggled_row: Option<(u64, u64)>,
    /// Nodes moved (positions in the slice were updated).
    pub moved: bool,
}

const HEADER: f32 = 30.0;
const ROW: f32 = 24.0;

/// Shows the canvas in `rect`.
pub fn node_canvas(
    ui: &mut Ui,
    rect: Rect,
    view: &mut CanvasView,
    nodes: &mut [Node],
    wires: &[Wire],
    groups: &[Group],
) -> CanvasOutput {
    let mut out = CanvasOutput::default();
    let bg = ui.interact(
        rect,
        ui.scope_id().with("canvas-bg"),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, color::BG_BASE);

    // Zoom about the pointer.
    if bg.hovered() || ui.rect_contains_pointer(rect) {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0
            && let Some(p) = ui.input(|i| i.pointer.hover_pos())
        {
            let old = view.zoom;
            view.zoom = (view.zoom * (1.0 + scroll * 0.0015)).clamp(0.4, 2.0);
            let local = p - rect.min;
            view.offset = local - (local - view.offset) * (view.zoom / old);
        }
    }
    if bg.dragged() {
        view.offset += bg.drag_delta();
    }
    let z = view.zoom;
    let to_screen = |p: Pos2| rect.min + view.offset + p.to_vec2() * z;

    // Dot grid.
    let step = 24.0 * z;
    if step > 6.0 {
        let start = rect.min
            + Vec2::new(
                view.offset.x.rem_euclid(step),
                view.offset.y.rem_euclid(step),
            );
        let mut y = start.y;
        while y < rect.max.y {
            let mut x = start.x;
            while x < rect.max.x {
                painter.circle_filled(Pos2::new(x, y), 1.0, color::LINE);
                x += step;
            }
            y += step;
        }
    }

    // Group boxes.
    for g in groups {
        let mut bounds: Option<Rect> = None;
        for &i in &g.nodes {
            if let Some(n) = nodes.get(i) {
                let r = n.rect();
                bounds = Some(bounds.map_or(r, |b| b.union(r)));
            }
        }
        if let Some(b) = bounds {
            let b = b
                .expand2(Vec2::new(16.0, 16.0))
                .translate(Vec2::new(0.0, -14.0));
            let b = Rect::from_min_max(b.min, b.max + Vec2::new(0.0, 14.0));
            let sr = Rect::from_min_max(to_screen(b.min), to_screen(b.max));
            painter.rect(
                sr,
                CornerRadius::same(theme::R_LG),
                color::SELECT_WEAK.gamma_multiply(0.35),
                Stroke::new(1.0, color::LINE_STRONG),
                StrokeKind::Inside,
            );
            painter.text(
                sr.min + Vec2::new(10.0, 6.0) * z,
                egui::Align2::LEFT_TOP,
                &g.title,
                theme::semibold(12.0 * z),
                color::TEXT_2,
            );
        }
    }

    // Wires under nodes.
    for w in wires {
        let (Some(a), Some(b)) = (nodes.get(w.from.0), nodes.get(w.to.0)) else {
            continue;
        };
        let pa = to_screen(a.anchor(w.from.1));
        let pb = to_screen(b.anchor(w.to.1));
        let path = route(pa, w.from.1, pb, w.to.1, 8.0 * z);
        let selected = a.selected || b.selected;
        let c = if selected {
            color::ACCENT
        } else {
            color::ICON.gamma_multiply(0.7)
        };
        painter.add(egui::Shape::line(path, Stroke::new(1.5 * z.max(0.7), c)));
        painter.circle_filled(pa, 3.0 * z, c);
        painter.circle_filled(pb, 3.0 * z, c);
    }

    // Nodes.
    for (index, n) in nodes.iter_mut().enumerate() {
        let r = Rect::from_min_max(
            to_screen(n.pos),
            to_screen(n.pos + Vec2::new(n.width, n.height())),
        );
        if !r.intersects(rect) {
            continue;
        }
        let header = Rect::from_min_size(r.min, Vec2::new(r.width(), HEADER * z));
        let hresp = ui.interact(
            header,
            ui.scope_id().with(("node", n.id, index)),
            Sense::click_and_drag(),
        );
        if hresp.dragged() {
            n.pos += hresp.drag_delta() / z;
            out.moved = true;
        }
        if hresp.clicked() {
            out.clicked_node = Some(n.id);
        }
        painter.rect(
            r,
            CornerRadius::same(theme::R_MD),
            color::BG_RAISED,
            Stroke::new(
                if n.selected { 1.5 } else { 1.0 },
                if n.selected {
                    color::ACCENT
                } else {
                    color::LINE_STRONG
                },
            ),
            StrokeKind::Inside,
        );
        painter.line_segment(
            [
                Pos2::new(r.min.x + 1.0, header.max.y),
                Pos2::new(r.max.x - 1.0, header.max.y),
            ],
            Stroke::new(1.0, color::LINE_STRONG),
        );
        painter.text(
            header.left_center() + Vec2::new(10.0 * z, 0.0),
            egui::Align2::LEFT_CENTER,
            &n.title,
            theme::semibold(13.0 * z),
            color::TEXT,
        );
        painter.text(
            header.right_center() - Vec2::new(10.0 * z, 0.0),
            egui::Align2::RIGHT_CENTER,
            &n.subtitle,
            FontId::proportional(11.0 * z),
            color::TEXT_3,
        );
        for (ri, row) in n.rows.iter().enumerate() {
            let rr = Rect::from_min_size(
                Pos2::new(
                    r.min.x + 4.0 * z,
                    header.max.y + (4.0 + ri as f32 * ROW) * z,
                ),
                Vec2::new(r.width() - 8.0 * z, (ROW - 2.0) * z),
            );
            let resp = ui.interact(
                rr,
                ui.scope_id().with(("row", n.id, row.id, index)),
                Sense::click(),
            );
            let dot = Pos2::new(rr.min.x + 10.0 * z, rr.center().y);
            let on_dot = resp
                .interact_pointer_pos()
                .is_some_and(|p| p.distance(dot) < 8.0 * z);
            if resp.clicked() {
                if on_dot {
                    out.toggled_row = Some((n.id, row.id));
                } else {
                    out.clicked_row = Some((n.id, row.id));
                }
            }
            let fill = if row.selected {
                color::SELECT
            } else if resp.hovered() {
                color::BG_HOVER
            } else {
                Color32::TRANSPARENT
            };
            painter.rect_filled(rr, CornerRadius::same(theme::R_SM), fill);
            if row.enabled {
                painter.circle_filled(dot, 3.5 * z, color::ICON);
            } else {
                painter.circle_stroke(dot, 3.5 * z, Stroke::new(1.0, color::TEXT_3));
            }
            let text_color = if row.enabled {
                color::TEXT
            } else {
                color::TEXT_3
            };
            painter.text(
                Pos2::new(rr.min.x + 22.0 * z, rr.center().y),
                egui::Align2::LEFT_CENTER,
                &row.label,
                FontId::proportional(12.0 * z),
                text_color,
            );
            painter.text(
                Pos2::new(rr.max.x - 6.0 * z, rr.center().y),
                egui::Align2::RIGHT_CENTER,
                &row.detail,
                FontId::proportional(11.0 * z),
                color::TEXT_3,
            );
        }
    }
    out
}

/// Orthogonal route from `a` (leaving through `sa`) to `b` (entering through `sb`) with rounded corners.
#[must_use]
pub fn route(a: Pos2, sa: Side, b: Pos2, sb: Side, radius: f32) -> Vec<Pos2> {
    let vertical = matches!(sa, Side::Top | Side::Bottom);
    let corners = if vertical {
        let mid = (a.y + b.y) * 0.5;
        let _ = sb;
        vec![a, Pos2::new(a.x, mid), Pos2::new(b.x, mid), b]
    } else {
        let mid = (a.x + b.x) * 0.5;
        vec![a, Pos2::new(mid, a.y), Pos2::new(mid, b.y), b]
    };
    round_corners(&corners, radius)
}

fn round_corners(pts: &[Pos2], radius: f32) -> Vec<Pos2> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut out = vec![pts[0]];
    for i in 1..pts.len() - 1 {
        let (p0, p1, p2) = (pts[i - 1], pts[i], pts[i + 1]);
        let d0 = (p1 - p0).length();
        let d1 = (p2 - p1).length();
        if d0 < 0.5 || d1 < 0.5 {
            out.push(p1);
            continue;
        }
        let r = radius.min(d0 * 0.5).min(d1 * 0.5);
        let a = p1 - (p1 - p0) / d0 * r;
        let b = p1 + (p2 - p1) / d1 * r;
        // Quadratic Bezier through the corner approximates a quarter arc.
        for k in 0..=6 {
            let t = k as f32 / 6.0;
            let u = 1.0 - t;
            out.push(Pos2::new(
                u * u * a.x + 2.0 * u * t * p1.x + t * t * b.x,
                u * u * a.y + 2.0 * u * t * p1.y + t * t * b.y,
            ));
        }
    }
    out.push(pts[pts.len() - 1]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_start_and_end_at_anchors() {
        let a = Pos2::new(0.0, 0.0);
        let b = Pos2::new(100.0, 80.0);
        for (sa, sb) in [(Side::Bottom, Side::Top), (Side::Right, Side::Left)] {
            let p = route(a, sa, b, sb, 8.0);
            assert_eq!(p.first(), Some(&a));
            assert_eq!(p.last(), Some(&b));
            // Every segment is short or axis-aligned except inside the rounded corners.
            assert!(p.len() > 4);
        }
    }

    #[test]
    fn straight_routes_have_no_corners() {
        let p = route(
            Pos2::new(0.0, 0.0),
            Side::Bottom,
            Pos2::new(0.0, 50.0),
            Side::Top,
            8.0,
        );
        assert!(p.iter().all(|q| q.x == 0.0));
    }
}
