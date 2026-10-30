// Lighting functions for the stylised Looks (research prototype, issue #37). The render-styles research
// owns the materials and outlines (docs/research/style-shaders/); this file covers how sun, sky ambient
// and shadows feed those looks so one Lighting setup works across styles.
//
//   Realistic: GGX + Lambert (scene.wgsl today), AgX "Default" curve.
//   Soft (LightingStyle.Soft): wrapped diffuse, softer shadows, lifted ambient, PBR Neutral.
//   Toon: shadow map and N.L are combined first, then banded, so cast shadows and the terminator use
//         the same hard edge; ambient flattened toward the hemisphere average; no AO darkening on lit side.
//   Pixel 3D: Toon bands, then light (not albedo) quantised in exposure steps so the palette survives.
//   Flat / low-poly: faceted normal from screen derivatives (computed before any branch).

const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// Soft: wrap lighting keeps the terminator gentle (w = 0.3); normalised so a fully lit face matches Lambert.
fn soft_diffuse(n_dot_l_raw: f32, wrap: f32) -> f32 {
    return clamp((n_dot_l_raw + wrap) / (1.0 + wrap), 0.0, 1.0);
}

// Toon: bands >= 1 lit steps; softness is the edge width in N.L units (0.02-0.05).
// shadow: 0..1 sun visibility from shadows_csm (use hard or tent mode for toon).
fn toon_sun(n_dot_l_raw: f32, shadow: f32, bands: f32, softness: f32) -> f32 {
    let lit = clamp(n_dot_l_raw, 0.0, 1.0) * shadow;
    if (bands <= 1.0) {
        return smoothstep(0.02 - softness, 0.02 + softness, lit);
    }
    let x = lit * bands;
    let step_i = floor(x);
    let f = x - step_i;
    let edge = smoothstep(0.5 - softness * bands, 0.5 + softness * bands, f);
    return clamp((step_i + edge) / bands, 0.0, 1.0);
}

// Toon ambient: mostly the average of sky and ground irradiance, with a little directionality left.
fn toon_ambient(irradiance_n: vec3<f32>, irradiance_up: vec3<f32>, irradiance_down: vec3<f32>) -> vec3<f32> {
    let flat_avg = 0.5 * (irradiance_up + irradiance_down);
    return mix(flat_avg, irradiance_n, 0.15);
}

// Tinted shadow side for toon/anime: instead of only darker, shift toward a creator colour.
fn toon_shade(albedo: vec3<f32>, sun_term: f32, sun_light: vec3<f32>, ambient: vec3<f32>, shadow_tint: vec3<f32>) -> vec3<f32> {
    let lit = albedo * sun_light * sun_term;
    let shade = albedo * ambient * mix(shadow_tint, vec3<f32>(1.0), sun_term);
    return lit + shade;
}

// Pixel look: quantise the light arriving at a surface (luminance) to steps of `ev_step` EV around
// mid-grey, keeping hue; albedo then maps to the palette in the post pass (pixel_art.wgsl).
fn quantise_light(light: vec3<f32>, ev_step: f32) -> vec3<f32> {
    let l = max(dot(light, LUMA), 1e-6);
    let ev = log2(l / 0.18);
    let q = floor(ev / ev_step + 0.5) * ev_step;
    return light * (0.18 * exp2(q) / l);
}

// Flat shading: faceted normal from screen-space derivatives of world position. Call at the top of the
// fragment shader, before any non-uniform branch (naga rejects derivatives in non-uniform control flow).
fn faceted_normal(world_pos: vec3<f32>) -> vec3<f32> {
    return normalize(cross(dpdx(world_pos), dpdy(world_pos)));
}

// Validation entry point that exercises every helper.
struct TestIn {
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) albedo: vec3<f32>,
    @location(3) @interpolate(flat) model: u32,
}

@fragment
fn fs_style_test(in: TestIn) -> @location(0) vec4<f32> {
    let flat_n = faceted_normal(in.world_pos);
    let sun_dir = normalize(vec3<f32>(0.4, 0.8, 0.3));
    let sun = vec3<f32>(3.0, 2.9, 2.7);
    let sky_up = vec3<f32>(0.35, 0.45, 0.7);
    let sky_down = vec3<f32>(0.12, 0.11, 0.1);
    var n = normalize(in.normal);
    if (in.model == 4u) {
        n = flat_n;
    }
    let ndl = dot(n, sun_dir);
    let amb_n = mix(sky_down, sky_up, n.y * 0.5 + 0.5);
    var c: vec3<f32>;
    switch in.model {
        case 1u: { c = in.albedo / 3.14159265 * (sun * soft_diffuse(ndl, 0.3) + amb_n * 1.2); }
        case 2u: {
            let t = toon_sun(ndl, 1.0, 2.0, 0.02);
            c = toon_shade(in.albedo / 3.14159265, t, sun, toon_ambient(amb_n, sky_up, sky_down),
                           vec3<f32>(0.85, 0.8, 1.0));
        }
        case 3u: {
            let t = toon_sun(ndl, 1.0, 3.0, 0.01);
            c = in.albedo / 3.14159265 * quantise_light(sun * t + amb_n, 1.0);
        }
        default: { c = in.albedo / 3.14159265 * (sun * max(ndl, 0.0) + amb_n); }
    }
    return vec4<f32>(c, 1.0);
}
