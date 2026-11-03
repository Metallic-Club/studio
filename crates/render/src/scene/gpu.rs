//! [`GpuScene`]: the instance buffers on the GPU, and the uploads that keep them equal to [`InstanceScene`].

use metallic_core::glam::Vec3;
use metallic_core::{DataModel, Id};

use super::instance::{GpuInstance, MeshKind};
use super::sync::InstanceScene;
use super::{MAX_UPLOAD_GAP, Region, RenderComponents, SyncStats};
use crate::{RenderContext, RenderError};

/// The fewest slots a region's buffer has, so small scenes do not grow it frame after frame.
const MIN_CAPACITY: u32 = 1024;

/// Bit 31 of a scatter destination marks the dynamic region, so slots stay below it.
const DYNAMIC_BIT: u32 = 1 << 31;

/// Threads per scatter workgroup (`scatter.wgsl`).
const SCATTER_WORKGROUP: u32 = 64;

/// The fewest ranges for which the scatter pass replaces per-range copies (see [`GpuScene::set_scatter_min_ranges`]).
/// Each copy costs wgpu about 0.4–0.5 µs of CPU and the pass about 5–10 µs more than one copy, measured on the dev
/// PC (#76), so they break even near 25 ranges; many tiny copies also cost the GPU more than one dispatch.
pub const SCATTER_MIN_RANGES: u32 = 32;

/// How shaders read the instance buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InstanceBinding {
    /// A read-only storage buffer, indexed by instance index (every backend whose vertex shaders can read storage
    /// buffers).
    Storage,
    /// A per-instance vertex buffer with [`GpuInstance::vertex_attributes`] (GLES, where they cannot).
    Vertex,
}

/// What one [`GpuScene::upload`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadStats {
    /// Ranges the dirty slots coalesced into.
    pub ranges: u32,
    /// Slots uploaded, clean slots merged into ranges included.
    pub slots: u32,
    /// Bytes written to the staging buffer.
    pub bytes: u64,
    /// `write_buffer` calls made: 1 when anything was uploaded, else 0 (one per range on the fallback for uploads
    /// too large to stage).
    pub writes: u32,
    /// `copy_buffer_to_buffer` commands recorded (the copy path: one per range).
    pub copies: u32,
    /// Scatter dispatches recorded (the scatter path: one compute pass for every range).
    pub dispatches: u32,
    /// Region buffers replaced by larger ones this upload.
    pub grown: u32,
    /// Slots beyond the largest buffer the device allows, not uploaded.
    pub dropped: u32,
    /// Draw-list entries uploaded (see [`GpuScene::list_buffer`]): only those whose list changed, or every one
    /// when the lists were laid out again.
    pub list_entries: u32,
    /// `copy_buffer_to_buffer` commands recorded for the draw lists (their staging write is one more
    /// `write_buffer`, not counted in [`UploadStats::writes`]).
    pub list_copies: u32,
}

/// Where one mesh kind's draw list sits in [`GpuScene::list_buffer`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListSegment {
    /// The position of the list's first entry: the first instance of the kind's draw.
    pub first: u32,
    /// How many entries the list has: the instance count of the kind's draw.
    pub len: u32,
    /// How many entries fit before the next segment.
    pub capacity: u32,
}

/// The fewest entries a list segment holds, so small scenes do not lay the lists out again frame after frame.
const MIN_LIST_CAPACITY: u32 = 256;

/// What one more copy command is worth in uploaded bytes: a list's dirty ranges are merged into one span when the
/// span is no larger than this times the number of ranges. A copy costs wgpu about 0.4 µs of CPU (#76).
const COPY_WORTH_BYTES: u64 = 512;

/// The draw lists' GPU buffer and its layout.
#[derive(Debug)]
struct ListBuffer {
    buffer: wgpu::Buffer,
    /// 4 (entries) or 64 (copies of the instances, on the vertex-buffer fallback).
    entry_size: u64,
    /// The lists hold copies of the instances.
    gather: bool,
    segments: Vec<ListSegment>,
    staging: wgpu::Buffer,
    revision: u64,
    /// This upload's ranges: list index, first position, count. Reused.
    ranges: Vec<(usize, u32, u32)>,
}

