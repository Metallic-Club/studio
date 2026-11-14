//! The local lights' shadow atlas: who holds which tiles, which tiles are drawn this frame, their casters, and the
//! pass that draws them (see the [module documentation](super)).

use std::time::{Duration, Instant};

use bytemuck::{Pod, Zeroable};
use metallic_core::Id;
use metallic_core::glam::{Mat4, Vec3, Vec4};

use super::{LightRuntime, LightSettings, MAX_SHADOW_TILES};
use crate::GpuScene;
use crate::context::in_heap_turn;
use crate::cull::{Casters, CullView, Frustum, VisibleSet};
use crate::lod::{LodView, MAX_LOD_LEVELS};
use crate::parts::MeshPool;
use crate::scene::{
    DYNAMIC_ENTRY, GpuInstance, InstanceBinding, InstanceScene, MeshKind, Region, SceneLight, bounding_sphere, entry,
    is_shadow_caster,
};
use crate::shadow::{
    SHADOW_FORMAT, SeeThroughCasters, SeeThroughMaps, SeeThroughPlaceholder, caster_pipeline, clear_light_pipeline,
};
use crate::textures::{Made, TextureMaker, TextureSpec, Ticket};

/// Texels kept clear around a tile's face, so the filter's taps near its edge stay inside the tile.
const BORDER: f32 = 2.0;
/// The near plane of every tile's projection, in metres.
const NEAR: f32 = 0.05;
/// A spot light's one tile always covers this half-angle (a 90° cone, a cube's face), whatever the spot's Angle
/// (#480); a wider spot is shadowed with a cube (#437).
pub(crate) const SPOT_SHADOW_HALF_DEGREES: f32 = crate::scene::SPOT_TILE_HALF_DEGREES;
/// Each tile's caster uniform sits at a multiple of this (the strictest dynamic offset alignment there is).
const UNIFORM_STRIDE: u64 = 256;
/// The bytes of a tile's caster uniform the shader reads: one matrix.
const UNIFORM_SIZE: u64 = 64;
/// The fewest bytes the caster list buffer has.
const MIN_LIST_BYTES: u64 = 4096;
/// No light holds the tile this frame.
const FREE: u32 = u32::MAX;
/// The rasterizer's depth bias on the casters: 2 texels of the caster's depth slope, a little past the 1.5 texels
/// the parts' four taps reach from a point (see [`LightSettings::depth_bias`]). No constant term and no clamp, as for
/// the sun's cascades (`shadow/maps.rs`).
const CASTER_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: 0,
    slope_scale: 2.0,
    clamp: 0.0,
};

/// A point light's faces, in the order the shader picks them by the major axis of the light-to-point direction (+X,
/// −X, +Y, −Y, +Z, −Z), each with an up direction for its view.
const FACES: [(Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::Y),
    (Vec3::NEG_X, Vec3::Y),
    (Vec3::Y, Vec3::Z),
    (Vec3::NEG_Y, Vec3::Z),
    (Vec3::Z, Vec3::Y),
    (Vec3::NEG_Z, Vec3::Y),
];

/// One tile's view from its light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TileView {
    /// Origin-relative positions to the tile's clip space (standard depth: 0 at the near plane, 1 at the range).
    pub(crate) clip: Mat4,
    /// The light, origin-relative, and the face's direction, for culling its casters.
    pub(crate) eye: Vec3,
    pub(crate) forward: Vec3,
    /// A texel's width per metre of depth: `2 tan(half-angle) / resolution`.
    pub(crate) texel_angle: f32,
    /// `near × far / (far − near)`: turns a depth difference in metres at view depth `w` into one in depth units
    /// (divided by `w²`).
    pub(crate) depth_scale: f32,
}

/// The views of a shadowed `light`'s tiles of `resolution` texels, origin-relative to `origin`: a cube's six faces,
/// each a little wider than 90° by [`BORDER`] texels (a point light, or a spot wider than one tile covers, #437), or a
/// spot's cone. Returns how many it wrote.
pub(crate) fn tile_views(light: &SceneLight, origin: Vec3, resolution: u32, out: &mut [TileView; 6]) -> usize {
    let resolution = resolution.max(16) as f32;
    let widen = resolution / (resolution - 2.0 * BORDER);
    let eye = light.position - origin;
    let far = light.range.max(NEAR * 2.0);
    let depth_scale = NEAR * far / (far - NEAR);
    let view = |forward: Vec3, up: Vec3, tan_half: f32| TileView {
        clip: perspective(tan_half, NEAR, far) * look_to(eye, forward, up),
        eye,
        forward,
        texel_angle: 2.0 * tan_half / resolution,
        depth_scale,
    };
    if light.cube_shadow() {
        for (slot, &(forward, up)) in out.iter_mut().zip(&FACES) {
            *slot = view(forward, up, widen);
        }
        return 6;
    }
    // A cube face's view whatever the cone (#480): a narrower spot uses the middle of it. Its texels per degree
    // then never change with the Angle, so neither does how sharp its shadow is, nor does a turn across the cube's
    // angle (#451) change it.
    let half = SPOT_SHADOW_HALF_DEGREES.to_radians();
    let forward = light.direction.normalize_or(Vec3::NEG_Z);
    let up = if forward.y.abs() < 0.99 { Vec3::Y } else { Vec3::Z };
    out[0] = view(forward, up, half.tan() * widen);
    1
}

/// A square right-handed perspective with standard depth (0 at `near`, 1 at `far`), `tan_half` the slope of its
/// edges.
fn perspective(tan_half: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / tan_half.max(1e-4);
    let r = far / (near - far);
    Mat4::from_cols(
        Vec4::new(f, 0.0, 0.0, 0.0),
        Vec4::new(0.0, f, 0.0, 0.0),
        Vec4::new(0.0, 0.0, r, -1.0),
        Vec4::new(0.0, 0.0, r * near, 0.0),
    )
}

/// The right-handed view from `eye` looking along `forward`, `up` roughly up.
fn look_to(eye: Vec3, forward: Vec3, up: Vec3) -> Mat4 {
    let f = forward.normalize_or(Vec3::NEG_Z);
    let s = f.cross(up).normalize_or(Vec3::X);
    let u = s.cross(f);
    Mat4::from_cols(
        Vec4::new(s.x, u.x, -f.x, 0.0),
        Vec4::new(s.y, u.y, -f.y, 0.0),
        Vec4::new(s.z, u.z, -f.z, 0.0),
        Vec4::new(-s.dot(eye), -u.dot(eye), f.dot(eye), 1.0),
    )
}

/// The bytes of one tile in the tiles' uniform array ([`GpuTile`]).
pub(crate) const TILE_SIZE: usize = size_of::<GpuTile>();

