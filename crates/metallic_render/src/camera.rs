//! The camera: where the viewer stands, where they look and how wide the lens is.
use glam::{Mat4, Vec3};

/// Distance of the near clip plane in studs.
pub const NEAR: f32 = 0.3;
/// Distance of the far clip plane in studs.
pub const FAR: f32 = 4000.0;

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
        glam::camera::rh::view::look_at_mat4(self.position, self.look_at, Vec3::Y)
    }

    /// Perspective matrix for the given width / height ratio, depth range 0..1.
    pub fn projection(&self, aspect: f32) -> Mat4 {
        glam::camera::rh::proj::directx::perspective(
            self.fov_deg.to_radians(),
            aspect.max(1e-3),
            NEAR,
            FAR,
        )
    }

    /// Combined world to clip matrix.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
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
}
