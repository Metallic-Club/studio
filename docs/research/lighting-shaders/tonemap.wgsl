// Finish pass for the lighting pipeline: exposure, grading in scene-linear, tonemap curve, vignette,
// output encoding and dither. Research prototype for issue #37 (complements
// docs/research/style-shaders/tonemap.wgsl on the render-styles branch, which adds the 3D LUT).
//
// Curves (Lighting/ColorGradingEffect.TonemapperPreset):
//   0 Default  = AgX + "punchy" look (power 1.35, saturation 1.4)       <- Realistic look default
//   1 AgX      = AgX base (Troy Sobotka; minimal polynomial fit by Benjamin Wrensch)
//   2 Neutral  = Khronos PBR Neutral (Apache-2.0 reference)             <- Soft / Toon / Pixel looks
//   3 ACES     = Stephen Hill's RRT+ODT fit (MIT, via BakingLab)
//   4 Filmic   = Narkowicz ACES fit (what Task 3 ships in common.wgsl today)
//   5 Retro    = clamp (pre-2019 Roblox-like flat look; pair with lower Brightness)
// Every curve gets a pre-scale so scene 0.18 lands on the same display value as AgX (0.214 linear):
// switching curves changes contrast and colour, not overall brightness. Values from
// reference/tonemap.py + render.py (TONEMAP_NORMALISE).

struct Exposure {
    exposure: f32,      // multiplier applied to the HDR target (from exposure_histogram.wgsl)
    ev100: f32,
    avg_log2_lum: f32,
    target_ev100: f32,
}

struct FinishParams {
    tint: vec3<f32>,          // ColorCorrectionEffect.TintColor, linear (white = 1)
    curve: u32,               // TonemapperPreset index above
    saturation: f32,          // ColorCorrectionEffect.Saturation (-1..1), applied as 1 + s
    contrast: f32,            // ColorCorrectionEffect.Contrast (-1..1), log contrast around 0.18
    brightness: f32,          // ColorCorrectionEffect.Brightness, added after the curve (display space)
    vignette: f32,            // 0..1
    night_adaptation: f32,    // Lighting.NightAdaptation 0..1; strength follows the metered EV100
    output_is_srgb_format: u32, // 1: target is *Srgb (hardware encodes), 0: encode here
    dither: f32,              // 1/255 for 8-bit targets
    pre_exposure_scale: f32,  // 1.0 when the HDR target is already pre-exposed with this frame's value
}

@group(0) @binding(0) var hdr_tex: texture_2d<f32>;
@group(0) @binding(1) var hdr_sampler: sampler; // linear clamp (also upsamples when render scale < 1)
@group(0) @binding(2) var<storage, read> exposure_buf: Exposure;
@group(0) @binding(3) var<uniform> params: FinishParams;

const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// ---- AgX ----------------------------------------------------------------------------------------
// WGSL mat3x3 takes columns, like GLSL mat3, so the published GLSL values are used in the same order.
const AGX_INSET: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
    vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336),
    vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104),
);
const AGX_OUTSET: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
    vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
    vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116),
);
const AGX_MIN_EV: f32 = -12.47393;
const AGX_MAX_EV: f32 = 4.026069;

fn agx_contrast(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

// look: 0 none, 1 punchy. Returns linear display light.
fn agx(c_in: vec3<f32>, look: u32) -> vec3<f32> {
    var c = AGX_INSET * max(c_in, vec3<f32>(1e-10));
    c = clamp(log2(max(c, vec3<f32>(1e-10))), vec3<f32>(AGX_MIN_EV), vec3<f32>(AGX_MAX_EV));
    c = (c - AGX_MIN_EV) / (AGX_MAX_EV - AGX_MIN_EV);
    c = agx_contrast(c);
    if (look == 1u) {
        c = pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.35));
        let l = dot(c, LUMA);
        c = l + 1.4 * (c - l);
    }
    c = AGX_OUTSET * c;
    // AgX output carries a 2.2 power; return linear so the sRGB encode below stays standard.
    return pow(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
}

// ---- Khronos PBR Neutral ----------------------------------------------------------------------
fn pbr_neutral(c_in: vec3<f32>) -> vec3<f32> {
    let start_compression = 0.8 - 0.04;
    let desaturation = 0.15;
    var c = c_in;
    let x = min(c.r, min(c.g, c.b));
    var offset = 0.04;
    if (x < 0.08) {
        offset = x - 6.25 * x * x;
    }
    c = c - offset;
    let peak = max(c.r, max(c.g, c.b));
    if (peak < start_compression) {
        return c;
    }
    let d = 1.0 - start_compression;
    let new_peak = 1.0 - d * d / (peak + d - start_compression);
    c = c * (new_peak / peak);
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(c, vec3<f32>(new_peak), g);
}

