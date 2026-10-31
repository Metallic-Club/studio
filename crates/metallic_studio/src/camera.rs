//! The editor camera (fly, orbit, pan, dolly, frame) and screen/world conversions.
//!
//! Metallic keymap: hold the right button to look, WASD to fly while it is held, Q down, E up,
//! Shift faster, wheel changes fly speed while held and dollies toward the cursor otherwise;
//! Alt+left drag orbits, middle drag pans; F frames the selection, Home frames everything.
use metallic_render::Camera;
use metallic_render::glam::{DMat4, DVec3, DVec4, Vec3};

use crate::transform::{Aabb, Ray};

/// Pitch limit, so the camera never flips over the pole.
const MAX_PITCH: f64 = 1.55;
/// Fly speed bounds in studs per second.
const SPEED_RANGE: (f64, f64) = (2.0, 400.0);

/// The Studio viewport camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditorCamera {
    /// Eye position.
    pub position: DVec3,
    /// Turn about +Y in radians (0 looks along -Z).
    pub yaw: f64,
    /// Up/down angle in radians (positive looks up).
    pub pitch: f64,
    /// Vertical field of view in degrees.
    pub fov_deg: f64,
    /// Fly speed in studs per second.
    pub speed: f64,
    /// Orbit and dolly target distance in front of the eye.
    pub focus_distance: f64,
}

impl Default for EditorCamera {
    fn default() -> Self {
        let mut c = EditorCamera {
            position: DVec3::new(0.0, 24.0, 48.0),
            yaw: 0.0,
            pitch: 0.0,
            fov_deg: 70.0,
            speed: 32.0,
            focus_distance: 50.0,
        };
        c.look_at(DVec3::ZERO);
        c
    }
}

impl EditorCamera {
    /// Unit view direction.
    pub fn forward(&self) -> DVec3 {
        DVec3::new(
            -self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -self.yaw.cos() * self.pitch.cos(),
        )
    }

    /// Unit right vector (horizontal).
    pub fn right(&self) -> DVec3 {
        DVec3::new(self.yaw.cos(), 0.0, -self.yaw.sin())
    }

    /// Unit up vector of the view.
    pub fn up(&self) -> DVec3 {
        self.right().cross(self.forward())
    }

    /// Turns the camera to look at `target` and sets the focus distance to it.
    pub fn look_at(&mut self, target: DVec3) {
        let d = target - self.position;
        let len = d.length();
        if len < 1e-6 {
            return;
        }
        let d = d / len;
        self.pitch = d.y.asin().clamp(-MAX_PITCH, MAX_PITCH);
        self.yaw = (-d.x).atan2(-d.z);
        self.focus_distance = len;
    }

    /// Mouse look by a pixel delta (right button held).
    pub fn rotate(&mut self, dx: f64, dy: f64) {
        self.yaw -= dx * 0.004;
        self.pitch = (self.pitch - dy * 0.004).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Fly by `input` (x right, y up, z forward, each -1..1) for `dt` seconds.
    pub fn fly(&mut self, input: DVec3, dt: f64, fast: bool) {
        let speed = self.speed * if fast { 3.0 } else { 1.0 };
        let mv = self.right() * input.x + DVec3::Y * input.y + self.forward() * input.z;
        self.position += mv * speed * dt;
    }

    /// Wheel while flying: changes speed by `steps` notches.
    pub fn change_speed(&mut self, steps: f64) {
        self.speed = (self.speed * 1.2f64.powf(steps)).clamp(SPEED_RANGE.0, SPEED_RANGE.1);
    }

    /// Wheel without the right button: moves along `toward` (the cursor ray) by a step that
    /// scales with the focus distance.
    pub fn dolly(&mut self, toward: DVec3, steps: f64) {
        let step = (self.focus_distance * 0.15).max(0.5) * steps;
        self.position += toward * step;
        self.focus_distance = (self.focus_distance - step).max(1.0);
    }

    /// Alt+left drag: turns around the focus point.
    pub fn orbit(&mut self, dx: f64, dy: f64) {
        let pivot = self.position + self.forward() * self.focus_distance;
        self.rotate(dx, dy);
        self.position = pivot - self.forward() * self.focus_distance;
    }

    /// Middle drag: slides the view by a pixel delta (`px_to_studs` at the focus distance).
    pub fn pan(&mut self, dx: f64, dy: f64, viewport_height: f64) {
        let studs_per_px = 2.0 * self.focus_distance * (self.fov_deg.to_radians() * 0.5).tan()
            / viewport_height.max(1.0);
        self.position += (-self.right() * dx + self.up() * dy) * studs_per_px;
    }

    /// Moves back so `bounds` fills about 60% of the view height, keeping the view direction.
    pub fn frame(&mut self, bounds: Aabb) {
        let radius = (bounds.size().length() * 0.5).max(0.5);
        let half_fov = (self.fov_deg.to_radians() * 0.5) * 0.6;
        let dist = radius / half_fov.tan();
        let centre = bounds.centre();
        self.position = centre - self.forward() * dist;
        self.focus_distance = dist;
    }

    /// The renderer camera.
    pub fn to_render(&self) -> Camera {
        let p = self.position;
        let t = p + self.forward() * self.focus_distance.max(1.0);
        Camera {
            position: Vec3::new(p.x as f32, p.y as f32, p.z as f32),
            look_at: Vec3::new(t.x as f32, t.y as f32, t.z as f32),
            fov_deg: self.fov_deg as f32,
        }
    }

    /// The world-to-screen mapping for a viewport of `size` pixels (points).
    pub fn view(&self, size: (f64, f64)) -> ViewProjection {
        let cam = self.to_render();
        let aspect = (size.0 / size.1.max(1.0)) as f32;
        let m = cam.view_proj(aspect).as_dmat4();
        ViewProjection {
            view_proj: m,
            inverse: m.inverse(),
            size,
            eye: self.position,
        }
    }
}

/// World/screen conversions for one frame. Screen coordinates are relative to the viewport's
/// top-left corner, in egui points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewProjection {
    view_proj: DMat4,
    inverse: DMat4,
    /// Viewport size in points.
    pub size: (f64, f64),
    /// Eye position.
    pub eye: DVec3,
}

