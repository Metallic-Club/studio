// Sun shadows from cascaded shadow maps: cascade pick with a dithered blend band, normal-offset bias,
// tent PCF from 9 hardware-filtered taps, and PCSS (Fernando 2005) for the High and Ultra tiers.
// Research prototype for issue #37. Matches Task 3's layout: Depth32Float 2D array, one layer per cascade,
// orthographic light matrices, depth 0 (near the sun) .. 1.
//
// Creator knob: Lighting.ShadowSoftness 0..1 -> sun angular radius used for the penumbra:
//   light_radius = max(0.27 deg, 3 deg * ShadowSoftness)   (0.2 default -> 0.6 deg)
//   LightingStyle Soft raises the floor to 1.5 deg; Clouds.Cover widens it toward 20 deg.
// Low/Medium tiers turn that radius into a fixed PCF radius per cascade (no blocker search).

struct ShadowParams {
    cascade_vp: array<mat4x4<f32>, 4>,
    cascade_splits: vec4<f32>,      // far view distance of each cascade (studs)
    cascade_texel: vec4<f32>,       // world size of one texel per cascade (studs)
    cascade_depth_range: vec4<f32>, // world length of each cascade's depth 0..1 (studs)
    sun_dir: vec3<f32>,             // toward the sun
    cascade_count: u32,
    light_tan_radius: f32,          // tan(light angular radius) from ShadowSoftness
    blend_band: f32,                // fraction of each cascade used to fade into the next (0.1)
    map_size: f32,                  // texels per side
    mode: u32,                      // 0 hard (1 tap), 1 tent PCF, 2 PCSS
    max_filter_texels: f32,         // clamp for PCSS kernel radius (12)
    normal_bias_texels: f32,        // 1.5
    depth_bias: f32,                // constant bias in depth units (0.0004 today)
    _pad: f32,
}

@group(0) @binding(0) var shadow_map: texture_depth_2d_array;
@group(0) @binding(1) var shadow_cmp: sampler_comparison; // linear filtering, compare less-equal
@group(0) @binding(2) var shadow_point: sampler;          // nearest/linear, no compare (blocker search)
@group(0) @binding(3) var<uniform> sp: ShadowParams;

// 16-point Vogel disk (golden angle), radius 1.
fn vogel(i: u32, n: u32, rot: f32) -> vec2<f32> {
    let r = sqrt((f32(i) + 0.5) / f32(n));
    let th = f32(i) * 2.39996323 + rot;
    return vec2<f32>(cos(th), sin(th)) * r;
}

fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

// Tent-weighted 3x3 grid of bilinear compare taps (weights 1-2-1 x 1-2-1): with radius_scale 1 this
// covers a 4x4 texel footprint smoothly; radius_scale widens it for softer shadows.
fn pcf_tent(uv: vec2<f32>, layer: i32, ref_depth: f32, texel: f32, radius_scale: f32) -> f32 {
    var sum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let w = f32(2 - abs(x)) * f32(2 - abs(y));
            let o = vec2<f32>(f32(x), f32(y)) * texel * radius_scale;
            sum = sum + w * textureSampleCompareLevel(shadow_map, shadow_cmp, uv + o, layer, ref_depth);
        }
    }
    return sum / 16.0;
}

