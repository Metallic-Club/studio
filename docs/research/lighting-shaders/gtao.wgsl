// Ground-truth ambient occlusion (Jimenez et al. 2016, "Practical Realtime Strategies for Accurate
// Indirect Occlusion"), written from the paper's equations. Research prototype for issue #37.
// XeGTAO (MIT, Intel) is the reference for presets and costs: High = 3 slices x 3 steps per side,
// 0.56 ms at 1080p on an RTX 2060, 2.39 ms on an i7-1195G7 iGPU.
//
// Passes:
//   gtao_main     - one thread per AO pixel (full or half resolution); reads the depth buffer, rebuilds
//                   view-space normals from depth, integrates the cosine-weighted visible arc per slice.
//   gtao_denoise  - 3x3 depth-aware blur (XeGTAO uses 5x5 plus TAA; enough for MSAA tiers).
// Tiers: Low = half res, 2 slices x 2 steps; Medium = half res, 2 x 3; High = full res, 3 x 3;
// Ultra = full res, 4 x 4 + bent normals (later).
//
// Requires the Task 3 hook "sampleable depth" (TEXTURE_BINDING + StoreOp::Store).

struct GtaoParams {
    inv_proj: mat4x4<f32>,
    proj_scale: f32,       // proj[1][1] * 0.5 * depth_height: pixels per unit at view distance 1
    radius: f32,           // world radius in studs (default 2.5)
    falloff: f32,          // fraction of the radius over which samples fade out (default 0.6)
    final_power: f32,      // contrast of the result (default 2.2)
    slice_count: u32,
    steps_per_side: u32,
    frame_index: u32,      // rotates the noise when TAA accumulates; 0 for MSAA tiers
    half_res: u32,         // 1: AO target is half the depth size
}

@group(0) @binding(0) var depth_tex: texture_depth_2d;
@group(0) @binding(1) var<uniform> params: GtaoParams;
@group(0) @binding(2) var ao_raw: texture_storage_2d<r32float, write>;
@group(1) @binding(0) var ao_in: texture_2d<f32>;
@group(1) @binding(1) var ao_out: texture_storage_2d<r32float, write>;

const PI: f32 = 3.14159265358979;
const HALF_PI: f32 = 1.57079632679490;

fn view_pos(pix: vec2<i32>) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(depth_tex));
    let p = clamp(pix, vec2<i32>(0), size - 1);
    let d = textureLoad(depth_tex, p, 0);
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(size);
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let v = params.inv_proj * ndc;
    return v.xyz / v.w;
}

// Normal from depth: per axis, use the neighbour with the smaller depth change (avoids smearing
// normals across silhouettes of hard-edged Parts).
fn view_normal(pix: vec2<i32>, c: vec3<f32>) -> vec3<f32> {
    let l = view_pos(pix + vec2<i32>(-1, 0));
    let r = view_pos(pix + vec2<i32>(1, 0));
    let u = view_pos(pix + vec2<i32>(0, -1));
    let d = view_pos(pix + vec2<i32>(0, 1));
    var dx = r - c;
    if (abs(c.z - l.z) < abs(r.z - c.z)) {
        dx = c - l;
    }
    var dy = c - u;
    if (abs(d.z - c.z) < abs(c.z - u.z)) {
        dy = d - c;
    }
    return normalize(cross(dy, dx));
}

fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