/// One tile as the parts read it (`ShadowTile` in lights.wgsl): 96 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub(crate) struct GpuTile {
    /// Camera-relative positions to the tile's clip space.
    clip: [[f32; 4]; 4],
    /// The tile's corner in the atlas (u, v) and its size, in atlas coordinates.
    rect: [f32; 4],
    /// x: [`TileView::depth_scale`], y: [`TileView::texel_angle`].
    params: [f32; 4],
}

/// What a tile's drawing was made for: its light, face and view, and the distance its casters' levels of detail
/// were chosen at ([`lod_distance`]; 0 at full detail).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TileKey {
    light: (u32, u32),
    face: u8,
    clip: [u32; 16],
    lod: u32,
    /// The camera's distance the casters' levels are chosen by ([`light_distance`], #614), as bits.
    near: u32,
}

impl TileKey {
    fn new(light: Id, face: usize, view: &TileView, lod: &LodView) -> Self {
        Self {
            light: (light.index(), light.generation()),
            face: face as u8,
            clip: view.clip.to_cols_array().map(f32::to_bits),
            lod: lod.distance().to_bits(),
            near: lod.slack().to_bits(),
        }
    }
}

/// The distance a light's tiles choose their casters' levels of detail at ([`crate::lod`]): a point the light
/// reaches is at most its distance from the camera plus its range from the camera, rounded up to a power of two so a
/// moving camera draws a kept tile again only when that doubles or halves.
fn lod_distance(light_to_camera: f32, range: f32) -> f32 {
    (light_to_camera + range).max(1.0).log2().ceil().exp2()
}

/// The camera's distance from a light as its tiles choose their casters' levels by ([`LodView::around`], #614):
/// rounded up to a quarter power of two, so it is never less than it is and a moving camera draws a kept tile again
/// only when it changes by a fifth or so.
fn light_distance(light_to_camera: f32) -> f32 {
    ((light_to_camera.max(0.25).log2() * 4.0).ceil() / 4.0).exp2()
}

/// What a tile's map holds.
#[derive(Clone, Copy, Debug, Default)]
struct TileCache {
    valid: bool,
    key: TileKey,
    /// [`InstanceScene::shadow_epoch`] and [`InstanceScene::revision`] when it was drawn.
    epoch: u64,
    revision: u64,
    /// It holds a moving caster (one in the dynamic region).
    dynamic: bool,
    /// Its view, and the light's origin-relative position and range, to test changes against while it is kept.
    clip: Mat4,
    center: Vec3,
    reach: f32,
}

impl TileCache {
    /// Whether a bounding sphere (origin-relative) reaches into the tile's view.
    fn touches(&self, frustum: &Frustum, (c, r): (Vec3, f32)) -> bool {
        c.distance_squared(self.center) <= (self.reach + r) * (self.reach + r) && frustum.contains_sphere(c, r)
    }
}

/// A tile drawn this frame.
#[derive(Clone, Copy, Debug)]
struct Job {
    tile: u16,
    key: TileKey,
    view: TileView,
    /// How its casters' levels of detail are chosen.
    lod: LodView,
    /// The part the light sits in, which casts no shadow for it.
    host: Option<u32>,
    /// The light, origin-relative, and its range.
    center: Vec3,
    reach: f32,
}

/// A light drawn this frame, as the atlas needs it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShadowedLight {
    pub(crate) light: SceneLight,
    pub(crate) tiles: [u16; 6],
    pub(crate) tile_count: u8,
}

/// The storage path's instance bind group for the caster pass, and the buffers it was made for.
#[derive(Debug)]
struct InstanceGroup {
    buffers: [wgpu::Buffer; 3],
    group: wgpu::BindGroup,
}

/// What a frame did in the atlas.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct AtlasStats {
    pub(crate) shadowed: u32,
    pub(crate) tiles: u32,
    pub(crate) redrawn: u32,
    pub(crate) casters: u32,
    pub(crate) cull_time: Duration,
}

/// The atlas's GPU memory: the depth texture and the casters' matrices. Made when a drawn light first has a
/// shadow, never for a scene without one.
#[derive(Debug)]
struct AtlasGpu {
    sample_view: wgpu::TextureView,
    render_view: wgpu::TextureView,
    caster_uniform: wgpu::Buffer,
    caster_group: wgpu::BindGroup,
}

/// The atlas's size, its tiles' size, tiles per row and tile count ([`layout`]).
type Layout = (u32, u32, u32, usize);

/// The atlas texture being made off the frame ([`crate::textures`]): the first one, or one of a new size while the
/// atlas before it stays in use.
#[derive(Debug)]
struct Incoming {
    layout: Layout,
    ticket: Ticket,
}

/// The shadow atlas and everything that draws it. [`super::LocalLights`] owns one.
#[derive(Debug)]
pub(crate) struct ShadowAtlas {
    device: wgpu::Device,
    queue: wgpu::Queue,
    maker: TextureMaker,
    /// The texture asked for and not yet made.
    incoming: Option<Incoming>,
    binding: InstanceBinding,
    resolution: u32,
    tile: u32,
    per_row: u32,
    /// Tiles in the atlas.
    capacity: usize,
    /// `None` until a drawn light first has a shadow (and the texture is made).
    gpu: Option<AtlasGpu>,
    /// Changes whenever the texture is made or dropped, so the bind groups that sample it are made again.
    revision: u64,
    caster_layout: wgpu::BindGroupLayout,
    /// Per tile: the light that holds it.
    owner: Vec<Option<Id>>,
    /// Per tile: the index of the light holding it this frame, or [`FREE`].
    holder: Vec<u32>,
    /// Per tile: the light holding it is drawn with a shadow this frame (it cannot be taken).
    active: Vec<bool>,
    caches: Vec<TileCache>,
    /// Per drawn light: it is one of the frame's shadowed lights.
    targets: Vec<bool>,
    jobs: Vec<Job>,
    /// This frame's caster entries, every job's in turn, and the draws over them: (job, kind, level, first, count,
    /// see-through, #122).
    list: Vec<u32>,
    gathered: Vec<GpuInstance>,
    ranges: Vec<(u16, u16, u8, u32, u32, bool)>,
    /// A job's see-through casters, sorted into draws (#122).
    see_through_casters: SeeThroughCasters,
    /// The settings draw see-through casters' shadows (#122).
    see_through: bool,
    /// The casters' lists on the GPU, made with the first tile drawn.
    list_buffer: Option<wgpu::Buffer>,
    visible: VisibleSet,
    /// The moving casters' bounding spheres, origin-relative, as of `dynamic_revision`.
    dynamic: Vec<(Vec3, f32)>,
    dynamic_revision: Option<u64>,
    caster_data: Vec<[f32; (UNIFORM_STRIDE / 4) as usize]>,
    caster: wgpu::RenderPipeline,
    /// The pipeline that draws the light see-through casters let through (#491).
    see_through_pipeline: wgpu::RenderPipeline,
    /// The see-through casters' maps beside the atlas (#491), and the pipeline that clears a tile of the light map.
    see: SeeThroughMaps,
    see_clear: wgpu::RenderPipeline,
    /// Per tile: its see-through maps hold see-through casters, as of when it was last drawn.
    see_tiles: Vec<bool>,
    /// The tile of each entry of `table`, this frame.
    table_tiles: Vec<u16>,
    clear: wgpu::RenderPipeline,
    instance_layout: Option<wgpu::BindGroupLayout>,
    instance_group: Option<InstanceGroup>,
    /// The tiles of the lights drawn this frame, as the parts read them.
    table: Vec<GpuTile>,
    uploaded_table: Vec<GpuTile>,
    stats: AtlasStats,
}

