// The storage-buffer path (appended to parts.wgsl): the draw's instance index selects an entry of the draw list,
// and the entry names the slot (bit 31: dynamic region) whose packed instance is read from the region buffers.
// Each draw covers one mesh kind's segment of the list (first instance = the segment's start).

// `GpuInstance` in scene/instance.rs: 64 bytes.
struct Instance {
    r0: vec4<f32>,
    r1: vec4<f32>,
    r2: vec4<f32>,
    color: u32,
    transparency: f32,
    mesh_material: u32,
    flags: u32,
}

@group(1) @binding(0) var<storage, read> statics: array<Instance>;
@group(1) @binding(1) var<storage, read> dynamics: array<Instance>;
@group(1) @binding(2) var<storage, read> entries: array<u32>;

// The draw's base-colour texture (parts/assets.rs): a mesh asset's, or white.
@group(2) @binding(0) var base_texture: texture_2d<f32>;
@group(2) @binding(1) var base_sampler: sampler;

fn instance_of(entry: u32) -> Instance {
    let slot = entry & 0x7fffffffu;
    if (entry >> 31u) == 0u {
        return statics[slot];
    }
    return dynamics[slot];
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(9) uv: vec2<f32>,
    @builtin(instance_index) index: u32,
    @builtin(vertex_index) vertex: u32,
) -> VertexOut {
    let instance = instance_of(entries[index]);
    return place(
        pool_level(vertex),
        position,
        normal,
        uv,
        instance.r0,
        instance.r1,
        instance.r2,
        instance.color,
        instance.transparency,
        instance.mesh_material,
    );
}

// The transparent pass's pulled vertices (parts/transparent.rs): every see-through instance in one draw, back to
// front, without vertex or index buffers. The draw's vertices are the sorted instances' mesh vertices one instance
// after another. `pulled.words` holds a chunk table and then the records (`PulledRecord` in parts/transparent.rs:
// entry, start, mesh; three words each): the chunk table names, for every `PULLED_CHUNK` vertices of the draw, the
// record whose vertices the chunk's first one is, so a vertex finds its own record a step or two after it. Its mesh
// vertex is in the unindexed pool (parts/mesh.rs) at the record's `mesh` (low 30 bits: the mesh's first vertex
// there; top 2: its level of detail) plus how far into the instance it is. One draw keeps the order exact: the GPU
// blends a draw's triangles in order.

// `PULLED_CHUNK` in parts/transparent.rs, as a shift.
const PULLED_CHUNK_SHIFT: u32 = 6u;

struct Pulled {
    count: u32,
    chunks: u32,
    words: array<u32>,
}

@group(1) @binding(3) var<storage, read> pulled: Pulled;
@group(1) @binding(4) var<storage, read> pulled_vertices: array<vec4<f32>>;

// Where record `i` starts in the draw.
fn pulled_start(i: u32) -> u32 {
    return pulled.words[pulled.chunks + 3u * i + 1u];
}

@vertex
fn vs_pulled(@builtin(vertex_index) vertex: u32) -> VertexOut {
    // From the record the chunk starts in, the last one starting at or before `vertex`: every mesh has at least a
    // triangle, so few records start inside one chunk. Neighbouring vertices take the same steps.
    var i = pulled.words[vertex >> PULLED_CHUNK_SHIFT];
    for (var step = 0u; step < (1u << PULLED_CHUNK_SHIFT); step += 1u) {
        let next = i + 1u;
        if next >= pulled.count || pulled_start(next) > vertex {
            break;
        }
        i = next;
    }
    let base = pulled.chunks + 3u * i;
    let entry = pulled.words[base];
    let start = pulled.words[base + 1u];
    let mesh = pulled.words[base + 2u];
    let instance = instance_of(entry);
    let at = 2u * ((mesh & 0x3fffffffu) + vertex - start);
    let a = pulled_vertices[at];
    let b = pulled_vertices[at + 1u];
    return place(
        mesh >> 30u,
        a.xyz,
        vec3<f32>(a.w, b.xy),
        b.zw,
        instance.r0,
        instance.r1,
        instance.r2,
        instance.color,
        instance.transparency,
        instance.mesh_material,
    );
}
