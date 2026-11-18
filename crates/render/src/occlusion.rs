//! Occlusion culling (#836): instances hidden behind large opaque blocks are not drawn.
//!
//! A camera's cull ([`crate::scene`]'s grid) picks the nearest large opaque blocks in view as **occluders** and draws
//! them, on the CPU, into a small depth buffer ([`DepthTiles`], about 128 pixels wide): each pixel holds the view depth
//! behind which something solid covers the *whole* pixel, or nothing. A pyramid of maxima over it then answers, for a
//! box's screen rectangle, how far the farthest of those depths is; a box whose nearest point is farther than that is
//! wholly hidden. Cells of the grid are tested first (a hidden cell skips its items unread), then large items and each
//! item of a cell that is not hidden.
//!
//! It never hides what can be seen:
//!
//! - an occluder covers a pixel only when the pixel lies wholly inside one of its faces, and at the farthest depth the
//!   face has over the pixel;
//! - an occluder is a `Block` part, opaque, with no corner behind the near plane (so its box is solid and wholly in
//!   front of the camera); balls, wedges, meshes and see-through parts never hide anything;
//! - a tested box with a corner behind the near plane, or off the buffer, is kept.
//!
//! The buffer is a few thousand pixels, so drawing a handful of occluders and testing thousands of boxes costs far less
//! than drawing the hidden instances; a view with no occluder in it pays only for looking for one.

use metallic_core::glam::{Mat4, Vec2, Vec3, Vec4};

/// The buffer's width in pixels; its height follows the view's aspect.
const WIDTH: usize = 128;

/// The nearest a corner may be to the camera (its view depth) for its box to be drawn or tested.
const NEAR: f32 = 0.05;

/// The depth of a pixel nothing covers.
const EMPTY: f32 = f32::INFINITY;

/// The small depth buffer occluders are drawn into, and its pyramid of maxima.
#[derive(Debug, Default)]
pub(crate) struct DepthTiles {
    /// The scene's (origin-relative) positions to clip space.
    clip: Mat4,
    width: usize,
    height: usize,
    /// Level 0 is the buffer; each next level holds the largest of each 2 × 2 of the one before.
    levels: Vec<Vec<f32>>,
    /// Each level's width and height.
    sizes: Vec<(usize, usize)>,
    /// How many occluders were drawn since [`DepthTiles::begin`].
    drawn: usize,
}

impl DepthTiles {
    /// Starts a new buffer for the view `clip` (scene positions to clip space) with `aspect` (width over height).
    pub(crate) fn begin(&mut self, clip: Mat4, aspect: f32) {
        self.clip = clip;
        self.width = WIDTH;
        self.height = ((WIDTH as f32 / aspect.max(0.1)).round() as usize).clamp(16, 2 * WIDTH);
        self.drawn = 0;
        // The levels' vectors are kept frame to frame: only `sizes` says how many are in use.
        if self.levels.is_empty() {
            self.levels.push(Vec::new());
        }
        let base = &mut self.levels[0];
        base.clear();
        base.resize(self.width * self.height, EMPTY);
        self.sizes.clear();
        self.sizes.push((self.width, self.height));
    }

    /// How many occluders were drawn.
    pub(crate) fn drawn(&self) -> usize {
        self.drawn
    }

    /// `p` in clip space.
    fn project(&self, p: Vec3) -> Vec4 {
        self.clip * p.extend(1.0)
    }

    /// A clip-space point on the buffer: its pixel position (y down) and its view depth.
    fn to_pixels(&self, c: Vec4) -> (Vec2, f32) {
        let ndc = Vec2::new(c.x, c.y) / c.w;
        let x = (ndc.x * 0.5 + 0.5) * self.width as f32;
        let y = (0.5 - ndc.y * 0.5) * self.height as f32;
        (Vec2::new(x, y), c.w)
    }

    /// Draws the box `rows` places (a unit cube's rotation × scale in `xyz` and its centre in `w`, one row per world
    /// axis, as [`crate::scene::GpuInstance::rows`]) seen from `eye`: its faces turned to the camera, each covering the
    /// pixels wholly inside it at its farthest depth over them. Nothing is drawn when a corner is behind the near
    /// plane. Returns whether it was drawn.
    pub(crate) fn add_box(&mut self, rows: &[[f32; 4]; 3], eye: Vec3) -> bool {
        let axis = |a: usize| Vec3::new(rows[0][a], rows[1][a], rows[2][a]);
        let (x, y, z, centre) = (axis(0), axis(1), axis(2), axis(3));
        let corner = |sx: f32, sy: f32, sz: f32| centre + (x * sx + y * sy + z * sz) * 0.5;
        let mut screen = [(Vec2::ZERO, 0.0f32); 8];
        for (i, slot) in screen.iter_mut().enumerate() {
            let sign = |bit: usize| if i & bit == 0 { -1.0 } else { 1.0 };
            let clip = self.project(corner(sign(1), sign(2), sign(4)));
            if clip.w <= NEAR {
                return false;
            }
            *slot = self.to_pixels(clip);
        }
        // Each face: the axis it faces along and its four corners, in order around it.
        const FACES: [(usize, f32, [usize; 4]); 6] = [
            (0, -1.0, [0, 2, 6, 4]),
            (0, 1.0, [1, 3, 7, 5]),
            (1, -1.0, [0, 1, 5, 4]),
            (1, 1.0, [2, 3, 7, 6]),
            (2, -1.0, [0, 1, 3, 2]),
            (2, 1.0, [4, 5, 7, 6]),
        ];
        let axes = [x, y, z];
        for (a, sign, quad) in FACES {
            let normal = axes[a] * sign;
            let face_centre = centre + normal * 0.5;
            // Only the faces turned to the camera: those behind them are farther anyway.
            if normal.dot(face_centre - eye) >= 0.0 {
                continue;
            }
            self.fill_quad(quad.map(|i| screen[i]));
        }
        self.drawn += 1;
        true
    }