impl ShadowAtlas {
    /// An atlas for `settings` on the instance path `binding`, whose storage-path instance group has
    /// `instance_layout`, its pipelines made through `cache` where there is one.
    pub(crate) fn new(
        (device, queue, maker): (&wgpu::Device, &wgpu::Queue, &TextureMaker),
        binding: InstanceBinding,
        instance_layout: Option<&wgpu::BindGroupLayout>,
        settings: &LightSettings,
        cache: Option<&wgpu::PipelineCache>,
        see_placeholder: &SeeThroughPlaceholder,
    ) -> Self {
        let caster_layout = in_heap_turn(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("metallic shadow atlas tile"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(UNIFORM_SIZE),
                    },
                    count: None,
                }],
            })
        });
        let (resolution, tile, per_row, capacity) = layout(device, settings);
        // The pipelines are made now, with the renderer (no hitch when the first shadow appears); the memory waits.
        let caster = caster_pipeline(
            device,
            cache,
            binding,
            &caster_layout,
            instance_layout,
            CASTER_BIAS,
            false,
        );
        let see_through_pipeline = caster_pipeline(
            device,
            cache,
            binding,
            &caster_layout,
            instance_layout,
            CASTER_BIAS,
            true,
        );
        Self {
            device: device.clone(),
            queue: queue.clone(),
            maker: maker.clone(),
            incoming: None,
            binding,
            resolution,
            tile,
            per_row,
            capacity,
            gpu: None,
            revision: 0,
            caster_layout,
            owner: vec![None; capacity],
            holder: vec![FREE; capacity],
            active: vec![false; capacity],
            caches: vec![TileCache::default(); capacity],
            targets: Vec::new(),
            jobs: Vec::with_capacity(MAX_SHADOW_TILES),
            list: Vec::new(),
            gathered: Vec::new(),
            ranges: Vec::new(),
            see_through_casters: SeeThroughCasters::default(),
            see_through: settings.see_through_shadows,
            list_buffer: None,
            visible: VisibleSet::new(),
            dynamic: Vec::new(),
            dynamic_revision: None,
            caster_data: Vec::new(),
            caster,
            see_through_pipeline,
            see: SeeThroughMaps::new(device, maker, see_placeholder),
            see_clear: clear_light_pipeline(device, cache),
            see_tiles: vec![false; capacity],
            table_tiles: Vec::new(),
            clear: clear_pipeline(device, cache),
            instance_layout: instance_layout.cloned(),
            instance_group: None,
            table: Vec::new(),
            uploaded_table: Vec::new(),
            stats: AtlasStats::default(),
        }
    }

    /// Makes sure the atlas's texture exists, or is being made at the size the settings say now: at once before a
    /// scene's first frame (or when small), otherwise off the frame ([`crate::textures`]). Runs when a drawn light
    /// has a shadow; whether the texture exists.
    fn ensure_gpu(&mut self) -> bool {
        self.poll_incoming();
        if self.gpu.is_none() && self.incoming.is_none() {
            let layout = (self.resolution, self.tile, self.per_row, self.capacity);
            match self.maker.make(atlas_spec(self.resolution)) {
                Made::Ready(texture) => self.install(layout, &texture),
                Made::Pending(ticket) => self.incoming = Some(Incoming { layout, ticket }),
            }
        }
        self.gpu.is_some()
    }

    /// Draws with the texture asked for once it is made.
    fn poll_incoming(&mut self) {
        let Some(incoming) = self.incoming.as_ref() else {
            return;
        };
        let Some(texture) = self.maker.take(&incoming.ticket) else {
            return;
        };
        let layout = incoming.layout;
        self.incoming = None;
        self.install(layout, &texture);
    }

    /// Draws with `texture`, cut as `layout` says, from now on: every tile is drawn again into it. The casters'
    /// uniform is made with the first texture.
    fn install(&mut self, layout: Layout, texture: &wgpu::Texture) {
        self.apply_layout(layout);
        let (caster_uniform, caster_group) = match self.gpu.take() {
            Some(gpu) => (gpu.caster_uniform, gpu.caster_group),
            None => {
                let caster_uniform = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("metallic shadow atlas tiles (casters)"),
                    size: UNIFORM_STRIDE * MAX_SHADOW_TILES as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let caster_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("metallic shadow atlas tile"),
                    layout: &self.caster_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &caster_uniform,
                            offset: 0,
                            size: wgpu::BufferSize::new(UNIFORM_SIZE),
                        }),
                    }],
                });
                (caster_uniform, caster_group)
            }
        };
        let (sample_view, render_view) = atlas_views(texture);
        self.gpu = Some(AtlasGpu {
            sample_view,
            render_view,
            caster_uniform,
            caster_group,
        });
        self.caster_data
            .resize(MAX_SHADOW_TILES, [0.0; (UNIFORM_STRIDE / 4) as usize]);
        self.table.reserve(MAX_SHADOW_TILES);
        self.uploaded_table.reserve(MAX_SHADOW_TILES);
        // A new texture holds no drawing.
        for cache in &mut self.caches {
            cache.valid = false;
        }
        self.revision = self.revision.wrapping_add(1);
    }

    /// Cuts the atlas as `layout` says. Lights keep their tiles (a tile's number stays its own, wherever it now
    /// sits), but for tiles past a smaller count, which are given back; tiles that moved are drawn again.
    fn apply_layout(&mut self, (resolution, tile, per_row, capacity): Layout) {
        let layout_changed = (resolution, tile, per_row) != (self.resolution, self.tile, self.per_row);
        if layout_changed {
            for cache in &mut self.caches {
                cache.valid = false;
            }
        }
        self.resolution = resolution;
        self.tile = tile;
        self.per_row = per_row;
        self.capacity = capacity;
        self.owner.resize(capacity, None);
        self.holder.resize(capacity, FREE);
        self.active.resize(capacity, false);
        self.caches.resize(capacity, TileCache::default());
        // Tiles that moved hold nothing in the see-through maps until drawn again (#491).
        self.see_tiles.resize(capacity, false);
        if layout_changed {
            self.see_tiles.fill(false);
        }
    }

    /// Applies `settings`' atlas and tile size (a settings change, never per frame). A new atlas size, once the atlas
    /// exists, makes a texture of that size, off the frame once a scene's first frame is drawn: frames draw with the
    /// atlas before it until it is made, and every tile is drawn again into it then. Without an atlas yet, the next
    /// frame with a shadow makes it at that size.
    pub(crate) fn configure(&mut self, settings: &LightSettings) {
        if settings.see_through_shadows != self.see_through {
            // Every tile holds the casters of the old rule: draw them again.
            self.see_through = settings.see_through_shadows;
            for cache in &mut self.caches {
                cache.valid = false;
            }
        }
        let layout = layout(&self.device, settings);
        if let Some(incoming) = self.incoming.take() {
            self.maker.cancel(incoming.ticket);
        }
        if self.gpu.is_none() || layout.0 == self.resolution {
            self.apply_layout(layout);
            return;
        }
        match self.maker.make(atlas_spec(layout.0)) {
            Made::Ready(texture) => self.install(layout, &texture),
            Made::Pending(ticket) => self.incoming = Some(Incoming { layout, ticket }),
        }
    }

    /// Whether the atlas is still being made for its first shadow (its lights are drawn without their shadows
    /// meanwhile).
    pub(crate) fn waiting(&self) -> bool {
        self.gpu.is_none() && self.incoming.is_some()
    }

    /// The atlas the parts sample, once it exists.
    pub(crate) fn sample_view(&self) -> Option<&wgpu::TextureView> {
        self.gpu.as_ref().map(|gpu| &gpu.sample_view)
    }

    /// The GPU memory the atlas holds, in bytes: its texture, the casters' uniform and their lists.
    pub(crate) fn gpu_bytes(&self) -> u64 {
        let texture = self.gpu.as_ref().map_or(0, |_| {
            u64::from(self.resolution) * u64::from(self.resolution) * 4 + UNIFORM_STRIDE * MAX_SHADOW_TILES as u64
        });
        texture + self.see.gpu_bytes() + self.list_buffer.as_ref().map_or(0, wgpu::Buffer::size)
    }

    /// Changes whenever [`ShadowAtlas::sample_view`] does.
    pub(crate) fn revision(&self) -> u64 {
        self.revision ^ (self.see.revision() << 32)
    }

    /// The see-through maps' views (#491): the nearest see-through casters' depth and the light they let through.
    pub(crate) fn see_through_views(&self) -> (&wgpu::TextureView, &wgpu::TextureView) {
        (self.see.depth_view(), self.see.light_view())
    }

    /// The see-through maps to draw into this frame (depth, light), when a tile drawn this frame has see-through
    /// casters (#491).
    pub(crate) fn see_through_targets(&self) -> Option<(&wgpu::TextureView, &wgpu::TextureView)> {
        let any = self.ranges.iter().any(|range| range.5);
        (any && self.see.ready()).then(|| self.see.targets(0)).flatten()
    }

    /// One texel in atlas coordinates.
    pub(crate) fn texel(&self) -> f32 {
        1.0 / self.resolution as f32
    }

    /// What the frame did.
    pub(crate) fn stats(&self) -> AtlasStats {
        self.stats
    }

    /// The render target of the atlas pass, when tiles are drawn this frame.
    pub(crate) fn target(&self) -> Option<&wgpu::TextureView> {
        self.gpu
            .as_ref()
            .filter(|_| !self.jobs.is_empty())
            .map(|gpu| &gpu.render_view)
    }

    /// Decides which drawn lights hold tiles this frame and moves their shadows' fades by `dt` seconds. `drawn` is
    /// the frame's drawn lights (indices into `lights`, most important first). Tiles of lights that are gone or lost
    /// them are freed; lights out of view or out of the budget keep theirs until another light needs the room.
    pub(crate) fn assign(
        &mut self,
        lights: &[SceneLight],
        runtime: &mut [LightRuntime],
        drawn: &[u32],
        settings: &LightSettings,
        dt: f32,
    ) {
        // A new atlas size made since the last frame: its layout is in effect for this frame's assignment.
        self.poll_incoming();
        // Who still holds what: a light holds its tiles while the atlas says it owns them.
        self.holder.fill(FREE);
        self.active.fill(false);
        for (index, (light, state)) in (0u32..).zip(lights.iter().zip(runtime.iter_mut())) {
            let n = usize::from(state.tile_count);
            if n == 0 {
                continue;
            }
            let owned = state.tiles[..n]
                .iter()
                .all(|&t| self.owner.get(usize::from(t)).copied().flatten() == Some(light.id));
            if owned && n == light.shadow_tiles() {
                for &t in &state.tiles[..n] {
                    self.holder[usize::from(t)] = index;
                }
            } else {
                state.tile_count = 0;
                // A spot turned across the angle where it takes a cube rather than one tile (#437) keeps its shadow
                // while it takes its new tiles, drawn this frame: from nothing it would vanish and fade back each
                // time (#451).
                if !owned {
                    state.shadow_fade = 0.0;
                }
            }
        }
        for (tile, owner) in self.owner.iter_mut().enumerate() {
            if owner.is_some() && self.holder[tile] == FREE {
                *owner = None;
                self.caches[tile].valid = false;
            }
        }

        // The shadowed lights: the most important drawn lights in the budget with `Shadows` on.
        let wanted = if settings.enabled && settings.shadows {
            settings.shadowed_lights as usize
        } else {
            0
        };
        self.targets.clear();
        let mut chosen = 0;
        for &index in drawn {
            let target = chosen < wanted
                && lights.get(index as usize).is_some_and(|l| l.shadows)
                && runtime.get(index as usize).is_some_and(|s| s.in_budget);
            chosen += usize::from(target);
            self.targets.push(target);
        }
        let step = if settings.fade_time > 0.0 {
            (dt / settings.fade_time).max(0.0)
        } else {
            1.0
        };
        // Lights that keep their tiles: their shadows fade towards whether they are shadowed.
        for (&index, &target) in drawn.iter().zip(&self.targets) {
            let Some(state) = runtime.get_mut(index as usize) else {
                continue;
            };
            if state.tile_count == 0 {
                continue;
            }
            state.shadow_fade = if state.fresh && target {
                1.0
            } else if target {
                (state.shadow_fade + step).min(1.0)
            } else {
                (state.shadow_fade - step).max(0.0)
            };
            if state.shadow_fade > 0.0 || target {
                for &t in &state.tiles[..usize::from(state.tile_count)] {
                    self.active[usize::from(t)] = true;
                }
            }
        }
        // Lights that need tiles: free ones first, then those of lights not drawn with a shadow.
        for (&index, &target) in drawn.iter().zip(&self.targets) {
            let Some(light) = lights.get(index as usize) else {
                continue;
            };
            if !target || runtime[index as usize].tile_count > 0 {
                continue;
            }
            let need = light.shadow_tiles();
            let mut got = [0u16; 6];
            let mut n = 0;
            for tile in 0..self.capacity {
                if n == need {
                    break;
                }
                if self.owner[tile].is_none() {
                    got[n] = tile as u16;
                    n += 1;
                }
            }
            let mut tile = 0;
            while n < need && tile < self.capacity {
                let holder = self.holder[tile];
                if self.owner[tile].is_some() && !self.active[tile] && holder != FREE {
                    // Take back every tile of that light.
                    if let Some(parked) = runtime.get_mut(holder as usize) {
                        for &t in &parked.tiles[..usize::from(parked.tile_count)] {
                            self.owner[usize::from(t)] = None;
                            self.holder[usize::from(t)] = FREE;
                            self.caches[usize::from(t)].valid = false;
                        }
                        parked.tile_count = 0;
                        parked.shadow_fade = 0.0;
                    }
                    // Free now: start the search again from here.
                    continue;
                }
                if self.owner[tile].is_none() && !got[..n].contains(&(tile as u16)) {
                    got[n] = tile as u16;
                    n += 1;
                }
                tile += 1;
            }
            if n < need {
                // The atlas is full of shadows being drawn: this light waits for room.
                continue;
            }
            let state = &mut runtime[index as usize];
            state.tiles = got;
            state.tile_count = need as u8;
            state.shadow_fade = if state.fresh {
                1.0
            } else {
                state.shadow_fade.max(step).min(1.0)
            };
            for &t in &got[..need] {
                self.owner[usize::from(t)] = Some(light.id);
                self.holder[usize::from(t)] = index;
                self.active[usize::from(t)] = true;
                self.caches[usize::from(t)].valid = false;
            }
        }
    }

    /// Works out this frame's tiles for `shadowed` (the drawn lights holding tiles with some shadow, each with its
    /// first entry in the tile table written to `bases`), decides which tiles to draw again, culls their casters in
    /// `scene` and uploads their lists, matrices and the tile table (into `table_buffer`, which exists whenever there
    /// are drawn lights). `camera_offset` is the camera − the scene origin, and `lod` the camera view's levels of
    /// detail, which the casters' follow ([`lod_distance`]). The atlas's memory is made here, the first time `shadowed`
    /// is not empty.
    pub(crate) fn prepare(
        &mut self,
        scene: &mut GpuScene,
        shadowed: &[ShadowedLight],
        bases: &mut [f32],
        (camera_offset, lod): (Vec3, &LodView),
        table_buffer: Option<&wgpu::Buffer>,
    ) {
        self.jobs.clear();
        self.table.clear();
        self.list.clear();
        self.ranges.clear();
        self.stats = AtlasStats::default();
        self.table_tiles.clear();
        self.poll_incoming();
        // The see-through maps (#491), at the atlas's size, once the scene has a see-through part while see-through
        // shadows are drawn. When they arrive every tile is drawn again into them.
        if self.see_through && self.gpu.is_some() && !scene.scene().transparent_list().is_empty() {
            self.see.want(self.resolution, 1);
        }
        if self.see.poll() {
            for cache in &mut self.caches {
                cache.valid = false;
            }
            self.see_tiles.fill(false);
        }
        if !shadowed.is_empty() && !self.ensure_gpu() {
            // The atlas is still being made: its lights are drawn without shadows this frame.
            bases.fill(-1.0);
            return;
        }
        let to_camera = Mat4::from_translation(camera_offset);
        let tile_uv = self.tile as f32 / self.resolution as f32;
        {
            let instances = scene.scene();
            let epoch = instances.shadow_epoch();
            let revision = instances.revision();
            if !shadowed.is_empty() && self.dynamic_revision != Some(revision) {
                collect_moving_casters(instances, &mut self.dynamic);
                self.dynamic_revision = Some(revision);
            }
            let changes = instances.shadow_changes();
            // Every kept drawing, of a light drawn this frame or not, is dropped when the scene was rebased or rebuilt
            // or a static caster changed inside it: a light that comes back finds it drawn again.
            for cache in self.caches.iter_mut().filter(|c| c.valid) {
                if cache.epoch != epoch {
                    cache.valid = false;
                } else if !changes.is_empty() {
                    let frustum = Frustum::from_clip(cache.clip);
                    cache.valid = !changes.iter().any(|&change| cache.touches(&frustum, change));
                }
            }
            let origin = instances.origin();
            let mut views = [TileView {
                clip: Mat4::IDENTITY,
                eye: Vec3::ZERO,
                forward: Vec3::NEG_Z,
                texel_angle: 0.0,
                depth_scale: 0.0,
            }; 6];
            for (shadowed, base) in shadowed.iter().zip(bases.iter_mut()) {
                let light = &shadowed.light;
                let n = tile_views(light, origin, self.tile, &mut views).min(usize::from(shadowed.tile_count));
                if n == 0 || self.table.len() + n > MAX_SHADOW_TILES {
                    *base = -1.0;
                    continue;
                }
                *base = self.table.len() as f32;
                self.stats.shadowed += 1;
                let host = light
                    .host
                    .and_then(|part| instances.slot_of(part))
                    .map(|slot| entry(slot.region, slot.slot));
                let reach = light.range;
                let center = light.position - origin;
                // Each caster by its own distance from the light plus the camera's (#614), rather than by the farthest
                // the light reaches: with a long Range that drew a ball near the camera as a four- or eight-sided one.
                // Kept from the light's first tile while it is still at least the distance and not much more, so a
                // camera moving about a step's edge does not draw the tiles again each time it crosses it.
                let distance = center.distance(camera_offset);
                let kept = shadowed
                    .tiles
                    .first()
                    .and_then(|&tile| self.caches.get(usize::from(tile)))
                    .filter(|cache| cache.valid && cache.key.light == (light.id.index(), light.id.generation()))
                    .map(|cache| f32::from_bits(cache.key.near))
                    .filter(|&near| near >= distance && near <= distance * 1.5);
                let near = kept.unwrap_or_else(|| light_distance(distance));
                let tile_lod = lod
                    .for_shadow(lod_distance(center.distance(camera_offset), reach))
                    .around(center, near);
                for (face, view) in views.iter().take(n).enumerate() {
                    let tile = shadowed.tiles[face];
                    let (x, y) = (u32::from(tile) % self.per_row, u32::from(tile) / self.per_row);
                    self.table.push(GpuTile {
                        clip: (view.clip * to_camera).to_cols_array_2d(),
                        rect: [
                            (x * self.tile) as f32 / self.resolution as f32,
                            (y * self.tile) as f32 / self.resolution as f32,
                            tile_uv,
                            0.0,
                        ],
                        params: [view.depth_scale, view.texel_angle, 0.0, 0.0],
                    });
                    self.table_tiles.push(tile);
                    self.stats.tiles += 1;
                    let key = TileKey::new(light.id, face, view, &tile_lod);
                    let Some(cache) = self.caches.get(usize::from(tile)) else {
                        continue;
                    };
                    let redraw = !cache.valid
                        || cache.key != key
                        || (cache.revision != revision && {
                            // The scene changed: a moving caster it held may have moved, or one may have come in.
                            let frustum = Frustum::from_clip(view.clip);
                            cache.dynamic || self.dynamic.iter().any(|&s| cache.touches(&frustum, s))
                        });
                    if redraw {
                        self.jobs.push(Job {
                            tile,
                            key,
                            view: *view,
                            lod: tile_lod,
                            host,
                            center,
                            reach,
                        });
                    } else if let Some(cache) = self.caches.get_mut(usize::from(tile)) {
                        // Still right as of now.
                        cache.revision = revision;
                    }
                }
            }
        }
        if !self.jobs.is_empty() {
            self.cull_jobs(scene);
            self.upload_lists(scene);
        }
        // Which tiles' see-through maps the parts read (#491): what each tile held when it was last drawn.
        for (entry, &tile) in self.table.iter_mut().zip(&self.table_tiles) {
            let held = self.see_tiles.get(usize::from(tile)).copied().unwrap_or(false);
            entry.params[2] = if held { 1.0 } else { 0.0 };
        }
        if let Some(table_buffer) = table_buffer
            && self.table != self.uploaded_table
            && !self.table.is_empty()
        {
            self.queue
                .write_buffer(table_buffer, 0, bytemuck::cast_slice(&self.table));
            self.uploaded_table.clear();
            self.uploaded_table.extend_from_slice(&self.table);
        }
    }

    /// Forgets what the tile table on the GPU holds (a new table buffer), so the next frame writes it.
    pub(crate) fn table_replaced(&mut self) {
        self.uploaded_table.clear();
    }

    /// Culls every job's casters into the frame's list and notes what each tile now holds.
    fn cull_jobs(&mut self, scene: &mut GpuScene) {
        let start = Instant::now();
        let (epoch, revision) = (scene.scene().shadow_epoch(), scene.scene().revision());
        for (index, job) in self.jobs.iter().enumerate() {
            let view = CullView {
                lod: job.lod,
                ..CullView::from_frustum(Frustum::from_clip(job.view.clip), job.view.eye, job.view.forward)
            };
            scene.cull_casters(
                &view.with_see_through(self.see_through),
                Casters::All,
                &mut self.visible,
            );
            let mut dynamic = false;
            for kind in 0..self.visible.opaque_count() {
                for level in 0..MAX_LOD_LEVELS {
                    let first = self.list.len();
                    for &e in self.visible.opaque(MeshKind(kind as u16), level) {
                        if Some(e) == job.host {
                            continue;
                        }
                        dynamic |= e & DYNAMIC_ENTRY != 0;
                        self.list.push(e);
                    }
                    let count = self.list.len() - first;
                    if count > 0 {
                        self.ranges.push((
                            index as u16,
                            kind as u16,
                            level as u8,
                            first as u32,
                            count as u32,
                            false,
                        ));
                    }
                }
            }
            // The see-through casters after the solid ones (#122), one draw per kind and level.
            let instances = scene.scene();
            self.see_through_casters
                .gather(&self.visible, instances, usize::MAX, job.host);
            let base = self.list.len() as u32;
            for e in self.see_through_casters.entries() {
                dynamic |= e & DYNAMIC_ENTRY != 0;
                self.list.push(e);
            }
            for run in self.see_through_casters.runs() {
                self.ranges
                    .push((index as u16, run.kind.0, run.level, base + run.first, run.count, true));
            }
            let holds = !self.see_through_casters.is_empty() && self.see.ready();
            if let Some(held) = self.see_tiles.get_mut(usize::from(job.tile)) {
                *held = holds;
            }
            if let Some(cache) = self.caches.get_mut(usize::from(job.tile)) {
                *cache = TileCache {
                    valid: true,
                    key: job.key,
                    epoch,
                    revision,
                    dynamic,
                    clip: job.view.clip,
                    center: job.center,
                    reach: job.reach,
                };
            }
            if let Some(slot) = self.caster_data.get_mut(usize::from(job.tile)) {
                slot[..16].copy_from_slice(&job.view.clip.to_cols_array());
            }
        }
        self.stats.redrawn = self.jobs.len() as u32;
        self.stats.casters = self.list.len() as u32;
        self.stats.cull_time = start.elapsed();
    }

    /// Uploads the frame's caster lists (slot entries, or copies of the instances on the vertex-buffer fallback) and
    /// the drawn tiles' matrices.
    fn upload_lists(&mut self, scene: &GpuScene) {
        let bytes: &[u8] = match self.binding {
            InstanceBinding::Storage => bytemuck::cast_slice(&self.list),
            InstanceBinding::Vertex => {
                self.gathered.clear();
                let instances = scene.scene();
                self.gathered.extend(
                    self.list
                        .iter()
                        .map(|&e| *instances.entry_instance(e).unwrap_or(&GpuInstance::FREE)),
                );
                bytemuck::cast_slice(&self.gathered)
            }
        };
        let size = (bytes.len() as u64).next_power_of_two().max(MIN_LIST_BYTES);
        if self.list_buffer.as_ref().is_none_or(|b| b.size() < bytes.len() as u64) {
            self.list_buffer = Some(list_buffer(&self.device, self.binding, size));
        }
        if let Some(list) = self.list_buffer.as_ref()
            && !bytes.is_empty()
        {
            self.queue.write_buffer(list, 0, bytes);
        }
        if let Some(gpu) = self.gpu.as_ref() {
            self.queue
                .write_buffer(&gpu.caster_uniform, 0, bytemuck::cast_slice(&self.caster_data));
        }
        self.refresh_instance_group(scene);
    }

    /// Makes the storage path's instance bind group again when a buffer it uses was replaced.
    fn refresh_instance_group(&mut self, scene: &GpuScene) {
        let (Some(layout), Some(list)) = (self.instance_layout.as_ref(), self.list_buffer.as_ref()) else {
            return;
        };
        let buffers = [scene.buffer(Region::Static), scene.buffer(Region::Dynamic), list];
        if self
            .instance_group
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
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("metallic shadow atlas instances"),
            layout,
            entries: &[entry(0, buffers[0]), entry(1, buffers[1]), entry(2, buffers[2])],
        });
        self.instance_group = Some(InstanceGroup {
            buffers: buffers.map(Clone::clone),
            group,
        });
    }

    /// Records the frame's tiles into `pass` (a depth pass into [`ShadowAtlas::target`] that loads the atlas): each
    /// tile is cleared and its casters drawn, one instanced draw per mesh kind and level of detail. Returns the draws,
    /// instances and triangles.
    pub(crate) fn draw(&self, meshes: &MeshPool, pass: &mut wgpu::RenderPass<'_>) -> (u32, u32, u64) {
        self.draw_tiles(meshes, pass, TilePass::Solid)
    }

    /// Records the tiles drawn this frame that have see-through casters into the see-through maps (#491): their depth
    /// (`light` false, a depth pass into the see-through depth map that loads it) or the light they let through
    /// (`light` true, a colour pass into the see-through light map that loads it). Each tile is cleared first.
    pub(crate) fn draw_see_through(
        &self,
        light: bool,
        meshes: &MeshPool,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> (u32, u32, u64) {
        self.draw_tiles(meshes, pass, if light { TilePass::Light } else { TilePass::Depth })
    }

    fn draw_tiles(&self, meshes: &MeshPool, pass: &mut wgpu::RenderPass<'_>, which: TilePass) -> (u32, u32, u64) {
        let (mut draws, mut instances, mut triangles) = (0, 0, 0u64);
        let (Some(gpu), Some(list)) = (self.gpu.as_ref(), self.list_buffer.as_ref()) else {
            return (draws, instances, triangles);
        };
        let see_through = which != TilePass::Solid;
        // The see-through maps are smaller than the atlas by a whole factor: a tile is the same share of them.
        let scale = |v: u32| {
            if see_through {
                (u64::from(v) * u64::from(self.see_resolution()) / u64::from(self.resolution.max(1))) as u32
            } else {
                v
            }
        };
        let (clear, caster) = match which {
            TilePass::Solid | TilePass::Depth => (&self.clear, &self.caster),
            TilePass::Light => (&self.see_clear, &self.see_through_pipeline),
        };
        let mut ranges = self.ranges.iter().peekable();
        for (index, job) in self.jobs.iter().enumerate() {
            let tile = u32::from(job.tile);
            let mine = |range: &&(u16, u16, u8, u32, u32, bool)| usize::from(range.0) == index;
            if see_through && !self.ranges.iter().any(|range| mine(&range) && range.5) {
                while ranges.next_if(mine).is_some() {}
                continue;
            }
            let (x, y) = ((tile % self.per_row) * self.tile, (tile / self.per_row) * self.tile);
            let (x, y, side) = (scale(x), scale(y), scale(self.tile));
            pass.set_viewport(x as f32, y as f32, side as f32, side as f32, 0.0, 1.0);
            pass.set_scissor_rect(x, y, side, side);
            pass.set_pipeline(clear);
            pass.draw(0..3, 0..1);
            pass.set_pipeline(caster);
            pass.set_bind_group(0, &gpu.caster_group, &[(u64::from(tile) * UNIFORM_STRIDE) as u32]);
            pass.set_vertex_buffer(0, meshes.vertices().slice(..));
            pass.set_index_buffer(meshes.indices().slice(..), wgpu::IndexFormat::Uint32);
            match self.binding {
                InstanceBinding::Storage => {
                    if let Some(group) = self.instance_group.as_ref() {
                        pass.set_bind_group(1, &group.group, &[]);
                    }
                }
                InstanceBinding::Vertex => pass.set_vertex_buffer(1, list.slice(..)),
            }
            while let Some(&&(job_index, kind, level, first, count, see)) = ranges.peek() {
                if usize::from(job_index) != index {
                    break;
                }
                ranges.next();
                // The atlas holds the solid casters; the see-through maps the see-through ones (#491).
                if see != see_through {
                    continue;
                }
                let mesh = meshes.range(MeshKind(kind), usize::from(level));
                if mesh.index_count == 0 {
                    continue;
                }
                pass.draw_indexed(
                    mesh.first_index..mesh.first_index + mesh.index_count,
                    0,
                    first..first + count,
                );
                draws += 1;
                instances += count;
                triangles += u64::from(count) * u64::from(mesh.index_count / 3);
            }
        }
        (draws, instances, triangles)
    }
}

