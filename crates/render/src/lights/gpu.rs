//! The GPU side of local lights: the lights' uniform array, the cluster texture and the shadow atlas, prepared once
//! per frame (see the [module documentation](super)).

use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use metallic_core::glam::Vec3;

use super::atlas::{ShadowAtlas, ShadowedLight, TILE_SIZE};
use super::cluster::{
    CLUSTERS_X, CLUSTERS_Y, ClusterCamera, ClusterGrid, ClusterStats, FIRST_SLICE, Slicing, TEXTURE_HEIGHT,
    TEXTURE_TEXELS, TEXTURE_WIDTH,
};
use super::select::{SelectView, influence, select};
use super::{LIGHT_INTENSITY, LightSettings, LightStats, MAX_GPU_LIGHTS, MAX_SHADOW_TILES, SPOT_SOFTNESS};
use crate::GpuScene;
use crate::cull::{CullSettings, CullView};
use crate::lod::LodView;
use crate::parts::{Camera, MeshPool};
use crate::scene::{InstanceBinding, LightKind, SceneLight};
use crate::shadow::SHADOW_FORMAT;

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

/// The GPU memory of local lights, and the CPU copy of the cluster texture: made on the first frame that draws a
/// light, never for a scene without one (the shadow atlas waits for the first shadow: [`ShadowAtlas`]).
#[derive(Debug)]
struct LightBuffers {
    light_buffer: wgpu::Buffer,
    /// The shadow tiles' uniform array.
    tile_buffer: wgpu::Buffer,
    cluster_texture: wgpu::Texture,
    cluster_view: wgpu::TextureView,
    /// What the frame group binds in place of the atlas until a light first has a shadow: 1 × 1 texel.
    no_atlas: wgpu::TextureView,
    grid: ClusterGrid,
    uploaded_texels: Vec<u32>,
    uploaded_rows: u32,
}

impl LightBuffers {
    fn new(device: &wgpu::Device) -> Self {
        let uniform = |label, size: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let texture = |label, width, height, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let cluster_texture = texture(
            "metallic light clusters",
            TEXTURE_WIDTH,
            TEXTURE_HEIGHT,
            wgpu::TextureFormat::R32Uint,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let cluster_view = cluster_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let no_atlas = texture(
            "metallic shadow atlas (none yet)",
            1,
            1,
            SHADOW_FORMAT,
            wgpu::TextureUsages::TEXTURE_BINDING,
        )
        .create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            light_buffer: uniform("metallic local lights", size_of::<GpuLight>() * MAX_GPU_LIGHTS),
            tile_buffer: uniform("metallic shadow atlas tiles", TILE_SIZE * MAX_SHADOW_TILES),
            cluster_texture,
            cluster_view,
            no_atlas,
            grid: ClusterGrid::default(),
            uploaded_texels: vec![0; TEXTURE_TEXELS],
            uploaded_rows: 0,
        }
    }
}

/// What the frame group binds for local lights ([`LocalLights::bindings`]).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LightBindings<'a> {
    pub(crate) lights: &'a wgpu::Buffer,
    pub(crate) clusters: &'a wgpu::TextureView,
    pub(crate) tiles: &'a wgpu::Buffer,
    pub(crate) atlas: &'a wgpu::TextureView,
}

/// Local lights on the GPU. [`crate::parts::PartRenderer`] owns one and binds its resources in the parts' frame
/// group. A scene without lights makes none of their GPU memory (see [`LightBuffers`] and the atlas).
#[derive(Debug)]
pub(crate) struct LocalLights {
    device: wgpu::Device,
    queue: wgpu::Queue,
    settings: LightSettings,
    buffers: Option<LightBuffers>,
    lights: Vec<GpuLight>,
    uploaded: Vec<GpuLight>,
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
    /// Local lights for `settings` on the instance path `binding`, whose storage-path instance group has
    /// `instance_layout`. Makes their pipelines (through `cache` where there is one); their buffers and textures wait
    /// for the first light.
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        binding: InstanceBinding,
        instance_layout: Option<&wgpu::BindGroupLayout>,
        settings: LightSettings,
        cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            settings,
            buffers: None,
            lights: Vec::with_capacity(MAX_GPU_LIGHTS),
            uploaded: Vec::with_capacity(MAX_GPU_LIGHTS),
            atlas: ShadowAtlas::new(device, queue, binding, instance_layout, &settings, cache),
            ranked: Vec::with_capacity(MAX_GPU_LIGHTS),
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

    /// Applies `settings` (a settings change, never per frame). A new atlas size drops the atlas (the next shadow
    /// makes it again), so the frame group must be made again: see [`LocalLights::revision`].
    pub(crate) fn configure(&mut self, settings: LightSettings) {
        let atlas_changed = (settings.atlas_resolution, settings.tile_resolution)
            != (self.settings.atlas_resolution, self.settings.tile_resolution);
        self.settings = settings;
        if atlas_changed {
            self.atlas.configure(&settings);
        }
    }

