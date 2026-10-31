// Shared by the sky and scene shaders: frame uniforms and the sky gradient. Both write linear
// HDR light; `finish.wgsl` tone maps and encodes it.
struct Globals {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    cascade_vp: array<mat4x4<f32>, 3>,
    cascade_splits: vec4<f32>,
    cascade_texel: vec4<f32>,
    camera_pos: vec4<f32>,   // w: fog density
    sun_dir: vec4<f32>,      // toward the sun
    sun_color: vec4<f32>,
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    sky_ground: vec4<f32>,
    params: vec4<f32>,       // x: cascades, y: tone curve index, z: exposure incl. curve pre-scale, w: shadow map size
    viewport: vec4<f32>,     // x, y: target size in pixels; z, w: their reciprocals
    // Lighting block (`LightingUniform` in src/lighting.rs; used by lighting.wgsl).
    light_ambient: vec4<f32>,  // rgb: Ambient, w: EnvironmentDiffuseScale
    light_outdoor: vec4<f32>,  // rgb: OutdoorAmbient, w: EnvironmentSpecularScale
    light_bounce: vec4<f32>,   // rgb: ground bounce tint, w: shadow filter radius (texels)
    light_flags: vec4<f32>,    // x: shadows on, y: diffuse wrap, z: star density, w: star brightness
    fog_params: vec4<f32>,     // x: density, y: height falloff, z: fog height, w: max opacity
    fog_toward: vec4<f32>,     // rgb: fog toward the sun, w: anisotropy g
    fog_away: vec4<f32>,       // rgb: fog away from the sun, w: glare strength
    fog_extinction: vec4<f32>, // rgb: extinction per channel, w: height fog on
    legacy_fog: vec4<f32>,     // rgb: FogColor, w: distance fog on
    legacy_range: vec4<f32>,   // x: FogStart, y: FogEnd
    sun_disc: vec4<f32>,       // xyz: toward the sun, w: cos(disc radius)
    moon_disc: vec4<f32>,      // xyz: toward the moon, w: cos(disc radius)
    celestial: vec4<f32>,      // x: sun disc scale, y: moon radiance, z: bodies shown, w: night
    light_toon: vec4<f32>,     // x: toon light of the default lighting (reference)
}

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var shadow_tex: texture_depth_2d_array;
@group(0) @binding(2) var shadow_samp: sampler_comparison;

const PI: f32 = 3.14159265;

fn sky_gradient(d: vec3<f32>) -> vec3<f32> {
    let up = clamp(d.y, -1.0, 1.0);
    let t = smoothstep(0.0, 0.6, clamp(up, 0.0, 1.0));
    let above = mix(g.sky_horizon.rgb, g.sky_zenith.rgb, t);
    let below = mix(g.sky_horizon.rgb, g.sky_ground.rgb, smoothstep(0.0, 0.35, -up));
    return select(above, below, up < 0.0);
}

// The scene colour that the finish pass maps to the display-linear colour `y` (0..1), before
// grading: the inverse of the tone curve and exposure. Toon shading picks display colours
// directly and goes through this, so it lands in the HDR target like any other light.
fn unfinish(y: vec3<f32>) -> vec3<f32> {
    return invert_curve(y, u32(g.params.y + 0.5)) / max(g.params.z, 1e-6);
}
