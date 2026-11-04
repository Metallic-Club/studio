//! Drawing parts: the primitive meshes, the material table, the camera, and the draws (ADR 0008 §4).
//!
//! # A constant number of draws
//!
//! Parts are drawn with **one instanced draw per mesh kind per shading model**, so the draw count depends on how
//! many kinds and models there are, never on how many parts. There is one shading model for now (PBR-lite, below),
//! so the opaque pass makes at most one draw per mesh kind: four for the built-in primitives. The material of each
//! instance comes from the material table ([`MaterialTable`]) through the instance's material row, so materials
//! cost no draws either.
//!
//! Which slots a draw covers comes from the scene's **draw lists** ([`crate::scene`]): one list of entries per
//! mesh kind, kept current from the dirty slots alone, laid out in one buffer with a segment per kind
//! ([`crate::GpuScene::list_segments`]). A kind's draw is instances `first .. first + len` of its segment:
//!
//! - **Storage path** (every backend whose vertex shaders read storage buffers): the instance index selects an
//!   entry, and the entry names the slot whose packed instance the shader reads from the region buffers. No
//!   instance data is copied or compacted; a list entry is 4 bytes and changes only when a slot starts or stops
//!   being drawn as that kind.
//! - **Vertex-buffer path** (GLES): vertex buffers cannot be indexed through a list, so there the list holds
//!   copies of the instances, read as per-instance attributes.
//!
//! Culling ([`crate::cull`], on by default: [`crate::RendererSettings::cull`]) fills the same lists with the
//! *visible* entries of each kind instead of all of them, front to back, and the transparent pass sorts only the
//! visible see-through instances. A GPU culling path can later write the counts into indirect draw arguments.
//! Indirect draws are not used yet: with nothing on the GPU writing the counts they would only add wgpu's
//! indirect-argument validation pass.
//!
//! # Transparency
//!
//! Partly see-through instances are drawn in the transparent pass, after opaque and the sky, with the depth test
//! on and depth writes off, blended with premultiplied alpha. They are **sorted back to front per instance**
//! (by distance from the camera to the instance's centre) whenever the camera moves or one of them changes, and
//! drawn as one draw per run of consecutive instances with the same mesh, so the order is exact across mesh kinds.
//! Fully transparent instances are not drawn at all.
//!
//! # Shading
//!
//! PBR-lite: Lambert diffuse and GGX specular (height-correlated Smith visibility, Schlick Fresnel) under one
//! directional light (the sun, or the moon at night), plus a sky/ground hemisphere ambient with a split-sum
//! environment term, and the material's emission. The light, the ambient and the sky come from `Lighting`
//! ([`crate::lighting`], [`PartRenderer::set_lighting`]). Colours are sRGB-encoded in the instance and decoded to
//! linear light in the shader; the output is linear, encoded by an sRGB target (or by the shader for other
//! formats).
//!
//! The directional light is shadowed by the sun's cascaded shadow maps ([`crate::shadow`]): the parts sample them
//! (bound in the frame group with a comparison sampler, their matrices in the frame uniform) only where the light
//! reaches the surface at all. [`PartRenderer`] owns the maps and draws their casters before the frame's passes.
//!
//! Point and spot lights ([`crate::lights`]) add their light through the same BRDF: each fragment loops over the
//! lights its cluster lists, read from a uniform array and a cluster texture in the frame group, with the shadow
//! atlas beside the sun's maps. [`PartRenderer`] owns them too and draws the atlas's tiles after the cascades.
//!
//! # Sky
//!
//! The sky pass draws one full-screen triangle at the far plane (depth 0 under reverse-Z) after the opaque parts,
//! testing depth with `GreaterEqual` against the cleared value, so it shades only the pixels no part covered
//! (ADR 0008 §7): one draw of three vertices, no textures. Its colour is an analytic function of the view
//! direction (`sky.wgsl`): a gradient from `SkyColor` overhead to `HorizonColor`, a twilight glow, and the sun and
//! moon discs. It reads the frame uniform the parts read, so it adds no upload. A textured sky (a cubemap looked up
//! with the same direction) can replace the gradient later inside the same pass. With no camera there is no view
//! and no sky: the frame keeps its clear colour.
//!
//! # Camera
//!
//! [`Camera`] is `Workspace.CurrentCamera` ([`CameraReader`]): a reverse-Z projection with an infinite far plane
//! from `FieldOfView` and the target's aspect ratio, and a rotation-only view. The translation goes into the
//! frame uniform as `camera − scene origin`, so positions reach the projection relative to the camera (ADR 0008
//! §11). Without a camera nothing is drawn.
//!
//! Every pipeline (shadow casters, depth prepass, opaque, opaque after a prepass, sky, transparent) is created up
//! front, when the renderer is made or its colour format or MSAA count change, never during a frame (ADR 0008 §9).

mod camera;
mod material;
mod mesh;
mod transparent;

pub use camera::{Camera, CameraReader};
pub use material::{MAX_MATERIAL_ROWS, MaterialParams, MaterialTable};
pub use mesh::{MeshData, MeshPool, MeshRange, MeshVertex};

use bytemuck::{Pod, Zeroable};
use metallic_core::glam::Vec3;

use crate::cull::CullSettings;
use crate::depth::{DEPTH_COMPARE, DEPTH_COMPARE_FAR_PLANE, DEPTH_FORMAT, depth_stencil_state};
use crate::lighting::{ResolvedLighting, SceneLighting};
use crate::lights::{
    FrameLights, LightBindings, LightSettings, LightStats, LocalLights, MAX_GPU_LIGHTS, MAX_SHADOW_TILES,
};
use crate::scene::{GpuInstance, InstanceBinding, MeshKind};
use crate::shadow::{FrameShadow, MAX_CASCADES, ShadowMaps, ShadowSettings, ShadowStats};
use crate::{GpuScene, PassId};
use material::MATERIAL_TABLE_SIZE;
use transparent::TransparentList;

