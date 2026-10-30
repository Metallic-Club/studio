//! Cascaded shadow maps: split distances, light matrices and the GPU depth array.
use crate::camera::Camera;
use glam::{Mat4, Vec3, Vec4};

/// Most cascades any quality level uses.
pub const MAX_CASCADES: usize = 3;
/// Depth format of the shadow map.
pub const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// How far from the camera shadows are drawn (studs).
pub const SHADOW_DISTANCE: f32 = 400.0;
/// Distance behind a cascade that casters are still captured (studs).
const CASTER_MARGIN: f32 = 300.0;

/// Far edges of each cascade as view distances, blending logarithmic and uniform splits.
pub fn cascade_splits(near: f32, far: f32, count: usize) -> Vec<f32> {
    const LAMBDA: f32 = 0.7;
    (1..=count)
        .map(|i| {
            let f = i as f32 / count as f32;
            let log = near * (far / near).powf(f);
            let uni = near + (far - near) * f;
            LAMBDA * log + (1.0 - LAMBDA) * uni
        })
        .collect()
}

/// One cascade: the light matrix and the world size of one shadow texel.
#[derive(Clone, Copy, Debug)]
pub struct Cascade {
    /// World to light-clip matrix (orthographic, depth 0..1).
    pub view_proj: Mat4,
    /// Edge length of one shadow-map texel in studs.
    pub texel_world: f32,
}

/// Fits an orthographic light volume around the part of the camera frustum between
/// `slice_near` and `slice_far`.
///
/// The volume is a square around the slice's bounding sphere. The sphere is measured in view
/// space, so its radius (and with it the texel size) depends only on the lens and the slice, not
/// on where the camera stands or looks. The light's rotation is fixed by `to_sun` alone, and the
/// sphere centre is snapped to whole texels in that rotated space, so moving the camera moves
/// the shadow-map grid over the world by whole texels only and shadow edges do not shimmer.
pub fn fit_cascade(
    camera: &Camera,
    aspect: f32,
    slice_near: f32,
    slice_far: f32,
    to_sun: Vec3,
    resolution: u32,
) -> Cascade {
    let proj = Mat4::perspective_rh(
        camera.fov_deg.to_radians(),
        aspect.max(1e-3),
        slice_near,
        slice_far,
    );
    // Slice corners in view space: the same numbers for every camera pose.
    let inv_proj = proj.inverse();
    let mut centre_view = Vec3::ZERO;
    let mut corners = [Vec3::ZERO; 8];
    let mut i = 0;
    for z in [0.0_f32, 1.0] {
        for y in [-1.0_f32, 1.0] {
            for x in [-1.0_f32, 1.0] {
                let p = inv_proj * Vec4::new(x, y, z, 1.0);
                corners[i] = p.truncate() / p.w;
                centre_view += corners[i];
                i += 1;
            }
        }
    }
    centre_view /= 8.0;
    let radius = corners
        .iter()
        .map(|c| c.distance(centre_view))
        .fold(0.0_f32, f32::max)
        .max(1.0);
    let radius = (radius * 16.0).ceil() / 16.0;
    // Half-width of the square: the radius plus exactly one texel, so the sphere still fits after
    // the centre moves by up to half a texel when it is snapped.
    let res = resolution.max(4) as f32;
    let half = radius * res / (res - 2.0);
    let texel = 2.0 * half / res;
    let centre = camera.view().inverse().transform_point3(centre_view);

    let up = if to_sun.y.abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    // Rotation only: looks from the origin along -to_sun, so +Z in light space points at the sun.
    let light_rot = Mat4::look_at_rh(Vec3::ZERO, -to_sun, up);
    let c_ls = light_rot.transform_point3(centre);
    let eye_ls = Vec3::new(
        (c_ls.x / texel).round() * texel,
        (c_ls.y / texel).round() * texel,
        c_ls.z + half + CASTER_MARGIN,
    );
    let light_view = Mat4::from_translation(-eye_ls) * light_rot;
    let light_proj =
        Mat4::orthographic_rh(-half, half, -half, half, 0.0, 2.0 * (half + CASTER_MARGIN));
    Cascade {
        view_proj: light_proj * light_view,
        texel_world: texel,
    }
}

/// The depth texture array the sun renders into, one layer per cascade.
pub struct ShadowMap {
    /// Edge length of each layer in texels.
    pub resolution: u32,
    /// Number of cascades (layers).
    pub cascades: usize,
    /// Array view sampled by the main shader.
    pub array_view: wgpu::TextureView,
    /// One 2D view per layer, used as the depth attachment.
    pub layer_views: Vec<wgpu::TextureView>,
    /// Comparison sampler for PCF lookups.
    pub sampler: wgpu::Sampler,
    _texture: wgpu::Texture,
}

