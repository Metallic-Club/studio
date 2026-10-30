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
    params: vec4<f32>,       // x: cascades, y, z: unused (exposure moved to the finish pass), w: shadow map size
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
