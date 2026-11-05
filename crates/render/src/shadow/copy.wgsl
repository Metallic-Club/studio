// Copies a cached cascade's static map into the cascade's map (shadow/maps.rs, `copy_pipeline`): each fragment writes
// the depth of its own texel of the static map, exactly. `vs_copy` covers the whole map with one triangle; `vs_restore`
// covers only the footprints of the moving casters the map was last built with (#133), one rectangle per instance.

@group(0) @binding(0) var static_map: texture_2d<f32>;

@vertex
fn vs_copy(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

// One rectangle, given by two opposite corners in clip space (`rect.xy` and `rect.zw`), as two triangles: vertex `i`
// takes corner x from bit `i` of 0b110010 and y from bit `i` of 0b101100. It covers the texels whose centres it holds,
// as the casters' triangles do.
@vertex
fn vs_restore(@builtin(vertex_index) i: u32, @location(0) rect: vec4<f32>) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((0x32u >> i) & 1u), f32((0x2cu >> i) & 1u));
    return vec4<f32>(mix(rect.xy, rect.zw, corner), 0.0, 1.0);
}

@fragment
fn fs_copy(@builtin(position) position: vec4<f32>) -> @builtin(frag_depth) f32 {
    return textureLoad(static_map, vec2<i32>(position.xy), 0).r;
}