impl ShadowMap {
    /// Creates a shadow map of `cascades` layers at `resolution` x `resolution`.
    pub fn new(device: &wgpu::Device, resolution: u32, cascades: usize) -> ShadowMap {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("metallic shadow map"),
            size: wgpu::Extent3d {
                width: resolution,
                height: resolution,
                depth_or_array_layers: cascades as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let array_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("metallic shadow array"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let layer_views = (0..cascades as u32)
            .map(|layer| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("metallic shadow layer"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("metallic shadow sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        ShadowMap {
            resolution,
            cascades,
            array_view,
            layer_views,
            sampler,
            _texture: texture,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_increase_and_end_at_far() {
        let s = cascade_splits(0.3, 400.0, 3);
        assert_eq!(s.len(), 3);
        assert!(s[0] < s[1] && s[1] < s[2]);
        assert!((s[2] - 400.0).abs() < 1e-3);
        assert!(s[0] > 0.3);
    }

    #[test]
    fn cascade_covers_its_frustum_slice() {
        let to_sun = Vec3::new(0.4, 0.8, 0.3).normalize();
        let cameras = [
            Camera::default(),
            Camera {
                position: Vec3::new(123.4, 7.7, -56.1),
                look_at: Vec3::new(80.0, 2.0, -10.0),
                fov_deg: 70.0,
            },
            Camera {
                position: Vec3::new(-3.3, 50.0, 9.9),
                look_at: Vec3::new(10.0, 0.0, -30.0),
                fov_deg: 90.0,
            },
        ];
        let splits = cascade_splits(0.3, SHADOW_DISTANCE, MAX_CASCADES);
        for cam in &cameras {
            for (i, &far) in splits.iter().enumerate() {
                let near = if i == 0 { 0.3 } else { splits[i - 1] };
                let c = fit_cascade(cam, 16.0 / 9.0, near, far, to_sun, 2048);
                let proj = Mat4::perspective_rh(cam.fov_deg.to_radians(), 16.0 / 9.0, near, far);
                let inv = (proj * cam.view()).inverse();
                for z in [0.0_f32, 1.0] {
                    for y in [-1.0_f32, 1.0] {
                        for x in [-1.0_f32, 1.0] {
                            let p = inv * Vec4::new(x, y, z, 1.0);
                            let world = p.truncate() / p.w;
                            let clip = c.view_proj * world.extend(1.0);
                            assert!(clip.x.abs() <= 1.0 && clip.y.abs() <= 1.0, "{clip:?}");
                            assert!((0.0..=1.0).contains(&clip.z), "depth {}", clip.z);
                        }
                    }
                }
                assert!(c.texel_world > 0.0);
            }
        }
    }

    /// Shadow-map texel coordinates of `world` in cascade `c` (the shader's clip-to-uv mapping).
    fn texel_coords(c: &Cascade, world: Vec3, resolution: u32) -> (f32, f32) {
        let clip = c.view_proj * world.extend(1.0);
        let res = resolution as f32;
        ((clip.x * 0.5 + 0.5) * res, (0.5 - clip.y * 0.5) * res)
    }

    /// Signed distance of `a - b` from the nearest whole number.
    fn frac_diff(a: f32, b: f32) -> f32 {
        let d = a - b;
        d - d.round()
    }

    #[test]
    fn shadow_snap_moves_world_point_by_whole_texels() {
        const RES: u32 = 2048;
        let to_sun = Vec3::new(0.4, 0.8, 0.3).normalize();
        let splits = cascade_splits(0.3, SHADOW_DISTANCE, MAX_CASCADES);
        let step = Vec3::new(0.137, 0.0, 0.0);
        let world = Vec3::ZERO;
        let base = Camera::default();
        let mut worst = 0.0_f32;
        for (i, &far) in splits.iter().enumerate() {
            let near = if i == 0 { 0.3 } else { splits[i - 1] };
            let first = fit_cascade(&base, 16.0 / 9.0, near, far, to_sun, RES);
            let (u0, v0) = texel_coords(&first, world, RES);
            for k in 1..=6 {
                let offset = step * k as f32;
                let cam = Camera {
                    position: base.position + offset,
                    look_at: base.look_at + offset,
                    ..base
                };
                let c = fit_cascade(&cam, 16.0 / 9.0, near, far, to_sun, RES);
                assert_eq!(
                    c.texel_world, first.texel_world,
                    "cascade {i}: texel size changed"
                );
                let (u, v) = texel_coords(&c, world, RES);
                let (du, dv) = (frac_diff(u, u0), frac_diff(v, v0));
                eprintln!("cascade {i} step {k}: off-grid ({du:.4}, {dv:.4}) texels");
                worst = worst.max(du.abs()).max(dv.abs());
            }
        }
        assert!(
            worst < 1e-3,
            "a fixed world point slid up to {worst} texels off the shadow grid"
        );
    }
}
