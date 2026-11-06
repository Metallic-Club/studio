//! The transparent pass's draw list: every partly see-through instance, sorted back to front for the current view.
//!
//! The order is exact per instance, across mesh kinds and levels of detail, and the draw count does not depend on how
//! the sorted instances interleave (#106):
//!
//! - **Pulled vertices** (the storage path, where a vertex shader can read four storage buffers: every backend with
//!   storage buffers in vertex shaders here): one draw of every sorted instance's mesh vertices, one instance after
//!   another, with no vertex or index buffer. The list holds a [`PulledRecord`] per instance, which says where its
//!   vertices start in the draw and where its mesh is in the pool's unindexed copy ([`super::MeshPool`]), after a
//!   chunk table naming the record each chunk of 64 vertices ([`PULLED_CHUNK_SHIFT`]) starts in; the vertex shader
//!   finds its record from its chunk's in a step or two (`vs_pulled` in instance_storage.wgsl). A draw's triangles
//!   are blended in order, so one draw keeps the back-to-front order. Every vertex is shaded once per triangle it is
//!   in (there is no index buffer to reuse it through): about three times the vertex work of indexed draws, which
//!   `docs/perf.md` weighs against the draws and CPU time saved.
//! - **Runs** (the vertex-buffer fallback, GLES, whose vertex shaders read no storage buffers; and a storage path
//!   with fewer than four): one instanced draw per run of consecutive sorted instances with the same mesh and level,
//!   so the draw count still grows with how the shapes interleave in depth.

use metallic_core::glam::Vec3;

use super::MeshPool;
use crate::lod::MAX_LOD_LEVELS;
use crate::scene::{GpuInstance, InstanceScene, MeshKind, TransparentEntries};

/// A run of consecutive sorted entries with the same mesh and level of detail: one draw on the runs path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) kind: MeshKind,
    pub(crate) level: u8,
    pub(crate) first: u32,
    pub(crate) count: u32,
}

/// One sorted instance of the pulled draw (`PulledRecord` in instance_storage.wgsl): 12 bytes, after the record
/// count.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct PulledRecord {
    /// The scene entry (bit 31: dynamic region, the rest the slot).
    pub(crate) entry: u32,
    /// The draw's vertex where this instance's vertices begin.
    pub(crate) start: u32,
    /// Low 30 bits: the mesh's first vertex in the pool's unindexed copy (its first index); top 2: its level.
    pub(crate) mesh: u32,
}

/// The largest first vertex a [`PulledRecord`] can hold.
const PULLED_MESH_MASK: u32 = (1 << 30) - 1;

/// The pulled draw's vertices per chunk-table entry, as a shift (`PULLED_CHUNK_SHIFT` in instance_storage.wgsl).
pub(crate) const PULLED_CHUNK_SHIFT: u32 = 6;

/// How the sorted list is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransparentPath {
    /// One draw, pulling the vertices: the list holds a record count and [`PulledRecord`]s.
    Pulled,
    /// One draw per run, the list holding slot entries for the storage path's `vs_main`.
    RunEntries,
    /// One draw per run, the list holding copies of the instances (the vertex-buffer fallback).
    RunCopies,
}

/// What the sorted list draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TransparentCounts {
    /// Instances drawn.
    pub(crate) instances: u32,
    /// Triangles drawn.
    pub(crate) triangles: u64,
    /// Instances at each level of detail.
    pub(crate) levels: [u32; MAX_LOD_LEVELS],
    /// The pulled draw's vertices (0 on the runs paths).
    pub(crate) vertices: u32,
}

/// The sorted transparent entries (records, or copies of their instances on the vertex-buffer fallback) on the GPU.
#[derive(Debug)]
pub(crate) struct TransparentList {
    buffer: wgpu::Buffer,
    usage: wgpu::BufferUsages,
    path: TransparentPath,
    /// (sort key, position in the drawn entries), reused. The key orders far before near.
    keys: Vec<(u32, u32)>,
    /// The upload, reused.
    bytes: Vec<u8>,
    /// The pulled draw's records and chunk table, reused.
    records: Vec<PulledRecord>,
    chunks: Vec<u32>,
    runs: Vec<Run>,
    counts: TransparentCounts,
    /// What the list was last sorted for: the scene's transparent revision, the revision of the entries drawn (a
    /// view's visible ones, or every one) and the camera offset.
    sorted_for: Option<(u64, u64, Vec3, u64)>,
}

