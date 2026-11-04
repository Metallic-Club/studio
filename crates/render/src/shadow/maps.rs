//! The GPU side of the sun's shadows: the cascade maps, the cached cascades' static maps and their copy, the
//! casters' lists and the caster pipeline (see the [module documentation](super)).

use std::time::{Duration, Instant};

use metallic_core::glam::{Mat4, Vec3};

use super::cascade::{CascadeCache, CascadeFit, SceneState, fit_cascades};
use super::{CascadeStats, MAX_CASCADES, MAX_RECEIVER_OFFSET, ShadowSettings, ShadowStats};
use crate::cull::{Casters, VisibleSet};
use crate::lod::{self, LodView, MAX_LOD_LEVELS};
use crate::parts::{Camera, MeshPool, MeshVertex};
use crate::scene::{GpuInstance, InstanceBinding, MeshKind, Region, bounding_sphere, entry, is_shadow_caster};
use crate::{GpuScene, InstanceScene};

/// The maps' format: 32-bit float depth, standard (0 at the light's end, cleared to 1).
pub(crate) const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Each cascade's uniform sits at a multiple of this (the strictest uniform offset alignment there is).
const UNIFORM_STRIDE: u64 = 256;
/// The bytes of a cascade's uniform that the shader reads: one matrix.
const UNIFORM_SIZE: u64 = 64;

/// The rasterizer's depth bias on the cascades' casters: 3 texels of the caster's depth slope, a texel past the 2
/// the default filter's taps reach from a point (see [`ShadowSettings::depth_bias`]). No constant term: its unit
/// depends on the depth's exponent, so the receivers' depth bias, in metres, does that part. No clamp: a clamp
/// needs `DownlevelFlags::DEPTH_BIAS_CLAMP`, which GLES does not have, and shadows must look the same everywhere.
const CASCADE_CASTER_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: 0,
    slope_scale: 3.0,
    clamp: 0.0,
};

/// The fewest entries a cascade's segment of a mesh kind holds.
const MIN_LIST_CAPACITY: u32 = 256;

/// The uniform of every cascade, each at [`UNIFORM_STRIDE`] (`Cascade` in shadow.wgsl).
type CascadeUniforms = [[f32; (UNIFORM_STRIDE / 4) as usize]; MAX_CASCADES];

/// What the parts' frame uniform needs of the shadows (the `shadow_*` members of `Frame` in frame.wgsl).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FrameShadow {
    /// Camera-relative positions to each cascade's clip space.
    pub(crate) matrices: [Mat4; MAX_CASCADES],
    /// The view depth each cascade ends at.
    pub(crate) splits: [f32; MAX_CASCADES],
    /// How far a receiving point moves along its normal, in metres, per cascade (the shader scales it by 1 − N·L).
    pub(crate) offsets: [f32; MAX_CASCADES],
    /// The receiving depth's bias, in depth units, per cascade.
    pub(crate) biases: [f32; MAX_CASCADES],
    /// Cascades in use (0: no shadows), the filter's reach in texels, one texel in map coordinates, and the view
    /// depth the shadows start fading out at.
    pub(crate) params: [f32; 4],
}

/// One cascade: where it is, what its map was drawn for, and its casters.
#[derive(Debug, Default)]
struct CascadeState {
    fit: CascadeFit,
    cache: CascadeCache,
    visible: VisibleSet,
    /// Its casters are drawn this frame: into its map, or for a cached cascade into its static map.
    redraw: bool,
    casters: u32,
    /// A cached cascade's static map was drawn this many times (so a map built from it can tell it is current).
    drawings: u64,
    /// A cached cascade's moving casters this frame, and how many.
    moving: VisibleSet,
    moving_count: u32,
    /// A cached cascade's map is built this frame: its static map copied in, the moving casters drawn on top.
    build: bool,
    /// What a cached cascade's map holds, `None` when not known (never built, or made again).
    built: Option<Built>,
}

/// What a cached cascade's map was built from: its static map's drawing ([`CascadeState::drawings`]), and the scene
/// revision of the moving casters drawn on top (`None` when there were none, so the map is the static map alone).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Built {
    drawing: u64,
    moving: Option<u64>,
}

/// A moving caster (in the dynamic region), as the cached cascades test it: its entry, its mesh kind (and its class
/// of levels of detail) and its bounding sphere, origin-relative.
#[derive(Clone, Copy, Debug)]
struct Mover {
    entry: u32,
    kind: u16,
    class: u8,
    center: Vec3,
    radius: f32,
}

