// The finish pass (`post/finish.rs`): the HDR scene colour to the output in one full-screen pass. Per pixel: the scene
// colour (upscaled with one bilinear tap when the scene is drawn at a render scale, nudged along MLAA's blend weights
// when MLAA runs), mixed with bloom, scaled by the exposure and the colour filter, the tone curve, the grade
// (saturation and contrast), sRGB encoding where the output does not encode, and a dither of half an output level.
//
// Four entry points, one per combination of bloom and MLAA, so each binds only the textures it reads: a frame
// without bloom or MLAA binds neither (ADR 0008 §8, disabled passes are free).

struct Finish {
    // xy: the drawn corner's share of the scene texture; zw: the farthest texture coordinate read, half a texel inside
    // the corner, so the bilinear filter never reaches texels outside it.
    rect: vec4<f32>,
    // x: exposure, y: bloom's share of the result, z: 1 when the shader encodes sRGB (the output does not), w: the
    // dither's amplitude (one output level; 0 for outputs finer than 8 bits).
    params: vec4<f32>,
    // rgb: the colour filter (a multiplier before the tone curve); w: what bloom's texture is scaled by (one over the
    // levels it sums).
    tint: vec4<f32>,
    // x: saturation (1 keeps it), y: contrast around mid-grey (1 keeps it); zw unused.
    grade: vec4<f32>,
    // xy: one scene texel in texture coordinates; zw: the drawn corner's size in texels (MLAA's weights).
    texel: vec4<f32>,
}

@group(0) @binding(0) var<uniform> finish: Finish;
@group(0) @binding(1) var scene_color: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;
@group(0) @binding(3) var bloom: texture_2d<f32>;
@group(0) @binding(4) var mlaa_weights: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    // 0..1 over the output, from the top left.
    @location(0) uv: vec2<f32>,
}

// One triangle covering the output.
@vertex
fn vs_finish(@builtin(vertex_index) index: u32) -> VertexOutput {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOutput;
    out.clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);
// The tone curve is the identity up to KNEE and rolls off smoothly towards 1 above it (`tone_curve` in finish.rs).
const KNEE: f32 = 0.85;

// Per channel: x up to the knee, then a shoulder with the same slope at the knee that approaches 1. Below the knee
// the image is what it was before the post chain (a clamp at 1); above it highlights keep a little detail instead of
// clipping.
fn tone(x: vec3<f32>) -> vec3<f32> {
    let over = max(x - vec3<f32>(KNEE), vec3<f32>(0.0));
    let shoulder = vec3<f32>(KNEE) + (1.0 - KNEE) * (vec3<f32>(1.0) - exp(-over / (1.0 - KNEE)));
    return select(x, shoulder, x > vec3<f32>(KNEE));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308));
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

// Where the scene is read for an output pixel at `uv`: the drawn corner, clamped half a texel inside it.
fn scene_uv(uv: vec2<f32>) -> vec2<f32> {
    return min(uv * finish.rect.xy, finish.rect.zw);
}

// MLAA's neighbourhood blending as one bilinear tap: the scene texel under `uv` blends with the neighbour across its
// strongest edge by that edge's weight, which reading between the two texels does. The weights of the texel itself
// (with the texel above, .r; to the left, .b) and of its right and lower neighbours (which blend with it: .a and .g)
// say how much (`post/mlaa.wgsl`). With a render scale the nearest texel's weights move the upscale's tap.
fn mlaa_uv(uv: vec2<f32>) -> vec2<f32> {
    let last = vec2<i32>(finish.texel.zw) - vec2<i32>(1);
    let p = min(vec2<i32>(uv / finish.texel.xy), last);
    let own = textureLoad(mlaa_weights, p, 0);
    let right = textureLoad(mlaa_weights, min(p + vec2<i32>(1, 0), last), 0).a;
    let down = textureLoad(mlaa_weights, min(p + vec2<i32>(0, 1), last), 0).g;
    let up = own.r;
    let left = own.b;
    var offset = vec2<f32>(0.0);
    if max(up, down) >= max(left, right) {
        offset.y = select(-up, down, down > up);
    } else {
        offset.x = select(-left, right, right > left);
    }
    return uv + offset * finish.texel.xy;
}