impl TransparentList {
    pub(crate) fn new(device: &wgpu::Device, path: TransparentPath) -> Self {
        let (usage, entry_size) = match path {
            TransparentPath::Pulled => (wgpu::BufferUsages::STORAGE, size_of::<PulledRecord>()),
            TransparentPath::RunEntries => (wgpu::BufferUsages::STORAGE, size_of::<u32>()),
            TransparentPath::RunCopies => (wgpu::BufferUsages::VERTEX, size_of::<GpuInstance>()),
        };
        let usage = usage | wgpu::BufferUsages::COPY_DST;
        Self {
            buffer: buffer(device, usage, 64 * entry_size as u64),
            usage,
            path,
            keys: Vec::new(),
            bytes: Vec::new(),
            records: Vec::new(),
            chunks: Vec::new(),
            runs: Vec::new(),
            counts: TransparentCounts::default(),
            sorted_for: None,
        }
    }

    /// How the list is drawn.
    pub(crate) fn path(&self) -> TransparentPath {
        self.path
    }

    /// The buffer the transparent draws read (records, entries or copies). Replaced when it grows.
    pub(crate) fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// One draw per run of the sorted list, on the runs paths; empty on the pulled path.
    pub(crate) fn runs(&self) -> &[Run] {
        &self.runs
    }

    /// What the list draws.
    pub(crate) fn counts(&self) -> TransparentCounts {
        self.counts
    }

    /// Whether there is anything to draw.
    pub(crate) fn is_empty(&self) -> bool {
        match self.path {
            TransparentPath::Pulled => self.counts.vertices == 0,
            TransparentPath::RunEntries | TransparentPath::RunCopies => self.runs.is_empty(),
        }
    }

    /// Sorts `drawn` (the scene's transparent entries to draw: every one, or a view's visible ones, with their levels
    /// and a revision that changes with them) back to front as seen from `camera` (relative to the scene origin) and
    /// uploads them, when anything changed since the last sort. Returns whether it uploaded.
    pub(crate) fn update(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        scene: &InstanceScene,
        drawn: TransparentEntries<'_>,
        camera: Vec3,
        meshes: &MeshPool,
    ) -> bool {
        // A mesh asset that arrived changes what its instances draw.
        let key = (scene.transparent_revision(), drawn.revision, camera, meshes.revision());
        if self.sorted_for == Some(key) {
            return false;
        }
        self.sorted_for = Some(key);
        sort_back_to_front(scene, drawn.entries, camera, &mut self.keys);
        self.runs.clear();
        self.bytes.clear();
        self.counts = TransparentCounts::default();
        self.records.clear();
        self.chunks.clear();

        let mut records = 0u32;
        for (position, &(_, index)) in (0u32..).zip(&self.keys) {
            let entry = drawn.entries.get(index as usize).copied().unwrap_or(0);
            let level = drawn.levels.get(index as usize).copied().unwrap_or(0);
            let instance = scene.entry_instance(entry).unwrap_or(&GpuInstance::FREE);
            let mesh = meshes.range(instance.mesh(), usize::from(level));
            match self.path {
                TransparentPath::Pulled => {
                    // A mesh the pool does not have draws nothing; a draw cannot reach past `u32::MAX` vertices.
                    let Some(end) = self.counts.vertices.checked_add(mesh.index_count) else {
                        break;
                    };
                    if mesh.index_count == 0 {
                        continue;
                    }
                    let record = PulledRecord {
                        entry,
                        start: self.counts.vertices,
                        mesh: (mesh.first_index & PULLED_MESH_MASK) | (u32::from(level.min(3)) << 30),
                    };
                    self.records.push(record);
                    push_chunks(&mut self.chunks, records, end);
                    self.counts.vertices = end;
                    records += 1;
                }
                TransparentPath::RunEntries | TransparentPath::RunCopies => {
                    match self.runs.last_mut() {
                        Some(run) if run.kind == instance.mesh() && run.level == level => run.count += 1,
                        _ => self.runs.push(Run {
                            kind: instance.mesh(),
                            level,
                            first: position,
                            count: 1,
                        }),
                    }
                    if self.path == TransparentPath::RunCopies {
                        self.bytes.extend_from_slice(bytemuck::bytes_of(instance));
                    } else {
                        self.bytes.extend_from_slice(&entry.to_le_bytes());
                    }
                }
            }
            self.counts.instances += 1;
            self.counts.triangles += u64::from(mesh.index_count / 3);
            if let Some(n) = self.counts.levels.get_mut(usize::from(level)) {
                *n += 1;
            }
        }
        if self.is_empty() {
            return false;
        }
        let chunks = u32::try_from(self.chunks.len()).unwrap_or(u32::MAX);
        let header = [records, chunks];
        let pulled = [
            bytemuck::cast_slice::<u32, u8>(&header),
            bytemuck::cast_slice(&self.chunks),
            bytemuck::cast_slice(&self.records),
        ];
        let len = match self.path {
            TransparentPath::Pulled => pulled.iter().map(|part| part.len()).sum(),
            TransparentPath::RunEntries | TransparentPath::RunCopies => self.bytes.len(),
        };
        if (self.buffer.size() as usize) < len {
            self.buffer = buffer(device, self.usage, len.next_power_of_two() as u64);
        }
        if self.path != TransparentPath::Pulled {
            queue.write_buffer(&self.buffer, 0, &self.bytes);
            return true;
        }
        // Written straight into the staging memory, rather than gathered into one slice first.
        let Some(size) = wgpu::BufferSize::new(len as u64) else {
            return false;
        };
        let Some(mut view) = queue.write_buffer_with(&self.buffer, 0, size) else {
            return false;
        };
        // The parts add up to the view's length.
        let mut at = 0;
        for part in pulled {
            view.slice(at..at + part.len()).copy_from_slice(part);
            at += part.len();
        }
        true
    }
}