@compute @workgroup_size(8, 8, 1)
fn gtao_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let out_size = textureDimensions(ao_raw);
    if (gid.x >= out_size.x || gid.y >= out_size.y) {
        return;
    }
    let scale = select(1, 2, params.half_res == 1u);
    let pix = vec2<i32>(gid.xy) * scale;
    let p = view_pos(pix);
    if (p.z > -1e-3 || p.z < -1e6) { // sky or invalid: no occlusion
        textureStore(ao_raw, vec2<i32>(gid.xy), vec4<f32>(1.0));
        return;
    }
    let n = view_normal(pix, p);
    let v = normalize(-p);

    let screen_radius = params.radius * params.proj_scale / -p.z; // pixels at full depth resolution
    if (screen_radius < 1.0) {
        textureStore(ao_raw, vec2<i32>(gid.xy), vec4<f32>(1.0));
        return;
    }
    let falloff_range = params.falloff * params.radius;
    let falloff_from = params.radius * (1.0 - params.falloff);
    let falloff_mul = -1.0 / falloff_range;
    let falloff_add = falloff_from / falloff_range + 1.0;

    let fi = f32(params.frame_index % 64u);
    let noise_slice = ign(vec2<f32>(gid.xy) + fi * 5.588238);
    let noise_step = ign(vec2<f32>(gid.yx) + 17.0 + fi * 3.141);

    let slices = f32(params.slice_count);
    let steps = f32(params.steps_per_side);
    var visibility = 0.0;
    for (var s = 0.0; s < slices; s = s + 1.0) {
        let phi = (s + noise_slice) * PI / slices;
        let omega = vec2<f32>(cos(phi), -sin(phi)); // screen direction (y down)
        let dir = vec3<f32>(cos(phi), sin(phi), 0.0); // the same direction in view space (y up)
        let ortho = dir - dot(dir, v) * v;
        let axis = normalize(cross(dir, v));
        let proj_n = n - axis * dot(n, axis);
        let proj_len = length(proj_n);
        let sign_n = sign(dot(ortho, proj_n));
        let cos_n = clamp(dot(proj_n / max(proj_len, 1e-6), v), -1.0, 1.0);
        let n_ang = sign_n * acos(cos_n);

        var hcos_pos = cos(n_ang + HALF_PI); // lowest possible horizons (tangent plane)
        var hcos_neg = cos(n_ang - HALF_PI);
        let low_pos = hcos_pos;
        let low_neg = hcos_neg;
        for (var j = 0.0; j < steps; j = j + 1.0) {
            var t = (j + noise_step) / steps;
            t = t * t; // distribution power 2 (XeGTAO uses about 2): denser near the pixel
            let off = max(t * screen_radius, 1.0 + j);
            let o = vec2<i32>(round(omega * off));
            let s_pos = view_pos(pix + o) - p;
            let s_neg = view_pos(pix - o) - p;
            let d_pos = length(s_pos);
            let d_neg = length(s_neg);
            let w_pos = clamp(d_pos * falloff_mul + falloff_add, 0.0, 1.0);
            let w_neg = clamp(d_neg * falloff_mul + falloff_add, 0.0, 1.0);
            let c_pos = mix(low_pos, dot(s_pos / max(d_pos, 1e-6), v), w_pos);
            let c_neg = mix(low_neg, dot(s_neg / max(d_neg, 1e-6), v), w_neg);
            hcos_pos = max(hcos_pos, c_pos);
            hcos_neg = max(hcos_neg, c_neg);
        }
        // Horizon angles, clamped to the hemisphere around the projected normal.
        var h_neg = -acos(clamp(hcos_neg, -1.0, 1.0));
        var h_pos = acos(clamp(hcos_pos, -1.0, 1.0));
        h_neg = n_ang + max(h_neg - n_ang, -HALF_PI);
        h_pos = n_ang + min(h_pos - n_ang, HALF_PI);
        // Cosine-weighted visible arc (paper eq. 10), integrated analytically.
        let sin_n = sin(n_ang);
        let arc_neg = (cos_n + 2.0 * h_neg * sin_n - cos(2.0 * h_neg - n_ang)) * 0.25;
        let arc_pos = (cos_n + 2.0 * h_pos * sin_n - cos(2.0 * h_pos - n_ang)) * 0.25;
        visibility = visibility + proj_len * (arc_neg + arc_pos);
    }
    visibility = clamp(visibility / slices, 0.0, 1.0);
    visibility = pow(visibility, params.final_power);
    textureStore(ao_raw, vec2<i32>(gid.xy), vec4<f32>(visibility, 0.0, 0.0, 1.0));
}

// 3x3 depth-aware blur at AO resolution.
@compute @workgroup_size(8, 8, 1)
fn gtao_denoise(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(ao_out);
    if (gid.x >= size.x || gid.y >= size.y) {
        return;
    }
    let scale = select(1, 2, params.half_res == 1u);
    let c = vec2<i32>(gid.xy);
    let zc = view_pos(c * scale).z;
    var sum = 0.0;
    var wsum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let q = clamp(c + vec2<i32>(x, y), vec2<i32>(0), vec2<i32>(size) - 1);
            let z = view_pos(q * scale).z;
            let w = exp(-abs(z - zc) / (abs(zc) * 0.03 + 1e-3)) * select(1.0, 2.0, x == 0 && y == 0);
            sum = sum + textureLoad(ao_in, q, 0).r * w;
            wsum = wsum + w;
        }
    }
    textureStore(ao_out, c, vec4<f32>(sum / max(wsum, 1e-6), 0.0, 0.0, 1.0));
}

// ---- helpers for the lighting shader ------------------------------------------------------------

// Multi-bounce approximation (Jimenez 2016): bright albedo gets back part of the occluded light.
fn gtao_multibounce(visibility: f32, albedo: vec3<f32>) -> vec3<f32> {
    let a = 2.0404 * albedo - 0.3324;
    let b = -4.7951 * albedo + 0.6417;
    let c = 2.7552 * albedo + 0.6903;
    let x = vec3<f32>(visibility);
    return max(x, ((x * a + b) * x + c) * x);
}

// Specular occlusion from AO (Lagarde, as in Filament): apply to sky/probe reflections only.
fn specular_occlusion(n_dot_v: f32, ao: f32, roughness: f32) -> f32 {
    return clamp(pow(n_dot_v + ao, exp2(-16.0 * roughness - 1.0)) - 1.0 + ao, 0.0, 1.0);
}
