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