/// What the last frame drew.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrawStats {
    /// Draws in the depth prepass (one per mesh kind with instances; 0 without a prepass).
    pub prepass_draws: u32,
    /// Draws in the opaque pass: one per mesh kind with instances, whatever the part count.
    pub opaque_draws: u32,
    /// Draws in the transparent pass: one per run of same-mesh instances in back-to-front order.
    pub transparent_draws: u32,
    /// Draws in the sky pass: one full-screen triangle when there is a camera and the pass runs.
    pub sky_draws: u32,
    /// Instances drawn in the opaque and transparent passes.
    pub instances: u32,
    /// Triangles drawn in the opaque and transparent passes.
    pub triangles: u64,
    /// Instances drawn in the opaque pass (and again in the depth prepass when there is one).
    pub opaque_instances: u32,
    /// Triangles drawn in the opaque pass (and again in the depth prepass when there is one).
    pub opaque_triangles: u64,
    /// Instances drawn in the transparent pass.
    pub transparent_instances: u32,
    /// Triangles drawn in the transparent pass.
    pub transparent_triangles: u64,
    /// Instances culled from the frame (outside the view, too far or too small; see [`crate::cull`]).
    pub culled_instances: u32,
    /// Sun shadow cascades in use ([`crate::shadow`]; 0 without shadows).
    pub shadow_cascades: u32,
    /// Bit `i` set when cascade `i`'s casters were drawn this frame (a cached cascade's: its static map), otherwise
    /// they were kept.
    pub shadow_redrawn: u8,
    /// Bit `i` set when cached cascade `i`'s map was built this frame: its static map copied in and its moving
    /// casters drawn on top ([`crate::shadow`]).
    pub shadow_built: u8,
    /// Draws in the shadow passes this frame: one per mesh kind in each cascade drawn and each cached cascade built
    /// (the copies of the static maps are not counted).
    pub shadow_draws: u32,
    /// Caster instances drawn into the cascades this frame.
    pub shadow_instances: u32,
    /// Triangles drawn into the cascades this frame.
    pub shadow_triangles: u64,
    /// The casters each cascade's map holds, as of its last drawing (a cached cascade's: its static map's).
    pub shadow_casters: [u32; MAX_CASCADES],
    /// The moving casters drawn on each cached cascade's map in this frame (or kept on it from an earlier one).
    pub shadow_moving: [u32; MAX_CASCADES],
    /// Draws in the local lights' shadow atlas this frame: a clear and one per mesh kind in each tile drawn
    /// ([`crate::lights`]); the clears are not counted.
    pub light_shadow_draws: u32,
    /// Caster instances drawn into the atlas this frame.
    pub light_shadow_instances: u32,
    /// Triangles drawn into the atlas this frame.
    pub light_shadow_triangles: u64,
    /// What the frame did for local lights: how many there are, are drawn and have shadows.
    pub lights: LightStats,
}

impl DrawStats {
    /// Every draw of the frame.
    pub fn total_draws(&self) -> u32 {
        self.prepass_draws
            + self.opaque_draws
            + self.sky_draws
            + self.transparent_draws
            + self.shadow_draws
            + self.light_shadow_draws
    }

    /// How many cascades were drawn this frame.
    pub fn shadow_redrawn_count(&self) -> u32 {
        self.shadow_redrawn.count_ones()
    }
}

/// The frame uniform (`Frame` in `frame.wgsl`), read by the parts and the sky.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
struct FrameUniforms {
    view_proj: [[f32; 4]; 4],
    camera_offset: [f32; 4],
    light_direction: [f32; 4],
    light_color: [f32; 4],
    sky_ambient: [f32; 4],
    ground_ambient: [f32; 4],
    output: [f32; 4],
    view_right: [f32; 4],
    view_up: [f32; 4],
    view_forward: [f32; 4],
    sky_zenith: [f32; 4],
    sky_horizon: [f32; 4],
    sky_ground: [f32; 4],
    sky_glow: [f32; 4],
    sun: [f32; 4],
    sun_disc: [f32; 4],
    sun_halo: [f32; 4],
    moon_disc: [f32; 4],
    shadow_matrices: [[[f32; 4]; 4]; MAX_CASCADES],
    shadow_splits: [f32; 4],
    shadow_offsets: [f32; 4],
    shadow_biases: [f32; 4],
    shadow_params: [f32; 4],
    lights_clusters: [f32; 4],
    lights_local: [f32; 4],
    lights_atlas: [f32; 4],
}

/// The frame group's entries a frame without local lights binds: the frame uniform, the materials, the sun's shadow
/// maps and their sampler.
const SUN_ENTRIES: usize = 4;

/// The local lights' uniform array (`local_lights` in lights.wgsl).
const LIGHTS_SIZE: u64 = 64 * MAX_GPU_LIGHTS as u64;
/// The shadow tiles' uniform array (`shadow_tiles` in lights.wgsl).
const TILES_SIZE: u64 = 96 * MAX_SHADOW_TILES as u64;

const _: () = assert!(MAX_CASCADES == 4, "frame.wgsl holds four cascades");
const _: () = assert!(
    MAX_GPU_LIGHTS == 256 && MAX_SHADOW_TILES == 64,
    "lights.wgsl holds 256 lights and 64 tiles"
);

fn vec4(v: Vec3) -> [f32; 4] {
    [v.x, v.y, v.z, 0.0]
}

