//! The third-person camera: orbits the character, zooms, and pulls in in front of walls.
use crate::input::FrameInput;
use metallic_core::{InstanceId, Vec3};
use metallic_physics::PhysicsWorld;
use metallic_render::Camera;
use metallic_render::glam::Vec3 as GVec3;

/// Distance from the character the camera starts at, in studs.
pub const DEFAULT_DISTANCE: f32 = 12.0;
/// Closest zoom, in studs.
pub const MIN_DISTANCE: f32 = 4.0;
/// Farthest zoom, in studs.
pub const MAX_DISTANCE: f32 = 40.0;
/// Starting pitch in radians (looking slightly down).
pub const DEFAULT_PITCH: f32 = 0.25;
/// The camera looks at this point above the `HumanoidRootPart` centre (studs).
pub const FOCUS_HEIGHT: f64 = 2.0;
/// Gap kept between the camera and a wall it was pulled in front of (studs).
const WALL_GAP: f32 = 0.6;
/// How fast the camera moves back out once a wall is gone (studs per second).
const EASE_OUT_SPEED: f32 = 24.0;
/// Pitch limits in radians: a little upward to almost straight down.
const PITCH_RANGE: (f32, f32) = (-0.35, 1.35);
/// Vertical field of view in degrees.
const FOV: f32 = 70.0;

/// Orbit camera state. Yaw 0 looks along -Z; positive yaw turns the view to the right.
#[derive(Debug, Clone, PartialEq)]
pub struct CameraController {
    /// Rotation around the vertical axis, radians.
    pub yaw: f32,
    /// Angle below the horizon, radians.
    pub pitch: f32,
    /// Wanted distance from the focus point, studs.
    pub distance: f32,
    /// Distance actually used last frame (shorter when a wall is in the way).
    shown_distance: f32,
}

impl Default for CameraController {
    fn default() -> Self {
        CameraController {
            yaw: 0.0,
            pitch: DEFAULT_PITCH,
            distance: DEFAULT_DISTANCE,
            shown_distance: DEFAULT_DISTANCE,
        }
    }
}

impl CameraController {
    /// A camera 12 studs behind, looking along -Z and slightly down.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies the frame's orbit and zoom.
    pub fn apply_input(&mut self, input: &FrameInput) {
        if input.orbit[0].is_finite() && input.orbit[1].is_finite() {
            self.yaw = (self.yaw + input.orbit[0]).rem_euclid(std::f32::consts::TAU);
            self.pitch = (self.pitch + input.orbit[1]).clamp(PITCH_RANGE.0, PITCH_RANGE.1);
        }
        if input.zoom.is_finite() {
            self.distance = (self.distance + input.zoom).clamp(MIN_DISTANCE, MAX_DISTANCE);
        }
    }

    /// Unit vector the camera looks along.
    fn look(&self) -> GVec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        GVec3::new(sy * cp, -sp, -cy * cp)
    }

    /// World-space walking direction for a camera-relative wish (x right, y forward). Flat (Y = 0).
    pub fn move_direction(&self, movement: [f32; 2]) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let forward = (f64::from(sy), f64::from(-cy));
        let right = (f64::from(cy), f64::from(sy));
        let (mx, my) = (f64::from(movement[0]), f64::from(movement[1]));
        Vec3::new(
            right.0 * mx + forward.0 * my,
            0.0,
            right.1 * mx + forward.1 * my,
        )
    }

    /// The camera for this frame, looking at `focus` (world studs). A ray from the focus to the
    /// wanted eye position pulls the camera in front of the first solid part, skipping `ignore`
    /// (the character's own parts); once clear it eases back out over `dt` seconds.
    pub fn camera(
        &mut self,
        focus: Vec3,
        physics: &PhysicsWorld,
        ignore: &[InstanceId],
        dt: f32,
    ) -> Camera {
        let look = self.look();
        let back = -look * self.distance;
        let ray = Vec3::new(f64::from(back.x), f64::from(back.y), f64::from(back.z));
        let allowed = match physics.raycast(focus, ray, ignore) {
            Some(hit) => (hit.distance as f32 - WALL_GAP).max(0.5),
            None => self.distance,
        };
        self.shown_distance = if allowed < self.shown_distance || !dt.is_finite() {
            allowed
        } else {
            (self.shown_distance + EASE_OUT_SPEED * dt.max(0.0)).min(allowed)
        };
        let f = GVec3::new(focus.x as f32, focus.y as f32, focus.z as f32);
        Camera {
            position: f - look * self.shown_distance,
            look_at: f,
            fov_deg: FOV,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_camera_is_behind_and_above_looking_forward() {
        let mut c = CameraController::new();
        let cam = c.camera(Vec3::new(0.0, 5.0, 0.0), &PhysicsWorld::new(), &[], 0.016);
        assert!(cam.position.z > 0.0, "behind the character (+Z): {cam:?}");
        assert!(cam.position.y > 5.0, "above the focus: {cam:?}");
        assert!((cam.position.distance(cam.look_at) - DEFAULT_DISTANCE).abs() < 1e-3);
    }

    #[test]
    fn forward_follows_yaw() {
        let mut c = CameraController::new();
        let f = c.move_direction([0.0, 1.0]);
        assert!((f.z + 1.0).abs() < 1e-9 && f.x.abs() < 1e-9);
        let r = c.move_direction([1.0, 0.0]);
        assert!((r.x - 1.0).abs() < 1e-9);
        c.yaw = std::f32::consts::FRAC_PI_2;
        let f = c.move_direction([0.0, 1.0]);
        assert!((f.x - 1.0).abs() < 1e-6, "yaw 90 deg looks along +X: {f:?}");
    }

    #[test]
    fn zoom_and_pitch_are_clamped() {
        let mut c = CameraController::new();
        c.apply_input(&FrameInput {
            zoom: 1000.0,
            orbit: [0.0, 100.0],
            ..FrameInput::default()
        });
        assert_eq!(c.distance, MAX_DISTANCE);
        assert_eq!(c.pitch, PITCH_RANGE.1);
    }
}
