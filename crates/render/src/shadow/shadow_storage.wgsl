// The storage-buffer path of the caster pass (appended to shadow.wgsl): the instance index selects an entry of the
// cascade's list, which names the slot (bit 31: dynamic region) read from the region buffers.

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

fn shadow_instance(index: u32) -> Instance {
    let entry = entries[index];
    let slot = entry & 0x7fffffffu;
    if (entry >> 31u) == 0u {
        return statics[slot];
    }
    return dynamics[slot];
}

@vertex
fn vs_shadow(@location(0) position: vec3<f32>, @builtin(instance_index) index: u32) -> @builtin(position) vec4<f32> {
    let instance = shadow_instance(index);
    return shadow_place(position, instance.r0, instance.r1, instance.r2);
}

@vertex
fn vs_see_through(@location(0) position: vec3<f32>, @builtin(instance_index) index: u32) -> SeeThrough {
    let instance = shadow_instance(index);
    var out: SeeThrough;
    out.position = shadow_place(position, instance.r0, instance.r1, instance.r2);
    out.transparency = instance.transparency;
    return out;
}
