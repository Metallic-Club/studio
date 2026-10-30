// Finish pass: reads the linear HDR scene, applies exposure, a scene-linear grade, the tone curve,
// vignette and output encoding with dither. Settings in `src/tonemap.rs` (`FinishUniform`).
struct Finish {
    tint_exposure: vec4<f32>, // rgb: tint (linear, white = none), w: exposure incl. curve pre-scale
    grade: vec4<f32>,         // x: saturation, y: contrast, z: brightness (display), w: vignette
    mode: vec4<f32>,          // x: curve index, y: 1 if the target encodes sRGB itself, z: dither on
}

@group(0) @binding(0) var hdr_tex: texture_2d<f32>;
@group(0) @binding(1) var<uniform> fin: Finish;

const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// ---- AgX (Troy Sobotka; polynomial fit). WGSL mat3x3 takes columns. ----
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

fn agx(c_in: vec3<f32>, punchy: bool) -> vec3<f32> {
    var c = AGX_INSET * max(c_in, vec3<f32>(1e-10));
    c = clamp(log2(max(c, vec3<f32>(1e-10))), vec3<f32>(AGX_MIN_EV), vec3<f32>(AGX_MAX_EV));
    c = agx_contrast((c - AGX_MIN_EV) / (AGX_MAX_EV - AGX_MIN_EV));
    if (punchy) {
        c = pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.35));
        let l = dot(c, LUMA);
        c = l + 1.4 * (c - l);
    }
    return pow(clamp(AGX_OUTSET * c, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
}

// ---- Khronos PBR Neutral ----
fn pbr_neutral(c_in: vec3<f32>) -> vec3<f32> {
    let start = 0.8 - 0.04;
    let desat = 0.15;
    let x = min(c_in.r, min(c_in.g, c_in.b));
    var offset = 0.04;
    if (x < 0.08) {
        offset = x - 6.25 * x * x;
    }
    var c = c_in - offset;
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

// ---- ACES: Hill's fit (rows as published, so transposed for WGSL) and Narkowicz's ----
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
    return transpose(ACES_OUT_ROWS) * (a / b);
}

fn narkowicz(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return (x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14);
}

// Same order as `ToneCurve::index`. The pre-scale is already in the exposure.
fn apply_curve(x: vec3<f32>, curve: u32) -> vec3<f32> {
    var c: vec3<f32>;
    switch curve {
        case 1u: { c = agx(x, false); }
        case 2u: { c = agx(x, true); }
        case 3u: { c = pbr_neutral(x); }
        case 4u: { c = aces_hill(x); }
        case 5u: { c = x; }
        default: { c = narkowicz(x); }
    }
    return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
}

// Grading in scene-linear light, before the curve, so highlights still roll off through it.
fn grade(c_in: vec3<f32>) -> vec3<f32> {
    var c = c_in * fin.tint_exposure.rgb;
    let l = dot(c, LUMA);
    c = max(vec3<f32>(l) + (1.0 + fin.grade.x) * (c - vec3<f32>(l)), vec3<f32>(0.0));
    if (fin.grade.y != 0.0) {
        c = 0.18 * pow(max(c, vec3<f32>(1e-8)) / 0.18, vec3<f32>(1.0 + fin.grade.y));
    }
    return c;
}

struct FinishOut {
    @builtin(position) clip: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> FinishOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: FinishOut;
    out.clip = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: FinishOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.clip.xy);
    let size = vec2<f32>(textureDimensions(hdr_tex));
    var c = max(textureLoad(hdr_tex, px, 0).rgb, vec3<f32>(0.0)) * fin.tint_exposure.w;
    c = grade(c);
    c = apply_curve(c, u32(fin.mode.x + 0.5));
    if (fin.grade.w > 0.0) {
        let d = in.clip.xy / size - 0.5;
        let r2 = dot(d * vec2<f32>(1.3, 1.0), d * vec2<f32>(1.3, 1.0)) * 2.2;
        c = c * (1.0 - fin.grade.w * clamp(r2, 0.0, 1.0));
    }
    var e = linear_to_srgb(c) + fin.grade.z;
    if (fin.mode.z > 0.5) {
        e = e + (hash12(in.clip.xy) - 0.5) / 255.0;
    }
    if (fin.mode.y > 0.5) {
        return vec4<f32>(srgb_to_linear(clamp(e, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
    }
    return vec4<f32>(e, 1.0);
}
