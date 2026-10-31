// Prelude of every `FullscreenPass` (src/post.rs). The pass appends its own `fs_main`.

// 1 when the output texture encodes sRGB in hardware (set per pipeline).
override POST_OUTPUT_SRGB: f32 = 0.0;

@group(0) @binding(0) var post_input: texture_2d<f32>;
@group(0) @binding(1) var post_sampler: sampler;
@group(0) @binding(2) var post_aux: texture_2d<f32>;

struct PostIn {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>, // 0,0 top left
}

@vertex
fn post_vs(@builtin(vertex_index) i: u32) -> PostIn {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: PostIn;
    out.clip = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(uv.x, 1.0 - uv.y);
    return out;
}

// Apply to the colour a pass returns. HDR and LDR intermediates store values as they are; a
// hardware-sRGB target (only the last LDR pass writes one) needs linear values.
fn post_output(c: vec3<f32>) -> vec3<f32> {
    if (POST_OUTPUT_SRGB > 0.5) {
        return srgb_to_linear(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)));
    }
    return c;
}
