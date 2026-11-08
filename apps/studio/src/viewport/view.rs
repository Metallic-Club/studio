//! The editor camera as the viewport panel shows it (#354), after Forge's `gizmo::View`: a screen position as a ray
//! into the world, a world position (or a line) as a place on the screen, and how big a world unit looks. Picking,
//! the selection outlines and the gizmos all read the scene through it, in egui points, the pointer's units.

use egui::{Pos2, Rect};
use metallic_engine::metallic_core::glam::{Quat, Vec2, Vec3};

use super::EditorCamera;

/// A ray: where it starts and its unit direction.
pub type Ray = (Vec3, Vec3);

/// The closest a point can be in front of the camera and still be drawn.
const NEAR: f32 = 0.05;

/// The editor camera and the panel it draws into.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Where the camera is.
    pub pos: Vec3,
    /// Which way it looks: −Z forward, +Y up.
    pub rot: Quat,
    /// The vertical field of view, in radians.
    pub fov: f32,
    /// The panel's area, in points.
    pub rect: Rect,
}

impl View {
    /// `camera` drawing into `rect`.
    pub fn new(camera: &EditorCamera, rect: Rect) -> Self {
        let camera = camera.camera();
        Self {
            pos: camera.position,
            rot: camera.rotation,
            fov: camera.fov_y,
            rect,
        }
    }

    fn size(&self) -> Option<Vec2> {
        let size = Vec2::new(self.rect.width(), self.rect.height());
        (size.x >= 2.0 && size.y >= 2.0).then_some(size)
    }

    /// The ray through screen point `p`.
    pub fn ray(&self, p: Pos2) -> Option<Ray> {
        let size = self.size()?;
        let ndc = (Vec2::new(p.x - self.rect.left(), p.y - self.rect.top())) / size * 2.0 - Vec2::ONE;
        let t = (self.fov * 0.5).tan();
        let local = Vec3::new(ndc.x * t * size.x / size.y, -ndc.y * t, -1.0);
        Some((self.pos, (self.rot * local).normalize()))
    }

    /// Where a point `v` in the camera's own space appears on the screen.
    fn screen(&self, v: Vec3, size: Vec2) -> Pos2 {
        let t = (self.fov * 0.5).tan();
        let ndc = Vec2::new(v.x / (-v.z * t * size.x / size.y), -v.y / (-v.z * t));
        let at = (ndc * 0.5 + Vec2::splat(0.5)) * size;
        Pos2::new(self.rect.left() + at.x, self.rect.top() + at.y)
    }

    /// Where `world` appears on the screen; `None` behind the camera.
    pub fn project(&self, world: Vec3) -> Option<Pos2> {
        let size = self.size()?;
        let v = self.rot.inverse() * (world - self.pos);
        (v.z <= -NEAR).then(|| self.screen(v, size))
    }

    /// The part of the line from `a` to `b` in front of the camera, on the screen.
    pub fn segment(&self, a: Vec3, b: Vec3) -> Option<[Pos2; 2]> {
        let size = self.size()?;
        let inverse = self.rot.inverse();
        let (mut va, mut vb) = (inverse * (a - self.pos), inverse * (b - self.pos));
        if va.z > -NEAR && vb.z > -NEAR {
            return None;
        }
        // Cut where it crosses the near plane.
        let cut = |front: Vec3, back: Vec3| front + (back - front) * ((-NEAR - front.z) / (back.z - front.z));
        if va.z > -NEAR {
            va = cut(vb, va);
        } else if vb.z > -NEAR {
            vb = cut(va, vb);
        }
        Some([self.screen(va, size), self.screen(vb, size)])
    }

    /// The way the camera looks.
    pub fn forward(&self) -> Vec3 {
        self.rot * Vec3::NEG_Z
    }

    /// How many points one world unit is across at `world`, facing the camera.
    pub fn points_per_unit(&self, world: Vec3) -> f32 {
        let depth = (world - self.pos).dot(self.forward()).max(NEAR);
        self.rect.height() / (2.0 * (self.fov * 0.5).tan() * depth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> View {
        let camera = EditorCamera::looking_at(Vec3::new(0.0, 5.0, 30.0), Vec3::ZERO);
        View::new(
            &camera,
            Rect::from_min_size(Pos2::new(250.0, 60.0), egui::vec2(900.0, 700.0)),
        )
    }

    #[test]
    fn projecting_and_casting_rays_are_inverses() {
        let v = view();
        for p in [Vec3::ZERO, Vec3::new(8.0, 3.0, -10.0), Vec3::new(-12.0, 9.0, 5.0)] {
            let screen = v.project(p).expect("in front of the camera");
            let (o, d) = v.ray(screen).unwrap();
            let closest = o + d * (p - o).dot(d);
            assert!((closest - p).length() < 1e-3, "{p:?}");
        }
        assert!(v.project(Vec3::new(0.0, 5.0, 60.0)).is_none(), "behind the camera");
        let centre = v.project(v.pos + v.forward() * 10.0).unwrap();
        assert!((centre - v.rect.center()).length() < 1e-2);
    }

    #[test]
    fn a_line_through_the_camera_is_cut_at_the_near_plane() {
        let v = view();
        let ahead = v.pos + v.forward() * 10.0;
        let behind = v.pos - v.forward() * 10.0 + Vec3::X;
        let [a, b] = v.segment(ahead, behind).expect("its front part");
        assert!(a.x.is_finite() && b.x.is_finite());
        assert!((a - v.rect.center()).length() < 1e-2);
        assert!(v.segment(behind, behind + Vec3::Y).is_none());
        assert!(v.points_per_unit(ahead) > v.points_per_unit(v.pos + v.forward() * 40.0));
    }
}
