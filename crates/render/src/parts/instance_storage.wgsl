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

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @builtin(instance_index) index: u32,
    @builtin(vertex_index) vertex: u32,
) -> VertexOut {
    let entry = entries[index];
    let slot = entry & 0x7fffffffu;
    var instance: Instance;
    if (entry >> 31u) == 0u {
        instance = statics[slot];
    } else {
        instance = dynamics[slot];
    }
    return place(
        vertex,
        position,
        normal,
        instance.r0,
        instance.r1,
        instance.r2,
        instance.color,
        instance.transparency,
        instance.mesh_material,
    );
}