/// A cached cascade's static map: its own depth texture, which the cascade's map is built from (see the
/// [module documentation](super)), and the bind group the copy samples it through.
#[derive(Debug)]
struct StaticMap {
    target: wgpu::TextureView,
    source: wgpu::BindGroup,
}

/// The cascades' caster lists on the GPU: for each cascade a block, and in it a segment per mesh kind, as the
/// camera's lists are laid out ([`crate::GpuScene::list_buffer`]).
#[derive(Debug)]
struct ShadowLists {
    buffer: wgpu::Buffer,
    staging: wgpu::Buffer,
    /// 4 (entries) or 64 (copies of the instances, on the vertex-buffer fallback).
    entry_size: u64,
    gather: bool,
    /// Per mesh kind, the entries its segment holds and where it starts in a cascade's block.
    capacities: Vec<u32>,
    offsets: Vec<u32>,
    /// Entries in a cascade's block.
    stride: u32,
    /// Changes whenever the lists are laid out again (every cached map is drawn again then).
    revision: u64,
    /// This frame's uploads: list, kind, count. Reused.
    ranges: Vec<(Block, usize, u32)>,
}

/// A list of casters a frame uploads: cascade `i`'s casters, or cached cascade `i`'s moving casters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Block {
    Casters(usize),
    Moving(usize),
}

/// The storage path's instance bind group for the caster pass, and the buffers it was made for.
#[derive(Debug)]
struct InstanceGroup {
    buffers: [wgpu::Buffer; 3],
    group: wgpu::BindGroup,
}

/// The sun's cascade maps and everything that draws them. [`crate::parts::PartRenderer`] owns one.
#[derive(Debug)]
pub(crate) struct ShadowMaps {
    device: wgpu::Device,
    queue: wgpu::Queue,
    binding: InstanceBinding,
    settings: ShadowSettings,
    /// Cascades the maps have room for (0 when off).
    count: usize,
    resolution: u32,
    /// The fewest layers the texture gets: 2 on OpenGL, where a one-layer texture cannot be viewed as an array.
    min_layers: u32,
    max_resolution: u32,
    sample_view: wgpu::TextureView,
    layer_views: Vec<wgpu::TextureView>,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    uniform_data: Option<CascadeUniforms>,
    cascade_groups: Vec<wgpu::BindGroup>,
    instance_layout: Option<wgpu::BindGroupLayout>,
    instance_group: Option<InstanceGroup>,
    pipeline: wgpu::RenderPipeline,
    /// The cached cascades' static maps, the first cached cascade's first (empty when every cascade is dynamic).
    static_maps: Vec<StaticMap>,
    copy_layout: wgpu::BindGroupLayout,
    copy_pipeline: wgpu::RenderPipeline,
    /// The moving casters as of `movers_revision` (the scene's revision), for the cached cascades.
    movers: Vec<Mover>,
    movers_revision: Option<u64>,
    lists: ShadowLists,
    cascades: [CascadeState; MAX_CASCADES],
    /// Cascades in use this frame.
    active: usize,
    /// The camera's levels of detail the maps were drawn for: when they change, every map is drawn again.
    lod: LodView,
    frame: FrameShadow,
    stats: ShadowStats,
}