/// One region's buffer.
#[derive(Debug)]
struct RegionBuffer {
    buffer: wgpu::Buffer,
    /// In slots.
    capacity: u32,
}

/// The compute pass that scatters uploaded instances into their slots.
#[derive(Debug)]
struct Scatter {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    /// Built for the buffers of [`GpuScene::bind_key`]; rebuilt when one is replaced.
    bind_group: Option<(u64, wgpu::BindGroup)>,
    /// The destination of every uploaded instance, reused frame to frame.
    destinations: Vec<u32>,
    /// The most instances one dispatch covers (the workgroup-count limit in two dimensions).
    max_instances: u64,
}

/// The GPU-resident scene (ADR 0008 §2): an [`InstanceScene`] and the two instance buffers it keeps up to date,
/// one per [`Region`].
///
/// Each frame: [`GpuScene::sync`] (CPU: drain the changes, repack the changed instances), then
/// [`GpuScene::upload`] into the frame's command encoder, before the passes that draw.
///
/// # Uploads
///
/// Only the dirty slots are uploaded, coalesced into ranges, and always through **one** staging write per frame:
/// wgpu creates a staging allocation for every `write_buffer` call, so writing each range on its own (measured:
/// 1.5 ms for 1,000 scattered ranges with the sync) costs a buffer creation per range. From the staging buffer the
/// instances reach their slots in one of two ways:
///
/// - **Copies:** one `copy_buffer_to_buffer` per range. Cheap for a few ranges, as when moving parts sit together
///   in the dynamic region, but each costs wgpu about 0.4 µs of CPU (1,000 scattered ranges: 0.68 ms with the
///   sync).
/// - **Scatter:** a compute pass with one thread per instance, which reads each instance's destination from the
///   staging buffer. Its cost hardly depends on the number of ranges (1,000 scattered ranges: 0.24 ms with the
///   sync), so it is used from [`SCATTER_MIN_RANGES`] ranges up, where the device has compute and storage
///   instance buffers (not on GLES).
///
/// Buffers grow by doubling, the old contents copied on the GPU; [`GpuScene::buffer_revision`] changes whenever a
/// buffer is replaced, so bind groups that use them are rebuilt then.
#[derive(Debug)]
pub struct GpuScene {
    scene: InstanceScene,
    device: wgpu::Device,
    queue: wgpu::Queue,
    binding: InstanceBinding,
    buffers: [RegionBuffer; 2],
    staging: wgpu::Buffer,
    /// The largest region buffer the device allows, in slots.
    max_slots: u32,
    /// The largest staging buffer: the device's buffer limit, and its storage binding limit when scattering.
    max_staging: u64,
    /// Counts replaced region buffers.
    revision: u64,
    /// Counts replaced staging buffers.
    staging_revision: u64,
    scatter: Option<Scatter>,
    scatter_min_ranges: u32,
    lists: ListBuffer,
}

impl InstanceBinding {
    /// How instance data reaches the vertex shaders on `context`'s device: storage buffers where vertex shaders
    /// can read at least three of them (the two regions and a draw list), otherwise vertex buffers.
    pub fn for_context(context: &RenderContext) -> Self {
        if context.capabilities().vertex_storage && context.device().limits().max_storage_buffers_per_shader_stage >= 3
        {
            InstanceBinding::Storage
        } else {
            InstanceBinding::Vertex
        }
    }
}

