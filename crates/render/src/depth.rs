//! Reverse-Z depth (ADR 0008 §7).
//!
//! The near plane maps to depth 1 and the far plane to 0. Floating-point values are densest near 0, so putting the
//! far distance there spreads precision evenly over distance instead of wasting it right in front of the camera;
//! with `Depth32Float` this removes the z-fighting the archived renderer's standard-Z 0.3/4000 setup had. Depth is
//! cleared to [`DEPTH_CLEAR`] (0, the far plane) and a fragment passes when it is nearer: [`DEPTH_COMPARE`]
//! (`Greater`).

use metallic_core::glam::Mat4;
use metallic_core::glam::camera::rh::proj::directx;

/// The depth buffer's format.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// The value depth is cleared to: the far plane under reverse-Z.
pub const DEPTH_CLEAR: f32 = 0.0;

/// The depth test: nearer fragments have larger depth under reverse-Z.
pub const DEPTH_COMPARE: wgpu::CompareFunction = wgpu::CompareFunction::Greater;

/// The depth test for geometry drawn at the far plane itself (the sky), which must pass where nothing was drawn.
pub const DEPTH_COMPARE_FAR_PLANE: wgpu::CompareFunction = wgpu::CompareFunction::GreaterEqual;

/// A reverse-Z, right-handed perspective projection (camera looks down −Z) with depth in `[0, 1]`: `near` maps
/// to 1 and `far` to 0.
///
/// `fov_y` is the vertical field of view in radians and `aspect` is width / height.
pub fn reverse_z_perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    // wgpu's clip space is DirectX's (depth 0 to 1, y up). A standard projection with the planes swapped is exactly
    // the reverse-Z one.
    directx::perspective(fov_y, aspect, far, near)
}

/// A reverse-Z, right-handed perspective projection with no far plane: `near` maps to 1 and infinity to 0.
///
/// With reverse-Z and a float depth buffer an infinite far plane costs no precision, so the view distance is a
/// culling setting instead of a property of the projection.
pub fn reverse_z_perspective_infinite(fov_y: f32, aspect: f32, near: f32) -> Mat4 {
    directx::perspective_infinite_reverse(fov_y, aspect, near)
}

/// Depth state for a pipeline under reverse-Z: [`DEPTH_COMPARE`] and [`DEPTH_FORMAT`], writing depth when `write`
/// is true (opaque geometry) and only testing it otherwise (transparent geometry, or opaque geometry after a depth
/// prepass).
pub fn depth_stencil_state(write: bool) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(write),
        depth_compare: Some(DEPTH_COMPARE),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metallic_core::glam::{Vec3, Vec4};

    /// The depth a view-space point ends up with after the perspective divide.
    fn depth(projection: Mat4, z: f32) -> f32 {
        let clip = projection * Vec4::new(0.0, 0.0, z, 1.0);
        clip.z / clip.w
    }

    const FOV: f32 = std::f32::consts::FRAC_PI_3;

    #[test]
    fn near_maps_to_one_and_far_to_zero() {
        let p = reverse_z_perspective(FOV, 16.0 / 9.0, 0.1, 1000.0);
        assert!((depth(p, -0.1) - 1.0).abs() < 1e-6);
        assert!(depth(p, -1000.0).abs() < 1e-6);
    }

    #[test]
    fn depth_decreases_with_distance() {
        let p = reverse_z_perspective(FOV, 1.0, 0.1, 10_000.0);
        let mut previous = f32::INFINITY;
        for distance in [0.1, 0.5, 1.0, 10.0, 100.0, 1000.0, 5000.0, 10_000.0] {
            let d = depth(p, -distance);
            assert!(d < previous, "depth at {distance} is {d}, not below {previous}");
            // The far plane itself lands on 0 give or take rounding.
            assert!((-1e-6..=1.0).contains(&d), "depth at {distance} is {d}");
            previous = d;
        }
    }

    #[test]
    fn far_parts_stay_distinguishable() {
        // Two surfaces 1 cm apart, 4 km away: standard-Z with f32 cannot tell them apart; reverse-Z can.
        let p = reverse_z_perspective(FOV, 1.0, 0.1, 5000.0);
        assert!(depth(p, -4000.0) > depth(p, -4000.01));
    }

    #[test]
    fn infinite_projection_never_reaches_zero() {
        let p = reverse_z_perspective_infinite(FOV, 1.0, 0.1);
        assert!((depth(p, -0.1) - 1.0).abs() < 1e-6);
        let far = depth(p, -1.0e7);
        assert!(far > 0.0 && far < 1e-6);
    }

    #[test]
    fn x_and_y_match_a_standard_projection() {
        let reverse = reverse_z_perspective(FOV, 1.5, 0.1, 100.0);
        let standard = directx::perspective(FOV, 1.5, 0.1, 100.0);
        let point = Vec3::new(1.0, 2.0, -5.0);
        let a = reverse.project_point3(point);
        let b = standard.project_point3(point);
        assert!((a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6);
    }

    #[test]
    fn pipelines_test_with_greater() {
        let state = depth_stencil_state(true);
        assert_eq!(state.depth_compare, Some(wgpu::CompareFunction::Greater));
        assert_eq!(state.depth_write_enabled, Some(true));
        assert_eq!(state.format, wgpu::TextureFormat::Depth32Float);
        assert_eq!(depth_stencil_state(false).depth_write_enabled, Some(false));
    }
}