    /// Covers the pixels wholly inside the convex quad `corners` (on the buffer, with their view depths), each at the
    /// quad's farthest depth over it.
    fn fill_quad(&mut self, corners: [(Vec2, f32); 4]) {
        let points = corners.map(|(p, _)| p);
        let cross = |a: Vec2, b: Vec2| a.x * b.y - a.y * b.x;
        let area: f32 = (0..4).map(|i| cross(points[i], points[(i + 1) % 4])).sum();
        if area.abs() < 1e-6 {
            return;
        }
        let side = area.signum();
        // 1 / depth is affine over a flat face on screen: fitted through three corners.
        let (p0, p1, p2) = (points[0], points[1], points[2]);
        let (i0, i1, i2) = (1.0 / corners[0].1, 1.0 / corners[1].1, 1.0 / corners[2].1);
        let det = cross(p1 - p0, p2 - p0);
        if det.abs() < 1e-9 {
            return;
        }
        let ddx = ((i1 - i0) * (p2.y - p0.y) - (i2 - i0) * (p1.y - p0.y)) / det;
        let ddy = ((i2 - i0) * (p1.x - p0.x) - (i1 - i0) * (p2.x - p0.x)) / det;
        let inverse = |p: Vec2| i0 + ddx * (p.x - p0.x) + ddy * (p.y - p0.y);
        let inside = |q: Vec2| (0..4).all(|i| cross(points[(i + 1) % 4] - points[i], q - points[i]) * side >= 0.0);
        let min = points.iter().fold(Vec2::splat(f32::INFINITY), |m, p| m.min(*p));
        let max = points.iter().fold(Vec2::splat(f32::NEG_INFINITY), |m, p| m.max(*p));
        let x0 = min.x.floor().max(0.0) as usize;
        let y0 = min.y.floor().max(0.0) as usize;
        let x1 = (max.x.ceil().max(0.0) as usize).min(self.width);
        let y1 = (max.y.ceil().max(0.0) as usize).min(self.height);
        let width = self.width;
        let base = &mut self.levels[0];
        for py in y0..y1 {
            for px in x0..x1 {
                let (fx, fy) = (px as f32, py as f32);
                let pixel = [
                    Vec2::new(fx, fy),
                    Vec2::new(fx + 1.0, fy),
                    Vec2::new(fx, fy + 1.0),
                    Vec2::new(fx + 1.0, fy + 1.0),
                ];
                if !pixel.iter().all(|&q| inside(q)) {
                    continue;
                }
                // The farthest depth over the pixel is at one of its corners (1 / depth is affine).
                let nearest_inverse = pixel.iter().map(|&q| inverse(q)).fold(f32::INFINITY, f32::min);
                if nearest_inverse <= 0.0 {
                    continue;
                }
                let depth = 1.0 / nearest_inverse;
                let at = py * width + px;
                if depth < base[at] {
                    base[at] = depth;
                }
            }
        }
    }

    /// Builds the pyramid of maxima once every occluder is drawn.
    pub(crate) fn finish(&mut self) {
        let mut level = 0;
        while let Some(&(w, h)) = self.sizes.get(level)
            && (w > 1 || h > 1)
        {
            let (nw, nh) = (w.div_ceil(2), h.div_ceil(2));
            if self.levels.len() <= level + 1 {
                self.levels.push(Vec::new());
            }
            let (lower, upper) = self.levels.split_at_mut(level + 1);
            let (below, next) = (&lower[level], &mut upper[0]);
            next.clear();
            for y in 0..nh {
                for x in 0..nw {
                    let (xa, ya) = (2 * x, 2 * y);
                    let (xb, yb) = ((xa + 1).min(w - 1), (ya + 1).min(h - 1));
                    let m = below[ya * w + xa]
                        .max(below[ya * w + xb])
                        .max(below[yb * w + xa])
                        .max(below[yb * w + xb]);
                    next.push(m);
                }
            }
            self.sizes.push((nw, nh));
            level += 1;
        }
    }

