//! Drawing parts: the primitive meshes, the material table, the camera, and the draws (ADR 0008 §4).
//!
//! # A constant number of draws
//!
//! Parts are drawn with **one instanced draw per mesh kind and level of detail per shading model**, so the draw
//! count depends on how many kinds, levels and models there are, never on how many parts. There is one shading model
//! for now (PBR-lite, below), so the opaque pass makes at most one draw per mesh kind and level ([`crate::lod`]):
//! nine for the built-in primitives (the Block and the Wedge have one level, the Ball four and the Cylinder three).
//! The material of each instance comes from the material table ([`MaterialTable`]) through the instance's material
//! row, so materials cost no draws either.
//!
//! **Mesh assets** (#189, `MeshPart`) join the same scheme: each pair of a mesh asset and a base-colour texture is a
//! mesh kind of the scene's ([`crate::InstanceScene::mesh_assets`]), its mesh in the shared pool
//! ([`crate::Renderer::add_mesh_asset`]) at one level of detail, so it is one more instanced draw, culled, sorted and drawn into
//! every shadow like the primitives. Each draw binds a base-colour texture: the asset's (an sRGB texture sampled with
//! the mesh's texture coordinates, multiplied into the colour), or a white one for the primitives, which therefore
//! look as before. See-through mesh parts draw untextured, in the sorted list with everything else.
//!
//! Which slots a draw covers comes from the scene's **draw lists** ([`crate::scene`]): one list of entries per
//! mesh kind, kept current from the dirty slots alone, laid out in one buffer with a segment per kind
//! ([`crate::GpuScene::list_segments`]). A kind's draws are instances `first .. first + len` of its segment, one
//! draw per level of detail over that level's part of it ([`crate::scene::ListSegment::level_range`]):
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
//! (by distance from the camera to the instance's centre) whenever the camera moves or one of them changes, so the
//! order is exact across mesh kinds and levels. On the storage path they are drawn in **one draw** whatever their
//! number and however their shapes interleave: the vertex shader pulls each instance's mesh from an unindexed copy of
//! the pool (`vs_pulled`; `parts/transparent.rs` explains it, #106). On the vertex-buffer fallback they are drawn as one
//! draw per run of consecutive instances with the same mesh and level.
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
//! Every pipeline (shadow casters, depth prepass, opaque, opaque after a prepass, sky, transparent) is compiled off
//! the frame (ADR 0008 §9, [`crate::pipelines`]): the shadow casters with the renderer, the rest on compile workers,
//! for every colour format and MSAA count the renderer can switch to. A scene's first frame waits for the lit ones
//! when it has local lights; a light made later, while they still compile, is drawn once they are (a
//! local light fades in).

mod assets;
mod camera;
mod material;
mod mesh;
mod shading;
mod transparent;

pub use camera::{Camera, CameraReader};
pub use material::{MAX_MATERIAL_ROWS, MaterialParams, MaterialTable};
pub use mesh::{MeshData, MeshPool, MeshRange, MeshVertex};
pub use shading::ShadingView;

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use metallic_core::glam::Vec3;

use crate::context::DescriptorHeapTurn;
use crate::context::Submitter;
use crate::cull::CullSettings;
use crate::depth::{DEPTH_COMPARE, DEPTH_COMPARE_FAR_PLANE, DEPTH_FORMAT, depth_stencil_state};
use crate::lighting::{ResolvedLighting, SceneLighting};
use crate::lights::{
    FrameLights, LightBindings, LightSettings, LightStats, LocalLights, MAX_GPU_LIGHTS, MAX_SHADOW_TILES,
};
use crate::lod::{LodView, MAX_LOD_LEVELS};
use crate::pipelines::{Compiler, PartPipeline, PipelineKey, Recipe};
use crate::scene::{GpuInstance, InstanceBinding, MeshKind};
use crate::shadow::{FrameShadow, MAX_CASCADES, ShadowMaps, ShadowSettings, ShadowStats};
use crate::textures::TextureMaker;
use crate::{GpuScene, PassId};
use assets::{AssetTextures, WHITE, texture_group};
use material::MATERIAL_TABLE_SIZE;
use transparent::{TransparentList, TransparentPath};

/// What the last frame drew.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrawStats {
    /// Draws in the depth prepass (one per mesh kind and level with instances; 0 without a prepass).
    pub prepass_draws: u32,
    /// Draws in the opaque pass: one per mesh kind and level of detail with instances, whatever the part count.
    pub opaque_draws: u32,
    /// Draws in the transparent pass: one on the storage path (pulled vertices); on the vertex-buffer fallback one per
    /// run of same-mesh, same-level instances in back-to-front order.
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
    /// Instances of the opaque and transparent passes drawn at each level of detail ([`crate::lod`]; index 0: full
    /// detail, which includes every Block and Wedge).
    pub level_instances: [u32; MAX_LOD_LEVELS],
    /// Sun shadow cascades in use ([`crate::shadow`]; 0 without shadows).
    pub shadow_cascades: u32,
    /// Bit `i` set when cascade `i`'s casters were drawn this frame (a cached cascade's: its static map), otherwise
    /// they were kept.
    pub shadow_redrawn: u8,
    /// Bit `i` set when cached cascade `i`'s map was built this frame: its static map copied in and its moving
    /// casters drawn on top ([`crate::shadow`]).
    pub shadow_built: u8,
    /// Bit `i` set when cached cascade `i`'s map was built this frame by restoring the static map only under the
    /// footprints of its previous moving casters, not by copying all of it ([`crate::shadow::CascadeStats::restored`]).
    pub shadow_restored: u8,
    /// The texels each cached cascade's restore drew this frame (0 for a full copy or no build).
    pub shadow_restored_texels: [u32; MAX_CASCADES],
    /// Draws in the shadow passes this frame: one per mesh kind in each cascade drawn and each cached cascade built
    /// (the copies and restores of the static maps are not counted).
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
    /// Per cascade: 1 when its see-through maps hold see-through casters (#491).
    shadow_see_through: [f32; 4],
    /// x: the shading view ([`ShadingView`], #478) the fragment shader draws.
    view: [f32; 4],
}