    /// Changes whenever what [`LocalLights::bindings`] returns does.
    pub(crate) fn revision(&self) -> (bool, u64) {
        (self.buffers.is_some(), self.atlas.revision())
    }

    /// What the frame group binds for local lights, once a frame has drawn one.
    pub(crate) fn bindings(&self) -> Option<LightBindings<'_>> {
        let buffers = self.buffers.as_ref()?;
        Some(LightBindings {
            lights: &buffers.light_buffer,
            clusters: &buffers.cluster_view,
            tiles: &buffers.tile_buffer,
            atlas: self.atlas.sample_view().unwrap_or(&buffers.no_atlas),
        })
    }

    /// The GPU memory local lights hold, in bytes.
    fn gpu_bytes(&self) -> u64 {
        let buffers = self.buffers.as_ref().map_or(0, |b| {
            b.light_buffer.size() + b.tile_buffer.size() + TEXTURE_TEXELS as u64 * 4 + 4
        });
        buffers + self.atlas.gpu_bytes()
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
        self.atlas.target()
    }

    /// Records the atlas's tiles into `pass` (see [`ShadowAtlas::draw`]).
    pub(crate) fn draw_atlas(&self, meshes: &MeshPool, pass: &mut wgpu::RenderPass<'_>) -> (u32, u32, u64) {
        self.atlas.draw(meshes, pass)
    }

    /// Starts every light's fade, and its shadow's, again from nothing: lights a frame could not draw yet (their
    /// pipelines were still compiling, #86) fade in over [`LightSettings::fade_time`] from this frame on instead of
    /// popping on.
    pub(crate) fn restart_fades(scene: &mut GpuScene) {
        let (_, runtime) = scene.lights_mut();
        for state in runtime {
            state.fade = 0.0;
            state.shadow_fade = 0.0;
        }
    }

    /// Selects, fades, clusters and shadows the local lights of `scene` for a frame seen from `camera` on a
    /// `width` × `height` target with the main view's `cull` settings and levels of detail `lod`, and writes what
    /// changed to the GPU. The scene is drawn at `render` pixels (the target's size, or less with a render scale),
    /// which the shader's cluster lookup divides. Run it after [`GpuScene::upload_view`] and before the sun's shadows
    /// take the scene's shadow-caster changes, every frame.
    pub(crate) fn prepare(
        &mut self,
        scene: &mut GpuScene,
        camera: Option<&Camera>,
        (width, height): (u32, u32),
        render: (u32, u32),
        cull: &CullSettings,
        lod: &LodView,
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
            self.atlas.prepare(scene, &[], &mut [], (Vec3::ZERO, lod), None);
            self.stats.gpu_bytes = self.gpu_bytes();
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
        if !self.lights.is_empty() && self.buffers.is_none() {
            // The first frame that draws a light: its buffers and textures from now on (the atlas waits for the
            // first shadow).
            self.buffers = Some(LightBuffers::new(&self.device));
            self.uploaded.clear();
            self.atlas.table_replaced();
        }
        self.bases.clear();
        self.bases.resize(self.shadowed.len(), -1.0);
        self.atlas.prepare(
            scene,
            &self.shadowed,
            &mut self.bases,
            (camera.position - origin, lod),
            self.buffers.as_ref().map(|b| &b.tile_buffer),
        );
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

        // With no light drawn the frame uses the pipelines without local lights: nothing to cluster or upload.
        let mut clusters = ClusterStats::default();
        if !self.lights.is_empty()
            && let Some(buffers) = self.buffers.as_mut()
        {
            let far = self
                .spheres
                .iter()
                .map(|&(center, radius)| center.z + radius)
                .fold(0.0f32, f32::max);
            let slicing = Slicing::new(far);
            let tan_y = (camera.fov_y.clamp(1e-3, 3.1) * 0.5).tan();
            clusters = buffers.grid.build(
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
                    CLUSTERS_X as f32 / render.0.max(1) as f32,
                    CLUSTERS_Y as f32 / render.1.max(1) as f32,
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
        }

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
            gpu_bytes: self.gpu_bytes(),
        };
    }

    /// Writes the lights and the clusters when they changed since the last frame.
    fn upload(&mut self) {
        let Some(buffers) = self.buffers.as_mut() else {
            return;
        };
        if self.lights != self.uploaded && !self.lights.is_empty() {
            self.queue
                .write_buffer(&buffers.light_buffer, 0, bytemuck::cast_slice(&self.lights));
            self.uploaded.clear();
            self.uploaded.extend_from_slice(&self.lights);
        }
        let grid = &buffers.grid;
        let rows = (grid.used as u32).div_ceil(TEXTURE_WIDTH).max(1);
        let texels = (rows * TEXTURE_WIDTH) as usize;
        if rows == buffers.uploaded_rows && grid.texels[..texels] == buffers.uploaded_texels[..texels] {
            return;
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &buffers.cluster_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&grid.texels[..texels]),
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
        buffers.uploaded_texels[..texels].copy_from_slice(&grid.texels[..texels]);
        buffers.uploaded_rows = rows;
    }
}
