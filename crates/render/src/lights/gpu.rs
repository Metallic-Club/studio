//! The GPU side of local lights: the lights' uniform array, the cluster texture and the shadow atlas, prepared once
//! per frame (see the [module documentation](super)).

use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use metallic_core::glam::Vec3;

use super::atlas::{ShadowAtlas, ShadowedLight};
use super::cluster::{
    CLUSTERS_X, CLUSTERS_Y, ClusterCamera, ClusterGrid, FIRST_SLICE, Slicing, TEXTURE_HEIGHT, TEXTURE_TEXELS,
    TEXTURE_WIDTH,
};
use super::select::{SelectView, influence, select};
use super::{LIGHT_INTENSITY, LightSettings, LightStats, MAX_GPU_LIGHTS, SPOT_SOFTNESS};
use crate::GpuScene;
use crate::cull::{CullSettings, CullView};
use crate::parts::{Camera, MeshPool};
use crate::scene::{InstanceBinding, LightKind, SceneLight};

/// The longest step a fade takes in one frame, in seconds: after a stall, lights fade on rather than jump.
const MAX_STEP: f32 = 0.1;

/// What the parts' frame uniform needs of the local lights (the `lights_*` members of `Frame` in frame.wgsl).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FrameLights {
    /// Columns per pixel, rows per pixel, and the depth slices' scale and bias ([`Slicing`]).
    pub(crate) clusters: [f32; 4],
    /// Lights drawn (0: none), the first slice's depth, the normal offset and the depth bias in texels.
    pub(crate) local: [f32; 4],
    /// One atlas texel in atlas coordinates, the filter's reach in texels, and 1 when any drawn light has a shadow.
    pub(crate) atlas: [f32; 4],
}

/// One light as the parts read it (`LocalLight` in lights.wgsl): 64 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub(crate) struct GpuLight {
    /// Camera-relative position, and the range.
    position: [f32; 4],
    /// Linear intensity: colour × brightness × [`LIGHT_INTENSITY`] × fade.
    color: [f32; 4],
    /// Where a spot points (unit), and the cosine of its cone's half-angle (−2 for a point light).
    direction: [f32; 4],
    /// The cone's falloff scale and offset, the light's first tile in the tile table (−1: unshadowed), and how much
    /// of its shadow is drawn.
    params: [f32; 4],
}

impl GpuLight {
    fn new(light: &SceneLight, camera: Vec3, fade: f32, shadow: f32) -> Self {
        let intensity = light.intensity * (LIGHT_INTENSITY * fade);
        let position = light.position - camera;
        let (cone, scale, offset) = match light.kind {
            LightKind::Point => (-2.0, 0.0, 0.0),
            LightKind::Spot => {
                let half = (light.angle * 0.5).to_radians();
                let outer = half.cos();
                let inner = (half * (1.0 - SPOT_SOFTNESS)).cos();
                let scale = 1.0 / (inner - outer).max(1e-4);
                (outer, scale, -outer * scale)
            }
        };
        Self {
            position: [position.x, position.y, position.z, light.range],
            color: [intensity.x, intensity.y, intensity.z, 0.0],
            direction: [light.direction.x, light.direction.y, light.direction.z, cone],
            params: [scale, offset, -1.0, shadow],
        }
    }
}

/// Local lights on the GPU. [`crate::parts::PartRenderer`] owns one and binds its resources in the parts' frame
/// group.
#[derive(Debug)]
pub(crate) struct LocalLights {
    queue: wgpu::Queue,
    settings: LightSettings,
    light_buffer: wgpu::Buffer,
    lights: Vec<GpuLight>,
    uploaded: Vec<GpuLight>,
    cluster_texture: wgpu::Texture,
    cluster_view: wgpu::TextureView,
    grid: ClusterGrid,
    uploaded_texels: Vec<u32>,
    uploaded_rows: u32,
    atlas: ShadowAtlas,
    ranked: Vec<(f32, u32)>,
    drawn: Vec<u32>,
    spheres: Vec<(Vec3, f32)>,
    shadowed: Vec<ShadowedLight>,
    /// For each shadowed light, its position in the drawn list, and its first entry in the tile table.
    shadow_slots: Vec<usize>,
    bases: Vec<f32>,
    frame: FrameLights,
    stats: LightStats,
    last: Option<Instant>,
    /// Seconds a frame advances the fades by, instead of the wall time since the last frame.
    fixed_step: Option<f32>,
}