// PCSS for an orthographic (directional) light: blocker search, penumbra from the depth gap, PCF.
fn pcss(uv: vec2<f32>, layer: i32, ref_depth: f32, ci: u32, frag: vec2<f32>) -> f32 {
    let texel = 1.0 / sp.map_size;
    let texel_world = sp.cascade_texel[ci];
    let depth_world = sp.cascade_depth_range[ci];
    let rot = ign(frag) * 6.2831853;
    // Search region: penumbra of a blocker 40 studs above the receiver, in texels, clamped.
    let search_texels = clamp(40.0 * sp.light_tan_radius / texel_world, 2.0, sp.max_filter_texels);
    var blocker_sum = 0.0;
    var blockers = 0.0;
    for (var i = 0u; i < 16u; i = i + 1u) {
        let o = vogel(i, 16u, rot) * search_texels * texel;
        let g = textureGather(shadow_map, shadow_point, uv + o, layer);
        for (var k = 0; k < 4; k = k + 1) {
            let d = g[k];
            if (d < ref_depth) {
                blocker_sum = blocker_sum + d;
                blockers = blockers + 1.0;
            }
        }
    }
    if (blockers < 0.5) {
        return 1.0;
    }
    let avg_blocker = blocker_sum / blockers;
    // Parallel-planes penumbra (Fernando 2005); for a sun the light-to-blocker distance is infinite,
    // so width = gap * 2 * tan(angular radius).
    let gap_world = (ref_depth - avg_blocker) * depth_world;
    let penumbra_texels = clamp(gap_world * 2.0 * sp.light_tan_radius / texel_world, 1.0, sp.max_filter_texels);
    var lit = 0.0;
    for (var i = 0u; i < 16u; i = i + 1u) {
        let o = vogel(i, 16u, rot + 1.7) * penumbra_texels * texel;
        lit = lit + textureSampleCompareLevel(shadow_map, shadow_cmp, uv + o, layer, ref_depth);
    }
    return lit / 16.0;
}

fn sample_cascade(ci: u32, world_pos: vec3<f32>, n: vec3<f32>, frag: vec2<f32>) -> f32 {
    // Normal offset scaled by the cascade's texel size and by how grazing the sun is.
    let n_dot_l = clamp(dot(n, sp.sun_dir), 0.0, 1.0);
    let slope = sqrt(max(1.0 - n_dot_l * n_dot_l, 0.0));
    let p = world_pos + n * sp.cascade_texel[ci] * sp.normal_bias_texels * (0.5 + slope);
    let clip = sp.cascade_vp[ci] * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
    let ref_depth = clip.z - sp.depth_bias;
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || ref_depth > 1.0) {
        return 1.0;
    }
    let texel = 1.0 / sp.map_size;
    let layer = i32(ci);
    switch sp.mode {
        case 0u: { return textureSampleCompareLevel(shadow_map, shadow_cmp, uv, layer, ref_depth); }
        case 1u: {
            // Fixed softness: tent radius grows with ShadowSoftness, measured at a 10-stud gap.
            let r = clamp(10.0 * sp.light_tan_radius / sp.cascade_texel[ci], 1.0, 4.0);
            return pcf_tent(uv, layer, ref_depth, texel, r);
        }
        default: { return pcss(uv, layer, ref_depth, ci, frag); }
    }
}

// Returns sun visibility 0..1. view_depth: distance along the view axis (studs). frag: pixel coords.
fn sun_shadow(world_pos: vec3<f32>, n: vec3<f32>, view_depth: f32, frag: vec2<f32>) -> f32 {
    let count = sp.cascade_count;
    var ci = count;
    for (var i = 0u; i < count; i = i + 1u) {
        if (view_depth < sp.cascade_splits[i]) {
            ci = i;
            break;
        }
    }
    if (ci >= count) {
        return 1.0;
    }
    var lit = sample_cascade(ci, world_pos, n, frag);
    // Blend band at the far end of the cascade: dithered pick between this and the next cascade
    // (one lookup per pixel; MSAA/TAA hides the dither).
    let near = select(0.0, sp.cascade_splits[max(ci, 1u) - 1u], ci > 0u);
    let far = sp.cascade_splits[ci];
    let t = (view_depth - near) / max(far - near, 1e-3);
    let band_start = 1.0 - sp.blend_band;
    if (t > band_start && ci + 1u < count) {
        let w = (t - band_start) / sp.blend_band;
        if (ign(frag + 0.5) < w) {
            lit = sample_cascade(ci + 1u, world_pos, n, frag);
        }
    }
    // Fade out at the shadow distance instead of a hard edge.
    let fade = smoothstep(0.9, 1.0, view_depth / sp.cascade_splits[count - 1u]);
    return mix(lit, 1.0, fade);
}

// Validation entry point: a fragment that shades world positions with sun visibility.
struct TestIn {
    @builtin(position) pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) view_depth: f32,
}

@fragment
fn fs_shadow_test(in: TestIn) -> @location(0) vec4<f32> {
    let v = sun_shadow(in.world_pos, normalize(in.normal), in.view_depth, in.pos.xy);
    return vec4<f32>(vec3<f32>(v), 1.0);
}
