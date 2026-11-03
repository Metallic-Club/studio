// Scatters a frame's uploaded instances into their slots: one thread per instance.
//
// `upload` holds, in 16-byte words: a header (x = how many instances), then the destination of each instance as
// one u32 (four per word; bit 31 set for the dynamic region, the rest the slot), then the instances themselves,
// four words each. See `GpuScene::upload` (scene/gpu.rs).

@group(0) @binding(0) var<storage, read> upload: array<vec4<u32>>;
@group(0) @binding(1) var<storage, read_write> statics: array<vec4<u32>>;
@group(0) @binding(2) var<storage, read_write> dynamics: array<vec4<u32>>;

const WORKGROUP: u32 = 64u;

@compute @workgroup_size(64)
fn scatter(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let count = upload[0].x;
    let i = id.x + id.y * groups.x * WORKGROUP;
    if i >= count {
        return;
    }
    let dest = upload[1u + i / 4u][i % 4u];
    let slot = dest & 0x7fffffffu;
    let src = 1u + (count + 3u) / 4u + i * 4u;
    let dst = slot * 4u;
    if (dest >> 31u) == 0u {
        statics[dst] = upload[src];
        statics[dst + 1u] = upload[src + 1u];
        statics[dst + 2u] = upload[src + 2u];
        statics[dst + 3u] = upload[src + 3u];
    } else {
        dynamics[dst] = upload[src];
        dynamics[dst + 1u] = upload[src + 1u];
        dynamics[dst + 2u] = upload[src + 2u];
        dynamics[dst + 3u] = upload[src + 3u];
    }
}
