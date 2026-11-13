//! Many screen lines as one mesh (#597): the selection's outlines with thousands selected. Each `Painter::line_segment`
//! is a shape of its own that egui then tessellates one by one, so 10,000 selected parts' twelve edges each took most
//! of a 50 ms frame. [`Lines`] builds the same strokes (a solid core with a one-pixel fade on each side, as egui
//! draws a line) straight into a single mesh, and leaves out what is wholly outside the panel.

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, Pos2, Rect, Shape, Stroke, vec2};

/// Screen lines gathered into one mesh. See the [module documentation](self).
#[derive(Debug)]
pub struct Lines {
    mesh: Mesh,
    /// Lines wholly outside it are left out.
    clip: Rect,
    /// Lines fade out over a pixel at their sides (anti-aliased), rather than ending sharply.
    smooth: bool,
    /// The fade on each side, in points: one physical pixel.
    feather: f32,
}

impl Lines {
    /// No lines yet, for a panel at `clip` drawn at `pixels_per_point`.
    pub fn new(clip: Rect, pixels_per_point: f32) -> Self {
        Self {
            mesh: Mesh::default(),
            clip,
            smooth: true,
            feather: 1.0 / pixels_per_point.max(0.1),
        }
    }

    /// Draws the lines added from now on with sharp sides, half the geometry of smooth ones: for thousands of lines,
    /// where egui's work on the mesh grows with its size and one pixel of fade cannot be told apart.
    pub fn set_smooth(&mut self, smooth: bool) {
        self.smooth = smooth;
    }

    /// Makes room for `lines` more lines.
    pub fn reserve(&mut self, lines: usize) {
        self.mesh.vertices.reserve(lines * 8);
        self.mesh.indices.reserve(lines * 18);
    }

    /// Whether anything inside `area` can show on the panel: what is wholly outside it is not drawn.
    pub fn meets(&self, area: Rect) -> bool {
        self.clip.expand(4.0).intersects(area)
    }

    /// Adds the line from `a` to `b` with `stroke`.
    pub fn segment(&mut self, [a, b]: [Pos2; 2], stroke: Stroke) {
        let reach = stroke.width + self.feather;
        let outside = (a.x.max(b.x) < self.clip.left() - reach)
            || (a.x.min(b.x) > self.clip.right() + reach)
            || (a.y.max(b.y) < self.clip.top() - reach)
            || (a.y.min(b.y) > self.clip.bottom() + reach);
        let along = b - a;
        let length = along.length();
        if outside || length < 1e-4 || stroke.color == Color32::TRANSPARENT || stroke.width <= 0.0 {
            return;
        }
        let across = vec2(-along.y, along.x) / length;
        let first = u32::try_from(self.mesh.vertices.len()).unwrap_or(u32::MAX);
        let clear = Color32::TRANSPARENT;
        let mut vertex = |pos: Pos2, color: Color32| {
            self.mesh.vertices.push(Vertex {
                pos,
                uv: WHITE_UV,
                color,
            });
        };
        if !self.smooth {
            // One band, solid to its edges.
            let half = stroke.width.max(self.feather) * 0.5;
            for end in [a, b] {
                vertex(end - across * half, stroke.color);
                vertex(end + across * half, stroke.color);
            }
            self.mesh
                .indices
                .extend_from_slice(&[first, first + 1, first + 3, first, first + 3, first + 2]);
            return;
        }
        let core = (stroke.width - self.feather).max(0.0) * 0.5;
        let fade = core + self.feather;
        // Across the line, from one edge of the fade to the other: clear, solid, solid, clear; at `a` and then `b`.
        for end in [a, b] {
            for (offset, color) in [
                (-fade, clear),
                (-core, stroke.color),
                (core, stroke.color),
                (fade, clear),
            ] {
                vertex(end + across * offset, color);
            }
        }
        for band in 0..3 {
            let (a0, a1, b0, b1) = (first + band, first + band + 1, first + 4 + band, first + 5 + band);
            self.mesh.indices.extend_from_slice(&[a0, a1, b1, a0, b1, b0]);
        }
    }

    /// Adds a square dot at `at`, as wide as two of `stroke`'s lines: what a box a few pixels across is drawn as, its
    /// twelve edges on top of one another.
    pub fn dot(&mut self, at: Pos2, stroke: Stroke) {
        let half = stroke.width.max(1.0);
        if !self.meets(Rect::from_center_size(at, vec2(half, half) * 2.0)) || stroke.color == Color32::TRANSPARENT {
            return;
        }
        let first = u32::try_from(self.mesh.vertices.len()).unwrap_or(u32::MAX);
        for corner in [
            vec2(-half, -half),
            vec2(half, -half),
            vec2(half, half),
            vec2(-half, half),
        ] {
            self.mesh.vertices.push(Vertex {
                pos: at + corner,
                uv: WHITE_UV,
                color: stroke.color,
            });
        }
        self.mesh
            .indices
            .extend_from_slice(&[first, first + 1, first + 2, first, first + 2, first + 3]);
    }

    /// Whether no line was added.
    pub fn is_empty(&self) -> bool {
        self.mesh.indices.is_empty()
    }

    /// The lines as one shape.
    pub fn into_shape(self) -> Shape {
        Shape::mesh(self.mesh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_three_bands_and_one_outside_the_panel_is_left_out() {
        let clip = Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0));
        let mut lines = Lines::new(clip, 1.0);
        lines.segment(
            [Pos2::new(10.0, 10.0), Pos2::new(90.0, 10.0)],
            Stroke::new(2.0, Color32::RED),
        );
        lines.segment(
            [Pos2::new(200.0, 10.0), Pos2::new(300.0, 10.0)],
            Stroke::new(2.0, Color32::RED),
        );
        lines.segment(
            [Pos2::new(50.0, 50.0), Pos2::new(50.0, 50.0)],
            Stroke::new(2.0, Color32::RED),
        );
        assert!(!lines.is_empty());
        let Shape::Mesh(mesh) = lines.into_shape() else {
            panic!("one mesh");
        };
        assert_eq!(mesh.vertices.len(), 8, "one line");
        assert_eq!(mesh.indices.len(), 18);
        // 2 points wide with a 1-point fade: solid from y 9.5 to 10.5, clear at 8.5 and 11.5.
        let ys: Vec<f32> = mesh.vertices[..4].iter().map(|v| v.pos.y).collect();
        assert_eq!(ys, [8.5, 9.5, 10.5, 11.5]);
        assert_eq!(mesh.vertices[1].color, Color32::RED);
        assert_eq!(mesh.vertices[0].color, Color32::TRANSPARENT);

        // Sharp: one solid band, half the vertices; a dot is one square.
        let mut sharp = Lines::new(clip, 1.0);
        sharp.set_smooth(false);
        sharp.segment(
            [Pos2::new(10.0, 10.0), Pos2::new(90.0, 10.0)],
            Stroke::new(2.0, Color32::RED),
        );
        sharp.dot(Pos2::new(50.0, 50.0), Stroke::new(2.0, Color32::RED));
        let Shape::Mesh(mesh) = sharp.into_shape() else {
            panic!("one mesh");
        };
        assert_eq!(mesh.vertices.len(), 8);
        assert_eq!(mesh.indices.len(), 12);
        assert!(mesh.vertices.iter().all(|v| v.color == Color32::RED));
        assert_eq!(mesh.vertices[0].pos.y, 9.0);
    }
}