// Everything after the scene colour is read: bloom mixed in, exposure and colour filter, tone curve, grade, encoding
// and dither.
// `c` with every channel that is not a finite number (NaN, ±infinity; an exponent of all ones) made 0, the rest kept
// between 0 and the largest half float. The scene colour holds whatever the scene's shaders wrote, and one NaN pixel
// would otherwise reach every pixel through bloom's blur and blank the image. Tested on the bits: comparisons with
// NaN may be folded away by shader compilers.
fn finite(c: vec3<f32>) -> vec3<f32> {
    let bits = bitcast<vec3<u32>>(c);
    let ok = (bits & vec3<u32>(0x7f800000u)) != vec3<u32>(0x7f800000u);
    return select(vec3<f32>(0.0), clamp(c, vec3<f32>(0.0), vec3<f32>(65504.0)), ok);
}

// Per channel, the dither's share: none at exactly 0 or 1 (in output levels, `encoded` being 0..1), so black stays
// black and white white whatever the noise and the output's rounding, and all of it one level in.
fn dither_weight(encoded: vec3<f32>) -> vec3<f32> {
    let steps = 1.0 / max(finish.params.w, 1e-6);
    return clamp(encoded * steps, vec3<f32>(0.0), vec3<f32>(1.0)) * clamp((1.0 - encoded) * steps, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn finish_color(scene: vec3<f32>, bloomed: vec3<f32>, pixel: vec2<f32>) -> vec4<f32> {
    let hdr = mix(finite(scene), bloomed, finish.params.y);
    var color = tone(hdr * finish.params.x * finish.tint.rgb);
    let luma = dot(color, LUMA);
    color = max(mix(vec3<f32>(luma), color, finish.grade.x), vec3<f32>(0.0));
    if finish.grade.y != 1.0 {
        // pow(0, c) is not a number for c ≤ 0 (black under a contrast of 0), so the contrast stays above 0.
        color = 0.18 * pow(color / 0.18, vec3<f32>(max(finish.grade.y, 1e-3)));
    }
    color = min(color, vec3<f32>(1.0));
    // Half an output level of noise, from the pixel's position, so smooth gradients do not band.
    let noise = fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715)))) - 0.5;
    let encoded = linear_to_srgb(color);
    let dithered = encoded + noise * finish.params.w * dither_weight(encoded);
    if finish.params.z > 0.5 {
        return vec4<f32>(dithered, 1.0);
    }
    if finish.params.w > 0.0 {
        // The output encodes: dither in its encoded steps.
        color = srgb_to_linear(clamp(dithered, vec3<f32>(0.0), vec3<f32>(1.0)));
    }
    return vec4<f32>(color, 1.0);
}

@fragment
fn fs_finish(in: VertexOutput) -> @location(0) vec4<f32> {
    let scene = textureSampleLevel(scene_color, linear_sampler, scene_uv(in.uv), 0.0).rgb;
    return finish_color(scene, scene, in.clip.xy);
}

@fragment
fn fs_finish_bloom(in: VertexOutput) -> @location(0) vec4<f32> {
    let scene = textureSampleLevel(scene_color, linear_sampler, scene_uv(in.uv), 0.0).rgb;
    let bloomed = textureSampleLevel(bloom, linear_sampler, in.uv, 0.0).rgb * finish.tint.w;
    return finish_color(scene, bloomed, in.clip.xy);
}

@fragment
fn fs_finish_mlaa(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = min(mlaa_uv(scene_uv(in.uv)), finish.rect.zw);
    let scene = textureSampleLevel(scene_color, linear_sampler, uv, 0.0).rgb;
    return finish_color(scene, scene, in.clip.xy);
}

@fragment
fn fs_finish_bloom_mlaa(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = min(mlaa_uv(scene_uv(in.uv)), finish.rect.zw);
    let scene = textureSampleLevel(scene_color, linear_sampler, uv, 0.0).rgb;
    let bloomed = textureSampleLevel(bloom, linear_sampler, in.uv, 0.0).rgb * finish.tint.w;
    return finish_color(scene, bloomed, in.clip.xy);
}
