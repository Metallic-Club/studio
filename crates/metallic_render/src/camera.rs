//! The camera: where the viewer stands, where they look and how wide the lens is.
use glam::{Mat4, Vec3};

/// Distance of the near clip plane in studs.
pub const NEAR: f32 = 0.3;
/// Distance of the far clip plane in studs.
pub const FAR: f32 = 4000.0;
/// Field of view used when `fov_deg` is not finite (degrees).
const DEFAULT_FOV_DEG: f32 = 70.0;

/// A perspective camera looking from `position` at `look_at` with +Y up.
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
    /// World to view matrix (right handed, +Y up).
    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.position, self.look_at, Vec3::Y)
    }

    /// Perspective matrix for the given width / height ratio, depth range 0..1, with
    /// [`Camera::fov_deg_clamped`] as the field of view.
    pub fn projection(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(
            self.fov_deg_clamped().to_radians(),
            aspect.max(1e-3),
            NEAR,
            FAR,
        )
    }

    /// Combined world to clip matrix.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
    }

    /// The field of view that is drawn: `fov_deg` clamped to 1..=179 degrees, or 70 when it is
    /// not finite.
    pub fn fov_deg_clamped(&self) -> f32 {
        if self.fov_deg.is_finite() {
            self.fov_deg.clamp(1.0, 179.0)
        } else {
            DEFAULT_FOV_DEG
        }
    }

    /// The eye position that is drawn: `position` when finite, else `look_at` when that is
    /// finite, else the origin.
    pub fn eye(&self) -> Vec3 {
        if self.position.is_finite() {
            self.position
        } else if self.look_at.is_finite() {
            self.look_at
        } else {
            Vec3::ZERO
        }
    }

    /// A copy with [`Camera::eye`] as the position and [`Camera::fov_deg_clamped`] as the field
    /// of view. [`crate::Renderer::render`] draws with this, so the eye in the shader uniform,
    /// the order of transparent parts and the shadow cascades never see a NaN position or
    /// field of view.
    pub fn sanitized(&self) -> Camera {
        Camera {
            position: self.eye(),
            look_at: self.look_at,
            fov_deg: self.fov_deg_clamped(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn look_at_point_projects_to_screen_centre() {
        let cam = Camera::default();
        let clip = cam.view_proj(16.0 / 9.0) * cam.look_at.extend(1.0);
        let ndc = clip / clip.w;
        assert!(ndc.x.abs() < 1e-4 && ndc.y.abs() < 1e-4);
        assert!(ndc.z > 0.0 && ndc.z < 1.0);
    }

    #[test]
    fn non_finite_fov_falls_back_and_extremes_are_clamped() {
        let with = |fov_deg| Camera {
            fov_deg,
            ..Camera::default()
        };
        for fov in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(with(fov).fov_deg_clamped(), DEFAULT_FOV_DEG);
            assert!(with(fov).projection(16.0 / 9.0).is_finite(), "fov {fov}");
        }
        assert_eq!(with(0.0).fov_deg_clamped(), 1.0);
        assert_eq!(with(400.0).fov_deg_clamped(), 179.0);
        assert_eq!(with(60.0).fov_deg_clamped(), 60.0);
        assert!(with(0.0).projection(1.0).is_finite());
        assert!(with(180.0).projection(1.0).is_finite());
    }

    #[test]
    fn non_finite_position_gives_a_finite_eye() {
        let nan = Vec3::new(f32::NAN, 1.0, 2.0);
        let c = Camera {
            position: nan,
            look_at: Vec3::new(1.0, 2.0, 3.0),
            fov_deg: f32::NAN,
        };
        assert_eq!(c.eye(), Vec3::new(1.0, 2.0, 3.0));
        let s = c.sanitized();
        assert_eq!(s.position, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(s.fov_deg, DEFAULT_FOV_DEG);
        let lost = Camera {
            position: nan,
            look_at: Vec3::splat(f32::INFINITY),
            fov_deg: 70.0,
        };
        assert_eq!(lost.eye(), Vec3::ZERO);
        // A finite camera is unchanged.
        assert_eq!(Camera::default().sanitized(), Camera::default());
    }
}