// ---- ACES fits ------------------------------------------------------------------------------------
// Hill: rows as published (HLSL row-major); transpose() gives WGSL's column-major equivalent.
const ACES_IN_ROWS: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.59719, 0.35458, 0.04823),
    vec3<f32>(0.07600, 0.90834, 0.01566),
    vec3<f32>(0.02840, 0.13383, 0.83777),
);
const ACES_OUT_ROWS: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(1.60475, -0.53108, -0.07367),
    vec3<f32>(-0.10208, 1.10813, -0.00605),
    vec3<f32>(-0.00327, -0.07276, 1.07602),
);

fn aces_hill(c: vec3<f32>) -> vec3<f32> {
    let v = transpose(ACES_IN_ROWS) * max(c, vec3<f32>(0.0));
    let a = v * (v + 0.0245786) - 0.000090537;
    let b = v * (0.983729 * v + 0.4329510) + 0.238081;
    return clamp(transpose(ACES_OUT_ROWS) * (a / b), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn aces_narkowicz(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Pre-scale per curve so 0.18 maps to AgX's display value (see header).
fn curve_prescale(curve: u32) -> f32 {
    switch curve {
        case 0u: { return 1.7316; } // AgX punchy
        case 1u: { return 1.0; }    // AgX
        case 2u: { return 1.4137; } // PBR Neutral
        case 3u: { return 1.6752; } // ACES Hill
        case 4u: { return 0.8306; } // Narkowicz
        default: { return 1.0; }    // clamp
    }
}

fn apply_curve(c: vec3<f32>, curve: u32) -> vec3<f32> {
    let x = c * curve_prescale(curve);
    switch curve {
        case 0u: { return agx(x, 1u); }
        case 1u: { return agx(x, 0u); }
        case 2u: { return clamp(pbr_neutral(x), vec3<f32>(0.0), vec3<f32>(1.0)); }
        case 3u: { return aces_hill(x); }
        case 4u: { return aces_narkowicz(x); }
        default: { return clamp(x, vec3<f32>(0.0), vec3<f32>(1.0)); }
    }
}

// ---- grading in scene-linear, before the curve (assistant's call: grading before the curve keeps
// highlights rolling off through the curve instead of clipping) --------------------------------------
fn grade(c_in: vec3<f32>) -> vec3<f32> {
    var c = c_in;
    // Scotopic night look: desaturate toward a blue-shifted luminance as the metered scene gets dark
    // (full at EV100 -3, none at EV100 5).
    let night = params.night_adaptation * 0.7 * (1.0 - smoothstep(-3.0, 5.0, exposure_buf.ev100));
    let l0 = dot(c, LUMA);
    c = mix(c, l0 * vec3<f32>(0.72, 0.82, 1.0), night);
    c = c * params.tint;
    let l = dot(c, LUMA);
    c = max(vec3<f32>(l) + (1.0 + params.saturation) * (c - vec3<f32>(l)), vec3<f32>(0.0));
    if (params.contrast != 0.0) {
        c = 0.18 * pow(max(c, vec3<f32>(1e-8)) / 0.18, vec3<f32>(1.0 + params.contrast));
    }
    return c;
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

// Interleaved gradient noise (Jimenez 2014): cheap, no texture, stable per pixel.
fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

struct FsIn {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> FsIn {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: FsIn;
    out.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(uv.x, 1.0 - uv.y);
    return out;
}

@fragment
fn fs_finish(in: FsIn) -> @location(0) vec4<f32> {
    var c = textureSampleLevel(hdr_tex, hdr_sampler, in.uv, 0.0).rgb;
    // The HDR target is pre-exposed with last frame's value; pre_exposure_scale corrects to this frame.
    c = c * params.pre_exposure_scale;
    c = grade(c);
    c = apply_curve(c, params.curve);
    // Vignette: smooth darkening toward the corners, aspect-aware.
    let d = in.uv - 0.5;
    let r2 = dot(d * vec2<f32>(1.3, 1.0), d * vec2<f32>(1.3, 1.0)) * 2.2;
    c = c * (1.0 - params.vignette * clamp(r2, 0.0, 1.0));
    var e = linear_to_srgb(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0))) + params.brightness;
    e = e + (ign(in.pos.xy) - 0.5) * params.dither;
    e = clamp(e, vec3<f32>(0.0), vec3<f32>(1.0));
    if (params.output_is_srgb_format == 1u) {
        return vec4<f32>(srgb_to_linear(e), 1.0);
    }
    return vec4<f32>(e, 1.0);
}