/// The frame group's entries a frame without local lights binds: the frame uniform, the materials, the sun's shadow
/// maps and their sampler.
const SUN_ENTRIES: usize = 7;

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

/// How the parts are written (`output` in frame.wgsl): whether the shader encodes sRGB itself, and for the debug view
/// of [`crate::Renderer::set_lod_tint`] where levels 1, 2 and 3 start in the vertex pool (exact in f32 below 2²⁴
/// vertices; 0 when the view is off).
#[derive(Clone, Copy, Debug)]
struct Output([f32; 4]);

impl Output {
    /// For a target that is `srgb` or not, with `tint` the first vertex of each level in the mesh pool.
    fn new(srgb: bool, tint: Option<[u32; MAX_LOD_LEVELS]>) -> Self {
        let [_, l1, l2, l3] = tint.map_or([0.0; MAX_LOD_LEVELS], |starts| starts.map(|s| s as f32));
        Self([if srgb { 0.0 } else { 1.0 }, l1, l2, l3])
    }
}

impl FrameUniforms {
    /// The uniform for `camera` on a target `aspect` (width / height) wide, `offset` (camera − scene origin) from
    /// the scene origin, lit by `light` with `shadow`'s cascades and the local lights `local`, written as `output`
    /// says.
    fn new(
        camera: &Camera,
        aspect: f32,
        offset: Vec3,
        light: &ResolvedLighting,
        shadow: &FrameShadow,
        local: &FrameLights,
        output: Output,
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
            output: output.0,
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
            shadow_see_through: shadow.see_through,
            view: [0.0; 4],
        }
    }
}

/// One opaque draw: a mesh kind's index range at one level over that level's part of its list segment, with its
/// base-colour texture (an index into the asset textures' groups; white for every kind but a mesh asset's).
#[derive(Clone, Copy, Debug, Default)]
struct KindDraw {
    mesh: MeshRange,
    first: u32,
    count: u32,
    texture: u32,
}

/// The render pipelines for one colour format and sample count, as their compiles finish ([`crate::pipelines`]),
/// indexed by [`PartPipeline::index`]: the depth prepass, the sky, and opaque, opaque after a prepass (testing depth
/// with `GreaterEqual` without writing it) and transparent for each [`Variant`].
///
/// The variant without local lights (lights_off.wgsl, the frame group without the lights' entries) is for frames that
/// draw none: the light loop would cost every pixel even when it runs no iteration, and such a frame binds exactly
/// what it did before local lights existed. The one with local lights but without their shadows (`LOCAL_SHADOWS`
/// off) is for frames whose drawn lights have none.
#[derive(Debug)]
struct Target {
    format: wgpu::TextureFormat,
    samples: u32,
    pipelines: [Option<wgpu::RenderPipeline>; PartPipeline::COUNT],
}

impl Target {
    fn new(format: wgpu::TextureFormat, samples: u32) -> Self {
        Self {
            format,
            samples,
            pipelines: Default::default(),
        }
    }

    fn get(&self, pipeline: PartPipeline) -> Option<&wgpu::RenderPipeline> {
        self.pipelines[pipeline.index()].as_ref()
    }

    /// Whether `variant`'s opaque, opaque-after-prepass and transparent pipelines are all compiled.
    fn has_variant(&self, variant: Variant) -> bool {
        let l = variant.number();
        [
            PartPipeline::Opaque(l),
            PartPipeline::Tested(l),
            PartPipeline::Transparent(l),
        ]
        .iter()
        .all(|&p| self.get(p).is_some())
    }
}

/// Which variant of the part pipelines a frame uses: made without local lights (lights_off.wgsl), or with them and
/// `LOCAL_SHADOWS` (lights.wgsl) off or on. The value is the variant's number in [`PartPipeline`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lights {
    /// No local light is drawn.
    None = 0,
    /// Local lights without shadows.
    Unshadowed = 1,
    /// Local lights, some with shadows.
    Shadowed = 2,
}

/// Which variant of the part pipelines a frame uses: its local lights ([`Lights`]). Its number in [`PartPipeline`] is
/// the lights'.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Variant {
    lights: Lights,
}

/// `LOCAL_SHADOWS` (lights.wgsl) off and on.
const UNSHADOWED: [(&str, f64); 1] = [("LOCAL_SHADOWS", 0.0)];
const SHADOWED: [(&str, f64); 1] = [("LOCAL_SHADOWS", 1.0)];

impl Variant {
    /// What a frame without local lights draws with: what it did before they existed.
    const BASE: Self = Self { lights: Lights::None };

    /// The variant's number in [`PartPipeline`].
    fn number(self) -> u8 {
        self.lights as u8
    }

    /// The variant numbered `n` in [`PartPipeline`].
    fn from_number(n: u8) -> Self {
        Self {
            lights: match n {
                0 => Lights::None,
                1 => Lights::Unshadowed,
                _ => Lights::Shadowed,
            },
        }
    }

    /// The variant's pipeline constants: none for the module without local lights (lights_off.wgsl).
    fn constants(self) -> &'static [(&'static str, f64)] {
        match self.lights {
            Lights::None => &[],
            Lights::Unshadowed => &UNSHADOWED,
            Lights::Shadowed => &SHADOWED,
        }
    }

    /// The variant's name in pipeline labels.
    fn name(self) -> &'static str {
        match self.lights {
            Lights::None => "no local lights",
            Lights::Unshadowed => "local lights",
            Lights::Shadowed => "local lights and shadows",
        }
    }
}

