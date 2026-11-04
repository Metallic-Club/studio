// The sky (appended to frame.wgsl): one full-screen triangle at the far plane, drawn after the opaque scene with a
// depth test, so only pixels nothing covered are shaded (ADR 0008 §7). In reverse-Z the far plane is depth 0, which
// is also what depth is cleared to, so `GreaterEqual` passes exactly where nothing was drawn.
//
// The colour is a function of the view direction alone: an analytic gradient from the zenith to the horizon, a
// darker band below it, a glow around the sun at sunrise and sunset, and the sun and moon discs. A textured sky
// (a cubemap looked up with the same direction) can replace `sky_color` later without changing the pass.

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    // Normalised device coordinates, interpolated across the screen.
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_sky(@builtin(vertex_index) index: u32) -> SkyOut {
    // (-1, -1), (3, -1), (-1, 3): one triangle covering the screen.
    let ndc = vec2<f32>(f32((index << 1u) & 2u) * 2.0 - 1.0, f32(index & 2u) * 2.0 - 1.0);
    var out: SkyOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.ndc = ndc;
    return out;
}

// A soft-edged disc of cosine radius `cos_radius` around `centre`, 1 inside.
fn disc(view: vec3<f32>, centre: vec3<f32>, cos_radius: f32) -> f32 {
    let c = dot(view, centre);
    let edge = (1.0 - cos_radius) * 0.25;
    return smoothstep(cos_radius - edge, cos_radius + edge, c);
}

fn sky_color(view: vec3<f32>) -> vec3<f32> {
    let sun = frame.sun.xyz;
    // Clamped: rounding takes a unit vector's y a little past ±1 straight up or down, and pow of the negative
    // 1 − up below is not a number on drivers that take it as exp2(y × log2(x)).
    let up = clamp(view.y, -1.0, 1.0);

    // Overhead to horizon: the horizon colour holds near the horizon and gives way to the zenith higher up.
    let above = mix(frame.sky_zenith.rgb, frame.sky_horizon.rgb, pow(1.0 - max(up, 0.0), 3.0));
    // The twilight glow, strongest at the horizon on the sun's side.
    let flat_view = normalize(vec3<f32>(view.x, 0.0, view.z) + vec3<f32>(0.0, 1e-4, 0.0));
    let flat_sun = normalize(vec3<f32>(sun.x, 0.0, sun.z) + vec3<f32>(0.0, 1e-4, 0.0));
    let towards_sun = max(dot(flat_view, flat_sun), 0.0);
    let glow = frame.sky_glow.rgb * pow(1.0 - abs(up), 6.0) * (0.25 + 0.75 * towards_sun * towards_sun);
    // Below the horizon the sky fades quickly to the ground colour.
    var color = mix(above + glow, frame.sky_ground.rgb, smoothstep(0.0, 0.12, -up));

    // The discs are hidden by the horizon.
    let over_horizon = smoothstep(-0.01, 0.01, up);
    // The halo: a wide faint part (c⁸, about 20° across) and a tight bright one (c²⁵⁶, about 4°) around the disc.
    let c = max(dot(view, sun), 0.0);
    let c2 = c * c;
    let c8 = c2 * c2 * c2 * c2;
    let c256 = pow(c8, 32.0);
    color += frame.sun_halo.rgb * (0.12 * c8 + 0.5 * c256) * over_horizon;
    color += frame.sun_disc.rgb * disc(view, sun, frame.sun.w) * over_horizon;
    color += frame.moon_disc.rgb * disc(view, -sun, frame.moon_disc.w) * over_horizon;
    return color;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let view = normalize(frame.view_forward.xyz + in.ndc.x * frame.view_right.xyz + in.ndc.y * frame.view_up.xyz);
    return vec4<f32>(encode(sky_color(view)), 1.0);
}