fn vec4w(v: Vec3, w: f32) -> [f32; 4] {
    [v.x, v.y, v.z, w]
}

impl FrameUniforms {
    /// The uniform for `camera` on a target `aspect` (width / height) wide, `offset` (camera − scene origin) from
    /// the scene origin, lit by `light` with `shadow`'s cascades and the local lights `local`.
    fn new(
        camera: &Camera,
        aspect: f32,
        offset: Vec3,
        light: &ResolvedLighting,
        shadow: &FrameShadow,
        local: &FrameLights,
        srgb: bool,
    ) -> Self {
        let aspect = if aspect.is_finite() && aspect > 0.0 {
            aspect
        } else {
            1.0
        };
        // The sky's view rays: through the screen's edges at NDC ±1.
        let tan_half = (camera.fov_y * 0.5).tan();
        Self {
            view_proj: camera.view_projection(aspect).to_cols_array_2d(),
            camera_offset: vec4(offset),
            light_direction: vec4(light.light_direction),
            light_color: vec4(light.light_color),
            sky_ambient: vec4(light.sky_ambient),
            ground_ambient: vec4(light.ground_ambient),
            output: [if srgb { 0.0 } else { 1.0 }, 0.0, 0.0, 0.0],
            view_right: vec4(camera.rotation * Vec3::X * (tan_half * aspect)),
            view_up: vec4(camera.rotation * Vec3::Y * tan_half),
            view_forward: vec4(camera.rotation * Vec3::NEG_Z),
            sky_zenith: vec4(light.sky_zenith),
            sky_horizon: vec4(light.sky_horizon),
            sky_ground: vec4(light.sky_ground),
            sky_glow: vec4(light.sky_glow),
            sun: vec4w(light.sun_direction, light.sun_cos_radius),
            sun_disc: vec4(light.sun_disc),
            sun_halo: vec4(light.sun_halo),
            moon_disc: vec4w(light.moon_disc, light.moon_cos_radius),
            shadow_matrices: shadow.matrices.map(|m| m.to_cols_array_2d()),
            shadow_splits: shadow.splits,
            shadow_offsets: shadow.offsets,
            shadow_biases: shadow.biases,
            shadow_params: shadow.params,
            lights_clusters: local.clusters,
            lights_local: local.local,
            lights_atlas: local.atlas,
        }
    }
}

/// One opaque draw: a mesh kind's index range over its list segment.
#[derive(Clone, Copy, Debug, Default)]
struct KindDraw {
    mesh: MeshRange,
    first: u32,
    count: u32,
}

/// The render pipelines for one colour format and sample count.
#[derive(Debug)]
struct Pipelines {
    format: wgpu::TextureFormat,
    samples: u32,
    prepass: wgpu::RenderPipeline,
    opaque: wgpu::RenderPipeline,
    /// Opaque after a depth prepass: tests depth (`GreaterEqual`) without writing it.
    opaque_tested: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
    /// The full-screen sky triangle at the far plane.
    sky: wgpu::RenderPipeline,
    /// `opaque`, `opaque_tested` and `transparent` made without local lights (lights_off.wgsl, the frame group
    /// without the lights' entries), for frames that draw none: the light loop would cost every pixel even when it
    /// runs no iteration, and such a frame binds exactly what it did before local lights existed.
    unlit: [wgpu::RenderPipeline; 3],
    /// The same with local lights but without their shadows (`LOCAL_SHADOWS` off), for frames whose drawn lights
    /// have none.
    unshadowed: [wgpu::RenderPipeline; 3],
}

/// Which variant of the part pipelines a frame uses: made without local lights (lights_off.wgsl), or with them and
/// `LOCAL_SHADOWS` (lights.wgsl) off or on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lights {
    /// No local light is drawn.
    None,
    /// Local lights without shadows.
    Unshadowed,
    /// Local lights, some with shadows.
    Shadowed,
}

/// `LOCAL_SHADOWS` (lights.wgsl) off and on.
const UNSHADOWED: [(&str, f64); 1] = [("LOCAL_SHADOWS", 0.0)];
const SHADOWED: [(&str, f64); 1] = [("LOCAL_SHADOWS", 1.0)];

impl Lights {
    /// The variant's pipeline constants: none for the module without local lights (lights_off.wgsl).
    fn constants(self) -> &'static [(&'static str, f64)] {
        match self {
            Lights::None => &[],
            Lights::Unshadowed => &UNSHADOWED,
            Lights::Shadowed => &SHADOWED,
        }
    }
}

impl Pipelines {
    /// `[opaque, opaque after a prepass, transparent]` for frames with `lights`.
    fn lit(&self, lights: Lights) -> [&wgpu::RenderPipeline; 3] {
        match lights {
            Lights::Shadowed => [&self.opaque, &self.opaque_tested, &self.transparent],
            Lights::Unshadowed => self.unshadowed.each_ref(),
            Lights::None => self.unlit.each_ref(),
        }
    }
}

/// What the pipelines are made from.
#[derive(Debug)]
struct Shaders {
    /// The parts' layout: the frame group, and on the storage path the instance group.
    parts_layout: wgpu::PipelineLayout,
    parts: wgpu::ShaderModule,
    /// The same without local lights (lights_off.wgsl): the frame group without the lights' entries.
    unlit_layout: wgpu::PipelineLayout,
    unlit: wgpu::ShaderModule,
    /// The sky's layout: the frame group without the lights' entries.
    sky_layout: wgpu::PipelineLayout,
    sky: wgpu::ShaderModule,
}

/// The storage path's instance bind group and the buffers it was made for.
#[derive(Debug)]
struct InstanceGroup {
    buffers: [wgpu::Buffer; 3],
    group: wgpu::BindGroup,
}