impl GpuScene {
    /// Attaches a scene to `dm` (see [`InstanceScene::attach`]) and creates its buffers (and the scatter pipeline,
    /// where the device can run it) on `context`'s device. The first [`GpuScene::sync`] places every instance
    /// already in the world.
    pub fn new(
        context: &RenderContext,
        dm: &mut DataModel,
        world: Option<Id>,
        components: &RenderComponents,
    ) -> Result<Self, RenderError> {
        let mut scene = InstanceScene::attach(dm, world, components)?;
        let device = context.device().clone();
        let capabilities = context.capabilities();
        let binding = InstanceBinding::for_context(context);
        scene.set_gather_lists(binding == InstanceBinding::Vertex);
        let limits = device.limits();
        let mut max_bytes = limits.max_buffer_size;
        if binding == InstanceBinding::Storage {
            max_bytes = max_bytes.min(limits.max_storage_buffer_binding_size);
        }
        let max_slots = u32::try_from(max_bytes / GpuInstance::SIZE)
            .unwrap_or(u32::MAX)
            .min(DYNAMIC_BIT - 1);
        let scatter = (binding == InstanceBinding::Storage
            && capabilities.compute
            && limits.max_storage_buffers_per_shader_stage >= 3)
            .then(|| Scatter::new(&device, &limits));
        let max_staging = if scatter.is_some() {
            limits.max_buffer_size.min(limits.max_storage_buffer_binding_size)
        } else {
            limits.max_buffer_size
        };
        let capacity = MIN_CAPACITY.min(max_slots);
        let buffers = Region::ALL.map(|region| RegionBuffer {
            buffer: instance_buffer(&device, binding, region, capacity),
            capacity,
        });
        let staging = staging_buffer(&device, u64::from(capacity) * GpuInstance::SIZE, scatter.is_some());
        let entry_size = match binding {
            InstanceBinding::Storage => size_of::<u32>() as u64,
            InstanceBinding::Vertex => GpuInstance::SIZE,
        };
        let lists = ListBuffer {
            buffer: list_buffer(
                &device,
                binding,
                u64::from(MIN_LIST_CAPACITY) * u64::from(MeshKind::BUILTIN_COUNT) * entry_size,
            ),
            entry_size,
            gather: binding == InstanceBinding::Vertex,
            segments: Vec::new(),
            staging: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("metallic draw list uploads"),
                size: u64::from(MIN_LIST_CAPACITY) * entry_size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            revision: 0,
            ranges: Vec::new(),
        };
        Ok(Self {
            scene,
            device,
            queue: context.queue().clone(),
            binding,
            buffers,
            staging,
            max_slots,
            max_staging,
            revision: 0,
            staging_revision: 0,
            scatter,
            scatter_min_ranges: SCATTER_MIN_RANGES,
            lists,
        })
    }

    /// Unregisters the scene's change consumer from `dm`.
    pub fn detach(self, dm: &mut DataModel) {
        self.scene.detach(dm);
    }

    /// The CPU side: slots, packed instances, origin.
    pub fn scene(&self) -> &InstanceScene {
        &self.scene
    }

    /// Applies what changed in `dm` (see [`InstanceScene::sync`]). CPU only; [`GpuScene::upload`] sends it.
    pub fn sync(&mut self, dm: &mut DataModel) -> SyncStats {
        self.scene.sync(dm)
    }

    /// Moves the scene origin near the camera when it has gone far (see [`InstanceScene::set_camera_position`]).
    pub fn set_camera_position(&mut self, dm: &DataModel, camera: Vec3) -> bool {
        self.scene.set_camera_position(dm, camera)
    }

    /// Re-places every instance on the next sync (see [`InstanceScene::rebuild_all`]).
    pub fn rebuild_all(&mut self, dm: &mut DataModel) {
        self.scene.rebuild_all(dm);
    }

    /// Whether this device can run the scatter pass.
    pub fn can_scatter(&self) -> bool {
        self.scatter.is_some()
    }

    /// From how many ranges on the scatter pass replaces per-range copies ([`SCATTER_MIN_RANGES`] by default; 0
    /// scatters every upload, `u32::MAX` never does). Has no effect where the device cannot scatter.
    pub fn set_scatter_min_ranges(&mut self, ranges: u32) {
        self.scatter_min_ranges = ranges;
    }

    /// Records the upload of every dirty slot into `encoder`, growing buffers first when the scene outgrew them.
    /// Submit `encoder` before the next upload: the staging buffer is reused, and every `write_buffer` lands at the
    /// start of the next submission. Growing submits its own copy at once (see `grow`).
    ///
    /// The draw lists' changed entries go out in the same encoder (see [`GpuScene::list_buffer`]).
    pub fn upload(&mut self, encoder: &mut wgpu::CommandEncoder) -> UploadStats {
        let mut stats = UploadStats::default();
        for region in Region::ALL {
            if self.grow(region) {
                stats.grown += 1;
            }
        }
        self.scene.coalesce();
        self.upload_instances(encoder, &mut stats);
        self.upload_lists(encoder, &mut stats);
        stats
    }

