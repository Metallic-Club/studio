// Metallic style sample: the "finish" pass. HDR scene colour -> display.
//
// Research sample, not engine code. The M1 renderer tonemaps inside the forward shader; moving this into a
// separate full-screen pass is the hook that lets every style insert passes before or after it.
// Steps: exposure -> white balance tint -> tonemap curve -> contrast/saturation -> optional 3D LUT -> encode.
// Curves: 0 ACES fit (Narkowicz), 1 Khronos PBR Neutral (keeps base colours true, good for stylised art),
//         2 clamp only (flat colours for pixel art and unlit 2D).
// Validate: naga docs/research/style-shaders/tonemap.wgsl

struct Grade {
    exposure: f32,          // linear multiplier
    curve: u32,             // 0 ACES fit, 1 PBR Neutral, 2 clamp
    contrast: f32,          // 1 = unchanged, pivot at mid grey in display space
    saturation: f32,        // 1 = unchanged
    tint: vec4<f32>,        // rgb white-balance multiplier
    lut_strength: f32,      // 0 = no LUT
    lut_size: f32,          // e.g. 32 for a 32^3 LUT
    encode_srgb: u32,       // 1 if the target is a UNORM (non-sRGB) view and the shader must encode
    _pad: u32,
}

@group(0) @binding(0) var<uniform> grade: Grade;
@group(0) @binding(1) var hdr_tex: texture_2d<f32>;
@group(0) @binding(2) var lut_tex: texture_3d<f32>;
@group(0) @binding(3) var lin_samp: sampler;

struct FsIn {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> FsIn {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: FsIn;
    out.pos = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

fn aces_fit(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Khronos PBR Neutral tone mapper (published by the Khronos 3D Commerce group).
fn pbr_neutral(c_in: vec3<f32>) -> vec3<f32> {
    let start = 0.8 - 0.04;
    let desat = 0.15;
    var c = c_in;
    let x = min(c.r, min(c.g, c.b));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    c = c - offset;
    let peak = max(c.r, max(c.g, c.b));
    if (peak < start) {
        return c;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    c = c * (new_peak / peak);
    let g = 1.0 - 1.0 / (desat * (peak - new_peak) + 1.0);
    return mix(c, vec3<f32>(new_peak), g);
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3<f32>(0.0)) + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@fragment
fn fs_main(in: FsIn) -> @location(0) vec4<f32> {
    let hdr = textureLoad(hdr_tex, vec2<i32>(in.pos.xy), 0).rgb;
    var c = max(hdr, vec3<f32>(0.0)) * grade.exposure * grade.tint.rgb;
    switch grade.curve {
        case 0u: { c = aces_fit(c); }
        case 1u: { c = clamp(pbr_neutral(c), vec3<f32>(0.0), vec3<f32>(1.0)); }
        default: { c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)); }
    }
    // Grade in display space so artists' numbers match what they see.
    var s = linear_to_srgb(c);
    s = (s - 0.5) * grade.contrast + 0.5;
    s = mix(vec3<f32>(luminance(s)), s, grade.saturation);
    s = clamp(s, vec3<f32>(0.0), vec3<f32>(1.0));
    // LUT lookup at texel centres. Sampled with an explicit level so no derivatives are needed.
    let scale = (grade.lut_size - 1.0) / grade.lut_size;
    let offset = 0.5 / grade.lut_size;
    let graded = textureSampleLevel(lut_tex, lin_samp, s * scale + offset, 0.0).rgb;
    s = mix(s, graded, grade.lut_strength);
    if (grade.encode_srgb == 1u) {
        return vec4<f32>(s, 1.0);
    }
    // sRGB target: the hardware encodes, so hand it linear values.
    return vec4<f32>(srgb_to_linear(s), 1.0);
}