impl ShadowMaps {
    /// The maps (off until [`ShadowMaps::configure`]) and the caster pipeline for the instance path `binding`, whose
    /// storage-path instance bind group has `instance_layout`.
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        binding: InstanceBinding,
        instance_layout: Option<&wgpu::BindGroupLayout>,
    ) -> Self {
        let cascade_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("metallic shadow cascade"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(UNIFORM_SIZE),
                },
                count: None,
            }],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic shadow cascades"),
            size: UNIFORM_STRIDE * MAX_CASCADES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cascade_groups = (0..MAX_CASCADES as u64)
            .map(|i| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("metallic shadow cascade"),
                    layout: &cascade_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &uniform,
                            offset: i * UNIFORM_STRIDE,
                            size: wgpu::BufferSize::new(UNIFORM_SIZE),
                        }),
                    }],
                })
            })
            .collect();
        let pipeline = caster_pipeline(device, binding, &cascade_layout, instance_layout, CASCADE_CASTER_BIAS);
        let entry_size = match binding {
            InstanceBinding::Storage => size_of::<u32>() as u64,
            InstanceBinding::Vertex => GpuInstance::SIZE,
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("metallic shadow comparison"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            // Lit where the receiving depth is at most the map's: nearer the light than the nearest caster.
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..wgpu::SamplerDescriptor::default()
        });
        let limits = device.limits();
        let (sample_view, layer_views) = maps(device, 1, 1, 1);
        let (copy_layout, copy_pipeline) = copy_pipeline(device);
        Self {
            device: device.clone(),
            queue: queue.clone(),
            binding,
            settings: ShadowSettings::OFF,
            count: 0,
            resolution: 1,
            min_layers: 1,
            max_resolution: limits.max_texture_dimension_2d,
            sample_view,
            layer_views,
            sampler,
            uniform,
            uniform_data: None,
            cascade_groups,
            instance_layout: instance_layout.cloned(),
            instance_group: None,
            pipeline,
            static_maps: Vec::new(),
            copy_layout,
            copy_pipeline,
            movers: Vec::new(),
            movers_revision: None,
            lists: ShadowLists {
                buffer: list_buffer(device, binding, MIN_LIST_CAPACITY as u64 * entry_size),
                staging: staging_buffer(device, MIN_LIST_CAPACITY as u64 * entry_size),
                entry_size,
                gather: binding == InstanceBinding::Vertex,
                capacities: Vec::new(),
                offsets: Vec::new(),
                stride: 0,
                revision: 0,
                ranges: Vec::new(),
            },
            cascades: Default::default(),
            active: 0,
            lod: LodView::OFF,
            frame: FrameShadow::default(),
            stats: ShadowStats::default(),
        }
    }

    /// Applies `settings`, making the maps again when their size or count changed (a settings change, never per
    /// frame). `min_layers` is 2 on OpenGL (see [`ShadowMaps`]). Returns whether the sampled view was replaced, so
    /// the bind groups that hold it must be made again. Every map is drawn again after a change.
    pub(crate) fn configure(&mut self, settings: ShadowSettings, min_layers: u32) -> bool {
        let limits = self.device.limits();
        let count = settings.cascade_count().min(limits.max_texture_array_layers as usize);
        let resolution = if count == 0 {
            1
        } else {
            settings.resolution.clamp(16, self.max_resolution)
        };
        let replaced = count != self.count || resolution != self.resolution || min_layers != self.min_layers;
        if replaced {
            let layers = (count as u32).max(1).max(min_layers);
            (self.sample_view, self.layer_views) = maps(&self.device, resolution, layers, count.max(1));
            self.count = count;
            self.resolution = resolution;
            self.min_layers = min_layers;
        }
        self.settings = ShadowSettings { resolution, ..settings };
        let cached = count.saturating_sub(self.settings.dynamic_count());
        if replaced || cached != self.static_maps.len() {
            self.static_maps = (0..cached)
                .map(|_| static_map(&self.device, &self.copy_layout, resolution))
                .collect();
            // The lists' blocks are per cascade and per cached cascade's moving casters: lay them out again.
            self.lists.capacities.clear();
        }
        for cascade in &mut self.cascades {
            cascade.cache = CascadeCache::default();
            cascade.built = None;
        }
        replaced
    }

    /// The settings in effect (the resolution clamped to the device).
    pub(crate) fn settings(&self) -> &ShadowSettings {
        &self.settings
    }

    /// Every map, as one depth array the parts sample.
    pub(crate) fn sample_view(&self) -> &wgpu::TextureView {
        &self.sample_view
    }

    /// The comparison sampler the parts sample the maps with.
    pub(crate) fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// What the frame uniform needs of this frame's shadows.
    pub(crate) fn frame(&self) -> &FrameShadow {
        &self.frame
    }

    /// What this frame did.
    pub(crate) fn stats(&self) -> ShadowStats {
        self.stats
    }

    /// Where cascade `cascade`'s casters are drawn ([`ShadowMaps::draw`]) when they are this frame: its map, or a
    /// cached cascade's static map.
    pub(crate) fn caster_target(&self, cascade: usize) -> Option<&wgpu::TextureView> {
        if cascade >= self.active || !self.cascades[cascade].redraw {
            return None;
        }
        match cascade.checked_sub(self.settings.dynamic_count()) {
            Some(cached) => self.static_maps.get(cached).map(|map| &map.target),
            None => self.layer_views.get(cascade),
        }
    }

    /// Cached cascade `cascade`'s map when it is built this frame ([`ShadowMaps::build`]).
    pub(crate) fn build_target(&self, cascade: usize) -> Option<&wgpu::TextureView> {
        if cascade < self.active && self.cascades[cascade].build {
            self.layer_views.get(cascade)
        } else {
            None
        }
    }

    /// Fits the cascades to `camera` for a light towards `light` (`None` when there is none, or no shadows are
    /// wanted), decides which maps to draw again, culls their casters (at levels of detail that follow `lod`, the
    /// camera view's) and records the upload of their lists into `encoder`. Run it after [`GpuScene::upload_view`]
    /// (the coalesce it runs is what the culls see), every frame: it also takes the scene's shadow-caster changes
    /// ([`crate::InstanceScene::clear_shadow_changes`]).
    pub(crate) fn prepare(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        scene: &mut GpuScene,
        camera: Option<&Camera>,
        aspect: f32,
        light: Option<Vec3>,
        lod: &LodView,
    ) {
        if *lod != self.lod {
            // The casters' levels follow the camera's: a map drawn for other levels could be finer than what now
            // receives it.
            self.lod = *lod;
            for cascade in &mut self.cascades {
                cascade.cache = CascadeCache::default();
            }
        }
        for cascade in &mut self.cascades {
            cascade.redraw = false;
            cascade.build = false;
            cascade.moving_count = 0;
        }
        self.stats = ShadowStats::default();
        self.frame = FrameShadow::default();
        let drawn = camera
            .zip(light)
            .filter(|_| self.count > 0 && scene.binding() == self.binding);
        let Some((camera, light)) = drawn else {
            // Nothing is drawn, so nothing kept can be trusted afterwards either.
            self.active = 0;
            for cascade in &mut self.cascades {
                cascade.cache = CascadeCache::default();
            }
            scene.clear_shadow_changes();
            return;
        };
        let origin = scene.scene().origin();
        let mut fits = [CascadeFit::default(); MAX_CASCADES];
        let count = fit_cascades(camera, aspect, origin, light, &self.settings, lod, &mut fits).min(self.count);
        self.active = count;
        self.layout_lists(scene);
        let dynamic = self.settings.dynamic_count();
        {
            let instances = scene.scene();
            let state = SceneState {
                epoch: instances.shadow_epoch(),
                revision: instances.revision(),
                layout: self.lists.revision,
                changes: instances.shadow_changes(),
            };
            for (i, (cascade, fit)) in self.cascades.iter_mut().zip(&fits).enumerate() {
                if i >= count {
                    cascade.cache = CascadeCache::default();
                    continue;
                }
                cascade.fit = *fit;
                cascade.redraw = cascade.cache.needs_redraw(fit, i < dynamic, &state);
                if cascade.redraw {
                    cascade.cache = CascadeCache::drawn(fit, &state);
                }
            }
        }
        scene.clear_shadow_changes();

        let mut cull_time = Duration::ZERO;
        for (i, cascade) in self.cascades.iter_mut().enumerate().take(count) {
            if cascade.redraw {
                let casters = if i < dynamic { Casters::All } else { Casters::Static };
                let stats = scene.cull_casters(&cascade.fit.cull_view(), casters, &mut cascade.visible);
                cull_time += stats.cpu_time;
                cascade.casters = stats.visible;
                cascade.drawings += 1;
            }
        }
        if count > dynamic {
            let start = Instant::now();
            self.place_movers(scene.scene(), dynamic, count);
            cull_time += start.elapsed();
        }
        self.upload_lists(encoder, scene);
        self.refresh_instance_group(scene);
        self.write_uniform();

        let offset = camera.position - origin;
        let to_camera = Mat4::from_translation(offset);
        let settings = &self.settings;
        let mut frame = FrameShadow {
            params: [
                count as f32,
                settings.filter_radius.min(2) as f32,
                1.0 / self.resolution as f32,
                settings.distance * 0.9,
            ],
            ..FrameShadow::default()
        };
        for (i, cascade) in self.cascades.iter().enumerate().take(count) {
            let fit = &cascade.fit;
            frame.matrices[i] = fit.clip * to_camera;
            frame.splits[i] = fit.split_far;
            frame.offsets[i] = settings.normal_offset.max(0.0) * fit.texel;
            frame.biases[i] = (settings.depth_bias.max(0.0) * fit.texel).min(MAX_RECEIVER_OFFSET) / fit.depth_range;
            self.stats.cascade[i] = CascadeStats {
                casters: cascade.casters,
                redrawn: cascade.redraw,
                cached: i >= dynamic,
                moving: cascade.moving_count,
                built: cascade.build,
            };
        }
        self.frame = frame;
        self.stats.cascades = count as u32;
        self.stats.cull_time = cull_time;
    }

    /// Finds the moving casters in each cached cascade (`dynamic..count`), and decides which cached cascades' maps
    /// are built this frame: those whose static map was drawn again, and those whose moving casters changed (moved,
    /// came or left). A map that holds its static map alone, and still has no moving caster, is kept.
    ///
    /// The moving casters are the dynamic region's, few next to the scene: each is tested against each cached
    /// cascade's volume, without the culling grid, whose walk would cost the whole static region every frame.
    fn place_movers(&mut self, instances: &InstanceScene, dynamic: usize, count: usize) {
        let revision = instances.revision();
        if self.movers_revision != Some(revision) {
            self.movers.clear();
            let region = instances.instances(Region::Dynamic);
            self.movers.extend(
                (0u32..)
                    .zip(region)
                    .filter(|(_, instance)| is_shadow_caster(instance))
                    .map(|(slot, instance)| {
                        let (center, radius) = bounding_sphere(instance);
                        Mover {
                            entry: entry(Region::Dynamic, slot),
                            kind: instance.mesh().0,
                            class: lod::lod_class(instance.mesh()),
                            center,
                            radius,
                        }
                    }),
            );
            self.movers_revision = Some(revision);
        }
        let kinds = self.lists.capacities.len();
        for cascade in self.cascades.iter_mut().take(count).skip(dynamic) {
            let set = &mut cascade.moving;
            let levels = cascade.fit.lod;
            // Every kind's list has room for every mover, so the pushes never allocate.
            set.begin(std::iter::repeat_n(self.movers.len(), kinds), 0, 0, levels.enabled());
            let mut moving = 0u32;
            for mover in &self.movers {
                if usize::from(mover.kind) < kinds && cascade.fit.frustum.contains_sphere(mover.center, mover.radius) {
                    // At the level the cascade's static casters take (crate::lod).
                    let level = if mover.class != 0 && levels.enabled() {
                        levels.choose_caster(mover.class, mover.radius)
                    } else {
                        0
                    };
                    set.push_level(mover.entry, mover.kind, level);
                    moving += 1;
                }
            }
            cascade.moving_count = moving;
            let built = Built {
                drawing: cascade.drawings,
                moving: (moving > 0).then_some(revision),
            };
            cascade.build = cascade.built != Some(built);
            cascade.built = Some(built);
        }
    }

    /// The list block holding cached cascade `cascade`'s moving casters: after every cascade's own block.
    fn moving_block(&self, cascade: usize) -> usize {
        self.count + cascade.saturating_sub(self.settings.dynamic_count())
    }

    /// Lays the cascades' lists out again when a kind's live list outgrew its segment (or a kind appeared).
    fn layout_lists(&mut self, scene: &GpuScene) {
        let instances = scene.scene();
        let kinds = instances.draw_list_count();
        let lists = &mut self.lists;
        let outgrown = lists.capacities.len() < kinds
            || (0..kinds).any(|kind| {
                let len = instances.draw_list(MeshKind(kind as u16)).len();
                len > lists.capacities[kind] as usize
            });
        if !outgrown {
            return;
        }
        lists.capacities.resize(kinds, 0);
        lists.offsets.resize(kinds, 0);
        let mut first = 0u32;
        for kind in 0..kinds {
            let len = u32::try_from(instances.draw_list(MeshKind(kind as u16)).len()).unwrap_or(u32::MAX);
            let capacity = lists.capacities[kind]
                .max(len.checked_next_power_of_two().unwrap_or(u32::MAX))
                .max(MIN_LIST_CAPACITY);
            lists.capacities[kind] = capacity;
            lists.offsets[kind] = first;
            first = first.saturating_add(capacity);
        }
        lists.stride = first;
        // A block per cascade, then one per cached cascade for its moving casters.
        let blocks = self.count.max(1) + self.static_maps.len();
        let needed = u64::from(first) * blocks as u64 * lists.entry_size;
        if lists.buffer.size() < needed {
            lists.buffer = list_buffer(&self.device, self.binding, needed);
        }
        lists.revision += 1;
    }

    /// Uploads the lists of the cascades drawn this frame and of the moving casters of the cached cascades built
    /// this frame: one staging write, then a copy per list and kind (its levels back to back).
    fn upload_lists(&mut self, encoder: &mut wgpu::CommandEncoder, scene: &GpuScene) {
        let lists = &mut self.lists;
        lists.ranges.clear();
        let mut total = 0u64;
        for (index, cascade) in self.cascades.iter().enumerate().take(self.active) {
            let sets = [
                (cascade.redraw, &cascade.visible, Block::Casters(index)),
                (cascade.build, &cascade.moving, Block::Moving(index)),
            ];
            for (drawn, set, block) in sets {
                if !drawn {
                    continue;
                }
                for (kind, &capacity) in lists.capacities.iter().enumerate() {
                    let count = level_counts(set, MeshKind(kind as u16), capacity).iter().sum::<u32>();
                    if count > 0 {
                        lists.ranges.push((block, kind, count));
                        total += u64::from(count);
                    }
                }
            }
        }
        let Some(size) = wgpu::BufferSize::new(total * lists.entry_size) else {
            return;
        };
        if lists.staging.size() < size.get() {
            lists.staging = staging_buffer(&self.device, size.get().next_power_of_two());
        }
        let Some(mut view) = self.queue.write_buffer_with(&lists.staging, 0, size) else {
            return;
        };
        let instances = scene.scene();
        let mut offset = 0usize;
        for &(block, kind, _) in &lists.ranges {
            let set = match block {
                Block::Casters(index) => &self.cascades[index].visible,
                Block::Moving(index) => &self.cascades[index].moving,
            };
            let kind = MeshKind(kind as u16);
            let capacity = lists.capacities.get(usize::from(kind.0)).copied().unwrap_or(0);
            for (level, n) in level_counts(set, kind, capacity).into_iter().enumerate() {
                let entries = &set.opaque(kind, level)[..n as usize];
                if lists.gather {
                    for &entry in entries {
                        let instance = instances.entry_instance(entry).unwrap_or(&GpuInstance::FREE);
                        let bytes = bytemuck::bytes_of(instance);
                        view.slice(offset..offset + bytes.len()).copy_from_slice(bytes);
                        offset += bytes.len();
                    }
                } else {
                    let bytes: &[u8] = bytemuck::cast_slice(entries);
                    view.slice(offset..offset + bytes.len()).copy_from_slice(bytes);
                    offset += bytes.len();
                }
            }
        }
        drop(view);
        let mut source = 0u64;
        for &(block, kind, count) in &self.lists.ranges {
            let index = match block {
                Block::Casters(index) => index,
                Block::Moving(index) => self.moving_block(index),
            };
            let lists = &self.lists;
            let first = index as u64 * u64::from(lists.stride) + u64::from(lists.offsets[kind]);
            let bytes = u64::from(count) * lists.entry_size;
            encoder.copy_buffer_to_buffer(&lists.staging, source, &lists.buffer, first * lists.entry_size, bytes);
            source += bytes;
        }
    }

    /// Makes the storage path's instance bind group again when a buffer it uses was replaced.
    fn refresh_instance_group(&mut self, scene: &GpuScene) {
        let Some(layout) = self.instance_layout.as_ref() else {
            return;
        };
        let buffers = [
            scene.buffer(Region::Static),
            scene.buffer(Region::Dynamic),
            &self.lists.buffer,
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
            label: Some("metallic shadow instances"),
            layout,
            entries: &[entry(0, buffers[0]), entry(1, buffers[1]), entry(2, buffers[2])],
        });
        self.instance_group = Some(InstanceGroup {
            buffers: buffers.map(Clone::clone),
            group,
        });
    }

    /// Writes the cascades' matrices when any changed: one write for all of them.
    fn write_uniform(&mut self) {
        let mut data: CascadeUniforms = [[0.0; (UNIFORM_STRIDE / 4) as usize]; MAX_CASCADES];
        for (slot, cascade) in data.iter_mut().zip(&self.cascades).take(self.active) {
            slot[..16].copy_from_slice(&cascade.fit.clip.to_cols_array());
        }
        if self.uniform_data != Some(data) {
            self.queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&data));
            self.uniform_data = Some(data);
        }
    }

    /// Records cascade `cascade`'s casters into `pass` (a depth pass into its [`ShadowMaps::caster_target`]): one
    /// instanced draw per mesh kind and level of detail. Returns the draws, instances and triangles.
    pub(crate) fn draw(&self, cascade: usize, meshes: &MeshPool, pass: &mut wgpu::RenderPass<'_>) -> (u32, u32, u64) {
        let Some(state) = self.cascades.get(cascade) else {
            return (0, 0, 0);
        };
        self.draw_list(cascade, cascade, &state.visible, meshes, pass)
    }

    /// Records the building of cached cascade `cascade`'s map into `pass` (a depth pass into its
    /// [`ShadowMaps::build_target`]): a copy of its static map (a full-screen triangle writing each texel's depth),
    /// then its moving casters on top, one instanced draw per mesh kind. Returns the casters' draws, instances and
    /// triangles (the copy is not counted).
    pub(crate) fn build(&self, cascade: usize, meshes: &MeshPool, pass: &mut wgpu::RenderPass<'_>) -> (u32, u32, u64) {
        let static_map = cascade
            .checked_sub(self.settings.dynamic_count())
            .and_then(|cached| self.static_maps.get(cached));
        let (Some(state), Some(static_map)) = (self.cascades.get(cascade), static_map) else {
            return (0, 0, 0);
        };
        pass.set_pipeline(&self.copy_pipeline);
        pass.set_bind_group(0, &static_map.source, &[]);
        pass.draw(0..3, 0..1);
        if state.moving_count == 0 {
            return (0, 0, 0);
        }
        self.draw_list(cascade, self.moving_block(cascade), &state.moving, meshes, pass)
    }

    /// Records the casters `set` holds, uploaded to list block `block`, seen from cascade `cascade`.
    fn draw_list(
        &self,
        cascade: usize,
        block: usize,
        set: &VisibleSet,
        meshes: &MeshPool,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> (u32, u32, u64) {
        let Some(group) = self.cascade_groups.get(cascade) else {
            return (0, 0, 0);
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.set_vertex_buffer(0, meshes.vertices().slice(..));
        pass.set_index_buffer(meshes.indices().slice(..), wgpu::IndexFormat::Uint32);
        match self.binding {
            InstanceBinding::Storage => {
                if let Some(group) = self.instance_group.as_ref() {
                    pass.set_bind_group(1, &group.group, &[]);
                }
            }
            InstanceBinding::Vertex => pass.set_vertex_buffer(1, self.lists.buffer.slice(..)),
        }
        let (mut draws, mut instances, mut triangles) = (0, 0, 0);
        let block = block as u32 * self.lists.stride;
        for (kind, (&capacity, &offset)) in self.lists.capacities.iter().zip(&self.lists.offsets).enumerate() {
            let kind = MeshKind(kind as u16);
            let mut first = block + offset;
            for (level, count) in level_counts(set, kind, capacity).into_iter().enumerate() {
                let mesh = meshes.range(kind, level);
                let start = first;
                first += count;
                if count == 0 || mesh.index_count == 0 {
                    continue;
                }
                pass.draw_indexed(
                    mesh.first_index..mesh.first_index + mesh.index_count,
                    0,
                    start..start + count,
                );
                draws += 1;
                instances += count;
                triangles += u64::from(count) * u64::from(mesh.index_count / 3);
            }
        }
        (draws, instances, triangles)
    }
}

/// How many of `set`'s casters of `kind` a segment of `capacity` entries holds at each level: the levels back to back,
/// cut where the segment is full.
fn level_counts(set: &VisibleSet, kind: MeshKind, capacity: u32) -> [u32; MAX_LOD_LEVELS] {
    let mut room = capacity;
    std::array::from_fn(|level| {
        let n = u32::try_from(set.opaque(kind, level).len())
            .unwrap_or(u32::MAX)
            .min(room);
        room -= n;
        n
    })
}

/// The maps: a `resolution`² depth array of `layers` layers, its view for sampling and a render view of each of
/// the first `targets` layers.
fn maps(
    device: &wgpu::Device,
    resolution: u32,
    layers: u32,
    targets: usize,
) -> (wgpu::TextureView, Vec<wgpu::TextureView>) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("metallic shadow maps"),
        size: wgpu::Extent3d {
            width: resolution,
            height: resolution,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SHADOW_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let sample_view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("metallic shadow maps"),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..wgpu::TextureViewDescriptor::default()
    });
    let layer_views = (0..targets as u32)
        .map(|layer| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("metallic shadow cascade"),
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..wgpu::TextureViewDescriptor::default()
            })
        })
        .collect();
    (sample_view, layer_views)
}