/// Which maps [`ShadowAtlas::draw_tiles`] draws into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TilePass {
    /// The atlas: the solid casters' depth.
    Solid,
    /// The see-through depth map (#491).
    Depth,
    /// The see-through light map (#491).
    Light,
}

impl ShadowAtlas {
    /// The see-through maps' resolution: half the atlas's ([`SeeThroughMaps::want`]).
    fn see_resolution(&self) -> u32 {
        (self.resolution / 2).max(16)
    }
}

/// The bounding spheres of the casters in the dynamic region (origin-relative) into `out`.
fn collect_moving_casters(instances: &InstanceScene, out: &mut Vec<(Vec3, f32)>) {
    out.clear();
    out.extend(
        instances
            .instances(Region::Dynamic)
            .iter()
            .filter(|i| is_shadow_caster(i))
            .map(bounding_sphere),
    );
}

/// The atlas's size, its tiles' size, tiles per row and tile count for `settings` on `device`.
fn layout(device: &wgpu::Device, settings: &LightSettings) -> Layout {
    let max = device.limits().max_texture_dimension_2d;
    let resolution = settings.atlas_resolution.clamp(64, max);
    let tile = settings.tile_resolution.clamp(32, resolution);
    let per_row = (resolution / tile).max(1);
    let capacity = ((per_row * per_row) as usize).min(MAX_SHADOW_TILES);
    (resolution, tile, per_row, capacity)
}

