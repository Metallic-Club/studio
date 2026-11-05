// Bloom's chain as compute dispatches (`post/bloom.rs`, #145), appended to `bloom.wgsl`, whose filters it runs: the
// same taps at the same coordinates as the fragment entry points there, one invocation per destination texel. The
// renderer records every dispatch of the chain in one compute pass, where the render passes cost about 18 µs of CPU
// each. The upsample cannot blend, so it writes each level's sum into a texture of its own: `level`'s own value plus
// the tent over the smaller sum, which is what the blend adds up to.

// The level being written. `post/bloom.rs` declares rg11b10ufloat here instead where the adapter writes it as
// storage (not WebGPU's minimum, nor OpenGL's).
@group(0) @binding(3) var dest: texture_storage_2d<rgba16float, write>;
// The upsample's own level, which its sum adds the smaller sum's tent onto.
@group(0) @binding(4) var level: texture_2d<f32>;

// The texel `id` of `dest` as the fragment entry points see it: its centre, 0..1 over the destination.
fn texel_uv(id: vec2<u32>, size: vec2<u32>) -> vec2<f32> {
    return (vec2<f32>(id) + vec2<f32>(0.5)) / vec2<f32>(size);
}

@compute @workgroup_size(8, 8)
fn cs_first_dual(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dest);
    if any(id.xy >= size) {
        return;
    }
    textureStore(dest, id.xy, vec4<f32>(threshold(down_dual(texel_uv(id.xy, size))), 1.0));
}

@compute @workgroup_size(8, 8)
fn cs_first_wide(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dest);
    if any(id.xy >= size) {
        return;
    }
    textureStore(dest, id.xy, vec4<f32>(threshold(down_wide(texel_uv(id.xy, size))), 1.0));
}

@compute @workgroup_size(8, 8)
fn cs_first_quarter(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dest);
    if any(id.xy >= size) {
        return;
    }
    textureStore(dest, id.xy, vec4<f32>(threshold(down_quarter(texel_uv(id.xy, size))), 1.0));
}

@compute @workgroup_size(8, 8)
fn cs_down_dual(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dest);
    if any(id.xy >= size) {
        return;
    }
    textureStore(dest, id.xy, vec4<f32>(down_dual(texel_uv(id.xy, size)), 1.0));
}

@compute @workgroup_size(8, 8)
fn cs_down_wide(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dest);
    if any(id.xy >= size) {
        return;
    }
    textureStore(dest, id.xy, vec4<f32>(down_wide(texel_uv(id.xy, size)), 1.0));
}

@compute @workgroup_size(8, 8)
fn cs_up(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dest);
    if any(id.xy >= size) {
        return;
    }
    let own = textureLoad(level, id.xy, 0).rgb;
    textureStore(dest, id.xy, vec4<f32>(own + up_tent(texel_uv(id.xy, size)), 1.0));
}