/// A cached cascade's static map: a `resolution`² depth texture of its own (one layer, so every backend can sample
/// it as a plain 2D texture), and the bind group the copy into the cascade's map reads it through.
fn static_map(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, resolution: u32) -> StaticMap {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("metallic shadow static map"),
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
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("metallic shadow static map"),
        dimension: Some(wgpu::TextureViewDimension::D2),
        ..wgpu::TextureViewDescriptor::default()
    });
    let source = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("metallic shadow static map"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&view),
        }],
    });
    StaticMap { target: view, source }
}

/// The copy of a static map into a cascade's map: a full-screen triangle whose fragments write the depth they read
/// at their own texel (`frag_depth`), whatever is there. A depth blit rather than `copy_texture_to_texture`: GLES
/// cannot copy depth textures (wgpu's GL backend reads the source through a colour attachment), and a draw also
/// shares the pass that draws the moving casters on top, with no copy command or extra pass on any backend. The
/// static map is read as an unfilterable float texture (allowed for depth formats) with `textureLoad`, so no
/// comparison or filtering touches the values: they are copied exactly.
fn copy_pipeline(device: &wgpu::Device) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline) {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("metallic shadow static copy"),
        source: wgpu::ShaderSource::Wgsl(include_str!("copy.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("metallic shadow static copy"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("metallic shadow static copy"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("metallic shadow static copy"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_copy"),
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
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_copy"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[],
        }),
        multiview_mask: None,
        cache: None,
    });
    (layout, pipeline)
}