    /// The instance half of [`GpuScene::upload`], after the coalesce.
    fn upload_instances(&mut self, encoder: &mut wgpu::CommandEncoder, stats: &mut UploadStats) {
        for range in self.scene.uploads() {
            let count = self.clipped(range.region, range.first, range.count);
            stats.dropped += range.count - count;
            stats.slots += count;
            stats.ranges += 1;
        }
        let slots = u64::from(stats.slots);
        if slots == 0 {
            return;
        }
        let scatter_bytes = 16 + slots.div_ceil(4) * 16 + slots * GpuInstance::SIZE;
        let use_scatter = self.scatter.as_ref().is_some_and(|s| {
            stats.ranges >= self.scatter_min_ranges && slots <= s.max_instances && scatter_bytes <= self.max_staging
        });
        let total = if use_scatter {
            scatter_bytes
        } else {
            slots * GpuInstance::SIZE
        };
        if total > self.max_staging {
            // More than one buffer can stage (a rebuild of millions of instances): write each range in place.
            self.write_ranges_in_place(stats);
            return;
        }
        if self.staging.size() < total {
            self.staging = staging_buffer(
                &self.device,
                total.next_power_of_two().min(self.max_staging),
                self.scatter.is_some(),
            );
            self.staging_revision += 1;
        }
        stats.bytes = total;
        if use_scatter {
            self.scatter_upload(encoder, stats);
        } else {
            self.copy_upload(encoder, stats);
        }
    }

