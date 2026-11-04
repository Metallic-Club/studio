//! The transparent pass's draw list: every partly see-through instance, sorted back to front for the current view.

use metallic_core::glam::Vec3;

use crate::scene::{GpuInstance, InstanceBinding, InstanceScene, MeshKind, TransparentEntries};

/// A run of consecutive sorted entries with the same mesh and level of detail: one draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) kind: MeshKind,
    pub(crate) level: u8,
    pub(crate) first: u32,
    pub(crate) count: u32,
}

/// The sorted transparent entries (or copies of their instances, on the vertex-buffer fallback) on the GPU.
#[derive(Debug)]
pub(crate) struct TransparentList {
    buffer: wgpu::Buffer,
    usage: wgpu::BufferUsages,
    gather: bool,
    /// (sort key, position in the drawn entries), reused. The key orders far before near.
    keys: Vec<(u32, u32)>,
    /// The upload, reused.
    bytes: Vec<u8>,
    runs: Vec<Run>,
    /// What the list was last sorted for: the scene's transparent revision, the revision of the entries drawn (a
    /// view's visible ones, or every one) and the camera offset.
    sorted_for: Option<(u64, u64, Vec3)>,
}

impl TransparentList {
    pub(crate) fn new(device: &wgpu::Device, binding: InstanceBinding) -> Self {
        let (usage, entry_size, gather) = match binding {
            InstanceBinding::Storage => (wgpu::BufferUsages::STORAGE, size_of::<u32>(), false),
            InstanceBinding::Vertex => (wgpu::BufferUsages::VERTEX, size_of::<GpuInstance>(), true),
        };
        let usage = usage | wgpu::BufferUsages::COPY_DST;
        Self {
            buffer: buffer(device, usage, 64 * entry_size as u64),
            usage,
            gather,
            keys: Vec::new(),
            bytes: Vec::new(),
            runs: Vec::new(),
            sorted_for: None,
        }
    }

    /// The buffer the transparent draws read (entries or copies). Replaced when it grows.
    pub(crate) fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// One draw per run of the sorted list.
    pub(crate) fn runs(&self) -> &[Run] {
        &self.runs
    }

    /// Sorts `drawn` (the scene's transparent entries to draw: every one, or a view's visible ones, with their levels
    /// and a revision that changes with them) back to front as seen from `camera` (relative to the scene origin) and
    /// uploads them, when anything changed since the last sort. Returns whether it uploaded.
    pub(crate) fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &InstanceScene,
        drawn: TransparentEntries<'_>,
        camera: Vec3,
    ) -> bool {
        let key = (scene.transparent_revision(), drawn.revision, camera);
        if self.sorted_for == Some(key) {
            return false;
        }
        self.sorted_for = Some(key);
        sort_back_to_front(scene, drawn.entries, camera, &mut self.keys);
        self.runs.clear();
        self.bytes.clear();

        for (position, &(_, index)) in (0u32..).zip(&self.keys) {
            let entry = drawn.entries.get(index as usize).copied().unwrap_or(0);
            let level = drawn.levels.get(index as usize).copied().unwrap_or(0);
            let instance = scene.entry_instance(entry).unwrap_or(&GpuInstance::FREE);
            match self.runs.last_mut() {
                Some(run) if run.kind == instance.mesh() && run.level == level => run.count += 1,
                _ => self.runs.push(Run {
                    kind: instance.mesh(),
                    level,
                    first: position,
                    count: 1,
                }),
            }
            if self.gather {
                self.bytes.extend_from_slice(bytemuck::bytes_of(instance));
            } else {
                self.bytes.extend_from_slice(&entry.to_le_bytes());
            }
        }
        if self.bytes.is_empty() {
            return false;
        }
        if (self.buffer.size() as usize) < self.bytes.len() {
            let size = (self.bytes.len().next_power_of_two()) as u64;
            self.buffer = buffer(device, self.usage, size);
        }
        queue.write_buffer(&self.buffer, 0, &self.bytes);
        true
    }
}

/// The positions in `entries` (the scene's), farthest first from `camera`, as `(key, position)`; ties broken by
/// position, so the order is stable for the same entries. Sorting positions rather than entries keeps the keys 8
/// bytes and finds each entry's level of detail at the same position of its list.
pub(crate) fn sort_back_to_front(scene: &InstanceScene, entries: &[u32], camera: Vec3, keys: &mut Vec<(u32, u32)>) {
    keys.clear();
    keys.extend((0u32..).zip(entries).map(|(i, &entry)| {
        let distance = scene
            .entry_instance(entry)
            .map_or(0.0, |instance| instance.translation().distance_squared(camera));
        // Non-negative floats order like their bits; inverted, the farthest sorts first.
        (!distance.max(0.0).to_bits(), i)
    }));
    keys.sort_unstable();
}

fn buffer(device: &wgpu::Device, usage: wgpu::BufferUsages, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("metallic transparent list"),
        size,
        usage,
        mapped_at_creation: false,
    })
}
