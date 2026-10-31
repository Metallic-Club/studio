// Lighting terms shared by the scene and sky shaders (after common.wgsl): shadow filtering,
// ambient light, height fog and the celestial bodies. Inputs are the `light_*`, `fog_*` and
// celestial fields of `Globals`, filled by `src/lighting.rs`.

// Interleaved gradient noise: stable per pixel, used to rotate filter kernels.
fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

// Percentage-closer filter around `uv` with a radius of `g.light_bounce.w` texels. Small radii
// use a 3x3 grid; larger ones a rotated 16-tap spiral so wide penumbras do not ghost.
fn shadow_filter(uv: vec2<f32>, layer: i32, ref_depth: f32, frag: vec2<f32>) -> f32 {
    let ts = 1.0 / g.params.w;
    let radius = g.light_bounce.w;
    var sum = 0.0;
    if (radius <= 1.5) {
        for (var dy = -1; dy <= 1; dy = dy + 1) {
            for (var dx = -1; dx <= 1; dx = dx + 1) {
                let o = vec2<f32>(f32(dx), f32(dy)) * ts * radius;
                sum = sum + textureSampleCompareLevel(shadow_tex, shadow_samp, uv + o, layer, ref_depth);
            }
        }
        return sum / 9.0;
    }
    let spin = ign(frag) * 6.2831853;
    for (var i = 0; i < 16; i = i + 1) {
        let fi = f32(i) + 0.5;
        let r = sqrt(fi / 16.0) * radius;
        let a = fi * 2.3999632 + spin;
        let o = vec2<f32>(cos(a), sin(a)) * r * ts;
        sum = sum + textureSampleCompareLevel(shadow_tex, shadow_samp, uv + o, layer, ref_depth);
    }
    return sum / 16.0;
}

// Diffuse cosine term; `LightingStyle.Soft` wraps it around the terminator.
fn diffuse_ndl(ndl_raw: f32) -> f32 {
    let w = g.light_flags.y;
    return max((ndl_raw + w) / ((1.0 + w) * (1.0 + w)), 0.0);
}

// Hemisphere sky light for a normal, before the environment scale.
fn sky_hemisphere(n: vec3<f32>) -> vec3<f32> {
    let ground = g.sky_ground.rgb * g.light_bounce.rgb;
    return mix(ground, mix(g.sky_horizon.rgb, g.sky_zenith.rgb, 0.45), n.y * 0.5 + 0.5);
}

// Indirect light: sky hemisphere (EnvironmentDiffuseScale), Ambient, OutdoorAmbient (more on
// open, up-facing, lit surfaces) and sky reflections (EnvironmentSpecularScale).
fn ambient_light(diffuse_col: vec3<f32>, f0: vec3<f32>, n: vec3<f32>, v: vec3<f32>, rough: f32, ndv: f32, sh: f32) -> vec3<f32> {
    let hemi = sky_hemisphere(n);
    let open = (0.5 + 0.5 * n.y) * mix(0.55, 1.0, sh);
    let flat_light = g.light_ambient.rgb + g.light_outdoor.rgb * open;
    let r = reflect(-v, n);
    let env = mix(sky_gradient(r), hemi, rough) * g.light_outdoor.w;
    let f_env = f0 + (max(vec3<f32>(1.0 - rough), f0) - f0) * pow(1.0 - ndv, 5.0);
    return diffuse_col * (hemi * g.light_ambient.w + flat_light) + env * f_env * mix(0.35, 1.0, sh);
}

// Optical depth of the height fog between two points (exponential density in height).
fn fog_depth(p0: vec3<f32>, p1: vec3<f32>) -> f32 {
    let k = g.fog_params.y;
    let d = length(p1 - p0);
    let y0 = clamp(p0.y - g.fog_params.z, -100.0, 1e5);
    let y1 = clamp(p1.y - g.fog_params.z, -100.0, 1e5);
    let dy = y1 - y0;
    let e0 = exp(min(-k * y0, 5.0));
    if (abs(k * dy) < 1e-3) {
        return g.fog_params.x * d * e0;
    }
    let e1 = exp(min(-k * y1, 5.0));
    return g.fog_params.x * d * (e0 - e1) / (k * dy);
}

// Fog colour seen along `dir` (from the eye): tinted toward/away from the sun, plus the sun's
// forward-scattered glare.
fn fog_colour(dir: vec3<f32>) -> vec3<f32> {
    let c = dot(dir, g.sun_disc.xyz);
    let toward = pow(clamp(c * 0.5 + 0.5, 0.0, 1.0), 2.0);
    var col = mix(g.fog_away.rgb, g.fog_toward.rgb, toward);
    let gg = g.fog_toward.w;
    let hg = (1.0 - gg * gg) / pow(max(1.0 + gg * gg - 2.0 * gg * c, 1e-4), 1.5);
    let hg_peak = (1.0 + gg) / pow(1.0 - gg, 2.0);
    let above = smoothstep(-0.05, 0.05, g.sun_disc.y);
    col = col + g.sun_color.rgb * g.fog_away.w * (hg / hg_peak) * 4.0 * above;
    return col;
}