/// Draws a [`GpuScene`]'s parts, and the sky behind them, into the renderer's passes. [`crate::Renderer`] owns
/// one; see the [module documentation](self) for the design.
#[derive(Debug)]
pub struct PartRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    binding: InstanceBinding,
    meshes: MeshPool,
    materials: MaterialTable,
    lighting: SceneLighting,
    /// `lighting` worked out for the shaders, once per change.
    resolved: ResolvedLighting,
    frame_buffer: wgpu::Buffer,
    /// What `frame_buffer` holds, so an unchanged frame skips the write.
    frame_uniforms: Option<FrameUniforms>,
    /// The frame group of frames with local lights: the sun's and the lights' resources. `None` until the first
    /// frame that draws a light (their buffers do not exist before).
    frame_layout: wgpu::BindGroupLayout,
    frame_group: Option<wgpu::BindGroup>,
    /// What the local lights' resources were when `frame_group` was made ([`LocalLights::revision`]).
    frame_group_lights: (bool, u64),
    /// The frame group of the sky and of frames without local lights: the frame uniform, the materials and the
    /// sun's shadows alone.
    sun_layout: wgpu::BindGroupLayout,
    sun_group: wgpu::BindGroup,
    instance_layout: Option<wgpu::BindGroupLayout>,
    /// The sun's shadow maps, which the frame group samples.
    shadows: ShadowMaps,
    /// The point and spot lights, which the frame group reads.
    lights: LocalLights,
    shaders: Shaders,
    pipelines: Pipelines,
    opaque_group: Option<InstanceGroup>,
    transparent_group: Option<InstanceGroup>,
    transparent: TransparentList,
    draws: Vec<KindDraw>,
    /// Whether this frame has a camera (and so draws anything).
    visible: bool,
    stats: DrawStats,
}