/// The atlas texture, `resolution` texels square.
fn atlas_spec(resolution: u32) -> TextureSpec {
    TextureSpec::new(
        "metallic shadow atlas",
        (resolution, resolution, 1),
        SHADOW_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    )
}

/// The atlas texture's views for sampling and for drawing.
fn atlas_views(texture: &wgpu::Texture) -> (wgpu::TextureView, wgpu::TextureView) {
    let view = |label| {
        in_heap_turn(|| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some(label),
                dimension: Some(wgpu::TextureViewDimension::D2),
                ..wgpu::TextureViewDescriptor::default()
            })
        })
    };
    (view("metallic shadow atlas"), view("metallic shadow atlas target"))
}

fn list_buffer(device: &wgpu::Device, binding: InstanceBinding, size: u64) -> wgpu::Buffer {
    let usage = match binding {
        InstanceBinding::Storage => wgpu::BufferUsages::STORAGE,
        InstanceBinding::Vertex => wgpu::BufferUsages::VERTEX,
    } | wgpu::BufferUsages::COPY_DST;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("metallic shadow atlas casters"),
        size,
        usage,
        mapped_at_creation: false,
    })
}

/// Clears a tile: a full-viewport triangle at depth 1, written whatever is there.
fn clear_pipeline(device: &wgpu::Device, cache: Option<&wgpu::PipelineCache>) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("metallic shadow atlas clear"),
        source: wgpu::ShaderSource::Wgsl(
            "@vertex fn vs_clear(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
                return vec4<f32>(uv * 2.0 - 1.0, 1.0, 1.0);
            }"
            .into(),
        ),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("metallic shadow atlas clear"),
        bind_group_layouts: &[],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("metallic shadow atlas clear"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_clear"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: SHADOW_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: None,
        multiview_mask: None,
        cache,
    })
}

