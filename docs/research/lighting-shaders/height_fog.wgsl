// Analytic exponential height fog coloured by the sky (research prototype, issue #37).
// One function for the forward shader (fog per pixel, also for transparents) and a full-screen pass for
// tiers that apply fog after opaque shading from the depth buffer. Cost: a few ALU ops and one
// sky-view LUT tap per pixel. Volumetric (froxel) fog replaces the in-scatter term on High/Ultra.
//
// Density model: sigma(y) = sigma0 * exp(-falloff * (y - fog_height)), integrated in closed form
// along the view ray (the standard exponential-height-fog integral).
// Creator mapping (Atmosphere, Roblox names; Metallic extras marked +):
//   sigma0       = 0.012 * Density^2 per stud        (Density 0.3 -> 5% visibility at ~2800 studs)
//   falloff      = +HeightFalloff (1/studs, default 0.02 -> halves every 35 studs)
//   fog_height   = +FogHeight (studs)
//   max opacity  = 1 - 0.75 * Offset                (Offset > 0 keeps distant silhouettes)
//   tint toward the sun = Color / default Color; away = Decay / default Decay (defaults are neutral)
//   extinction per channel *= 1 + 3 * +Absorption   (water, smoke)
//   sun in-scatter: Henyey-Greenstein g = 0.55; Glare 0 dims it to 60%
//   Lighting.Ambient also lights the fog (exposure-relative), which gives city glow at night.

struct FogParams {
    camera_pos: vec3<f32>,     // studs
    sigma0: f32,               // per stud at fog_height
    sun_dir: vec3<f32>,
    falloff: f32,              // per stud; 0 = uniform fog
    sun_illuminance: vec3<f32>, // pre-exposed, after atmosphere transmittance (and cloud cover)
    fog_height: f32,
    moon_dir: vec3<f32>,
    max_opacity: f32,
    moon_illuminance: vec3<f32>, // pre-exposed
    sun_scatter: f32,          // 1.0, or 0.6 when Glare = 0
    tint_toward: vec3<f32>,    // linear Color / default Color
    camera_height_km: f32,     // for the sky-view LUT horizon lookup
    tint_away: vec3<f32>,      // linear Decay / default Decay
    planet_radius_km: f32,     // 6360
    absorption: vec3<f32>,     // 1 + 3 * Absorption (linear 0..1 per channel)
    sky_scale: f32,            // pre-exposed sun illuminance at the top of the atmosphere (sky-view LUT unit)
    ambient: vec3<f32>,        // Lighting.Ambient, linear * 0.5 (exposure-relative air glow)
    max_distance: f32,         // distance used for sky pixels (studs), e.g. 100000
    inv_proj_view: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> fog: FogParams;
@group(0) @binding(1) var skyview_lut: texture_2d<f32>;
@group(0) @binding(2) var lut_sampler: sampler;
@group(0) @binding(3) var depth_tex: texture_depth_2d;

const PI: f32 = 3.14159265358979;

struct FogResult {
    transmittance: vec3<f32>,
    inscatter: vec3<f32>,
}

// Optical depth from the camera along dir for dist studs.
fn fog_optical_depth(dir: vec3<f32>, dist: f32) -> f32 {
    let base = fog.sigma0 * exp(-fog.falloff * (fog.camera_pos.y - fog.fog_height));
    let k = fog.falloff * dir.y;
    if (abs(k * dist) < 1e-4) {
        return base * dist;
    }
    return base * (1.0 - exp(-k * dist)) / k;
}

// Sky radiance at the horizon below/around this view direction (sky-view LUT, v just above 0.5).
fn horizon_sky(dir: vec3<f32>) -> vec3<f32> {
    var d = dir;
    d.y = max(d.y, 0.03);
    d = normalize(d);
    let r = fog.planet_radius_km + max(fog.camera_height_km, 0.001);
    let beta = acos(clamp(sqrt(max(r * r - fog.planet_radius_km * fog.planet_radius_km, 0.0)) / r, -1.0, 1.0));
    let zenith_horizon = PI - beta;
    let c = clamp(acos(d.y) / zenith_horizon, 0.0, 1.0);
    let v = 0.5 * (1.0 - sqrt(1.0 - c));
    let vh = d.xz;
    let sh = fog.sun_dir.xz;
    var cos_az = 1.0;
    if (length(vh) > 1e-5 && length(sh) > 1e-5) {
        cos_az = clamp(dot(normalize(vh), normalize(sh)), -1.0, 1.0);
    }
    let u = sqrt(acos(cos_az) / PI);
    let res = vec2<f32>(textureDimensions(skyview_lut));
    let uv = vec2<f32>((u + 0.5 / res.x) * (res.x / (res.x + 1.0)), (v + 0.5 / res.y) * (res.y / (res.y + 1.0)));
    return textureSampleLevel(skyview_lut, lut_sampler, uv, 0.0).rgb * fog.sky_scale;
}

fn hg(cos_t: f32, g: f32) -> f32 {
    return (1.0 - g * g) / (4.0 * PI * pow(max(1.0 + g * g - 2.0 * g * cos_t, 1e-6), 1.5));
}

fn height_fog(dir: vec3<f32>, dist: f32) -> FogResult {
    let tau = fog_optical_depth(dir, dist) * fog.absorption;
    let t_raw = exp(-tau);
    let opacity = (vec3<f32>(1.0) - t_raw) * fog.max_opacity;
    var res: FogResult;
    res.transmittance = vec3<f32>(1.0) - opacity;

    let toward = pow((1.0 + dot(dir, fog.sun_dir)) * 0.5, 4.0);
    let tint = mix(fog.tint_away, fog.tint_toward, toward);
    let ambient = horizon_sky(dir) * 0.5 * (fog.tint_toward + fog.tint_away);
    let sun = tint * fog.sun_illuminance * hg(dot(dir, fog.sun_dir), 0.55) * fog.sun_scatter;
    let moon = tint * fog.moon_illuminance * hg(dot(dir, fog.moon_dir), 0.55) * fog.sun_scatter;
    res.inscatter = (ambient + sun + moon + fog.ambient) * opacity;
    return res;
}

// Forward-shader use: col = col * f.transmittance + f.inscatter.
fn apply_height_fog(col: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let to_p = world_pos - fog.camera_pos;
    let dist = length(to_p);
    let f = height_fog(to_p / max(dist, 1e-4), dist);
    return col * f.transmittance + f.inscatter;
}

// Full-screen pass: blends fog over the lit HDR target using the depth buffer.
struct FsIn {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_fog(@builtin(vertex_index) i: u32) -> FsIn {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: FsIn;
    out.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(uv.x, 1.0 - uv.y);
    return out;
}

// Output: in-scatter in rgb, luminance-weighted transmittance in alpha. Blend state: src factor One,
// dst factor SrcAlpha, so result = inscatter + scene * transmittance. Alpha carries one channel only, so
// per-channel absorption (Underwater) uses the forward path (apply_height_fog) instead.
@fragment
fn fs_fog(in: FsIn) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(depth_tex));
    let d = textureLoad(depth_tex, vec2<i32>(in.uv * size), 0);
    let ndc = vec4<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, d, 1.0);
    let wp = fog.inv_proj_view * ndc;
    let world = wp.xyz / wp.w;
    let to_p = world - fog.camera_pos;
    var dist = length(to_p);
    if (d >= 1.0) {
        dist = fog.max_distance;
    }
    let f = height_fog(to_p / max(length(to_p), 1e-4), dist);
    let t = dot(f.transmittance, vec3<f32>(0.2126, 0.7152, 0.0722));
    return vec4<f32>(f.inscatter, t);
}
