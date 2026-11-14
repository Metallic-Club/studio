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
    // x: 1 when the target is not an sRGB format and the shader must encode. yzw: the level-of-detail debug view
    // (`lod_tint` in parts.wgsl), where levels 1, 2 and 3 start in the mesh pool; 0 when it is off.
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
    // The sun's shadow cascades (shadow/mod.rs): camera-relative positions to each cascade's map (x, y in −1..1,
    // depth 0 at the light's end).
    shadow_matrices: array<mat4x4<f32>, 4>,
    // Per cascade: the view depth it ends at, the normal offset in metres (scaled by 1 − N·L), the depth bias in
    // depth units.
    shadow_splits: vec4<f32>,
    shadow_offsets: vec4<f32>,
    shadow_biases: vec4<f32>,
    // x: cascades in use (0: no shadows), y: the filter's reach in texels, z: one texel in map coordinates,
    // w: the view depth the shadows start fading out at.
    shadow_params: vec4<f32>,
    // Local lights (lights/mod.rs, lights.wgsl). x, y: cluster columns and rows per pixel; z, w: the depth slices'
    // scale and bias on log2(depth).
    lights_clusters: vec4<f32>,
    // x: lights drawn (0: none), y: the depth the first slice ends at, z: the shadows' normal offset (scaled by
    // 1 − N·L) and w: their depth bias, in texels.
    lights_local: vec4<f32>,
    // x: one shadow-atlas texel in atlas coordinates, y: the filter's reach in texels, z: 1 when any drawn light has
    // a shadow.
    lights_atlas: vec4<f32>,
    // Per cascade: 1 when its see-through maps hold see-through casters (shadow/see_through.rs, #491).
    shadow_see_through: vec4<f32>,
    // x: the shading view (parts/shading.rs, #478): 0 shaded, 1 unlit, 2 lighting only, 3 light density.
    view: vec4<f32>,
    // rgb: light on surfaces facing away from the sun or moon (Lighting.ColorShift_Bottom, #600).
    color_shift_bottom: vec4<f32>,
    // rgb: the fog's colour; w: 1 when there is fog (#600).
    fog_color: vec4<f32>,
    // x, y: where the fog starts and is whole, in studs; z: how far the sky is for it.
    fog_range: vec4<f32>,
    // x, y: the environment's diffuse and specular scales; z: the shadow filter's spread; w: the exposure.
    environment: vec4<f32>,
    // rgb: Lighting.Ambient, light added evenly everywhere, indoors too (#615).
    ambient: vec4<f32>,
    // The sky map (#615): camera-relative positions to its clip space; x: 1 when there is one, y: one texel in map
    // coordinates, z: the lookup's normal offset in metres, w: its depth bias; and the view depth it ends at (x),
    // starts fading out at (y), how far its filter reaches from a point in metres (z, #625), and a texel in metres (w,
    // #648).
    sky_matrix: mat4x4<f32>,
    sky_params: vec4<f32>,
    sky_range: vec4<f32>,
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

// How much of the fog's colour a point `distance` away takes (Lighting.FogStart and FogEnd, #600): none before the
// start, all of it from the end on, linearly between.
fn fog_amount(distance: f32) -> f32 {
    if frame.fog_color.w < 0.5 {
        return 0.0;
    }
    let range = max(frame.fog_range.y - frame.fog_range.x, 1e-3);
    return clamp((distance - frame.fog_range.x) / range, 0.0, 1.0);
}

// `color` seen through the fog from `distance` away, at the frame's exposure (Lighting.ExposureCompensation, #600).
fn finish(color: vec3<f32>, distance: f32) -> vec3<f32> {
    return mix(color, frame.fog_color.rgb, fog_amount(distance)) * frame.environment.w;
}
