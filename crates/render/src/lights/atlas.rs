//! The local lights' shadow atlas: who holds which tiles, which tiles are drawn this frame, their casters, and the
//! pass that draws them (see the [module documentation](super)).

use std::time::{Duration, Instant};

use bytemuck::{Pod, Zeroable};
use metallic_core::Id;
use metallic_core::glam::{Mat4, Vec3, Vec4};

use super::{LightRuntime, LightSettings, MAX_SHADOW_TILES};
use crate::GpuScene;
use crate::cull::{Casters, CullView, Frustum, VisibleSet};
use crate::parts::MeshPool;
use crate::scene::{
    DYNAMIC_ENTRY, GpuInstance, InstanceBinding, InstanceScene, LightKind, MeshKind, Region, SceneLight,
    bounding_sphere, entry, is_shadow_caster,
};
use crate::shadow::{SHADOW_FORMAT, caster_pipeline};

/// Texels kept clear around a tile's face, so the filter's taps near its edge stay inside the tile.
const BORDER: f32 = 2.0;
/// The near plane of every tile's projection, in metres.
const NEAR: f32 = 0.05;
/// A spot light's shadow covers at most this half-angle (a 150° cone); a wider cone is unshadowed outside it.
pub(crate) const MAX_SPOT_SHADOW_HALF_DEGREES: f32 = 75.0;
/// Each tile's caster uniform sits at a multiple of this (the strictest dynamic offset alignment there is).
const UNIFORM_STRIDE: u64 = 256;
/// The bytes of a tile's caster uniform the shader reads: one matrix.
const UNIFORM_SIZE: u64 = 64;
/// The fewest bytes the caster list buffer has.
const MIN_LIST_BYTES: u64 = 4096;
/// No light holds the tile this frame.
const FREE: u32 = u32::MAX;

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

/// The views of a shadowed `light`'s tiles of `resolution` texels, origin-relative to `origin`: a point light's six
/// cube faces, each a little wider than 90° by [`BORDER`] texels, or a spot's cone (up to
/// [`MAX_SPOT_SHADOW_HALF_DEGREES`]). Returns how many it wrote.
pub(crate) fn tile_views(light: &SceneLight, origin: Vec3, resolution: u32, out: &mut [TileView; 6]) -> usize {
    let resolution = resolution.max(16) as f32;
    let widen = resolution / (resolution - 2.0 * BORDER);
    let eye = light.position - origin;
    let far = light.range.max(NEAR * 2.0);
    let depth_scale = NEAR * far / (far - NEAR);
    let view = |forward: Vec3, up: Vec3, tan_half: f32| {
        TileView {
            clip: perspective(tan_half, NEAR, far) * look_to(eye, forward, up),
            eye,
            forward,
            texel_angle: 2.0 * tan_half / resolution,
            depth_scale,
        }
    };
    match light.kind {
        LightKind::Point => {
            for (slot, &(forward, up)) in out.iter_mut().zip(&FACES) {
                *slot = view(forward, up, widen);
            }
            6
        }
        LightKind::Spot => {
            let half = (light.angle * 0.5).min(MAX_SPOT_SHADOW_HALF_DEGREES).to_radians();
            let forward = light.direction.normalize_or(Vec3::NEG_Z);
            let up = if forward.y.abs() < 0.99 { Vec3::Y } else { Vec3::Z };
            out[0] = view(forward, up, half.tan() * widen);
            1
        }
    }
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

/// What a tile's drawing was made for: its light, face and view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TileKey {
    light: (u32, u32),
    face: u8,
    clip: [u32; 16],
}