#[cfg(test)]
mod tests {
    use metallic_core::Color3;

    use super::*;
    use crate::scene::LightKind;

    /// #614: a light's tiles take a ball next to the light by about the light's distance from the camera, not by the
    /// farthest the light reaches (which drew it as a four- or eight-sided ball under a long-reaching light), and never
    /// finer than the camera draws it; the distance moves in steps of a fifth or so.
    #[test]
    fn a_light_s_tiles_take_a_ball_beside_it_by_the_light_s_distance() {
        use crate::lod::LodSettings;

        let camera = LodView::for_camera(500.0, &LodSettings::MEDIUM);
        let (eye, light) = (Vec3::new(0.0, 4.0, 14.0), Vec3::new(4.0, 3.0, 3.0));
        let ball = (Vec3::new(0.0, 2.0, 0.0), 2.0 * 3f32.sqrt());
        let class = crate::lod::lod_class(crate::scene::MeshKind::BALL);
        let reach = 150.0;
        let old = camera.for_shadow(lod_distance(light.distance(eye), reach));
        let near = light_distance(light.distance(eye));
        let tile = old.around(light, near);
        let (was, now) = (
            old.choose_caster(class, ball.1, ball.0),
            tile.choose_caster(class, ball.1, ball.0),
        );
        let drawn = camera.choose(class, ball.1 * ball.1, (ball.0 - eye).length_squared(), 0);
        assert!(was >= 2 && now < was, "old {was}, now {now}");
        assert!(now >= drawn, "never finer than the camera draws it: {now} < {drawn}");
        // Never less than the distance, and the same over small moves.
        for d in [0.3f32, 1.0, 7.5, 12.0, 100.0] {
            assert!(light_distance(d) >= d && light_distance(d) < d * 1.2, "{d}");
        }
        assert_eq!(light_distance(12.0), light_distance(12.5));
    }