impl PartRenderer {
    /// Creates the meshes, the material table and every pipeline for targets in `format` with `samples` samples,
    /// on the instance path `binding` (which must be the one the scenes it draws use:
    /// [`InstanceBinding::for_context`]).
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        binding: InstanceBinding,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let (source, unlit_source) = match binding {
            InstanceBinding::Storage => (
                concat!(
                    include_str!("frame.wgsl"),
                    include_str!("parts.wgsl"),
                    include_str!("lights.wgsl"),
                    include_str!("instance_storage.wgsl")
                ),
                concat!(
                    include_str!("frame.wgsl"),
                    include_str!("parts.wgsl"),
                    include_str!("lights_off.wgsl"),
                    include_str!("instance_storage.wgsl")
                ),
            ),
            InstanceBinding::Vertex => (
                concat!(
                    include_str!("frame.wgsl"),
                    include_str!("parts.wgsl"),
                    include_str!("lights.wgsl"),
                    include_str!("instance_vertex.wgsl")
                ),
                concat!(
                    include_str!("frame.wgsl"),
                    include_str!("parts.wgsl"),
                    include_str!("lights_off.wgsl"),
                    include_str!("instance_vertex.wgsl")
                ),
            ),
        };
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("metallic parts"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let unlit_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("metallic parts (no local lights)"),
            source: wgpu::ShaderSource::Wgsl(unlit_source.into()),
        });
        let sky = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("metallic sky"),
            source: wgpu::ShaderSource::Wgsl(concat!(include_str!("frame.wgsl"), include_str!("sky.wgsl")).into()),
        });
        let uniform = |binding, visibility, size: u64| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(size),
            },
            count: None,
        };
        let frame_entries = [
            uniform(
                0,
                wgpu::ShaderStages::VERTEX_FRAGMENT,
                size_of::<FrameUniforms>() as u64,
            ),
            uniform(1, wgpu::ShaderStages::FRAGMENT, MATERIAL_TABLE_SIZE),
            // The sun's shadow maps and their comparison sampler (`crate::shadow`).
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
            // The local lights (`crate::lights`): their uniform array, the cluster texture, the shadow tiles and
            // the shadow atlas.
            uniform(4, wgpu::ShaderStages::FRAGMENT, LIGHTS_SIZE),
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            uniform(6, wgpu::ShaderStages::FRAGMENT, TILES_SIZE),
            wgpu::BindGroupLayoutEntry {
                binding: 7,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ];
        // The pipelines of frames with local lights bind all of it; the sky and the pipelines of frames without
        // (lights_off.wgsl) bind the first four entries alone, so such a frame binds what it did before local lights.
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("metallic parts frame"),
            entries: &frame_entries,
        });
        let sun_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("metallic parts frame (no local lights)"),
            entries: &frame_entries[..SUN_ENTRIES],
        });
        let instance_layout = (binding == InstanceBinding::Storage).then(|| {
            let storage = |binding, size: u64| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size),
                },
                count: None,
            };
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("metallic parts instances"),
                entries: &[
                    storage(0, GpuInstance::SIZE),
                    storage(1, GpuInstance::SIZE),
                    storage(2, size_of::<u32>() as u64),
                ],
            })
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("metallic parts"),
            bind_group_layouts: &[Some(&frame_layout), instance_layout.as_ref()],
            immediate_size: 0,
        });
        let unlit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("metallic parts (no local lights)"),
            bind_group_layouts: &[Some(&sun_layout), instance_layout.as_ref()],
            immediate_size: 0,
        });
        let sky_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("metallic sky"),
            bind_group_layouts: &[Some(&sun_layout)],
            immediate_size: 0,
        });
        let shaders = Shaders {
            parts_layout: pipeline_layout,
            parts: module,
            unlit_layout,
            unlit: unlit_module,
            sky_layout,
            sky,
        };
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic parts frame"),
            size: size_of::<FrameUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let materials = MaterialTable::new(device);
        debug_assert_eq!(materials.buffer().size(), MATERIAL_TABLE_SIZE);
        let shadows = ShadowMaps::new(device, queue, binding, instance_layout.as_ref());
        let lights = LocalLights::new(
            device,
            queue,
            binding,
            instance_layout.as_ref(),
            LightSettings::default(),
        );
        let sun_group = frame_group_without_lights(device, &sun_layout, &frame_buffer, &materials, &shadows);
        let frame_group_lights = lights.revision();
        let pipelines = create_pipelines(device, &shaders, binding, format, samples);
        let lighting = SceneLighting::default();
        Self {
            device: device.clone(),
            queue: queue.clone(),
            binding,
            meshes: MeshPool::new(device, queue),
            materials,
            lighting,
            resolved: lighting.resolve(),
            frame_buffer,
            frame_uniforms: None,
            frame_layout,
            frame_group: None,
            frame_group_lights,
            sun_layout,
            sun_group,
            instance_layout,
            shadows,
            lights,
            shaders,
            pipelines,
            opaque_group: None,
            transparent_group: None,
            transparent: TransparentList::new(device, binding),
            draws: Vec::new(),
            visible: false,
            stats: DrawStats::default(),
        }
    }

    /// The instance path the pipelines were made for.
    pub fn binding(&self) -> InstanceBinding {
        self.binding
    }

    /// Recreates the pipelines for another colour format or sample count (a settings change, never per frame).
    pub(crate) fn set_target(&mut self, format: wgpu::TextureFormat, samples: u32) {
        if (format, samples) != (self.pipelines.format, self.pipelines.samples) {
            self.pipelines = create_pipelines(&self.device, &self.shaders, self.binding, format, samples);
        }
    }

    /// The material table. Rows changed here are uploaded with the next frame.
    pub fn materials_mut(&mut self) -> &mut MaterialTable {
        &mut self.materials
    }

    /// The material table.
    pub fn materials(&self) -> &MaterialTable {
        &self.materials
    }

    /// What the scene is lit with: the sun or moon, the ambient and the sky.
    pub fn lighting(&self) -> SceneLighting {
        self.lighting
    }

    /// Changes what the scene is lit with, from the next frame. Works the new lighting out for the shaders here,
    /// so call it when the lighting changes (the engine does when `Lighting` does), not every frame.
    pub fn set_lighting(&mut self, lighting: SceneLighting) {
        if lighting != self.lighting {
            self.lighting = lighting;
            self.resolved = lighting.resolve();
        }
    }

    /// The meshes.
    pub fn meshes(&self) -> &MeshPool {
        &self.meshes
    }

    /// How the sun's shadows are drawn (the resolution as clamped to the device).
    pub fn shadow_settings(&self) -> &ShadowSettings {
        self.shadows.settings()
    }

    /// What the last frame did for the sun's shadows.
    pub fn shadow_stats(&self) -> ShadowStats {
        self.shadows.stats()
    }

    /// Changes how the sun's shadows are drawn (a settings change: it may make the maps again; every map is drawn
    /// again next frame). `min_layers` is 2 on OpenGL, where a one-layer texture cannot be sampled as an array.
    pub(crate) fn set_shadows(&mut self, settings: ShadowSettings, min_layers: u32) {
        if self.shadows.configure(settings, min_layers) {
            self.rebuild_frame_group();
        }
    }

    /// How local lights are drawn (the atlas resolution as clamped to the device is applied when drawn).
    pub fn light_settings(&self) -> &LightSettings {
        self.lights.settings()
    }

    /// What the last frame did for local lights.
    pub fn light_stats(&self) -> LightStats {
        self.lights.stats()
    }

    /// Changes how local lights are drawn (a settings change: a new atlas size makes the atlas again, and every
    /// tile is drawn again).
    pub(crate) fn set_lights(&mut self, settings: LightSettings) {
        self.lights.configure(settings);
        self.refresh_light_group();
    }

    /// Advances the local lights' fades by `step` seconds a frame instead of the wall time between frames.
    pub(crate) fn set_light_fade_step(&mut self, step: Option<f32>) {
        self.lights.set_fixed_step(step);
    }

    /// Makes both frame groups again (the shadow maps were replaced).
    fn rebuild_frame_group(&mut self) {
        self.frame_group = None;
        self.refresh_light_group();
        self.sun_group = frame_group_without_lights(
            &self.device,
            &self.sun_layout,
            &self.frame_buffer,
            &self.materials,
            &self.shadows,
        );
    }

    /// Selects, clusters and shadows the local lights for a frame of `scene` seen from `camera` (see
    /// [`crate::lights`]). Call it after [`GpuScene::upload_view`] and before [`PartRenderer::prepare_shadows`], which
    /// takes the scene's shadow-caster changes the atlas also reads.
    pub(crate) fn prepare_lights(
        &mut self,
        scene: &mut GpuScene,
        camera: Option<&Camera>,
        width: u32,
        height: u32,
        cull: &CullSettings,
    ) {
        self.lights.prepare(scene, camera, width, height, cull);
        self.refresh_light_group();
    }

    /// Makes the frame group with the local lights' resources when they first exist, or again when one of them was
    /// replaced (the atlas made at its first shadow or dropped for a new size). A scene without lights never has
    /// one.
    fn refresh_light_group(&mut self) {
        let revision = self.lights.revision();
        if self.frame_group.is_some() && revision == self.frame_group_lights {
            return;
        }
        self.frame_group_lights = revision;
        self.frame_group = self.lights.bindings().map(|lights| {
            frame_group(
                &self.device,
                &self.frame_layout,
                &self.frame_buffer,
                &self.materials,
                &self.shadows,
                Some(lights),
            )
        });
    }

    /// The shadow atlas's render target when tiles are drawn this frame.
    pub(crate) fn light_shadow_target(&self) -> Option<&wgpu::TextureView> {
        self.lights.atlas_target()
    }

    /// Records the atlas's tiles into `render_pass`, a depth pass into [`PartRenderer::light_shadow_target`] that
    /// loads it.
    pub(crate) fn draw_light_shadows(&mut self, render_pass: &mut wgpu::RenderPass<'_>) {
        let (draws, instances, triangles) = self.lights.draw_atlas(&self.meshes, render_pass);
        self.stats.light_shadow_draws += draws;
        self.stats.light_shadow_instances += instances;
        self.stats.light_shadow_triangles += triangles;
    }

    /// Fits and culls the shadow cascades for a frame of `scene` seen from `camera` on a `width` × `height` target,
    /// and records the upload of the casters' lists into `encoder`. Call it after [`GpuScene::upload_view`] and
    /// before [`PartRenderer::prepare`], every frame (see [`crate::shadow`]).
    pub(crate) fn prepare_shadows(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        scene: &mut GpuScene,
        camera: Option<&Camera>,
        width: u32,
        height: u32,
    ) {
        let lit = self.lighting.global_shadows && self.resolved.light_color.max_element() > 0.0;
        let aspect = width as f32 / height.max(1) as f32;
        self.shadows
            .prepare(encoder, scene, camera, aspect, self.resolved.light_direction, lit);
    }

    /// Where shadow cascade `cascade`'s casters are drawn when they are this frame: its map, or a cached cascade's
    /// static map.
    pub(crate) fn shadow_target(&self, cascade: usize) -> Option<&wgpu::TextureView> {
        self.shadows.caster_target(cascade)
    }

    /// Records shadow cascade `cascade`'s casters into `render_pass`, a depth pass into its
    /// [`PartRenderer::shadow_target`].
    pub(crate) fn draw_shadow(&mut self, cascade: usize, render_pass: &mut wgpu::RenderPass<'_>) {
        let drawn = self.shadows.draw(cascade, &self.meshes, render_pass);
        self.count_shadow_draws(drawn);
    }

    /// Cached shadow cascade `cascade`'s map when it is built this frame (after its static map is drawn, if it is).
    pub(crate) fn shadow_build_target(&self, cascade: usize) -> Option<&wgpu::TextureView> {
        self.shadows.build_target(cascade)
    }

    /// Records the building of cached cascade `cascade`'s map into `render_pass`, a depth pass into its
    /// [`PartRenderer::shadow_build_target`]: the copy of its static map and its moving casters.
    pub(crate) fn build_shadow(&mut self, cascade: usize, render_pass: &mut wgpu::RenderPass<'_>) {
        let drawn = self.shadows.build(cascade, &self.meshes, render_pass);
        self.count_shadow_draws(drawn);
    }

    fn count_shadow_draws(&mut self, (draws, instances, triangles): (u32, u32, u64)) {
        self.stats.shadow_draws += draws;
        self.stats.shadow_instances += instances;
        self.stats.shadow_triangles += triangles;
    }

    /// What the last frame drew.
    pub fn stats(&self) -> DrawStats {
        self.stats
    }

    /// Prepares a frame of `scene` seen from `camera` on a `width` × `height` target: the uniforms, the material
    /// table, the sorted transparent list and the bind groups. Call it after [`GpuScene::upload_view`] (which may
    /// replace buffers, and decides what is drawn) and before the passes. `None` draws nothing.
    pub(crate) fn prepare(&mut self, scene: &GpuScene, camera: Option<&Camera>, width: u32, height: u32, srgb: bool) {
        self.stats = DrawStats::default();
        self.draws.clear();
        self.visible = false;
        let Some(camera) = camera else {
            return;
        };
        if scene.binding() != self.binding {
            // A scene made for another device; never happens with one context.
            return;
        }
        self.visible = true;
        let instances = scene.scene();
        let offset = camera.position - instances.origin();

        self.materials.fill_from_names(instances.material_names());
        self.materials.upload(&self.queue);
        let aspect = width as f32 / height.max(1) as f32;
        let uniforms = FrameUniforms::new(
            camera,
            aspect,
            offset,
            &self.resolved,
            self.shadows.frame(),
            self.lights.frame(),
            srgb,
        );
        self.stats.lights = self.lights.stats();
        let shadow = self.shadows.stats();
        self.stats.shadow_cascades = shadow.cascades;
        self.stats.shadow_redrawn = shadow.redrawn_mask();
        self.stats.shadow_built = shadow.built_mask();
        self.stats.shadow_casters = shadow.cascade.map(|c| c.casters);
        self.stats.shadow_moving = shadow.cascade.map(|c| c.moving);
        if self.frame_uniforms != Some(uniforms) {
            self.queue
                .write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&uniforms));
            self.frame_uniforms = Some(uniforms);
        }

        for (kind, segment) in scene.list_segments().iter().enumerate() {
            let mesh = self.meshes.range(MeshKind(u16::try_from(kind).unwrap_or(u16::MAX)));
            if segment.len == 0 || mesh.index_count == 0 {
                continue;
            }
            self.draws.push(KindDraw {
                mesh,
                first: segment.first,
                count: segment.len,
            });
            self.stats.opaque_instances += segment.len;
            self.stats.opaque_triangles += u64::from(segment.len) * u64::from(mesh.index_count / 3);
        }
        self.transparent.update(
            &self.device,
            &self.queue,
            instances,
            scene.transparent_entries(),
            offset,
        );
        self.stats.culled_instances = scene.cull_stats().culled();
        for run in self.transparent.runs() {
            let mesh = self.meshes.range(run.kind);
            self.stats.transparent_instances += run.count;
            self.stats.transparent_triangles += u64::from(run.count) * u64::from(mesh.index_count / 3);
        }
        self.stats.instances = self.stats.opaque_instances + self.stats.transparent_instances;
        self.stats.triangles = self.stats.opaque_triangles + self.stats.transparent_triangles;

        if let Some(layout) = self.instance_layout.as_ref() {
            let statics = scene.buffer(crate::scene::Region::Static);
            let dynamics = scene.buffer(crate::scene::Region::Dynamic);
            refresh_group(
                &self.device,
                layout,
                &mut self.opaque_group,
                [statics, dynamics, scene.list_buffer()],
            );
            refresh_group(
                &self.device,
                layout,
                &mut self.transparent_group,
                [statics, dynamics, self.transparent.buffer()],
            );
        }
    }

    /// Records the draws of `pass`: the parts in the prepass, opaque and transparent passes, and the sky in the sky
    /// pass. `prepass` says whether the frame has a depth prepass, so the opaque pass only tests depth.
    pub(crate) fn draw(
        &mut self,
        pass: PassId,
        prepass: bool,
        scene: &GpuScene,
        render_pass: &mut wgpu::RenderPass<'_>,
    ) {
        if !self.visible {
            return;
        }
        match pass {
            PassId::DepthPrepass | PassId::Opaque => {
                if self.draws.is_empty() {
                    return;
                }
                let variant = self.light_variant();
                let [opaque, opaque_tested, _] = self.pipelines.lit(variant);
                // The prepass writes depth only, with the pipeline made without local lights.
                let (pipeline, variant) = match (pass, prepass) {
                    (PassId::DepthPrepass, _) => (&self.pipelines.prepass, Lights::None),
                    (_, true) => (opaque_tested, variant),
                    (_, false) => (opaque, variant),
                };
                render_pass.set_pipeline(pipeline);
                self.bind(render_pass, variant, self.opaque_group.as_ref(), scene.list_buffer());
                for draw in &self.draws {
                    let indices = draw.mesh.first_index..draw.mesh.first_index + draw.mesh.index_count;
                    render_pass.draw_indexed(indices, 0, draw.first..draw.first + draw.count);
                }
                if pass == PassId::DepthPrepass {
                    self.stats.prepass_draws += self.draws.len() as u32;
                } else {
                    self.stats.opaque_draws += self.draws.len() as u32;
                }
            }
            PassId::Transparent => {
                if self.transparent.runs().is_empty() {
                    return;
                }
                let variant = self.light_variant();
                render_pass.set_pipeline(self.pipelines.lit(variant)[2]);
                self.bind(
                    render_pass,
                    variant,
                    self.transparent_group.as_ref(),
                    self.transparent.buffer(),
                );
                for run in self.transparent.runs() {
                    let mesh = self.meshes.range(run.kind);
                    if mesh.index_count == 0 {
                        continue;
                    }
                    let indices = mesh.first_index..mesh.first_index + mesh.index_count;
                    render_pass.draw_indexed(indices, 0, run.first..run.first + run.count);
                    self.stats.transparent_draws += 1;
                }
            }
            PassId::Sky => {
                render_pass.set_pipeline(&self.pipelines.sky);
                render_pass.set_bind_group(0, &self.sun_group, &[]);
                render_pass.draw(0..3, 0..1);
                self.stats.sky_draws += 1;
            }
            PassId::Post | PassId::Ui => {}
        }
    }

    /// Which variant of the lit pipelines this frame draws with: none when no local light is drawn, without shadows
    /// when no drawn light has one.
    fn light_variant(&self) -> Lights {
        let frame = self.lights.frame();
        if frame.local[0] <= 0.0 || self.frame_group.is_none() {
            Lights::None
        } else if frame.atlas[2] > 0.0 {
            Lights::Shadowed
        } else {
            Lights::Unshadowed
        }
    }

    /// Binds the frame group of the pipeline `variant` (without the lights' entries for frames without local lights),
    /// the meshes and the instances: the storage path's instance group, or `copies` as the per-instance vertex
    /// buffer.
    fn bind(
        &self,
        render_pass: &mut wgpu::RenderPass<'_>,
        variant: Lights,
        group: Option<&InstanceGroup>,
        copies: &wgpu::Buffer,
    ) {
        let frame = match variant {
            Lights::None => &self.sun_group,
            Lights::Unshadowed | Lights::Shadowed => self.frame_group.as_ref().unwrap_or(&self.sun_group),
        };
        render_pass.set_bind_group(0, frame, &[]);
        render_pass.set_vertex_buffer(0, self.meshes.vertices().slice(..));
        render_pass.set_index_buffer(self.meshes.indices().slice(..), wgpu::IndexFormat::Uint32);
        match self.binding {
            InstanceBinding::Storage => {
                if let Some(group) = group {
                    render_pass.set_bind_group(1, &group.group, &[]);
                }
            }
            InstanceBinding::Vertex => render_pass.set_vertex_buffer(1, copies.slice(..)),
        }
    }
}