    /// The draw-list half of [`GpuScene::upload`]: lays the lists out again when one outgrew its segment (and then
    /// uploads them whole), otherwise uploads the changed entries, through one staging write and a copy per range.
    fn upload_lists(&mut self, encoder: &mut wgpu::CommandEncoder, stats: &mut UploadStats) {
        let entry_size = self.lists.entry_size;
        let count = self.scene.draw_list_count();
        let lists = &mut self.lists;
        let draw_lists = self.scene.lists_mut();
        lists.ranges.clear();
        let outgrown = lists.segments.len() < count
            || draw_lists
                .opaque
                .iter()
                .zip(&lists.segments)
                .any(|(list, segment)| list.entries.len() > segment.capacity as usize);
        if outgrown {
            lists.segments.resize(count, ListSegment::default());
            let mut first = 0u32;
            for (segment, list) in lists.segments.iter_mut().zip(&mut draw_lists.opaque) {
                let len = u32::try_from(list.entries.len()).unwrap_or(u32::MAX);
                segment.capacity = segment
                    .capacity
                    .max(len.checked_next_power_of_two().unwrap_or(u32::MAX))
                    .max(MIN_LIST_CAPACITY);
                segment.first = first;
                first = first.saturating_add(segment.capacity);
                list.dirty.clear();
            }
            let needed = u64::from(first) * entry_size;
            if lists.buffer.size() < needed {
                lists.buffer = list_buffer(&self.device, self.binding, needed);
            }
            lists.revision += 1;
            for (index, list) in draw_lists.opaque.iter().enumerate() {
                if !list.entries.is_empty() {
                    let len = u32::try_from(list.entries.len()).unwrap_or(u32::MAX);
                    lists.ranges.push((index, 0, len));
                }
            }
        } else {
            for (index, list) in draw_lists.opaque.iter_mut().enumerate() {
                let len = u32::try_from(list.entries.len()).unwrap_or(u32::MAX);
                list.dirty.sort_unstable();
                list.dirty.dedup();
                let start = lists.ranges.len();
                let mut run: Option<(usize, u32, u32)> = None;
                for &pos in list.dirty.iter().take_while(|&&pos| pos < len) {
                    match run.as_mut() {
                        Some((_, first, count)) if pos - (*first + *count) <= MAX_UPLOAD_GAP => {
                            *count = pos - *first + 1;
                        }
                        _ => {
                            lists.ranges.extend(run.take());
                            run = Some((index, pos, 1));
                        }
                    }
                }
                lists.ranges.extend(run);
                list.dirty.clear();
                // Many small ranges of one list: one span costs less once a copy is dearer than the bytes between.
                let ranges = &lists.ranges[start..];
                if let (Some(&(_, lo, _)), Some(&(_, last, last_count))) = (ranges.first(), ranges.last()) {
                    let span = u64::from(last + last_count - lo);
                    if ranges.len() > 1 && span * entry_size <= ranges.len() as u64 * COPY_WORTH_BYTES {
                        lists.ranges.truncate(start);
                        lists.ranges.push((index, lo, last + last_count - lo));
                    }
                }
            }
        }
        for (segment, list) in lists.segments.iter_mut().zip(&draw_lists.opaque) {
            segment.len = u32::try_from(list.entries.len()).unwrap_or(u32::MAX);
        }

        let entries: u64 = lists.ranges.iter().map(|&(_, _, count)| u64::from(count)).sum();
        let Some(size) = wgpu::BufferSize::new(entries * entry_size) else {
            return;
        };
        if lists.staging.size() < size.get() {
            lists.staging = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("metallic draw list uploads"),
                size: size.get().next_power_of_two(),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
        }
        let Some(mut view) = self.queue.write_buffer_with(&lists.staging, 0, size) else {
            return;
        };
        let mut offset = 0usize;
        let scene = &self.scene;
        for &(index, first, count) in &lists.ranges {
            let entries = scene.draw_list(MeshKind(u16::try_from(index).unwrap_or(u16::MAX)));
            let Some(entries) = entries.get(first as usize..(first + count) as usize) else {
                continue;
            };
            if lists.gather {
                for &entry in entries {
                    let instance = scene.entry_instance(entry).unwrap_or(&GpuInstance::FREE);
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
        for &(index, first, count) in &lists.ranges {
            let Some(segment) = lists.segments.get(index) else {
                continue;
            };
            let bytes = u64::from(count) * entry_size;
            encoder.copy_buffer_to_buffer(
                &lists.staging,
                source,
                &lists.buffer,
                u64::from(segment.first + first) * entry_size,
                bytes,
            );
            source += bytes;
            stats.list_copies += 1;
        }
        stats.list_entries = u32::try_from(entries).unwrap_or(u32::MAX);
    }

    /// The draw lists on the GPU: every mesh kind's opaque list in its own segment ([`GpuScene::list_segments`]),
    /// as entries (`u32`, see [`super::entry`]) where the instance buffers are storage buffers, or as copies of
    /// the instances ([`GpuInstance`], a per-instance vertex buffer) on the vertex-buffer fallback. A draw of a
    /// kind's list is instances `first .. first + len` of its segment. Replaced when it grows
    /// ([`GpuScene::list_revision`]).
    pub fn list_buffer(&self) -> &wgpu::Buffer {
        &self.lists.buffer
    }

    /// Where each mesh kind's list sits in [`GpuScene::list_buffer`], indexed by kind, as of the last upload.
    pub fn list_segments(&self) -> &[ListSegment] {
        &self.lists.segments
    }

    /// Changes whenever the list buffer is laid out again (and possibly replaced).
    pub fn list_revision(&self) -> u64 {
        self.lists.revision
    }

    /// The copy path: the ranges' instances back to back in the staging buffer, then one copy per range.
    fn copy_upload(&self, encoder: &mut wgpu::CommandEncoder, stats: &mut UploadStats) {
        let Some(size) = wgpu::BufferSize::new(u64::from(stats.slots) * GpuInstance::SIZE) else {
            return;
        };
        let Some(mut view) = self.queue.write_buffer_with(&self.staging, 0, size) else {
            return;
        };
        let mut offset = 0usize;
        for range in self.scene.uploads() {
            let bytes = self.range_bytes(range.region, range.first, range.count);
            view.slice(offset..offset + bytes.len()).copy_from_slice(bytes);
            offset += bytes.len();
        }
        drop(view);
        stats.writes = 1;

        let mut source = 0u64;
        for range in self.scene.uploads() {
            let count = self.clipped(range.region, range.first, range.count);
            if count == 0 {
                continue;
            }
            let bytes = u64::from(count) * GpuInstance::SIZE;
            encoder.copy_buffer_to_buffer(
                &self.staging,
                source,
                &self.buffers[range.region.index()].buffer,
                u64::from(range.first) * GpuInstance::SIZE,
                bytes,
            );
            source += bytes;
            stats.copies += 1;
        }
    }

    /// The scatter path: a header, every instance's destination, then the instances, in the staging buffer; one
    /// compute pass puts each instance in its slot.
    fn scatter_upload(&mut self, encoder: &mut wgpu::CommandEncoder, stats: &mut UploadStats) {
        let key = self.bind_key();
        let Some(scatter) = self.scatter.as_mut() else {
            return;
        };
        scatter.destinations.clear();
        for range in self.scene.uploads() {
            let count = self.buffers[range.region.index()]
                .capacity
                .saturating_sub(range.first)
                .min(range.count);
            let bit = if range.region == Region::Dynamic {
                DYNAMIC_BIT
            } else {
                0
            };
            scatter
                .destinations
                .extend((range.first..range.first + count).map(|slot| slot | bit));
        }
        let Some(size) = wgpu::BufferSize::new(stats.bytes) else {
            return;
        };
        let Some(mut view) = self.queue.write_buffer_with(&self.staging, 0, size) else {
            return;
        };
        let count = stats.slots;
        view.slice(0..16).copy_from_slice(bytemuck::bytes_of(&[count, 0, 0, 0]));
        let destinations: &[u8] = bytemuck::cast_slice(&scatter.destinations);
        view.slice(16..16 + destinations.len()).copy_from_slice(destinations);
        let mut offset = 16 + (count as usize).div_ceil(4) * 16;
        for range in self.scene.uploads() {
            let capacity = self.buffers[range.region.index()].capacity;
            let count = capacity.saturating_sub(range.first).min(range.count) as usize;
            let first = range.first as usize;
            let Some(instances) = self.scene.instances(range.region).get(first..first + count) else {
                continue;
            };
            let bytes: &[u8] = bytemuck::cast_slice(instances);
            view.slice(offset..offset + bytes.len()).copy_from_slice(bytes);
            offset += bytes.len();
        }
        drop(view);
        stats.writes = 1;

        if scatter.bind_group.as_ref().is_none_or(|(k, _)| *k != key) {
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("metallic instance scatter"),
                layout: &scatter.layout,
                entries: &[
                    buffer_entry(0, &self.staging),
                    buffer_entry(1, &self.buffers[Region::Static.index()].buffer),
                    buffer_entry(2, &self.buffers[Region::Dynamic.index()].buffer),
                ],
            });
            scatter.bind_group = Some((key, bind_group));
        }
        let Some((_, bind_group)) = scatter.bind_group.as_ref() else {
            return;
        };
        let groups = count.div_ceil(SCATTER_WORKGROUP);
        let x = groups.min(MAX_WORKGROUPS_X);
        let y = groups.div_ceil(x);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("metallic instance scatter"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&scatter.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.dispatch_workgroups(x, y, 1);
        stats.dispatches = 1;
    }

    /// The fallback for uploads larger than a staging buffer can be: one `write_buffer` per range, straight into
    /// the region buffers.
    fn write_ranges_in_place(&self, stats: &mut UploadStats) {
        for range in self.scene.uploads() {
            let bytes = self.range_bytes(range.region, range.first, range.count);
            if bytes.is_empty() {
                continue;
            }
            self.queue.write_buffer(
                &self.buffers[range.region.index()].buffer,
                u64::from(range.first) * GpuInstance::SIZE,
                bytes,
            );
            stats.writes += 1;
        }
    }

    /// `count` cut to what `region`'s buffer holds from `first`.
    fn clipped(&self, region: Region, first: u32, count: u32) -> u32 {
        self.buffers[region.index()].capacity.saturating_sub(first).min(count)
    }

    /// The packed bytes of `count` slots of `region` from `first`, cut to the buffer's capacity.
    fn range_bytes(&self, region: Region, first: u32, count: u32) -> &[u8] {
        let count = self.clipped(region, first, count) as usize;
        let first = first as usize;
        self.scene
            .instances(region)
            .get(first..first + count)
            .map_or(&[], bytemuck::cast_slice)
    }

    /// Identifies the current set of buffers the scatter bind group uses.
    fn bind_key(&self) -> u64 {
        (self.revision << 32) ^ self.staging_revision
    }

    /// The instance buffer of `region`: [`GpuInstance`]s indexed by slot. Bind it as [`GpuScene::binding`] says.
    /// Replaced when it grows ([`GpuScene::buffer_revision`]).
    pub fn buffer(&self, region: Region) -> &wgpu::Buffer {
        &self.buffers[region.index()].buffer
    }

    /// How many slots `region`'s buffer holds.
    pub fn capacity(&self, region: Region) -> u32 {
        self.buffers[region.index()].capacity
    }

    /// How shaders read the instance buffers on this device.
    pub fn binding(&self) -> InstanceBinding {
        self.binding
    }

    /// Changes whenever an instance buffer is replaced by a larger one.
    pub fn buffer_revision(&self) -> u64 {
        self.revision
    }

    /// Replaces `region`'s buffer with one twice as large (or more) when the scene has more slots than it holds,
    /// copying the old contents on the GPU. Returns whether it did.
    ///
    /// The copy is submitted at once, on its own: every `write_buffer` lands at the start of the next submission,
    /// so a copy recorded into the frame's encoder would run after this frame's writes and overwrite them.
    fn grow(&mut self, region: Region) -> bool {
        let needed = self.scene.instances(region).len();
        let old = &self.buffers[region.index()];
        if needed <= old.capacity as usize || old.capacity >= self.max_slots {
            return false;
        }
        let needed = u32::try_from(needed).unwrap_or(u32::MAX);
        let capacity = needed
            .checked_next_power_of_two()
            .unwrap_or(u32::MAX)
            .max(old.capacity.saturating_mul(2))
            .min(self.max_slots);
        let buffer = instance_buffer(&self.device, self.binding, region, capacity);
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("metallic instance buffer growth"),
        });
        encoder.copy_buffer_to_buffer(&old.buffer, 0, &buffer, 0, old.buffer.size());
        self.queue.submit(std::iter::once(encoder.finish()));
        self.buffers[region.index()] = RegionBuffer { buffer, capacity };
        self.revision += 1;
        true
    }
}