    fn point(at: Vec3) -> SceneLight {
        SceneLight {
            id: Id::new(3, 1),
            kind: LightKind::Point,
            position: at,
            direction: Vec3::NEG_Z,
            color: Color3::WHITE,
            brightness: 1.0,
            intensity: Vec3::ONE,
            range: 12.0,
            angle: 0.0,
            enabled: true,
            shadows: true,
            host: None,
        }
    }

    fn views(light: &SceneLight, origin: Vec3) -> ([TileView; 6], usize) {
        let mut out = [TileView {
            clip: Mat4::IDENTITY,
            eye: Vec3::ZERO,
            forward: Vec3::NEG_Z,
            texel_angle: 0.0,
            depth_scale: 0.0,
        }; 6];
        let n = tile_views(light, origin, 256, &mut out);
        (out, n)
    }

    /// The face the shader picks for a direction from the light (the major axis).
    fn face_of(d: Vec3) -> usize {
        let a = d.abs();
        if a.x >= a.y && a.x >= a.z {
            if d.x > 0.0 { 0 } else { 1 }
        } else if a.y >= a.z {
            if d.y > 0.0 { 2 } else { 3 }
        } else if d.z > 0.0 {
            4
        } else {
            5
        }
    }

    /// Every point within a point light's range lands inside the face the shader picks for it, away from the tile's
    /// edge by the border, with a depth between 0 and 1.
    #[test]
    fn each_point_lands_in_the_face_its_major_axis_picks() {
        let origin = Vec3::new(64.0, 0.0, 0.0);
        let light = point(Vec3::new(70.0, 3.0, -2.0));
        let (views, n) = views(&light, origin);
        assert_eq!(n, 6);
        let inner = 1.0 - 2.0 * BORDER / 256.0;
        for i in 0..4000 {
            let t = i as f32 / 4000.0;
            let dir = Vec3::new((t * 91.0).sin(), (t * 53.0).cos(), (t * 29.0).sin()).normalize_or(Vec3::X);
            let p = light.position + dir * (0.2 + 11.5 * (t * 7.0).fract());
            let face = face_of(dir);
            let ndc = views[face].clip.project_point3(p - origin);
            assert!(
                ndc.x.abs() <= inner + 1e-4 && ndc.y.abs() <= inner + 1e-4,
                "{p} in face {face} at {ndc}"
            );
            assert!((0.0..=1.0).contains(&ndc.z), "depth {}", ndc.z);
        }
    }

