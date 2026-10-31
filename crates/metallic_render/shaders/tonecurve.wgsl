// Tone curves shared by the finish pass and the scene shaders (no bindings). Same order as
// `ToneCurve::index`; the curve pre-scale is applied by the caller, in the exposure.

const CURVE_LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// ---- AgX (Troy Sobotka), minimal polynomial fit. WGSL mat3x3 takes columns. ----
// Ported from "Minimal AgX Implementation" by Benjamin Wrensch,
// https://iolite-engine.com/blog_posts/minimal_agx_implementation
// Copyright (c) 2024 Missing Deadlines (Benjamin Wrensch). MIT License; full notice in
// LICENSES/THIRD-PARTY-CODE.md.
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
        let l = dot(c, CURVE_LUMA);
        c = l + 1.4 * (c - l);
    }
    return pow(clamp(AGX_OUTSET * c, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
}

// ---- Khronos PBR Neutral ----
// Ported from the Khronos PBR Neutral reference (pbrNeutral.glsl),
// https://github.com/KhronosGroup/ToneMapping/tree/main/PBR_Neutral
// Copyright (c) 2024 The Khronos Group Inc. Licensed under the Apache License, Version 2.0
// (LICENSES/Apache-2.0.txt); notice and changes in LICENSES/THIRD-PARTY-CODE.md. Modified:
// GLSL to WGSL.
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
// Hill's fit is ported from Baking Lab (https://github.com/TheRealMJP/BakingLab, ACES.hlsl),
// Copyright (c) 2016 MJP, MIT License (LICENSES/THIRD-PARTY-CODE.md); fit by Stephen Hill.
// Narkowicz's fit: "ACES Filmic Tone Mapping Curve", Krzysztof Narkowicz, 2016 (a formula).
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

// Inverse of a curve: the scene colour that maps to `y` (0..1). Closed form for the per-channel
// curves (Filmic, Clamp). The others mix channels (AgX, ACES, Neutral), so they start from the
// per-channel inverse of the grey response and refine through the full curve.
fn invert_curve(y_in: vec3<f32>, curve: u32) -> vec3<f32> {
    let y = clamp(y_in, vec3<f32>(0.0), vec3<f32>(0.98));
    if (curve == 5u) {
        return y;
    }
    if (curve == 0u) {
        let a = 2.51 - 2.43 * y;
        let b = 0.03 - 0.59 * y;
        let c = -0.14 * y;
        return (-b + sqrt(b * b - 4.0 * a * c)) / (2.0 * a);
    }
    // Bisection in log2 space over [2^-16, 2^10].
    var lo = vec3<f32>(-16.0);
    var hi = vec3<f32>(10.0);
    for (var i = 0; i < 18; i = i + 1) {
        let mid = 0.5 * (lo + hi);
        let m = exp2(mid);
        let f = vec3<f32>(
            apply_curve(vec3<f32>(m.x), curve).x,
            apply_curve(vec3<f32>(m.y), curve).x,
            apply_curve(vec3<f32>(m.z), curve).x,
        );
        let below = f < y;
        lo = select(lo, mid, below);
        hi = select(mid, hi, below);
    }
    var x = select(exp2(0.5 * (lo + hi)), vec3<f32>(0.0), y <= vec3<f32>(0.0));
    // Damped multiplicative steps: converges for local contrasts below 4.
    let target_y = max(y, vec3<f32>(1e-5));
    for (var i = 0; i < 6; i = i + 1) {
        let f = max(apply_curve(x, curve), vec3<f32>(1e-5));
        x = x * sqrt(target_y / f);
    }
    return select(x, vec3<f32>(0.0), y <= vec3<f32>(0.0));
}
