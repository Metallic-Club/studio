// Shared by the sky and scene shaders: frame uniforms, sky gradient and output encoding.
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
    params: vec4<f32>,       // x: cascades, y: 1 if the target encodes sRGB itself, z: exposure, w: shadow map size
}

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var shadow_tex: texture_depth_2d_array;
@group(0) @binding(2) var shadow_samp: sampler_comparison;

const PI: f32 = 3.14159265;

fn sky_gradient(d: vec3<f32>) -> vec3<f32> {
    let up = clamp(d.y, -1.0, 1.0);
    let t = pow(clamp(up, 0.0, 1.0), 0.55);
    let above = mix(g.sky_horizon.rgb, g.sky_zenith.rgb, t);
    let below = mix(g.sky_horizon.rgb, g.sky_ground.rgb, smoothstep(0.0, 0.35, -up));
    return select(above, below, up < 0.0);
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

fn hash12(p: vec2<f32>) -> f32 {
    var q = fract(vec3<f32>(p.xyx) * 0.1031);
    q = q + dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

// Tone maps linear light, encodes it for the target and adds a little dither against banding.
fn finish(c: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0)) * g.params.z;
    let mapped = clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
    let encoded = linear_to_srgb(mapped) + (hash12(frag) - 0.5) / 255.0;
    if (g.params.y > 0.5) {
        return srgb_to_linear(clamp(encoded, vec3<f32>(0.0), vec3<f32>(1.0)));
    }
    return encoded;
}