/// What the pipelines are made from, shared with every compile.
#[derive(Debug)]
struct Shaders {
    /// The instance path the pipelines are for.
    binding: InstanceBinding,
    /// The parts' layout: the frame group, and on the storage path the instance group.
    parts_layout: wgpu::PipelineLayout,
    parts: wgpu::ShaderModule,
    /// The same without local lights (lights_off.wgsl): the frame group without the lights' entries.
    unlit_layout: wgpu::PipelineLayout,
    unlit: wgpu::ShaderModule,
    /// The sky's layout: the frame group without the lights' entries.
    sky_layout: wgpu::PipelineLayout,
    sky: wgpu::ShaderModule,
    /// The transparent pipelines' layouts with and without local lights where they pull their vertices
    /// ([`TransparentPath::Pulled`]): the frame group and the pulled instance group.
    pulled_layouts: Option<[wgpu::PipelineLayout; 2]>,
}

/// The storage path's instance bind group (`N` buffers) and the buffers it was made for.
#[derive(Debug)]
struct InstanceGroup<const N: usize> {
    buffers: [wgpu::Buffer; N],
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
    /// The mesh assets' base-colour textures, and the white one every other draw binds.
    textures: AssetTextures,
    /// The texture of each mesh kind (an index into `textures`' groups), as of the last [`PartRenderer::resolve_assets`].
    kind_textures: Vec<u32>,
    /// The scene's mesh-asset kinds and the asset revisions the kinds were last pointed at the assets for.
    assets_for: Option<(u64, u64, u64)>,
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
    /// The instance group of the transparent pass where it pulls its vertices: the regions, the sorted records and
    /// the meshes' unindexed copy.
    pulled_layout: Option<wgpu::BindGroupLayout>,
    /// The sun's shadow maps, which the frame group samples.
    shadows: ShadowMaps,
    /// The point and spot lights, which the frame group reads.
    lights: LocalLights,
    shaders: Arc<Shaders>,
    /// The pipelines of every colour format and sample count compiled or compiling, never removed (a switch back
    /// compiles nothing).
    targets: Vec<Target>,
    /// The target drawn into: an index into `targets`.
    current: usize,
    opaque_group: Option<InstanceGroup<3>>,
    /// The transparent pass's instance group on the runs path ([`TransparentPath::RunEntries`]).
    transparent_group: Option<InstanceGroup<3>>,
    /// The transparent pass's instance group where it pulls its vertices ([`TransparentPath::Pulled`]).
    pulled_group: Option<InstanceGroup<4>>,
    transparent: TransparentList,
    draws: Vec<KindDraw>,
    /// Whether this frame has a camera (and so draws anything).
    visible: bool,
    /// Colour parts by level of detail (a debug view).
    lod_tint: bool,
    /// How the parts are drawn (#478).
    shading: ShadingView,
    /// A frame had local lights to draw while their pipelines were compiling: they fade in once they exist.
    lights_held: bool,
    stats: DrawStats,
}