/// The frame group without the local lights' entries: the sky's, and that of frames without local lights.
fn frame_group_without_lights(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    frame: &wgpu::Buffer,
    materials: &MaterialTable,
    shadows: &ShadowMaps,
) -> wgpu::BindGroup {
    frame_group(device, layout, frame, materials, shadows, None)
}

/// The frame group: the frame uniform, the material table, the shadow maps with their sampler, and with `lights`
/// the local lights with their clusters and shadow atlas.
fn frame_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    frame: &wgpu::Buffer,
    materials: &MaterialTable,
    shadows: &ShadowMaps,
    lights: Option<LightBindings<'_>>,
) -> wgpu::BindGroup {
    let sun = [
        wgpu::BindGroupEntry {
            binding: 0,
            resource: frame.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: materials.buffer().as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 2,
            resource: wgpu::BindingResource::TextureView(shadows.sample_view()),
        },
        wgpu::BindGroupEntry {
            binding: 3,
            resource: wgpu::BindingResource::Sampler(shadows.sampler()),
        },
    ];
    let Some(lights) = lights else {
        return device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("metallic parts frame (no local lights)"),
            layout,
            entries: &sun,
        });
    };
    let [e0, e1, e2, e3] = sun;
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("metallic parts frame"),
        layout,
        entries: &[
            e0,
            e1,
            e2,
            e3,
            wgpu::BindGroupEntry {
                binding: 4,
                resource: lights.lights.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(lights.clusters),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: lights.tiles.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(lights.atlas),
            },
        ],
    })
}

