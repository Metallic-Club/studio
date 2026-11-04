// Copies a cached cascade's static map into the cascade's map (shadow/maps.rs, `copy_pipeline`): a full-screen
// triangle, each fragment writing the depth of its own texel of the static map, exactly.

@group(0) @binding(0) var static_map: texture_2d<f32>;

@vertex
fn vs_copy(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_copy(@builtin(position) position: vec4<f32>) -> @builtin(frag_depth) f32 {
    return textureLoad(static_map, vec2<i32>(position.xy), 0).r;
}