impl PartRenderer {
    /// Creates the meshes, the material table, the shadows and lights, and the shaders, on the instance path
    /// `binding` (which must be the one the scenes it draws use: [`InstanceBinding::for_context`]). The pipelines
    /// a frame without local lights draws with into `format` at `samples` are queued on `compiler` first, so they
    /// compile while the rest is made; the renderer installs them ([`PartRenderer::install`]).
    pub(crate) fn new(
        (device, queue, maker): (&wgpu::Device, &wgpu::Queue, &TextureMaker),
        binding: InstanceBinding,
        (format, samples): (wgpu::TextureFormat, u32),
        compiler: &mut Compiler,
        cache: Option<&wgpu::PipelineCache>,
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
            // The cascades' see-through maps (#491): the nearest see-through casters' depth, the light they let
            // through, and the sampler that filters it.
            wgpu::BindGroupLayoutEntry {
                binding: 8,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 9,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 10,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
            // The atlas's see-through maps (#491), filtered with the cascades' sampler.
            wgpu::BindGroupLayoutEntry {
                binding: 11,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 12,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
        ];
        // The pipelines of frames with local lights bind all of it; the sky and the pipelines of frames without
        // (lights_off.wgsl) bind the sun's entries alone (the first seven), so such a frame binds nothing of the lights'.
        let layouts_turn = DescriptorHeapTurn::take();
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
        // The transparent pass pulls its vertices where a vertex shader reads the two regions, the records and the
        // meshes (and every backend with storage buffers in vertex shaders has four here); otherwise it draws runs.
        let transparent_path = match binding {
            InstanceBinding::Storage if device.limits().max_storage_buffers_per_shader_stage >= 4 => {
                TransparentPath::Pulled
            }
            InstanceBinding::Storage => TransparentPath::RunEntries,
            InstanceBinding::Vertex => TransparentPath::RunCopies,
        };
        let pulled_layout = (transparent_path == TransparentPath::Pulled).then(|| {
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
                label: Some("metallic parts instances (pulled)"),
                entries: &[
                    storage(0, GpuInstance::SIZE),
                    storage(1, GpuInstance::SIZE),
                    // The record and chunk counts, a chunk and a record.
                    storage(
                        3,
                        (3 * size_of::<u32>() + size_of::<transparent::PulledRecord>()) as u64,
                    ),
                    // One unindexed vertex.
                    storage(4, 32),
                ],
            })
        });
        drop(layouts_turn);
        // The base-colour texture's group follows the instance group, or the frame group where there is none.
        let textures = AssetTextures::new(device, queue);
        debug_assert_eq!(texture_group(binding), if instance_layout.is_some() { 2 } else { 1 });
        let groups = |frame| -> Vec<Option<&wgpu::BindGroupLayout>> {
            let mut groups = vec![Some(frame)];
            if let Some(instances) = instance_layout.as_ref() {
                groups.push(Some(instances));
            }
            groups.push(Some(textures.layout()));
            groups
        };
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("metallic parts"),
            bind_group_layouts: &groups(&frame_layout),
            immediate_size: 0,
        });
        let unlit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("metallic parts (no local lights)"),
            bind_group_layouts: &groups(&sun_layout),
            immediate_size: 0,
        });
        let pulled_layouts = pulled_layout.as_ref().map(|pulled| {
            [
                (&frame_layout, "metallic parts transparent"),
                (&sun_layout, "metallic parts transparent (no local lights)"),
            ]
            .map(|(frame, label)| {
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: &[Some(frame), Some(pulled), Some(textures.layout())],
                    immediate_size: 0,
                })
            })
        });
        let sky_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("metallic sky"),
            bind_group_layouts: &[Some(&sun_layout)],
            immediate_size: 0,
        });
        let shaders = Arc::new(Shaders {
            binding,
            parts_layout: pipeline_layout,
            parts: module,
            unlit_layout,
            unlit: unlit_module,
            sky_layout,
            sky,
            pulled_layouts,
        });
        // Queued before the shadows' and lights' pipelines are made here, so the workers start at once.
        let key = |pipeline| PipelineKey::Part {
            format,
            samples,
            pipeline,
        };
        for pipeline in PartPipeline::BASE {
            let recipe = part_recipe(&shaders, format, samples, pipeline);
            compiler.submit(key(pipeline), recipe, pipeline.shares_stages_of().map(key));
        }
        let first = Target::new(format, samples);
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic parts frame"),
            size: size_of::<FrameUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let materials = MaterialTable::new(device);
        debug_assert_eq!(materials.buffer().size(), MATERIAL_TABLE_SIZE);
        // Their pipelines are made here, in the device's turn with the workers' compiles (Direct3D 12 with FXC).
        let turn = compiler.turn().take();
        // One pair of placeholders for the cascades' and the atlas's see-through maps (#491).
        let see_placeholder = crate::shadow::SeeThroughPlaceholder::new(device);
        let shadows = ShadowMaps::new(
            (device, queue, maker),
            binding,
            instance_layout.as_ref(),
            cache,
            &see_placeholder,
        );
        let lights = LocalLights::new(
            (device, queue, maker),
            binding,
            instance_layout.as_ref(),
            LightSettings::default(),
            cache,
            &see_placeholder,
        );
        drop(turn);
        let sun_group = frame_group_without_lights(device, &sun_layout, &frame_buffer, &materials, &shadows);
        let frame_group_lights = lights.revision();
        let lighting = SceneLighting::default();
        Self {
            device: device.clone(),
            queue: queue.clone(),
            binding,
            meshes: MeshPool::with_pulled(device, queue, transparent_path == TransparentPath::Pulled),
            textures,
            kind_textures: Vec::new(),
            assets_for: None,
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
            pulled_layout,
            shadows,
            lights,
            shaders,
            targets: vec![first],
            current: 0,
            opaque_group: None,
            transparent_group: None,
            pulled_group: None,
            transparent: TransparentList::new(device, transparent_path),
            draws: Vec::new(),
            visible: false,
            lod_tint: false,
            shading: ShadingView::Shaded,
            lights_held: false,
            stats: DrawStats::default(),
        }
    }

    /// Colours every part by the level of detail it is drawn at, from the next frame (see
    /// [`crate::Renderer::set_lod_tint`]).
    pub(crate) fn set_lod_tint(&mut self, on: bool) {
        self.lod_tint = on;
    }

    /// Whether parts are coloured by level of detail.
    pub(crate) fn lod_tint(&self) -> bool {
        self.lod_tint
    }

    /// Draws the parts as `view` says, from the next frame (see [`crate::Renderer::set_shading_view`]).
    pub(crate) fn set_shading_view(&mut self, view: ShadingView) {
        self.shading = view;
    }

    /// How the parts are drawn.
    pub(crate) fn shading_view(&self) -> ShadingView {
        self.shading
    }

    /// Whether this frame draws the wireframe: chosen, and its pipelines compiled for the current target.
    fn wireframe(&self) -> bool {
        self.shading == ShadingView::Wireframe
            && PartPipeline::WIREFRAME
                .iter()
                .all(|&pipeline| self.targets[self.current].get(pipeline).is_some())
    }

    /// The instance path the pipelines were made for.
    pub fn binding(&self) -> InstanceBinding {
        self.binding
    }

    /// Draws into `format` at `samples` from now on (a settings change, never per frame). The renderer switches only
    /// once the pipelines a frame without local lights needs are compiled ([`PartRenderer::has`]).
    pub(crate) fn set_target(&mut self, format: wgpu::TextureFormat, samples: u32) {
        self.current = self.target_index(format, samples);
    }

    /// The index of the target for `format` at `samples` in `targets`, made (empty) if there is none yet.
    fn target_index(&mut self, format: wgpu::TextureFormat, samples: u32) -> usize {
        match self
            .targets
            .iter()
            .position(|t| (t.format, t.samples) == (format, samples))
        {
            Some(index) => index,
            None => {
                self.targets.push(Target::new(format, samples));
                self.targets.len() - 1
            }
        }
    }

    /// The compile of `pipeline` for `format` at `samples`. Makes the target's slots now, so installing the result
    /// later allocates nothing.
    pub(crate) fn recipe(&mut self, format: wgpu::TextureFormat, samples: u32, pipeline: PartPipeline) -> Recipe {
        self.target_index(format, samples);
        part_recipe(&self.shaders, format, samples, pipeline)
    }

    /// Puts a compiled pipeline in its target's slot.
    pub(crate) fn install(
        &mut self,
        format: wgpu::TextureFormat,
        samples: u32,
        pipeline: PartPipeline,
        compiled: wgpu::RenderPipeline,
    ) {
        let index = self.target_index(format, samples);
        self.targets[index].pipelines[pipeline.index()] = Some(compiled);
    }

    /// Whether `pipeline` exists for `format` at `samples`.
    pub(crate) fn has(&self, format: wgpu::TextureFormat, samples: u32, pipeline: PartPipeline) -> bool {
        self.targets
            .iter()
            .find(|t| (t.format, t.samples) == (format, samples))
            .is_some_and(|t| t.get(pipeline).is_some())
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

    /// Adds the mesh asset `key` to the pool (see [`crate::Renderer::add_mesh_asset`]).
    pub(crate) fn add_mesh_asset(&mut self, submitter: &Submitter, key: &str, mesh: &MeshData) -> Result<(), String> {
        self.meshes
            .add_asset((&self.device, &self.queue, submitter), key, mesh)?;
        // The kinds that name it are pointed at it at the next frame.
        self.assets_for = None;
        Ok(())
    }

    /// Adds the texture asset `key` (see [`crate::Renderer::add_texture_asset`]).
    pub(crate) fn add_texture_asset(&mut self, maker: &TextureMaker, key: &str, size: (u32, u32), rgba: Vec<u8>) {
        self.textures.add((&self.device, &self.queue, maker), key, size, rgba);
    }

    /// Whether the mesh asset `key` was added.
    pub(crate) fn has_mesh_asset(&self, key: &str) -> bool {
        self.meshes.asset(key).is_some()
    }

    /// Whether the texture asset `key` was added (drawable or still being made).
    pub(crate) fn has_texture_asset(&self, key: &str) -> bool {
        self.textures.contains(key)
    }

    /// At the start of a frame of `scene`: writes the asset textures that arrived, and points the scene's mesh-asset
    /// kinds at the meshes and textures that are here, when the kinds or the assets changed since the last frame.
    /// When they did, every cached shadow is drawn again (a mesh that arrived casts now). Allocates nothing in a frame
    /// where neither changed, and nothing at all in a scene without mesh assets once its first frame is drawn.
    pub(crate) fn resolve_assets(&mut self, scene: &mut GpuScene, maker: &TextureMaker) {
        self.textures.poll(&self.device, &self.queue, maker);
        let key = (
            scene.scene().mesh_asset_revision(),
            self.meshes.assets_len() as u64,
            self.textures.revision(),
        );
        if self.assets_for == Some(key) {
            return;
        }
        self.assets_for = Some(key);
        self.point_kinds(scene);
    }

    /// Points every mesh-asset kind of `scene` at its mesh's range and its texture's group (see
    /// [`PartRenderer::resolve_assets`]). Out of line: it runs only when assets or kinds change.
    #[cold]
    #[inline(never)]
    fn point_kinds(&mut self, scene: &mut GpuScene) {
        let kinds = scene.scene().mesh_assets();
        self.meshes
            .forget_kinds_from(MeshKind::BUILTIN_COUNT as usize + kinds.len());
        self.kind_textures.clear();
        self.kind_textures
            .resize(usize::from(MeshKind::BUILTIN_COUNT) + kinds.len(), WHITE);
        let before = self.meshes.revision();
        for (kind, asset) in (MeshKind::BUILTIN_COUNT..).zip(kinds) {
            let range = self.meshes.asset(&asset.mesh);
            self.meshes.set_kind(MeshKind(kind), range);
            if let Some(texture) = self.kind_textures.get_mut(usize::from(kind)) {
                *texture = self.textures.index(&asset.texture);
            }
        }
        if self.meshes.revision() != before {
            // Cached cascades and atlas tiles were drawn without the meshes that just arrived.
            scene.invalidate_shadows();
        }
    }

    /// How the sun's shadows are drawn (the resolution as clamped to the device).
    pub fn shadow_settings(&self) -> &ShadowSettings {
        self.shadows.settings()
    }

    /// What the last frame did for the sun's shadows.
    pub fn shadow_stats(&self) -> ShadowStats {
        self.shadows.stats()
    }

    /// Changes how the sun's shadows are drawn (a settings change: it may make the maps again, off the frame once a
    /// scene's first frame is drawn; every map is drawn again once the change takes effect).
    pub(crate) fn set_shadows(&mut self, settings: ShadowSettings) {
        if self.shadows.configure(settings) {
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
    /// [`crate::lights`]), the casters at levels of detail no finer than `lod` (the camera view's) draws what
    /// receives their shadows. Call it after [`GpuScene::upload_view`] and before [`PartRenderer::prepare_shadows`],
    /// which takes the scene's shadow-caster changes the atlas also reads. `size` is the target's, `render` the size
    /// the scene is drawn at this frame (smaller with a render scale), which the clusters' pixels follow.
    pub(crate) fn prepare_lights(
        &mut self,
        scene: &mut GpuScene,
        camera: Option<&Camera>,
        size: (u32, u32),
        render: (u32, u32),
        cull: &CullSettings,
        lod: &LodView,
    ) {
        // Lights a frame wanted while their pipelines were compiling were not drawn: now that they can be, they fade
        // in rather than pop on (#86).
        let lit_ready = {
            let target = &self.targets[self.current];
            let lit = |lights| Variant { lights };
            target.has_variant(lit(Lights::Unshadowed)) && target.has_variant(lit(Lights::Shadowed))
        };
        if self.lights_held && lit_ready {
            LocalLights::restart_fades(scene);
            self.lights_held = false;
        }
        self.lights.prepare(scene, camera, size, render, cull, lod);
        self.refresh_light_group();
        let wanted = self.wanted_variant();
        if !lit_ready && wanted.lights != Lights::None {
            self.lights_held = true;
        }
    }

    /// Whether this frame has local lights to draw whose pipelines are still compiling (so it draws without them; they
    /// fade in once the pipelines exist).
    pub(crate) fn lights_held(&self) -> bool {
        self.lights_held
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

    /// The atlas's see-through maps to draw into this frame (depth, light), when it draws them (#491).
    pub(crate) fn light_see_through_targets(&self) -> Option<(&wgpu::TextureView, &wgpu::TextureView)> {
        self.lights.see_through_targets()
    }

    /// Records the atlas's see-through casters into `render_pass`: their depth, or with `light` the light they let
    /// through (#491).
    pub(crate) fn draw_light_see_through(&mut self, light: bool, render_pass: &mut wgpu::RenderPass<'_>) {
        let (draws, instances, triangles) = self.lights.draw_see_through(light, &self.meshes, render_pass);
        self.stats.light_shadow_draws += draws;
        self.stats.light_shadow_instances += instances;
        self.stats.light_shadow_triangles += triangles;
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
    /// with the casters at levels of detail no finer than `lod` (the camera view's) draws what receives their shadows,
    /// and records the upload of the casters' lists into `encoder`. Call it after [`GpuScene::upload_view`] and
    /// before [`PartRenderer::prepare`], every frame (see [`crate::shadow`]).
    pub(crate) fn prepare_shadows(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        scene: &mut GpuScene,
        camera: Option<&Camera>,
        (width, height): (u32, u32),
        lod: &LodView,
    ) {
        let lit = self.lighting.global_shadows && self.resolved.light_color.max_element() > 0.0;
        let aspect = width as f32 / height.max(1) as f32;
        let light = lit.then_some(self.resolved.light_direction);
        if self.shadows.prepare(encoder, scene, camera, aspect, light, lod) {
            // A settings change's new maps took over this frame.
            self.rebuild_frame_group();
        }
    }

    /// Cascade `cascade`'s see-through maps to draw into this frame (depth, light), when it draws them (#491).
    pub(crate) fn see_through_targets(&self, cascade: usize) -> Option<(&wgpu::TextureView, &wgpu::TextureView)> {
        self.shadows.see_through_targets(cascade)
    }

    /// Records cascade `cascade`'s see-through casters into `render_pass`: their depth, or with `light` the light
    /// they let through (#491).
    pub(crate) fn draw_see_through(&mut self, cascade: usize, light: bool, render_pass: &mut wgpu::RenderPass<'_>) {
        let (draws, instances, triangles) = self.shadows.draw_see_through(cascade, light, &self.meshes, render_pass);
        self.stats.shadow_draws += draws;
        self.stats.shadow_instances += instances;
        self.stats.shadow_triangles += triangles;
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

    /// Whether building cached shadow cascade `cascade`'s map this frame restores footprints of what it holds, so
    /// its pass loads the map rather than clearing it.
    pub(crate) fn shadow_build_restores(&self, cascade: usize) -> bool {
        self.shadows.build_restores(cascade)
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
        // The wireframe draws with pipelines of its own; the other views are the shader's (#478). A wireframe still
        // compiling is drawn shaded.
        let view = match self.shading {
            ShadingView::Wireframe => ShadingView::Shaded,
            view => view,
        };
        let uniforms = FrameUniforms {
            view: [view.number(), 0.0, 0.0, 0.0],
            ..FrameUniforms::new(
                camera,
                aspect,
                offset,
                &self.resolved,
                self.shadows.frame(),
                self.lights.frame(),
                Output::new(srgb, self.lod_tint.then(|| self.meshes.level_starts())),
            )
        };
        self.stats.lights = self.lights.stats();
        let shadow = self.shadows.stats();
        self.stats.shadow_cascades = shadow.cascades;
        self.stats.shadow_redrawn = shadow.redrawn_mask();
        self.stats.shadow_built = shadow.built_mask();
        self.stats.shadow_restored = shadow.restored_mask();
        self.stats.shadow_restored_texels = shadow.cascade.map(|c| c.restored_texels);
        self.stats.shadow_casters = shadow.cascade.map(|c| c.casters);
        self.stats.shadow_moving = shadow.cascade.map(|c| c.moving);
        if self.frame_uniforms != Some(uniforms) {
            self.queue
                .write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&uniforms));
            self.frame_uniforms = Some(uniforms);
        }

        for (kind, segment) in scene.list_segments().iter().enumerate() {
            let kind = MeshKind(u16::try_from(kind).unwrap_or(u16::MAX));
            for level in 0..MAX_LOD_LEVELS {
                let instances = segment.level_range(level);
                let mesh = self.meshes.range(kind, level);
                if instances.is_empty() || mesh.index_count == 0 {
                    continue;
                }
                let count = instances.end - instances.start;
                self.draws.push(KindDraw {
                    mesh,
                    first: instances.start,
                    count,
                    texture: self.kind_textures.get(usize::from(kind.0)).copied().unwrap_or(WHITE),
                });
                self.stats.opaque_instances += count;
                self.stats.opaque_triangles += u64::from(count) * u64::from(mesh.index_count / 3);
                self.stats.level_instances[level] += count;
            }
        }
        self.transparent.update(
            (&self.device, &self.queue),
            instances,
            scene.transparent_entries(),
            offset,
            &self.meshes,
        );
        self.stats.culled_instances = scene.cull_stats().culled();
        let transparent = self.transparent.counts();
        self.stats.transparent_instances = transparent.instances;
        self.stats.transparent_triangles = transparent.triangles;
        for (n, drawn) in self.stats.level_instances.iter_mut().zip(transparent.levels) {
            *n += drawn;
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
                [(0, statics), (1, dynamics), (2, scene.list_buffer())],
            );
            match (
                self.transparent.path(),
                self.pulled_layout.as_ref(),
                self.meshes.pulled(),
            ) {
                (TransparentPath::Pulled, Some(pulled_layout), Some(vertices)) => refresh_group(
                    &self.device,
                    pulled_layout,
                    &mut self.pulled_group,
                    [
                        (0, statics),
                        (1, dynamics),
                        (3, self.transparent.buffer()),
                        (4, vertices),
                    ],
                ),
                _ => refresh_group(
                    &self.device,
                    layout,
                    &mut self.transparent_group,
                    [(0, statics), (1, dynamics), (2, self.transparent.buffer())],
                ),
            }
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
        let target = &self.targets[self.current];
        match pass {
            PassId::DepthPrepass | PassId::Opaque => {
                if self.draws.is_empty() {
                    return;
                }
                let variant = self.variant();
                let l = variant.number();
                let wireframe = self.wireframe();
                // The prepass writes depth only, with the pipeline made without local lights. The wireframe has none
                // (its lines would be hidden by the faces' depth) and draws its lines without local lights (#478).
                let (pipeline, variant) = match (pass, prepass) {
                    (PassId::DepthPrepass, _) if wireframe => return,
                    (PassId::DepthPrepass, _) => (PartPipeline::Prepass, Variant::BASE),
                    _ if wireframe => (PartPipeline::Wireframe, Variant::BASE),
                    (_, true) => (PartPipeline::Tested(l), variant),
                    (_, false) => (PartPipeline::Opaque(l), variant),
                };
                let Some(pipeline) = target.get(pipeline) else {
                    return;
                };
                render_pass.set_pipeline(pipeline);
                self.bind(render_pass, variant, self.opaque_group.as_ref(), scene.list_buffer());
                // The primitives' draws come first and share the white texture; each mesh asset's binds its own.
                let mut bound = u32::MAX;
                for draw in &self.draws {
                    if draw.texture != bound {
                        bound = draw.texture;
                        render_pass.set_bind_group(texture_group(self.binding), self.textures.group(bound), &[]);
                    }
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
                if self.transparent.is_empty() {
                    return;
                }
                let (pipeline, variant) = if self.wireframe() {
                    (PartPipeline::WireframeTransparent, Variant::BASE)
                } else {
                    let variant = self.variant();
                    (PartPipeline::Transparent(variant.number()), variant)
                };
                let Some(pipeline) = target.get(pipeline) else {
                    return;
                };
                render_pass.set_pipeline(pipeline);
                // See-through mesh assets draw untextured: the sorted list is one draw (or runs that cross kinds).
                render_pass.set_bind_group(texture_group(self.binding), self.textures.group(WHITE), &[]);
                if self.transparent.path() == TransparentPath::Pulled {
                    // Every see-through instance in one draw, back to front, with no vertex or index buffer.
                    let Some(group) = self.pulled_group.as_ref() else {
                        return;
                    };
                    render_pass.set_bind_group(0, self.frame_group_of(variant), &[]);
                    render_pass.set_bind_group(1, &group.group, &[]);
                    render_pass.draw(0..self.transparent.counts().vertices, 0..1);
                    self.stats.transparent_draws += 1;
                    return;
                }
                self.bind(
                    render_pass,
                    variant,
                    self.transparent_group.as_ref(),
                    self.transparent.buffer(),
                );
                for run in self.transparent.runs() {
                    let mesh = self.meshes.range(run.kind, usize::from(run.level));
                    if mesh.index_count == 0 {
                        continue;
                    }
                    let indices = mesh.first_index..mesh.first_index + mesh.index_count;
                    render_pass.draw_indexed(indices, 0, run.first..run.first + run.count);
                    self.stats.transparent_draws += 1;
                }
            }
            PassId::Sky => {
                let Some(pipeline) = target.get(PartPipeline::Sky) else {
                    return;
                };
                render_pass.set_pipeline(pipeline);
                render_pass.set_bind_group(0, &self.sun_group, &[]);
                render_pass.draw(0..3, 0..1);
                self.stats.sky_draws += 1;
            }
            PassId::Scene | PassId::Mlaa | PassId::Bloom | PassId::Post | PassId::Upscale | PassId::Ui => {}
        }
    }

    /// Which variant of the pipelines this frame draws with: without local lights when none is drawn, without their
    /// shadows when no drawn light has one. Until the variant's pipelines are compiled (in the background, after the
    /// first frame's), the frame draws without the local lights, as it did before they appeared.
    fn variant(&self) -> Variant {
        let wanted = self.wanted_variant();
        let target = &self.targets[self.current];
        [wanted, Variant::BASE]
            .into_iter()
            .find(|&variant| target.has_variant(variant))
            .unwrap_or(Variant::BASE)
    }

    /// Which variant of the pipelines this frame's lights call for, compiled or not.
    fn wanted_variant(&self) -> Variant {
        let frame = self.lights.frame();
        let lights = if frame.local[0] <= 0.0 || self.frame_group.is_none() {
            Lights::None
        } else if frame.atlas[2] > 0.0 {
            Lights::Shadowed
        } else {
            Lights::Unshadowed
        };
        Variant { lights }
    }

    /// The frame group of the pipeline `variant`: without the lights' entries for frames without local lights.
    fn frame_group_of(&self, variant: Variant) -> &wgpu::BindGroup {
        match variant.lights {
            Lights::None => &self.sun_group,
            Lights::Unshadowed | Lights::Shadowed => self.frame_group.as_ref().unwrap_or(&self.sun_group),
        }
    }

    /// Binds the frame group of the pipeline `variant` (without the lights' entries for frames without local lights),
    /// the meshes and the instances: the storage path's instance group, or `copies` as the per-instance vertex
    /// buffer.
    fn bind(
        &self,
        render_pass: &mut wgpu::RenderPass<'_>,
        variant: Variant,
        group: Option<&InstanceGroup<3>>,
        copies: &wgpu::Buffer,
    ) {
        render_pass.set_bind_group(0, self.frame_group_of(variant), &[]);
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
    let (see_depth, see_light, see_sampler) = shadows.see_through_views();
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
        wgpu::BindGroupEntry {
            binding: 8,
            resource: wgpu::BindingResource::TextureView(see_depth),
        },
        wgpu::BindGroupEntry {
            binding: 9,
            resource: wgpu::BindingResource::TextureView(see_light),
        },
        wgpu::BindGroupEntry {
            binding: 10,
            resource: wgpu::BindingResource::Sampler(see_sampler),
        },
    ];
    let Some(lights) = lights else {
        return device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("metallic parts frame (no local lights)"),
            layout,
            entries: &sun,
        });
    };
    let [e0, e1, e2, e3, e8, e9, e10] = sun;
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("metallic parts frame"),
        layout,
        entries: &[
            e0,
            e1,
            e2,
            e3,
            e8,
            e9,
            e10,
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
            wgpu::BindGroupEntry {
                binding: 11,
                resource: wgpu::BindingResource::TextureView(lights.see_depth),
            },
            wgpu::BindGroupEntry {
                binding: 12,
                resource: wgpu::BindingResource::TextureView(lights.see_light),
            },
        ],
    })
}