impl ViewProjection {
    /// Screen position of `p`, or `None` when it is behind the camera.
    pub fn project(&self, p: DVec3) -> Option<(f64, f64)> {
        let c = self.view_proj * DVec4::new(p.x, p.y, p.z, 1.0);
        if c.w <= 1e-6 {
            return None;
        }
        let ndc = c / c.w;
        Some((
            (ndc.x * 0.5 + 0.5) * self.size.0,
            (0.5 - ndc.y * 0.5) * self.size.1,
        ))
    }

    /// The ray through a screen position.
    pub fn ray(&self, x: f64, y: f64) -> Ray {
        let nx = x / self.size.0.max(1.0) * 2.0 - 1.0;
        let ny = 1.0 - y / self.size.1.max(1.0) * 2.0;
        let near = self.inverse * DVec4::new(nx, ny, 0.0, 1.0);
        let far = self.inverse * DVec4::new(nx, ny, 1.0, 1.0);
        let near = near.truncate() / near.w;
        let far = far.truncate() / far.w;
        Ray {
            origin: self.eye,
            dir: (far - near).normalize_or(DVec3::NEG_Z),
        }
    }

    /// Studs per screen point at distance `depth` from the eye (constant-size gizmos).
    pub fn studs_per_point(&self, p: DVec3) -> f64 {
        let a = self.project(p);
        let right = self
            .ray(self.size.0 * 0.5, self.size.1 * 0.5)
            .dir
            .cross(DVec3::Y);
        let right = right.normalize_or(DVec3::X);
        let b = self.project(p + right);
        match (a, b) {
            (Some(a), Some(b)) => {
                let px = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
                if px > 1e-9 { 1.0 / px } else { 1.0 }
            }
            _ => (p - self.eye).length() * 0.002,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centre_ray_matches_forward_and_projection_round_trips() {
        let cam = EditorCamera::default();
        let view = cam.view((800.0, 600.0));
        let ray = view.ray(400.0, 300.0);
        assert!((ray.dir - cam.forward()).length() < 1e-4);
        let p = ray.at(30.0) + cam.right() * 3.0;
        let (x, y) = view.project(p).unwrap();
        let back = view.ray(x, y);
        let closest = back.dir.dot(p - back.origin);
        assert!((back.at(closest) - p).length() < 1e-3);
    }

    #[test]
    fn frame_puts_bounds_in_front() {
        let mut cam = EditorCamera::default();
        let b = Aabb {
            min: DVec3::new(100.0, 0.0, 100.0),
            max: DVec3::new(104.0, 4.0, 104.0),
        };
        cam.frame(b);
        let view = cam.view((800.0, 600.0));
        let (x, y) = view.project(b.centre()).unwrap();
        assert!((x - 400.0).abs() < 1.0 && (y - 300.0).abs() < 1.0);
    }

    #[test]
    fn look_at_faces_target() {
        let mut cam = EditorCamera {
            position: DVec3::new(10.0, 0.0, 0.0),
            ..EditorCamera::default()
        };
        cam.look_at(DVec3::ZERO);
        assert!((cam.forward() - DVec3::NEG_X).length() < 1e-9);
    }
}
