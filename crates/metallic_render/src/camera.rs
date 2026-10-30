//! The camera: where the viewer stands, where they look and how wide the lens is.
use glam::{Mat4, Vec3};

/// Distance of the near clip plane in studs.
pub const NEAR: f32 = 0.3;
/// Distance of the far clip plane in studs.
pub const FAR: f32 = 4000.0;

/// Above this `|forward.y|` the view direction counts as vertical and +Y can no longer be the up vector.
const VERTICAL_LIMIT: f32 = 0.999;

/// A perspective camera looking from `position` at `look_at` with +Y up.
///
/// The view matrix is always finite, also for degenerate inputs:
/// - Looking (nearly) straight down or up (`|forward.y| > 0.999`): +Y cannot be the up vector, so screen-up becomes
///   world -Z when looking down and +Z when looking up. That is where a camera facing -Z (the default forward)
///   ends up when pitched to -90 / +90 degrees, so a top-down view keeps -Z at the top of the screen and nothing
///   flips when the orbit camera crosses the limit.
/// - `position == look_at`, or a non-finite `look_at`: the camera looks along -Z from `position`.
/// - A non-finite `position`: the eye is placed at `look_at` (or the origin if that is not finite either).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Eye position in world space (studs).
    pub position: Vec3,
    /// The point the camera looks at.
    pub look_at: Vec3,
    /// Vertical field of view in degrees.
    pub fov_deg: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            position: Vec3::new(0.0, 20.0, 40.0),
            look_at: Vec3::ZERO,
            fov_deg: 70.0,
        }
    }
}

impl Camera {
    /// World to view matrix (right handed, +Y up). Always finite; see the type docs for the fallbacks used when
    /// the camera looks straight down/up, at its own position, or has non-finite coordinates.
    pub fn view(&self) -> Mat4 {
        let eye = if self.position.is_finite() {
            self.position
        } else if self.look_at.is_finite() {
            self.look_at
        } else {
            Vec3::ZERO
        };
        let forward = (self.look_at - eye).try_normalize().unwrap_or(Vec3::NEG_Z);
        let up = if forward.y.abs() > VERTICAL_LIMIT {
            Vec3::Z * forward.y.signum()
        } else {
            Vec3::Y
        };
        Mat4::look_to_rh(eye, forward, up)
    }

    /// Perspective matrix for the given width / height ratio, depth range 0..1.
    pub fn projection(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(self.fov_deg.to_radians(), aspect.max(1e-3), NEAR, FAR)
    }

    /// Combined world to clip matrix.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(position: Vec3, look_at: Vec3) -> Camera {
        Camera {
            position,
            look_at,
            fov_deg: 70.0,
        }
    }

    fn assert_finite(c: &Camera) {
        let vp = c.view_proj(16.0 / 9.0);
        assert!(vp.is_finite(), "{c:?} gives {vp:?}");
    }

    /// Normalised device coordinates of a world point.
    fn ndc(c: &Camera, p: Vec3) -> Vec3 {
        let clip = c.view_proj(16.0 / 9.0) * p.extend(1.0);
        clip.truncate() / clip.w
    }

    #[test]
    fn look_at_point_projects_to_screen_centre() {
        let cam = Camera::default();
        let clip = cam.view_proj(16.0 / 9.0) * cam.look_at.extend(1.0);
        let ndc = clip / clip.w;
        assert!(ndc.x.abs() < 1e-4 && ndc.y.abs() < 1e-4);
        assert!(ndc.z > 0.0 && ndc.z < 1.0);
    }

    #[test]
    fn straight_down_is_finite_with_minus_z_at_the_top() {
        let c = cam(Vec3::new(0.0, 50.0, 0.0), Vec3::ZERO);
        assert_finite(&c);
        let centre = ndc(&c, Vec3::ZERO);
        assert!(centre.x.abs() < 1e-4 && centre.y.abs() < 1e-4, "{centre:?}");
        assert!(centre.z > 0.0 && centre.z < 1.0);
        // -Z is screen-up and +X is screen-right: not mirrored.
        assert!(ndc(&c, Vec3::new(0.0, 0.0, -5.0)).y > 0.1);
        assert!(ndc(&c, Vec3::new(5.0, 0.0, 0.0)).x > 0.05);
    }

    #[test]
    fn straight_up_is_finite_with_plus_z_at_the_top() {
        let c = cam(Vec3::new(3.0, -20.0, 1.0), Vec3::new(3.0, 30.0, 1.0));
        assert_finite(&c);
        let centre = ndc(&c, Vec3::new(3.0, 30.0, 1.0));
        assert!(centre.x.abs() < 1e-4 && centre.y.abs() < 1e-4, "{centre:?}");
        assert!(ndc(&c, Vec3::new(3.0, 30.0, 6.0)).y > 0.01);
        assert!(ndc(&c, Vec3::new(8.0, 30.0, 1.0)).x > 0.01);
    }

    #[test]
    fn nearly_vertical_matches_the_fallback_orientation() {
        // Just inside the +Y-up range, facing -Z: -Z is already screen-up, like the fallback.
        let c = cam(Vec3::new(0.0, 50.0, 5.0), Vec3::ZERO);
        assert_finite(&c);
        assert!(ndc(&c, Vec3::new(0.0, 0.0, -5.0)).y > 0.1);
        assert!(ndc(&c, Vec3::new(5.0, 0.0, 0.0)).x > 0.05);
    }

    #[test]
    fn eye_on_look_at_is_finite() {
        assert_finite(&cam(Vec3::splat(4.0), Vec3::splat(4.0)));
        assert_finite(&cam(Vec3::ZERO, Vec3::ZERO));
    }

    #[test]
    fn non_finite_inputs_give_a_finite_matrix() {
        assert_finite(&cam(Vec3::new(f32::NAN, 1.0, 2.0), Vec3::ZERO));
        assert_finite(&cam(Vec3::new(0.0, 5.0, 10.0), Vec3::splat(f32::NAN)));
        assert_finite(&cam(Vec3::splat(f32::INFINITY), Vec3::splat(f32::NAN)));
    }
}
