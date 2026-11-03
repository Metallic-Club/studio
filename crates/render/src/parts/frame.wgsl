// The frame uniform, shared by the parts (parts.wgsl) and the sky (sky.wgsl): each shader is this file followed by
// its own. Every colour is linear light, in the same units for surfaces and sky (HDR-ready: nothing is tone mapped
// yet, and the output is clamped by the target).

// `FrameUniforms` in parts/mod.rs.
struct Frame {
    view_proj: mat4x4<f32>,
    // xyz: camera position − scene origin.
    camera_offset: vec4<f32>,
    // xyz: unit vector towards the light (the sun by day, the moon by night).
    light_direction: vec4<f32>,
    // rgb: the light's radiance.
    light_color: vec4<f32>,
    // rgb: ambient light from above.
    sky_ambient: vec4<f32>,
    // rgb: ambient light from below.
    ground_ambient: vec4<f32>,
    // x: 1 when the target is not an sRGB format and the shader must encode.
    output: vec4<f32>,
    // The camera's view rays: right and up scaled to the edges of the screen (NDC ±1), and forward.
    view_right: vec4<f32>,
    view_up: vec4<f32>,
    view_forward: vec4<f32>,
    // rgb: the sky overhead, at the horizon, below the horizon, and the twilight glow around the sun.
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    sky_ground: vec4<f32>,
    sky_glow: vec4<f32>,
    // xyz: unit vector towards the sun (below the horizon at night); w: cosine of the sun disc's radius.
    sun: vec4<f32>,
    // rgb: the sun disc's radiance, and its halo's.
    sun_disc: vec4<f32>,
    sun_halo: vec4<f32>,
    // rgb: the moon disc's radiance (opposite the sun); w: cosine of its radius.
    moon_disc: vec4<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308));
}

fn encode(color: vec3<f32>) -> vec3<f32> {
    if frame.output.x > 0.5 {
        return linear_to_srgb(color);
    }
    return color;
}