    #[test]
    fn a_spot_tile_covers_its_cone() {
        let origin = Vec3::ZERO;
        let spot = SceneLight {
            kind: LightKind::Spot,
            angle: 60.0,
            direction: Vec3::new(0.0, -1.0, 0.3).normalize(),
            ..point(Vec3::new(1.0, 9.0, 2.0))
        };
        let (views, n) = views(&spot, origin);
        assert_eq!(n, 1);
        let half = 30f32.to_radians();
        let (side, _) = spot.direction.any_orthonormal_pair();
        for k in 0..64 {
            let turn = metallic_core::glam::Quat::from_axis_angle(spot.direction, k as f32 * 0.1);
            let edge = turn * (spot.direction * half.cos() + side * half.sin());
            let ndc = views[0].clip.project_point3(spot.position + edge * 8.0);
            assert!(
                ndc.x.abs() < 1.0 && ndc.y.abs() < 1.0 && (0.0..=1.0).contains(&ndc.z),
                "{ndc}"
            );
        }
    }

    /// #480: a spot's shadow has a cube face's texels per degree at any Angle, through the switch to a cube.
    #[test]
    fn a_spot_shadow_s_texels_do_not_change_with_its_angle() {
        let origin = Vec3::ZERO;
        let light = point(Vec3::new(1.0, 9.0, 2.0));
        let face = views(&light, origin).0[0].texel_angle;
        for angle in [1.0, 30.0, 60.0, 90.0, 91.0, 120.0, 149.0, 150.0, 151.0, 180.0] {
            let spot = SceneLight {
                kind: LightKind::Spot,
                angle,
                direction: Vec3::new(0.0, -1.0, 0.3).normalize(),
                ..light
            };
            let (tiles, n) = views(&spot, origin);
            for tile in &tiles[..n] {
                assert!(
                    (tile.texel_angle - face).abs() < 1e-7,
                    "{angle}°: {} against {face}",
                    tile.texel_angle
                );
            }
        }
    }
}