// Applies the height fog and the legacy distance fog to `col` seen at `p` from the camera.
fn apply_fog(col_in: vec3<f32>, p: vec3<f32>) -> vec3<f32> {
    var col = col_in;
    let eye = g.camera_pos.xyz;
    let to_p = p - eye;
    let dist = length(to_p);
    let dir = to_p / max(dist, 1e-4);
    if (g.fog_extinction.w > 0.5) {
        let tau = fog_depth(eye, p) * g.fog_extinction.rgb;
        let opacity = min(vec3<f32>(1.0) - exp(-tau), vec3<f32>(g.fog_params.w));
        col = mix(col, fog_colour(dir), opacity);
    }
    if (g.legacy_fog.w > 0.5) {
        let f = clamp((dist - g.legacy_range.x) / (g.legacy_range.y - g.legacy_range.x), 0.0, 1.0);
        col = mix(col, g.legacy_fog.rgb, f);
    }
    return col;
}

// Distance the sky is treated as for fog.
const SKY_FOG_DISTANCE: f32 = 500.0;

fn star_field(dir: vec3<f32>) -> vec3<f32> {
    if (dir.y <= 0.0 || g.light_flags.w <= 0.0) {
        return vec3<f32>(0.0);
    }
    // Cells on an octahedral-ish projection of the upper hemisphere; one candidate star each.
    let cells = 220.0;
    let q = dir.xz / (1.0 + dir.y) * cells;
    let cell = floor(q);
    let h = hash12(cell);
    let density = 0.08 * g.light_flags.z;
    if (h > density) {
        return vec3<f32>(0.0);
    }
    let centre = cell + vec2<f32>(hash12(cell + 17.1), hash12(cell + 41.7));
    let d = length(q - centre);
    let size = 0.12 + 0.18 * hash12(cell + 7.3);
    let core = smoothstep(size, 0.0, d);
    let bright = pow(hash12(cell + 3.9), 3.0) * 1.5 + 0.15;
    let warm = hash12(cell + 9.2);
    let tint = mix(vec3<f32>(0.75, 0.82, 1.0), vec3<f32>(1.0, 0.9, 0.75), warm);
    // Stars sink into the horizon haze.
    let height = smoothstep(0.0, 0.25, dir.y);
    return tint * core * bright * g.light_flags.w * height;
}

// The sky seen along `dir`: gradient, stars, moon and sun discs, then fog.
fn sky_colour(dir: vec3<f32>) -> vec3<f32> {
    var col = sky_gradient(dir);
    if (g.celestial.z > 0.5) {
        col = col + star_field(dir);
        let mc = dot(dir, g.moon_disc.xyz);
        let moon_edge = 1.0 - g.moon_disc.w;
        let moon = smoothstep(g.moon_disc.w - moon_edge * 0.15, g.moon_disc.w + moon_edge * 0.05, mc);
        if (g.moon_disc.y > -0.05) {
            // Faint maria so the disc reads as the moon, not a lamp.
            let m = 0.82 + 0.18 * hash12(floor(dir.xz * 900.0));
            col = mix(col, vec3<f32>(0.86, 0.9, 1.0) * g.celestial.y * 40.0 * m, moon);
        }
        let sc = dot(dir, g.sun_disc.xyz);
        let sun_edge = 1.0 - g.sun_disc.w;
        let disc = smoothstep(g.sun_disc.w - sun_edge * 0.08, g.sun_disc.w + sun_edge * 0.02, sc);
        let sun_up = smoothstep(-0.03, 0.01, g.sun_disc.y);
        col = col + g.sun_color.rgb * g.celestial.x * disc * sun_up;
    }
    if (g.fog_extinction.w > 0.5) {
        let eye = g.camera_pos.xyz;
        let p = eye + dir * SKY_FOG_DISTANCE;
        let tau = fog_depth(eye, p) * g.fog_extinction.rgb;
        let opacity = min(vec3<f32>(1.0) - exp(-tau), vec3<f32>(g.fog_params.w));
        col = mix(col, fog_colour(dir), opacity);
    }
    if (g.legacy_fog.w > 0.5) {
        let f = clamp((SKY_FOG_DISTANCE - g.legacy_range.x) / (g.legacy_range.y - g.legacy_range.x), 0.0, 1.0);
        col = mix(col, g.legacy_fog.rgb, f);
    }
    return col;
}