/// Adds `record` (whose vertices end at `end`, the records before it ending where it starts) to the chunk table: as
/// the record of every chunk whose first vertex is one of its own.
fn push_chunks(chunks: &mut Vec<u32>, record: u32, end: u32) {
    while ((chunks.len() as u64) << PULLED_CHUNK_SHIFT) < u64::from(end) {
        chunks.push(record);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The record `vs_pulled` (instance_storage.wgsl) finds for `vertex`: the chunk's record, then forward while the
    /// next record starts at or before the vertex.
    fn lookup(chunks: &[u32], starts: &[u32], vertex: u32) -> u32 {
        let mut i = chunks[(vertex >> PULLED_CHUNK_SHIFT) as usize];
        for _ in 0..1u32 << PULLED_CHUNK_SHIFT {
            let next = i + 1;
            if next as usize >= starts.len() || starts[next as usize] > vertex {
                break;
            }
            i = next;
        }
        i
    }

    #[test]
    fn every_pulled_vertex_finds_its_instance() {
        // The built-in meshes' unindexed vertex counts, mixed: from the 24 of a Wedge to the 2,880 of a full Ball.
        let sizes = [36u32, 2880, 24, 672, 144, 24, 384, 192, 96, 36, 36, 24, 24, 24];
        let (mut chunks, mut starts) = (Vec::new(), Vec::new());
        let mut end = 0;
        for (record, size) in (0u32..).zip(sizes.iter().cycle().take(500)) {
            starts.push(end);
            end += size;
            push_chunks(&mut chunks, record, end);
        }
        assert_eq!(chunks.len(), end.div_ceil(1 << PULLED_CHUNK_SHIFT) as usize);
        for vertex in 0..end {
            let i = lookup(&chunks, &starts, vertex) as usize;
            let next = starts.get(i + 1).copied().unwrap_or(end);
            assert!(starts[i] <= vertex && vertex < next, "vertex {vertex} found record {i}");
        }
    }

    #[test]
    fn a_record_packs_its_mesh_and_level() {
        let record = PulledRecord {
            entry: 7,
            start: 0,
            mesh: (1234 & PULLED_MESH_MASK) | (3 << 30),
        };
        assert_eq!((record.mesh & PULLED_MESH_MASK, record.mesh >> 30), (1234, 3));
        // `PulledRecord` in instance_storage.wgsl: three words.
        assert_eq!(size_of::<PulledRecord>(), 12);
    }

    #[test]
    fn the_shader_and_the_list_agree_on_the_chunk_size() {
        let wgsl = include_str!("instance_storage.wgsl");
        assert!(
            wgsl.contains(&format!("const PULLED_CHUNK_SHIFT: u32 = {PULLED_CHUNK_SHIFT}u;")),
            "instance_storage.wgsl's PULLED_CHUNK_SHIFT differs"
        );
    }
}
