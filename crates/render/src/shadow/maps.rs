//! The GPU side of the sun's shadows: the cascade maps, the casters' lists and the caster pipeline (see the
//! [module documentation](super)).

use std::time::Duration;

use metallic_core::glam::{Mat4, Vec3};

use super::cascade::{CascadeCache, CascadeFit, SceneState, fit_cascades};
use super::{CascadeStats, MAX_CASCADES, ShadowSettings, ShadowStats};
use crate::GpuScene;
use crate::cull::{Casters, VisibleSet};
use crate::parts::{Camera, MeshPool, MeshVertex};
use crate::scene::{GpuInstance, InstanceBinding, MeshKind, Region};

/// The maps' format: 32-bit float depth, standard (0 at the light's end, cleared to 1).
pub(crate) const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Each cascade's uniform sits at a multiple of this (the strictest uniform offset alignment there is).
const UNIFORM_STRIDE: u64 = 256;
/// The bytes of a cascade's uniform that the shader reads: one matrix.
const UNIFORM_SIZE: u64 = 64;

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
    /// How far a receiving point moves along its normal, in metres, per cascade.
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
    redraw: bool,
    casters: u32,
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
    /// This frame's uploads: cascade, kind, count. Reused.
    ranges: Vec<(usize, usize, u32)>,
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
    lists: ShadowLists,
    cascades: [CascadeState; MAX_CASCADES],
    /// Cascades in use this frame.
    active: usize,
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
        let pipeline = caster_pipeline(device, binding, &cascade_layout, instance_layout);
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
            // The lists' blocks are per cascade: lay them out again.
            self.lists.capacities.clear();
        }
        self.settings = ShadowSettings { resolution, ..settings };
        for cascade in &mut self.cascades {
            cascade.cache = CascadeCache::default();
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

    /// Whether cascade `cascade` is drawn this frame.
    pub(crate) fn redraws(&self, cascade: usize) -> bool {
        cascade < self.active && self.cascades[cascade].redraw
    }

    /// The render target of cascade `cascade`'s map.
    pub(crate) fn layer_view(&self, cascade: usize) -> Option<&wgpu::TextureView> {
        self.layer_views.get(cascade)
    }

    /// Fits the cascades to `camera` for a light towards `light` (`lit` false when there is none, or no shadows are
    /// wanted), decides which maps to draw again, culls their casters and records the upload of their lists into
    /// `encoder`. Run it after [`GpuScene::upload_view`] (the coalesce it runs is what the culls see), every frame:
    /// it also takes the scene's shadow-caster changes ([`crate::InstanceScene::clear_shadow_changes`]).
    pub(crate) fn prepare(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        scene: &mut GpuScene,
        camera: Option<&Camera>,
        aspect: f32,
        light: Vec3,
        lit: bool,
    ) {
        for cascade in &mut self.cascades {
            cascade.redraw = false;
        }
        self.stats = ShadowStats::default();
        self.frame = FrameShadow::default();
        let camera = camera.filter(|_| lit && self.count > 0 && scene.binding() == self.binding);
        let Some(camera) = camera else {
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
        let count = fit_cascades(camera, aspect, origin, light, &self.settings, &mut fits).min(self.count);
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
            }
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
            frame.biases[i] = settings.depth_bias.max(0.0) * fit.texel / fit.depth_range;
            self.stats.cascade[i] = CascadeStats {
                casters: cascade.casters,
                redrawn: cascade.redraw,
                cached: i >= dynamic,
            };
        }
        self.frame = frame;
        self.stats.cascades = count as u32;
        self.stats.cull_time = cull_time;
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
        let needed = u64::from(first) * self.count.max(1) as u64 * lists.entry_size;
        if lists.buffer.size() < needed {
            lists.buffer = list_buffer(&self.device, self.binding, needed);
        }
        lists.revision += 1;
    }

    /// Uploads the lists of the cascades drawn this frame: one staging write, then a copy per cascade and kind.
    fn upload_lists(&mut self, encoder: &mut wgpu::CommandEncoder, scene: &GpuScene) {
        let lists = &mut self.lists;
        lists.ranges.clear();
        let mut total = 0u64;
        for (index, cascade) in self.cascades.iter().enumerate().take(self.active) {
            if !cascade.redraw {
                continue;
            }
            for (kind, &capacity) in lists.capacities.iter().enumerate() {
                let len = cascade.visible.opaque(MeshKind(kind as u16)).len();
                let count = u32::try_from(len).unwrap_or(u32::MAX).min(capacity);
                if count > 0 {
                    lists.ranges.push((index, kind, count));
                    total += u64::from(count);
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
        for &(index, kind, count) in &lists.ranges {
            let entries = &self.cascades[index].visible.opaque(MeshKind(kind as u16))[..count as usize];
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
        drop(view);
        let mut source = 0u64;
        for &(index, kind, count) in &lists.ranges {
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

    /// Records cascade `cascade`'s casters into `pass` (a depth pass into its [`ShadowMaps::layer_view`]): one
    /// instanced draw per mesh kind. Returns the draws, instances and triangles.
    pub(crate) fn draw(&self, cascade: usize, meshes: &MeshPool, pass: &mut wgpu::RenderPass<'_>) -> (u32, u32, u64) {
        let (Some(state), Some(group)) = (self.cascades.get(cascade), self.cascade_groups.get(cascade)) else {
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
        let block = cascade as u32 * self.lists.stride;
        for (kind, (&capacity, &offset)) in self.lists.capacities.iter().zip(&self.lists.offsets).enumerate() {
            let kind = MeshKind(kind as u16);
            let count = u32::try_from(state.visible.opaque(kind).len())
                .unwrap_or(u32::MAX)
                .min(capacity);
            let mesh = meshes.range(kind);
            if count == 0 || mesh.index_count == 0 {
                continue;
            }
            let first = block + offset;
            pass.draw_indexed(
                mesh.first_index..mesh.first_index + mesh.index_count,
                0,
                first..first + count,
            );
            draws += 1;
            instances += count;
            triangles += u64::from(count) * u64::from(mesh.index_count / 3);
        }
        (draws, instances, triangles)
    }
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

/// The caster pipeline: depth only, front faces culled (the casters' back faces are what the map holds, which
/// keeps lit surfaces clear of their own depth), with a slope-scaled bias for surfaces edge-on to the light. Its group
/// 0 is `cascade_layout`, one uniform matrix (the local lights' atlas binds it with a dynamic offset per tile).
pub(crate) fn caster_pipeline(
    device: &wgpu::Device,
    binding: InstanceBinding,
    cascade_layout: &wgpu::BindGroupLayout,
    instance_layout: Option<&wgpu::BindGroupLayout>,
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
            cull_mode: Some(wgpu::Face::Front),
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: SHADOW_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState {
                constant: 0,
                slope_scale: 1.0,
                clamp: 0.0,
            },
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: None,
        multiview_mask: None,
        cache: None,
    })
}
