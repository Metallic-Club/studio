// The upscale pass (`upscale.rs`): the scene colour, drawn at the render scale into the top-left corner of its
// texture, stretched over the whole output with bilinear filtering.

struct Upscale {
    // xy: the drawn corner's share of the texture (render size / texture size); zw: the farthest texture coordinate
    // read, half a texel inside the corner, so the filter never reaches texels outside it.
    rect: vec4<f32>,
}

@group(0) @binding(0) var<uniform> upscale: Upscale;
@group(0) @binding(1) var scene_color: texture_2d<f32>;
@group(0) @binding(2) var scene_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    // 0..1 over the output, from the top left.
    @location(0) uv: vec2<f32>,
}

// One triangle covering the output.
@vertex
fn vs_upscale(@builtin(vertex_index) index: u32) -> VertexOutput {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOutput;
    out.clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_upscale(in: VertexOutput) -> @location(0) vec4<f32> {
    // The sampler clamps to the texture's edge, which is the corner's top and left edges.
    let uv = min(in.uv * upscale.rect.xy, upscale.rect.zw);
    return textureSampleLevel(scene_color, scene_sampler, uv, 0.0);
}