/// Makes `slot`'s bind group again when any of `buffers` is not the one it was made with.
/// `buffers` are (binding, buffer).
fn refresh_group<const N: usize>(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    slot: &mut Option<InstanceGroup<N>>,
    buffers: [(u32, &wgpu::Buffer); N],
) {
    if slot
        .as_ref()
        .is_some_and(|g| g.buffers.iter().zip(buffers).all(|(a, (_, b))| a == b))
    {
        return;
    }
    let entries = buffers.map(|(binding, buffer)| wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("metallic parts instances"),
        layout,
        entries: &entries,
    });
    *slot = Some(InstanceGroup {
        buffers: buffers.map(|(_, buffer)| buffer.clone()),
        group,
    });
}

/// The compile of `pipeline` for `format` at `samples`, made from `shaders` on any thread.
fn part_recipe(shaders: &Arc<Shaders>, format: wgpu::TextureFormat, samples: u32, pipeline: PartPipeline) -> Recipe {
    let shaders = Arc::clone(shaders);
    Box::new(move |device, cache| create_pipeline(device, cache, &shaders, format, samples, pipeline).into())
}

/// Makes the scene's `pipeline` for `format` at `samples`.
fn create_pipeline(
    device: &wgpu::Device,
    cache: Option<&wgpu::PipelineCache>,
    shaders: &Shaders,
    format: wgpu::TextureFormat,
    samples: u32,
    pipeline: PartPipeline,
) -> wgpu::RenderPipeline {
    let (layout, module) = (&shaders.parts_layout, &shaders.parts);
    let binding = shaders.binding;
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
                   variant: Variant,
                   pulled: Option<&[wgpu::PipelineLayout; 2]>,
                   lines: bool| {
        let constants = variant.constants();
        let lights = variant.lights;
        // Frames without local lights draw with the module and layout that have none of their bindings.
        let (mut layout, module) = match lights {
            Lights::None => (&shaders.unlit_layout, &shaders.unlit),
            Lights::Unshadowed | Lights::Shadowed => (layout, module),
        };
        // The transparent pass's pulled vertices need no vertex buffer (parts/transparent.rs).
        let (mut entry_point, mut buffers) = ("vs_main", buffers);
        if let Some([lit, unlit]) = pulled {
            layout = if lights == Lights::None { unlit } else { lit };
            (entry_point, buffers) = ("vs_pulled", &[]);
        }
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
                entry_point: Some(entry_point),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers,
            },
            // The wireframe view (#478) draws every triangle's edges, its back faces' too.
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: if lines { None } else { Some(wgpu::Face::Back) },
                polygon_mode: if lines {
                    wgpu::PolygonMode::Line
                } else {
                    wgpu::PolygonMode::Fill
                },
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
            cache,
        })
    };
    let transparent_blend = Some(("fs_transparent", Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)));
    debug_assert_eq!(depth_stencil_state(true).depth_compare, Some(DEPTH_COMPARE));
    debug_assert_eq!(depth_stencil_state(true).format, DEPTH_FORMAT);
    match pipeline {
        PartPipeline::Prepass => variant(
            "metallic parts depth prepass",
            depth_stencil_state(true),
            None,
            Variant::BASE,
            None,
            false,
        ),
        PartPipeline::Sky => sky_pipeline(device, cache, shaders, format, samples),
        PartPipeline::Opaque(lights) => {
            let lights = Variant::from_number(lights);
            variant(
                &format!("metallic parts opaque ({})", lights.name()),
                depth_stencil_state(true),
                Some(("fs_opaque", None)),
                lights,
                None,
                false,
            )
        }
        PartPipeline::Tested(lights) => {
            let lights = Variant::from_number(lights);
            let tested = wgpu::DepthStencilState {
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                ..depth_stencil_state(false)
            };
            variant(
                &format!("metallic parts opaque after prepass ({})", lights.name()),
                tested,
                Some(("fs_opaque", None)),
                lights,
                None,
                false,
            )
        }
        PartPipeline::Transparent(lights) => {
            let lights = Variant::from_number(lights);
            variant(
                &format!("metallic parts transparent ({})", lights.name()),
                depth_stencil_state(false),
                transparent_blend,
                lights,
                shaders.pulled_layouts.as_ref(),
                false,
            )
        }
        PartPipeline::Wireframe => variant(
            "metallic parts wireframe",
            depth_stencil_state(true),
            Some(("fs_wireframe", None)),
            Variant::BASE,
            None,
            true,
        ),
        PartPipeline::WireframeTransparent => variant(
            "metallic parts wireframe, see-through",
            depth_stencil_state(true),
            Some(("fs_wireframe", None)),
            Variant::BASE,
            shaders.pulled_layouts.as_ref(),
            true,
        ),
    }
}

/// The sky: a full-screen triangle at the far plane, tested against the depth the opaque pass left (`GreaterEqual`
/// passes only where it is still the clear value) and never writing it.
fn sky_pipeline(
    device: &wgpu::Device,
    cache: Option<&wgpu::PipelineCache>,
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
        cache,
    })
}