/// Makes `slot`'s bind group again when any of `buffers` is not the one it was made with.
fn refresh_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    slot: &mut Option<InstanceGroup>,
    buffers: [&wgpu::Buffer; 3],
) {
    if slot
        .as_ref()
        .is_some_and(|g| g.buffers.iter().zip(buffers).all(|(a, b)| a == b))
    {
        return;
    }
    fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding,
            resource: buffer.as_entire_binding(),
        }
    }
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("metallic parts instances"),
        layout,
        entries: &[entry(0, buffers[0]), entry(1, buffers[1]), entry(2, buffers[2])],
    });
    *slot = Some(InstanceGroup {
        buffers: buffers.map(Clone::clone),
        group,
    });
}

fn create_pipelines(
    device: &wgpu::Device,
    shaders: &Shaders,
    binding: InstanceBinding,
    format: wgpu::TextureFormat,
    samples: u32,
) -> Pipelines {
    let (layout, module) = (&shaders.parts_layout, &shaders.parts);
    let instance_attributes = GpuInstance::vertex_attributes(2);
    let all_buffers = [
        Some(MeshVertex::layout()),
        Some(wgpu::VertexBufferLayout {
            array_stride: GpuInstance::SIZE,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &instance_attributes,
        }),
    ];
    // The storage path reads instances from storage buffers, not from a second vertex buffer.
    let buffers = match binding {
        InstanceBinding::Storage => &all_buffers[..1],
        InstanceBinding::Vertex => &all_buffers[..],
    };
    let variant = |label: &str,
                   depth: wgpu::DepthStencilState,
                   fragment: Option<(&str, Option<wgpu::BlendState>)>,
                   lights: Lights| {
        let constants = lights.constants();
        // Frames without local lights draw with the module and layout that have none of their bindings.
        let (layout, module) = match lights {
            Lights::None => (&shaders.unlit_layout, &shaders.unlit),
            Lights::Unshadowed | Lights::Shadowed => (layout, module),
        };
        let targets = [Some(wgpu::ColorTargetState {
            format,
            blend: fragment.and_then(|(_, blend)| blend),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: Some(depth),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..wgpu::MultisampleState::default()
            },
            fragment: fragment.map(|(entry_point, _)| wgpu::FragmentState {
                module,
                entry_point: Some(entry_point),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants,
                    ..wgpu::PipelineCompilationOptions::default()
                },
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let transparent_blend = Some(("fs_transparent", Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)));
    let tested = wgpu::DepthStencilState {
        depth_write_enabled: Some(false),
        depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
        ..depth_stencil_state(false)
    };
    debug_assert_eq!(depth_stencil_state(true).depth_compare, Some(DEPTH_COMPARE));
    debug_assert_eq!(depth_stencil_state(true).format, DEPTH_FORMAT);
    // Opaque, opaque after a prepass, and transparent, for frames with `lights`.
    let lit = |lights: Lights, name: &str| {
        [
            variant(
                &format!("metallic parts opaque ({name})"),
                depth_stencil_state(true),
                Some(("fs_opaque", None)),
                lights,
            ),
            variant(
                &format!("metallic parts opaque after prepass ({name})"),
                tested.clone(),
                Some(("fs_opaque", None)),
                lights,
            ),
            variant(
                &format!("metallic parts transparent ({name})"),
                depth_stencil_state(false),
                transparent_blend,
                lights,
            ),
        ]
    };
    let [opaque, opaque_tested, transparent] = lit(Lights::Shadowed, "local lights and shadows");
    Pipelines {
        format,
        samples,
        prepass: variant(
            "metallic parts depth prepass",
            depth_stencil_state(true),
            None,
            Lights::None,
        ),
        opaque,
        opaque_tested,
        transparent,
        sky: sky_pipeline(device, shaders, format, samples),
        unlit: lit(Lights::None, "no local lights"),
        unshadowed: lit(Lights::Unshadowed, "local lights"),
    }
}

/// The sky: a full-screen triangle at the far plane, tested against the depth the opaque pass left (`GreaterEqual`
/// passes only where it is still the clear value) and never writing it.
fn sky_pipeline(
    device: &wgpu::Device,
    shaders: &Shaders,
    format: wgpu::TextureFormat,
    samples: u32,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("metallic sky"),
        layout: Some(&shaders.sky_layout),
        vertex: wgpu::VertexState {
            module: &shaders.sky,
            entry_point: Some("vs_sky"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            depth_write_enabled: Some(false),
            depth_compare: Some(DEPTH_COMPARE_FAR_PLANE),
            ..depth_stencil_state(false)
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..wgpu::MultisampleState::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: &shaders.sky,
            entry_point: Some("fs_sky"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