impl LocalLights {
    /// The lights' buffers, the cluster texture and the atlas for `settings`, on the instance path `binding` whose
    /// storage-path instance group has `instance_layout`.
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        binding: InstanceBinding,
        instance_layout: Option<&wgpu::BindGroupLayout>,
        settings: LightSettings,
    ) -> Self {
        let light_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic local lights"),
            size: (size_of::<GpuLight>() * MAX_GPU_LIGHTS) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cluster_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("metallic light clusters"),
            size: wgpu::Extent3d {
                width: TEXTURE_WIDTH,
                height: TEXTURE_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let cluster_view = cluster_texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            queue: queue.clone(),
            settings,
            light_buffer,
            lights: Vec::with_capacity(MAX_GPU_LIGHTS),
            uploaded: Vec::with_capacity(MAX_GPU_LIGHTS),
            cluster_texture,
            cluster_view,
            grid: ClusterGrid::default(),
            uploaded_texels: vec![0; TEXTURE_TEXELS],
            uploaded_rows: 0,
            atlas: ShadowAtlas::new(device, queue, binding, instance_layout, &settings),
            ranked: Vec::new(),
            drawn: Vec::with_capacity(MAX_GPU_LIGHTS),
            spheres: Vec::with_capacity(MAX_GPU_LIGHTS),
            shadowed: Vec::with_capacity(MAX_GPU_LIGHTS),
            shadow_slots: Vec::with_capacity(MAX_GPU_LIGHTS),
            bases: Vec::with_capacity(MAX_GPU_LIGHTS),
            frame: FrameLights::default(),
            stats: LightStats::default(),
            last: None,
            fixed_step: None,
        }
    }

    /// Advances the fades by `step` seconds a frame instead of the wall time between frames (`None`, the default).
    pub(crate) fn set_fixed_step(&mut self, step: Option<f32>) {
        self.fixed_step = step.filter(|s| s.is_finite()).map(|s| s.max(0.0));
    }

    /// The settings in effect.
    pub(crate) fn settings(&self) -> &LightSettings {
        &self.settings
    }

    /// Applies `settings` (a settings change, never per frame). Returns whether the atlas was made again, so the
    /// bind groups that sample it must be made again.
    pub(crate) fn configure(&mut self, settings: LightSettings) -> bool {
        let atlas_changed = (settings.atlas_resolution, settings.tile_resolution)
            != (self.settings.atlas_resolution, self.settings.tile_resolution);
        self.settings = settings;
        atlas_changed && self.atlas.configure(&settings)
    }

    /// The lights' uniform array.
    pub(crate) fn light_buffer(&self) -> &wgpu::Buffer {
        &self.light_buffer
    }

    /// The cluster texture.
    pub(crate) fn cluster_view(&self) -> &wgpu::TextureView {
        &self.cluster_view
    }

    /// The shadow atlas, sampled.
    pub(crate) fn atlas_view(&self) -> &wgpu::TextureView {
        self.atlas.sample_view()
    }

    /// The shadow tiles' uniform array.
    pub(crate) fn tile_buffer(&self) -> &wgpu::Buffer {
        self.atlas.table_buffer()
    }

    /// What the frame uniform needs this frame.
    pub(crate) fn frame(&self) -> &FrameLights {
        &self.frame
    }

    /// What the last frame did.
    pub(crate) fn stats(&self) -> LightStats {
        self.stats
    }

    /// The atlas's render target when tiles are drawn this frame.
    pub(crate) fn atlas_target(&self) -> Option<&wgpu::TextureView> {
        self.atlas.has_jobs().then(|| self.atlas.render_view())
    }

    /// Records the atlas's tiles into `pass` (see [`ShadowAtlas::draw`]).
    pub(crate) fn draw_atlas(&self, meshes: &MeshPool, pass: &mut wgpu::RenderPass<'_>) -> (u32, u32, u64) {
        self.atlas.draw(meshes, pass)
    }

    /// Selects, fades, clusters and shadows the local lights of `scene` for a frame seen from `camera` on a
    /// `width` × `height` target with the main view's `cull` settings, and writes what changed to the GPU. Run it
    /// after [`GpuScene::upload_view`] and before the sun's shadows take the scene's shadow-caster changes, every
    /// frame.
    pub(crate) fn prepare(
        &mut self,
        scene: &mut GpuScene,
        camera: Option<&Camera>,
        width: u32,
        height: u32,
        cull: &CullSettings,
    ) {
        let start = Instant::now();
        let dt = match (self.fixed_step, self.last) {
            (Some(step), Some(_)) => step,
            (_, None) => 0.0,
            (None, Some(last)) => start.saturating_duration_since(last).as_secs_f32().min(MAX_STEP),
        };
        self.last = Some(start);
        self.stats = LightStats {
            in_scene: scene.scene().light_count() as u32,
            ..LightStats::default()
        };
        self.frame = FrameLights::default();
        self.lights.clear();
        let camera = camera.filter(|_| self.settings.enabled && self.stats.in_scene > 0);
        let Some(camera) = camera else {
            self.atlas.prepare(scene, &[], &mut [], Vec3::ZERO);
            return;
        };
        let origin = scene.scene().origin();
        let aspect = width as f32 / height.max(1) as f32;
        let view = CullView::for_camera(camera, origin, width, height, cull);
        let (right, up, forward) = (
            camera.rotation * Vec3::X,
            camera.rotation * Vec3::Y,
            camera.rotation * Vec3::NEG_Z,
        );
        self.spheres.clear();
        self.shadowed.clear();
        self.shadow_slots.clear();
        let selection = {
            let (lights, runtime) = scene.lights_mut();
            let selection = select(
                lights,
                runtime,
                &SelectView {
                    frustum: &view.frustum,
                    origin,
                    eye: camera.position,
                    max_distance: view.max_distance,
                },
                &self.settings,
                dt,
                &mut self.ranked,
                &mut self.drawn,
            );
            self.atlas.assign(lights, runtime, &self.drawn, &self.settings, dt);
            for (slot, &index) in self.drawn.iter().enumerate() {
                let (light, state) = (&lights[index as usize], &runtime[index as usize]);
                let shadow = if state.tile_count > 0 { state.shadow_fade } else { 0.0 };
                self.lights
                    .push(GpuLight::new(light, camera.position, state.fade, shadow));
                if shadow > 0.0 {
                    self.shadowed.push(ShadowedLight {
                        light: *light,
                        tiles: state.tiles,
                        tile_count: state.tile_count,
                    });
                    self.shadow_slots.push(slot);
                }
                let (center, radius) = influence(light);
                let relative = center - camera.position;
                self.spheres.push((
                    Vec3::new(relative.dot(right), relative.dot(up), relative.dot(forward)),
                    radius,
                ));
            }
            selection
        };
        self.bases.clear();
        self.bases.resize(self.shadowed.len(), -1.0);
        self.atlas
            .prepare(scene, &self.shadowed, &mut self.bases, camera.position - origin);
        let mut any_shadow = false;
        for (&slot, &base) in self.shadow_slots.iter().zip(&self.bases) {
            if let Some(light) = self.lights.get_mut(slot) {
                light.params[2] = base;
                if base < 0.0 {
                    // No room in the tile table: drawn without its shadow.
                    light.params[3] = 0.0;
                }
                any_shadow |= light.params[3] > 0.0;
            }
        }

        let far = self
            .spheres
            .iter()
            .map(|&(center, radius)| center.z + radius)
            .fold(0.0f32, f32::max);
        let slicing = Slicing::new(far);
        let tan_y = (camera.fov_y.clamp(1e-3, 3.1) * 0.5).tan();
        let clusters = self.grid.build(
            &self.spheres,
            &ClusterCamera {
                tan_x: tan_y * aspect,
                tan_y,
            },
            &slicing,
            self.settings.cluster_cap(),
        );
        self.frame = FrameLights {
            clusters: [
                CLUSTERS_X as f32 / width.max(1) as f32,
                CLUSTERS_Y as f32 / height.max(1) as f32,
                slicing.scale,
                slicing.bias,
            ],
            local: [
                self.lights.len() as f32,
                FIRST_SLICE,
                self.settings.normal_offset.max(0.0),
                self.settings.depth_bias.max(0.0),
            ],
            atlas: [
                self.atlas.texel(),
                self.settings.filter_radius.min(1) as f32,
                if any_shadow { 1.0 } else { 0.0 },
                0.0,
            ],
        };
        self.upload();

        let atlas = self.atlas.stats();
        self.stats = LightStats {
            in_scene: self.stats.in_scene,
            in_view: selection.in_view,
            in_budget: selection.in_budget,
            drawn: selection.drawn,
            fading: selection.fading,
            shadowed: atlas.shadowed,
            shadow_tiles: atlas.tiles,
            tiles_redrawn: atlas.redrawn,
            shadow_casters: atlas.casters,
            lit_clusters: clusters.lit_clusters,
            cluster_entries: clusters.entries,
            max_per_cluster: clusters.max_per_cluster,
            overflow: clusters.overflow,
            cpu_time: start.elapsed().saturating_sub(atlas.cull_time),
            shadow_cull_time: atlas.cull_time,
        };
    }

    /// Writes the lights and the clusters when they changed since the last frame.
    fn upload(&mut self) {
        if self.lights != self.uploaded && !self.lights.is_empty() {
            self.queue
                .write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(&self.lights));
            self.uploaded.clear();
            self.uploaded.extend_from_slice(&self.lights);
        }
        let rows = (self.grid.used as u32).div_ceil(TEXTURE_WIDTH).max(1);
        let texels = (rows * TEXTURE_WIDTH) as usize;
        if rows == self.uploaded_rows && self.grid.texels[..texels] == self.uploaded_texels[..texels] {
            return;
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.cluster_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&self.grid.texels[..texels]),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(TEXTURE_WIDTH * 4),
                rows_per_image: Some(rows),
            },
            wgpu::Extent3d {
                width: TEXTURE_WIDTH,
                height: rows,
                depth_or_array_layers: 1,
            },
        );
        self.uploaded_texels[..texels].copy_from_slice(&self.grid.texels[..texels]);
        self.uploaded_rows = rows;
    }
}