/// The most workgroups in one dimension every device allows (WebGPU's minimum `maxComputeWorkgroupsPerDimension`).
const MAX_WORKGROUPS_X: u32 = 65_535;

impl Scatter {
    fn new(device: &wgpu::Device, limits: &wgpu::Limits) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("metallic instance scatter"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scatter.wgsl").into()),
        });
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("metallic instance scatter"),
            entries: &[storage(0, true), storage(1, false), storage(2, false)],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("metallic instance scatter"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("metallic instance scatter"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("scatter"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let per_dimension = u64::from(limits.max_compute_workgroups_per_dimension.min(MAX_WORKGROUPS_X));
        Self {
            pipeline,
            layout,
            bind_group: None,
            destinations: Vec::new(),
            max_instances: per_dimension * per_dimension * u64::from(SCATTER_WORKGROUP),
        }
    }
}

fn buffer_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

fn instance_buffer(device: &wgpu::Device, binding: InstanceBinding, region: Region, capacity: u32) -> wgpu::Buffer {
    let usage = match binding {
        InstanceBinding::Storage => wgpu::BufferUsages::STORAGE,
        InstanceBinding::Vertex => wgpu::BufferUsages::VERTEX,
    } | wgpu::BufferUsages::COPY_DST
        | wgpu::BufferUsages::COPY_SRC;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(match region {
            Region::Static => "metallic instances (static)",
            Region::Dynamic => "metallic instances (dynamic)",
        }),
        size: u64::from(capacity) * GpuInstance::SIZE,
        usage,
        mapped_at_creation: false,
    })
}

fn list_buffer(device: &wgpu::Device, binding: InstanceBinding, size: u64) -> wgpu::Buffer {
    let usage = match binding {
        InstanceBinding::Storage => wgpu::BufferUsages::STORAGE,
        InstanceBinding::Vertex => wgpu::BufferUsages::VERTEX,
    } | wgpu::BufferUsages::COPY_DST
        | wgpu::BufferUsages::COPY_SRC;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("metallic draw lists"),
        size,
        usage,
        mapped_at_creation: false,
    })
}

fn staging_buffer(device: &wgpu::Device, size: u64, scatter: bool) -> wgpu::Buffer {
    let mut usage = wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
    if scatter {
        usage |= wgpu::BufferUsages::STORAGE;
    }
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("metallic instance uploads"),
        size,
        usage,
        mapped_at_creation: false,
    })
}
