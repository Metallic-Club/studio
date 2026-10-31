//! The cut: Metallic's identity motif, one 45 degree chamfer on the top-right corner of the single
//! most important thing in a view (research doc section 3.3). The other three corners are rounded.
//!
//! Everything here is a convex polygon, so egui fills it with anti-aliased edges, and images can be
//! clipped to it with [`paint_image`].

use eframe::egui::{
    Color32, CornerRadius, Mesh, Painter, Pos2, Rect, Shape, Stroke, TextureId, epaint::Vertex,
};
use std::f32::consts::FRAC_PI_2;

/// The outline of `rect` with the top-right corner cut at 45 degrees by `cut` px and the other
/// corners rounded by `radius` (`radius.ne` is used only when `cut` is 0).
///
/// Points run clockwise on screen (y down), starting where the top edge leaves the top-left
/// corner. A square corner gives one point, a rounded one an arc, the cut two. `cut` and every
/// radius are clamped to half the shorter side.
pub fn points(rect: Rect, radius: CornerRadius, cut: f32) -> Vec<Pos2> {
    let max = 0.5 * rect.width().min(rect.height()).max(0.0);
    let r = |v: u8| f32::from(v).min(max);
    let cut = cut.clamp(0.0, max);
    let mut out = Vec::with_capacity(24);
    // Top-left: from the left edge up to the top edge (angles 180 -> 270 degrees on screen).
    corner(&mut out, rect.left_top(), r(radius.nw), 1.0, 1.0, 2.0);
    if cut > 0.0 {
        out.push(Pos2::new(rect.right() - cut, rect.top()));
        out.push(Pos2::new(rect.right(), rect.top() + cut));
    } else {
        corner(&mut out, rect.right_top(), r(radius.ne), -1.0, 1.0, 3.0);
    }
    corner(&mut out, rect.right_bottom(), r(radius.se), -1.0, -1.0, 0.0);
    corner(&mut out, rect.left_bottom(), r(radius.sw), 1.0, -1.0, 1.0);
    out
}

/// Adds the arc for one corner at `at`. `sx`/`sy` point from the corner into the rectangle,
/// `quarter` is the arc's start angle in quarter turns (screen angles, y down).
fn corner(out: &mut Vec<Pos2>, at: Pos2, r: f32, sx: f32, sy: f32, quarter: f32) {
    if r <= 0.0 {
        out.push(at);
        return;
    }
    let centre = Pos2::new(at.x + sx * r, at.y + sy * r);
    let steps = arc_steps(r);
    for i in 0..=steps {
        let a = (quarter + i as f32 / steps as f32) * FRAC_PI_2;
        out.push(Pos2::new(centre.x + r * a.cos(), centre.y + r * a.sin()));
    }
}

/// Segments per quarter circle: enough that a 10 px radius looks round, few for small ones.
fn arc_steps(r: f32) -> usize {
    ((r * 0.6).ceil() as usize).clamp(2, 8)
}

/// The cut of a shape inset by `inset` px on every side, so the chamfer edge stays parallel and
/// `inset` away from the outer one: `cut - inset * (2 - sqrt 2)`. A 2 px frame around a 12 px cut
/// leaves about 11 px for the plate inside it.
pub fn inset_cut(cut: f32, inset: f32) -> f32 {
    (cut - inset * (2.0 - std::f32::consts::SQRT_2)).max(0.0)
}

/// Shrinks every corner radius by `by` (for the plate inside a frame).
pub fn inset_radius(radius: CornerRadius, by: f32) -> CornerRadius {
    let shrink = |v: u8| (f32::from(v) - by).max(0.0).round() as u8;
    CornerRadius {
        nw: shrink(radius.nw),
        ne: shrink(radius.ne),
        sw: shrink(radius.sw),
        se: shrink(radius.se),
    }
}

/// A filled cut shape.
pub fn fill(painter: &Painter, rect: Rect, radius: CornerRadius, cut: f32, color: Color32) {
    painter.add(Shape::convex_polygon(
        points(rect, radius, cut),
        color,
        Stroke::NONE,
    ));
}

/// The frame variant: a ring of `width` px drawn inside `rect`, following the cut. Used for the
/// selected card and the primary button's focus ring.
pub fn frame(painter: &Painter, rect: Rect, radius: CornerRadius, cut: f32, stroke: Stroke) {
    let half = stroke.width / 2.0;
    let path = points(
        rect.shrink(half),
        inset_radius(radius, half),
        inset_cut(cut, half),
    );
    painter.add(Shape::closed_line(path, stroke));
}

/// Paints `texture` (its `uv` part) clipped to a cut shape: `rect` maps to `uv`, and pixels outside
/// the shape are not drawn. The edges are not anti-aliased, so keep a plate underneath.
pub fn paint_image(
    painter: &Painter,
    rect: Rect,
    radius: CornerRadius,
    cut: f32,
    texture: TextureId,
    uv: Rect,
    tint: Color32,
) {
    let outline = points(rect, radius, cut);
    painter.add(Shape::mesh(image_mesh(&outline, rect, texture, uv, tint)));
}

