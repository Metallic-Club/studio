// Finish pass: reads the linear HDR scene, applies exposure, a scene-linear grade, the tone curve,
// vignette and output encoding with dither. Settings in `src/tonemap.rs` (`FinishUniform`).
struct Finish {
    tint_exposure: vec4<f32>, // rgb: tint (linear, white = none), w: exposure incl. curve pre-scale
    grade: vec4<f32>,         // x: saturation, y: contrast, z: brightness (display), w: vignette
    mode: vec4<f32>,          // x: curve index, y: 1 if the target encodes sRGB itself, z: dither on,
                              // w: auto exposure multiplier (copied from the meter on the GPU)
    debug: vec4<f32>,         // x: 1 shows the exposure light meter (#23) instead of the image
}

// Light meter (#23): the exposed scene light `c` in false colour, one band per stop around the
// light the tone curve shows as middle grey (`mid`): neutral grey = correctly exposed, blues
// under, green to red over, white clipped.
fn exposure_zones(c: vec3<f32>, mid: f32) -> vec3<f32> {
    let ev = log2(max(dot(c, CURVE_LUMA), 1e-6) / max(mid, 1e-6));
    if (ev < -5.0) { return vec3<f32>(0.01, 0.01, 0.015); }
    if (ev < -3.0) { return vec3<f32>(0.03, 0.04, 0.22); }
    if (ev < -1.5) { return vec3<f32>(0.06, 0.18, 0.55); }
    if (ev < -0.5) { return vec3<f32>(0.08, 0.38, 0.4); }
    if (ev < 0.5) { return vec3<f32>(0.2, 0.2, 0.2); }
    if (ev < 1.5) { return vec3<f32>(0.3, 0.5, 0.12); }
    if (ev < 2.5) { return vec3<f32>(0.75, 0.62, 0.1); }
    if (ev < 4.0) { return vec3<f32>(0.85, 0.3, 0.05); }
    if (ev < 6.0) { return vec3<f32>(0.75, 0.05, 0.04); }
    return vec3<f32>(1.0);
}

@group(0) @binding(0) var hdr_tex: texture_2d<f32>;
@group(0) @binding(1) var<uniform> fin: Finish;

// Grading in scene-linear light, before the curve, so highlights still roll off through it.
fn grade(c_in: vec3<f32>) -> vec3<f32> {
    var c = c_in * fin.tint_exposure.rgb;
    let l = dot(c, CURVE_LUMA);
    c = max(vec3<f32>(l) + (1.0 + fin.grade.x) * (c - vec3<f32>(l)), vec3<f32>(0.0));
    if (fin.grade.y != 0.0) {
        c = 0.18 * pow(max(c, vec3<f32>(1e-8)) / 0.18, vec3<f32>(1.0 + fin.grade.y));
    }
    return c;
}

// NaN and negatives become black, infinity the largest half float, so one bad pixel cannot
// poison the curve (bit tests: compilers may fold `x != x` away).
fn finite_light(c: vec3<f32>) -> vec3<f32> {
    var o = vec3<f32>(0.0);
    for (var i = 0; i < 3; i = i + 1) {
        let bits = bitcast<u32>(c[i]);
        let nan = (bits & 0x7fffffffu) > 0x7f800000u;
        if (!nan && (bits & 0x80000000u) == 0u) {
            o[i] = min(c[i], 65504.0);
        }
    }
    return o;
}

// The meter's multiplier; anything not positive and finite means none.
fn auto_multiplier(m: f32) -> f32 {
    if (m > 0.0 && m < 1e30) {
        return m;
    }
    return 1.0;
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
    var c = finite_light(textureLoad(hdr_tex, px, 0).rgb) * (fin.tint_exposure.w * auto_multiplier(fin.mode.w));
    if (fin.debug.x > 0.5) {
        let curve = u32(fin.mode.x + 0.5);
        let zone = exposure_zones(c, invert_curve(vec3<f32>(0.18), curve).x);
        if (fin.mode.y > 0.5) {
            return vec4<f32>(zone, 1.0);
        }
        return vec4<f32>(linear_to_srgb(zone), 1.0);
    }
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