    /// Whether the box at `center` with half extents `half` (in the scene's coordinates, along its axes) is wholly
    /// hidden behind what was drawn.
    pub(crate) fn hides_box(&self, center: Vec3, half: Vec3) -> bool {
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        let mut nearest = f32::INFINITY;
        for i in 0..8 {
            let sign = |bit: usize| if i & bit == 0 { -1.0 } else { 1.0 };
            let clip = self.project(center + half * Vec3::new(sign(1), sign(2), sign(4)));
            if clip.w <= NEAR {
                return false;
            }
            let (p, depth) = self.to_pixels(clip);
            min = min.min(p);
            max = max.max(p);
            nearest = nearest.min(depth);
        }
        self.hides_rect(min, max, nearest)
    }

    /// Whether the sphere at `center` of `radius` is wholly hidden behind what was drawn.
    #[inline]
    pub(crate) fn hides_sphere(&self, center: Vec3, radius: f32) -> bool {
        self.hides_box(center, Vec3::splat(radius))
    }

    /// Whether everything in the screen rectangle `min` to `max` (pixels) is covered by something nearer than
    /// `nearest`. A rectangle reaching off the buffer is not hidden.
    fn hides_rect(&self, min: Vec2, max: Vec2, nearest: f32) -> bool {
        if self.sizes.is_empty() || min.x < 0.0 || min.y < 0.0 {
            return false;
        }
        if max.x > self.width as f32 || max.y > self.height as f32 {
            return false;
        }
        // The level at which the rectangle spans two texels or so: a few lookups whatever its size.
        let span = (max - min).max_element().max(1.0);
        let level = ((span * 0.5).log2().ceil().max(0.0) as usize).min(self.sizes.len() - 1);
        let scale = (1usize << level) as f32;
        let (w, h) = self.sizes[level];
        let x0 = (min.x / scale).floor() as usize;
        let y0 = (min.y / scale).floor() as usize;
        let x1 = ((max.x / scale).ceil() as usize).clamp(x0 + 1, w);
        let y1 = ((max.y / scale).ceil() as usize).clamp(y0 + 1, h);
        let texels = &self.levels[level];
        for y in y0..y1 {
            for x in x0..x1 {
                if texels[y * w + x] >= nearest {
                    return false;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Looking down -Z from the origin, 90° wide and tall.
    fn tiles() -> DepthTiles {
        let camera = crate::parts::Camera {
            position: Vec3::ZERO,
            rotation: metallic_core::glam::Quat::IDENTITY,
            fov_y: std::f32::consts::FRAC_PI_2,
            near: 0.1,
        };
        let projection = camera.view_projection(1.0);
        let mut tiles = DepthTiles::default();
        tiles.begin(projection, 1.0);
        tiles
    }

    /// A box of `size` at `centre`, unturned.
    fn rows(centre: Vec3, size: Vec3) -> [[f32; 4]; 3] {
        [
            [size.x, 0.0, 0.0, centre.x],
            [0.0, size.y, 0.0, centre.y],
            [0.0, 0.0, size.z, centre.z],
        ]
    }

    #[test]
    fn a_wall_hides_what_is_wholly_behind_it_and_nothing_else() {
        let mut tiles = tiles();
        // A wall 10 m in front, 8 m wide and tall.
        assert!(tiles.add_box(&rows(Vec3::new(0.0, 0.0, -10.0), Vec3::new(8.0, 8.0, 1.0)), Vec3::ZERO));
        tiles.finish();
        assert_eq!(tiles.drawn(), 1);
        // A small part right behind it is hidden; one beside it, one in front and one poking out are not.
        assert!(tiles.hides_sphere(Vec3::new(0.0, 0.0, -20.0), 1.0));
        assert!(!tiles.hides_sphere(Vec3::new(10.0, 0.0, -20.0), 1.0));
        assert!(!tiles.hides_sphere(Vec3::new(0.0, 0.0, -5.0), 1.0));
        assert!(!tiles.hides_sphere(Vec3::new(0.0, 0.0, -20.0), 10.0));
        // A grid cell's box behind it too.
        assert!(tiles.hides_box(Vec3::new(0.0, 0.0, -30.0), Vec3::splat(2.0)));
        // The wall does not hide itself.
        assert!(!tiles.hides_box(Vec3::new(0.0, 0.0, -10.0), Vec3::new(4.0, 4.0, 0.5)));
    }

    #[test]
    fn a_box_across_the_near_plane_neither_hides_nor_is_hidden() {
        let mut tiles = tiles();
        assert!(!tiles.add_box(&rows(Vec3::new(0.0, 0.0, 0.0), Vec3::splat(4.0)), Vec3::ZERO));
        assert!(tiles.add_box(
            &rows(Vec3::new(0.0, 0.0, -10.0), Vec3::new(30.0, 30.0, 1.0)),
            Vec3::ZERO
        ));
        tiles.finish();
        assert!(!tiles.hides_box(Vec3::new(0.0, 0.0, 0.0), Vec3::splat(1.0)));
        // Off the buffer: kept.
        assert!(!tiles.hides_sphere(Vec3::new(0.0, 100.0, -20.0), 1.0));
    }
}