/// A triangle fan over the convex `outline`, with UVs mapped from `rect` to `uv`.
fn image_mesh(outline: &[Pos2], rect: Rect, texture: TextureId, uv: Rect, tint: Color32) -> Mesh {
    let mut mesh = Mesh::with_texture(texture);
    let to_uv = |p: Pos2| {
        let t = (p - rect.min) / rect.size();
        uv.min + t * uv.size()
    };
    for pos in std::iter::once(rect.center()).chain(outline.iter().copied()) {
        mesh.vertices.push(Vertex {
            pos,
            uv: to_uv(pos),
            color: tint,
        });
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{pos2, vec2};

    fn rect() -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), vec2(100.0, 40.0))
    }

    #[test]
    fn square_corners_and_a_cut_give_five_points_clockwise() {
        let p = points(rect(), CornerRadius::ZERO, 6.0);
        assert_eq!(
            p,
            vec![
                pos2(10.0, 20.0),
                pos2(104.0, 20.0),
                pos2(110.0, 26.0),
                pos2(110.0, 60.0),
                pos2(10.0, 60.0),
            ]
        );
        // Clockwise on screen (y down) means a positive shoelace sum.
        let area: f32 = (0..p.len())
            .map(|i| {
                let (a, b) = (p[i], p[(i + 1) % p.len()]);
                a.x * b.y - b.x * a.y
            })
            .sum();
        assert!(area > 0.0);
    }

    #[test]
    fn the_cut_is_45_degrees() {
        let p = points(rect(), CornerRadius::same(4), 14.0);
        let i = p
            .iter()
            .position(|q| (q.y - 20.0).abs() < 1e-4 && (q.x - 96.0).abs() < 1e-4)
            .expect("cut starts on the top edge");
        let d = p[i + 1] - p[i];
        assert!((d.x - 14.0).abs() < 1e-4 && (d.y - 14.0).abs() < 1e-4);
    }

    #[test]
    fn rounded_corners_stay_on_their_arcs_and_inside_the_rect() {
        let r = rect();
        let p = points(r, CornerRadius::same(10), 14.0);
        for q in &p {
            assert!(r.expand(1e-3).contains(*q), "{q:?}");
        }
        // The top-left arc: centre (20, 30), radius 10, ends on both edges.
        assert!((p[0] - pos2(10.0, 30.0)).length() < 1e-3);
        let tl_end = p[arc_steps(10.0)];
        assert!((tl_end - pos2(20.0, 20.0)).length() < 1e-3);
        for q in &p[..=arc_steps(10.0)] {
            assert!(((*q - pos2(20.0, 30.0)).length() - 10.0).abs() < 1e-3);
        }
        // The cut corner itself is never part of the outline.
        assert!(!p.contains(&r.right_top()));
    }

    #[test]
    fn without_a_cut_the_top_right_is_rounded_like_the_others() {
        let p = points(rect(), CornerRadius::same(6), 0.0);
        let per_corner = arc_steps(6.0) + 1;
        assert_eq!(p.len(), 4 * per_corner);
    }

    #[test]
    fn cut_and_radii_are_clamped_to_half_the_short_side() {
        let small = Rect::from_min_size(pos2(0.0, 0.0), vec2(30.0, 10.0));
        let p = points(small, CornerRadius::ZERO, 50.0);
        assert_eq!(p[1], pos2(25.0, 0.0));
        assert_eq!(p[2], pos2(30.0, 5.0));
    }

    #[test]
    fn frame_inset_keeps_the_chamfer_parallel() {
        assert!((inset_cut(12.0, 2.0) - 10.828).abs() < 1e-3);
        assert_eq!(inset_cut(1.0, 10.0), 0.0);
        // The inset chamfer line is exactly `inset` away from the outer one.
        let outer = points(rect(), CornerRadius::ZERO, 12.0);
        let inner = points(rect().shrink(2.0), CornerRadius::ZERO, inset_cut(12.0, 2.0));
        let n = vec2(1.0, -1.0).normalized();
        let dist = (outer[1] - inner[1]).dot(-n).abs();
        assert!((dist - 2.0).abs() < 1e-3, "{dist}");
        assert_eq!(
            inset_radius(CornerRadius::same(8), 2.0),
            CornerRadius::same(6)
        );
    }

    #[test]
    fn image_mesh_maps_the_rect_corners_to_the_uv_corners() {
        let r = rect();
        let outline = points(r, CornerRadius::ZERO, 6.0);
        let uv = Rect::from_min_max(pos2(0.25, 0.0), pos2(0.75, 1.0));
        let mesh = image_mesh(&outline, r, TextureId::default(), uv, Color32::WHITE);
        assert_eq!(mesh.vertices.len(), 6);
        assert_eq!(mesh.indices.len(), 15);
        assert_eq!(mesh.vertices[1].uv, pos2(0.25, 0.0));
        assert_eq!(mesh.vertices[4].uv, pos2(0.75, 1.0));
        assert_eq!(mesh.vertices[0].uv, pos2(0.5, 0.5));
    }
}