impl TileKey {
    fn new(light: Id, face: usize, view: &TileView) -> Self {
        Self {
            light: (light.index(), light.generation()),
            face: face as u8,
            clip: view.clip.to_cols_array().map(f32::to_bits),
        }
    }
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

/// The shadow atlas and everything that draws it. [`super::LocalLights`] owns one.
#[derive(Debug)]
pub(crate) struct ShadowAtlas {
    device: wgpu::Device,
    queue: wgpu::Queue,
    binding: InstanceBinding,
    resolution: u32,
    tile: u32,
    per_row: u32,
    /// Tiles in the atlas.
    capacity: usize,
    sample_view: wgpu::TextureView,
    render_view: wgpu::TextureView,
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
    /// This frame's caster entries, every job's in turn, and the draws over them: (job, kind, first, count).
    list: Vec<u32>,
    gathered: Vec<GpuInstance>,
    ranges: Vec<(u16, u16, u32, u32)>,
    list_buffer: wgpu::Buffer,
    visible: VisibleSet,
    /// The moving casters' bounding spheres, origin-relative, as of `dynamic_revision`.
    dynamic: Vec<(Vec3, f32)>,
    dynamic_revision: Option<u64>,
    caster_uniform: wgpu::Buffer,
    caster_data: Vec<[f32; (UNIFORM_STRIDE / 4) as usize]>,
    caster_group: wgpu::BindGroup,
    caster: wgpu::RenderPipeline,
    clear: wgpu::RenderPipeline,
    instance_layout: Option<wgpu::BindGroupLayout>,
    instance_group: Option<InstanceGroup>,
    /// The tiles of the lights drawn this frame, as the parts read them.
    table: Vec<GpuTile>,
    table_buffer: wgpu::Buffer,
    uploaded_table: Vec<GpuTile>,
    stats: AtlasStats,
}

impl ShadowAtlas {
    /// An atlas for `settings` on the instance path `binding`, whose storage-path instance group has
    /// `instance_layout`.
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        binding: InstanceBinding,
        instance_layout: Option<&wgpu::BindGroupLayout>,
        settings: &LightSettings,
    ) -> Self {
        let caster_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
        });
        let caster_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic shadow atlas tiles (casters)"),
            size: UNIFORM_STRIDE * MAX_SHADOW_TILES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let caster_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("metallic shadow atlas tile"),
            layout: &caster_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &caster_uniform,
                    offset: 0,
                    size: wgpu::BufferSize::new(UNIFORM_SIZE),
                }),
            }],
        });
        let table_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic shadow atlas tiles"),
            size: (size_of::<GpuTile>() * MAX_SHADOW_TILES) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (resolution, tile, per_row, capacity) = layout(device, settings);
        let (sample_view, render_view) = atlas_texture(device, resolution);
        Self {
            device: device.clone(),
            queue: queue.clone(),
            binding,
            resolution,
            tile,
            per_row,
            capacity,
            sample_view,
            render_view,
            owner: vec![None; capacity],
            holder: vec![FREE; capacity],
            active: vec![false; capacity],
            caches: vec![TileCache::default(); capacity],
            targets: Vec::new(),
            jobs: Vec::with_capacity(MAX_SHADOW_TILES),
            list: Vec::new(),
            gathered: Vec::new(),
            ranges: Vec::new(),
            list_buffer: list_buffer(device, binding, MIN_LIST_BYTES),
            visible: VisibleSet::new(),
            dynamic: Vec::new(),
            dynamic_revision: None,
            caster_uniform,
            caster_data: vec![[0.0; (UNIFORM_STRIDE / 4) as usize]; MAX_SHADOW_TILES],
            caster_group,
            caster: caster_pipeline(device, binding, &caster_layout, instance_layout),
            clear: clear_pipeline(device),
            instance_layout: instance_layout.cloned(),
            instance_group: None,
            table: Vec::with_capacity(MAX_SHADOW_TILES),
            table_buffer,
            uploaded_table: Vec::with_capacity(MAX_SHADOW_TILES),
            stats: AtlasStats::default(),
        }
    }

    /// Applies `settings`' atlas and tile size, making the atlas again when they changed (a settings change, never per
    /// frame). Returns whether the sampled view was replaced. Every tile is drawn again after a change.
    pub(crate) fn configure(&mut self, settings: &LightSettings) -> bool {
        let (resolution, tile, per_row, capacity) = layout(&self.device, settings);
        let replaced = resolution != self.resolution;
        if replaced {
            (self.sample_view, self.render_view) = atlas_texture(&self.device, resolution);
        }
        self.resolution = resolution;
        self.tile = tile;
        self.per_row = per_row;
        self.capacity = capacity;
        // Every light's tiles are given back (their owners no longer match) and drawn again.
        self.owner.clear();
        self.owner.resize(capacity, None);
        self.holder.resize(capacity, FREE);
        self.active.resize(capacity, false);
        self.caches.clear();
        self.caches.resize(capacity, TileCache::default());
        replaced
    }

    /// The atlas the parts sample.
    pub(crate) fn sample_view(&self) -> &wgpu::TextureView {
        &self.sample_view
    }

    /// The tiles' uniform the parts read.
    pub(crate) fn table_buffer(&self) -> &wgpu::Buffer {
        &self.table_buffer
    }

    /// One texel in atlas coordinates.
    pub(crate) fn texel(&self) -> f32 {
        1.0 / self.resolution as f32
    }

    /// What the frame did.
    pub(crate) fn stats(&self) -> AtlasStats {
        self.stats
    }

    /// Whether any tile is drawn this frame.
    pub(crate) fn has_jobs(&self) -> bool {
        !self.jobs.is_empty()
    }

    /// The render target of the atlas pass.
    pub(crate) fn render_view(&self) -> &wgpu::TextureView {
        &self.render_view
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
        // Who still holds what: a light holds its tiles while the atlas says it owns them.
        self.holder.fill(FREE);
        self.active.fill(false);
        for (index, (light, state)) in (0u32..).zip(lights.iter().zip(runtime.iter_mut())) {
            let n = usize::from(state.tile_count);
            if n == 0 {
                continue;
            }
            let held = n == light.kind.shadow_tiles()
                && state.tiles[..n]
                    .iter()
                    .all(|&t| self.owner.get(usize::from(t)).copied().flatten() == Some(light.id));
            if held {
                for &t in &state.tiles[..n] {
                    self.holder[usize::from(t)] = index;
                }
            } else {
                state.tile_count = 0;
                state.shadow_fade = 0.0;
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
            let need = light.kind.shadow_tiles();
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
            state.shadow_fade = if state.fresh { 1.0 } else { step.min(1.0) };
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
    /// `scene` and uploads their lists, matrices and the tile table. `camera_offset` is the camera − the scene origin.
    pub(crate) fn prepare(
        &mut self,
        scene: &mut GpuScene,
        shadowed: &[ShadowedLight],
        bases: &mut [f32],
        camera_offset: Vec3,
    ) {
        self.jobs.clear();
        self.table.clear();
        self.list.clear();
        self.ranges.clear();
        self.stats = AtlasStats::default();
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
                    self.stats.tiles += 1;
                    let key = TileKey::new(light.id, face, view);
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
        if self.table != self.uploaded_table && !self.table.is_empty() {
            self.queue
                .write_buffer(&self.table_buffer, 0, bytemuck::cast_slice(&self.table));
            self.uploaded_table.clear();
            self.uploaded_table.extend_from_slice(&self.table);
        }
    }

    /// Culls every job's casters into the frame's list and notes what each tile now holds.
    fn cull_jobs(&mut self, scene: &mut GpuScene) {
        let start = Instant::now();
        let (epoch, revision) = (scene.scene().shadow_epoch(), scene.scene().revision());
        for (index, job) in self.jobs.iter().enumerate() {
            let view = CullView::from_frustum(Frustum::from_clip(job.view.clip), job.view.eye, job.view.forward);
            scene.cull_casters(&view, Casters::All, &mut self.visible);
            let mut dynamic = false;
            for kind in 0..self.visible.opaque_count() {
                let first = self.list.len();
                for &e in self.visible.opaque(MeshKind(kind as u16)) {
                    if Some(e) == job.host {
                        continue;
                    }
                    dynamic |= e & DYNAMIC_ENTRY != 0;
                    self.list.push(e);
                }
                let count = self.list.len() - first;
                if count > 0 {
                    self.ranges.push((index as u16, kind as u16, first as u32, count as u32));
                }
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
        if bytes.len() as u64 > self.list_buffer.size() {
            let size = (bytes.len() as u64).next_power_of_two().max(MIN_LIST_BYTES);
            self.list_buffer = list_buffer(&self.device, self.binding, size);
        }
        if !bytes.is_empty() {
            self.queue.write_buffer(&self.list_buffer, 0, bytes);
        }
        self.queue
            .write_buffer(&self.caster_uniform, 0, bytemuck::cast_slice(&self.caster_data));
        self.refresh_instance_group(scene);
    }

    /// Makes the storage path's instance bind group again when a buffer it uses was replaced.
    fn refresh_instance_group(&mut self, scene: &GpuScene) {
        let Some(layout) = self.instance_layout.as_ref() else {
            return;
        };
        let buffers = [
            scene.buffer(Region::Static),
            scene.buffer(Region::Dynamic),
            &self.list_buffer,
        ];
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

    /// Records the frame's tiles into `pass` (a depth pass into [`ShadowAtlas::render_view`] that loads the atlas):
    /// each tile is cleared and its casters drawn, one instanced draw per mesh kind. Returns the draws, instances and
    /// triangles.
    pub(crate) fn draw(&self, meshes: &MeshPool, pass: &mut wgpu::RenderPass<'_>) -> (u32, u32, u64) {
        let (mut draws, mut instances, mut triangles) = (0, 0, 0u64);
        let mut ranges = self.ranges.iter().peekable();
        for (index, job) in self.jobs.iter().enumerate() {
            let tile = u32::from(job.tile);
            let (x, y) = ((tile % self.per_row) * self.tile, (tile / self.per_row) * self.tile);
            pass.set_viewport(x as f32, y as f32, self.tile as f32, self.tile as f32, 0.0, 1.0);
            pass.set_scissor_rect(x, y, self.tile, self.tile);
            pass.set_pipeline(&self.clear);
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&self.caster);
            pass.set_bind_group(0, &self.caster_group, &[(u64::from(tile) * UNIFORM_STRIDE) as u32]);
            pass.set_vertex_buffer(0, meshes.vertices().slice(..));
            pass.set_index_buffer(meshes.indices().slice(..), wgpu::IndexFormat::Uint32);
            match self.binding {
                InstanceBinding::Storage => {
                    if let Some(group) = self.instance_group.as_ref() {
                        pass.set_bind_group(1, &group.group, &[]);
                    }
                }
                InstanceBinding::Vertex => pass.set_vertex_buffer(1, self.list_buffer.slice(..)),
            }
            while let Some(&&(job_index, kind, first, count)) = ranges.peek() {
                if usize::from(job_index) != index {
                    break;
                }
                ranges.next();
                let mesh = meshes.range(MeshKind(kind));
                if mesh.index_count == 0 {
                    continue;
                }
                pass.draw_indexed(mesh.first_index..mesh.first_index + mesh.index_count, 0, first..first + count);
                draws += 1;
                instances += count;
                triangles += u64::from(count) * u64::from(mesh.index_count / 3);
            }
        }
        (draws, instances, triangles)
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
fn layout(device: &wgpu::Device, settings: &LightSettings) -> (u32, u32, u32, usize) {
    let max = device.limits().max_texture_dimension_2d;
    let resolution = settings.atlas_resolution.clamp(64, max);
    let tile = settings.tile_resolution.clamp(32, resolution);
    let per_row = (resolution / tile).max(1);
    let capacity = ((per_row * per_row) as usize).min(MAX_SHADOW_TILES);
    (resolution, tile, per_row, capacity)
}

/// The atlas texture: its view for sampling and for drawing.
fn atlas_texture(device: &wgpu::Device, resolution: u32) -> (wgpu::TextureView, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("metallic shadow atlas"),
        size: wgpu::Extent3d {
            width: resolution,
            height: resolution,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SHADOW_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = |label| {
        texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some(label),
            dimension: Some(wgpu::TextureViewDimension::D2),
            ..wgpu::TextureViewDescriptor::default()
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
fn clear_pipeline(device: &wgpu::Device) -> wgpu::RenderPipeline {
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
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use metallic_core::Color3;

    use super::*;

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
            assert!(ndc.x.abs() < 1.0 && ndc.y.abs() < 1.0 && (0.0..=1.0).contains(&ndc.z), "{ndc}");
        }
        // The texel's size grows with the cone.
        let wide = SceneLight { angle: 120.0, ..spot };
        assert!(self::views(&wide, origin).0[0].texel_angle > views[0].texel_angle);
    }
}