fn list_buffer(device: &wgpu::Device, binding: InstanceBinding, size: u64) -> wgpu::Buffer {
    let usage = match binding {
        InstanceBinding::Storage => wgpu::BufferUsages::STORAGE,
        InstanceBinding::Vertex => wgpu::BufferUsages::VERTEX,
    } | wgpu::BufferUsages::COPY_DST;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("metallic shadow caster lists"),
        size,
        usage,
        mapped_at_creation: false,
    })
}

fn staging_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("metallic shadow list uploads"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

/// The caster pipeline: depth only, **back faces culled**, so a map holds each caster's faces towards the light,
/// pushed away from the light by `bias`. Its group 0 is `cascade_layout`, one uniform matrix (the local lights' atlas
/// binds it with a dynamic offset per tile).
///
/// Front faces, not back faces: a back face is where a caster ends, and the bottom of a part resting on another is
/// a back face level with the surface under it. Stored, it put the shadow's start at the receiving surface's own
/// depth right where the parts touch, and the receivers' bias lit a band along every contact (#126). A front face is
/// a whole part's thickness nearer the light than anything it shades. Parts are closed solids, so every caster has
/// front faces.
pub(crate) fn caster_pipeline(
    device: &wgpu::Device,
    binding: InstanceBinding,
    cascade_layout: &wgpu::BindGroupLayout,
    instance_layout: Option<&wgpu::BindGroupLayout>,
    bias: wgpu::DepthBiasState,
) -> wgpu::RenderPipeline {
    let source = match binding {
        InstanceBinding::Storage => concat!(include_str!("shadow.wgsl"), include_str!("shadow_storage.wgsl")),
        InstanceBinding::Vertex => concat!(include_str!("shadow.wgsl"), include_str!("shadow_vertex.wgsl")),
    };
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("metallic shadow casters"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("metallic shadow casters"),
        bind_group_layouts: &[
            Some(cascade_layout),
            instance_layout.filter(|_| binding == InstanceBinding::Storage),
        ],
        immediate_size: 0,
    });
    let instance_attributes = GpuInstance::vertex_attributes(2);
    let all_buffers = [
        Some(MeshVertex::layout()),
        Some(wgpu::VertexBufferLayout {
            array_stride: GpuInstance::SIZE,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &instance_attributes,
        }),
    ];
    let buffers = match binding {
        InstanceBinding::Storage => &all_buffers[..1],
        InstanceBinding::Vertex => &all_buffers[..],
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("metallic shadow casters"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_shadow"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: SHADOW_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias,
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: None,
        multiview_mask: None,
        cache: None,
    })
}
