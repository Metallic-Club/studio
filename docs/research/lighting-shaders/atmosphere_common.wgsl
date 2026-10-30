// Shared code for the Hillaire 2020 sky LUTs (research prototype for issue #37, not engine code).
//
// WGSL has no include, so the atmosphere_*.wgsl files expect this file to be prepended; validate.sh does
// that, and the engine's shader loader will do the same (simple concatenation or a WESL import).
//
// Units: kilometres for distances. Luminance is "per unit of light illuminance": the LUTs are transfer
// tables, and the renderer multiplies them by the (pre-exposed) sun illuminance at draw time, so the
// LUTs do not depend on Lighting.Brightness or exposure. Frame: +y up, planet centre at the origin.
//
// Earth constants come from the MIT reference code (github.com/sebh/UnrealEngineSkyAtmosphere,
// SetupEarthAtmosphere). The engine fills this struct from Atmosphere/Lighting properties; see
// docs/research/2026-10-30-lighting.md section 5 for the mapping.

struct AtmosphereParams {
    rayleigh_scattering: vec3<f32>,  // 1/km at sea level; Earth (0.005802, 0.013558, 0.033100)
    bottom_radius: f32,              // km; Earth 6360
    mie_scattering: vec3<f32>,       // 1/km; Earth 0.003996
    top_radius: f32,                 // km; Earth 6460
    mie_extinction: vec3<f32>,       // 1/km; Earth 0.004440
    mie_g: f32,                      // Cornette-Shanks asymmetry; Earth 0.8
    ozone_absorption: vec3<f32>,     // 1/km at the ozone peak; Earth (0.000650, 0.001881, 0.000085)
    rayleigh_exp_scale: f32,         // -1 / scale height; Earth -1/8
    ground_albedo: vec3<f32>,        // used by the multiple-scattering LUT; 0.3 grey looks right for Parts
    mie_exp_scale: f32,              // -1 / scale height; Earth -1/1.2
}

// Per-frame inputs shared by the sky-view LUT and the sky draw.
struct SkyFrame {
    sun_dir: vec3<f32>,        // unit, toward the sun
    moon_relative: f32,        // moon illuminance / sun illuminance (about 2e-6 for a full moon)
    moon_dir: vec3<f32>,       // unit, toward the moon
    camera_height_km: f32,     // camera altitude above the ground, km (studs * 0.00028)
    light_illuminance: vec3<f32>, // sun illuminance at the top of the atmosphere, pre-exposed, linear RGB
    cloud_cover: f32,          // Clouds.Cover 0..1 (overcast blend, see sky.wgsl)
    overcast_zenith: vec3<f32>, // overcast zenith luminance, pre-exposed (CPU computes from the clear sky)
    sun_angular_radius: f32,   // radians, visual disk (Sky.SunAngularSize / 2)
    moon_angular_radius: f32,  // radians
    star_intensity: f32,       // pre-exposed luminance of a star cell; 0 hides stars
    star_density: f32,         // fraction of cells that hold a star (StarCount / cell count)
    time_seconds: f32,         // for twinkle
}

const PI: f32 = 3.14159265358979;
const PLANET_RADIUS_OFFSET: f32 = 0.01; // km; keeps samples inside the atmosphere shell

struct MediumSample {
    scattering_ray: vec3<f32>,
    scattering_mie: vec3<f32>,
    extinction: vec3<f32>,
}

fn sample_medium(a: AtmosphereParams, height_km: f32) -> MediumSample {
    let h = max(height_km, 0.0);
    let d_ray = exp(a.rayleigh_exp_scale * h);
    let d_mie = exp(a.mie_exp_scale * h);
    // Ozone: tent from 10 km to 40 km, peak at 25 km.
    let d_ozo = max(0.0, 1.0 - abs(h - 25.0) / 15.0);
    var m: MediumSample;
    m.scattering_ray = d_ray * a.rayleigh_scattering;
    m.scattering_mie = d_mie * a.mie_scattering;
    m.extinction = m.scattering_ray + d_mie * a.mie_extinction + d_ozo * a.ozone_absorption;
    return m;
}

fn phase_rayleigh(cos_theta: f32) -> f32 {
    return 3.0 / (16.0 * PI) * (1.0 + cos_theta * cos_theta);
}

fn phase_cornette_shanks(cos_theta: f32, g: f32) -> f32 {
    let g2 = g * g;
    let k = 3.0 / (8.0 * PI) * (1.0 - g2) / (2.0 + g2);
    return k * (1.0 + cos_theta * cos_theta) / pow(max(1.0 + g2 - 2.0 * g * cos_theta, 1e-6), 1.5);
}

// Nearest positive hit of a ray with a sphere at the origin, or -1.
fn ray_sphere_nearest(ro: vec3<f32>, rd: vec3<f32>, radius: f32) -> f32 {
    let b = dot(ro, rd);
    let c = dot(ro, ro) - radius * radius;
    let disc = b * b - c;
    if (disc < 0.0) {
        return -1.0;
    }
    let s = sqrt(disc);
    let t0 = -b - s;
    let t1 = -b + s;
    if (t0 > 0.0) {
        return t0;
    }
    if (t1 > 0.0) {
        return t1;
    }
    return -1.0;
}

// ---- LUT parameterisations ---------------------------------------------------------------

// Keeps lookups on texel centres at the LUT borders.
fn from_unit_to_sub_uv(u: f32, res: f32) -> f32 {
    return (u + 0.5 / res) * (res / (res + 1.0));
}

fn from_sub_uv_to_unit(u: f32, res: f32) -> f32 {
    return (u - 0.5 / res) * (res / (res - 1.0));
}

// Transmittance LUT (Bruneton-style mapping): uv -> (radius r, zenith cosine mu).
fn transmittance_uv_to_r_mu(a: AtmosphereParams, uv: vec2<f32>) -> vec2<f32> {
    let big_h = sqrt(a.top_radius * a.top_radius - a.bottom_radius * a.bottom_radius);
    let rho = big_h * uv.y;
    let r = sqrt(rho * rho + a.bottom_radius * a.bottom_radius);
    let d_min = a.top_radius - r;
    let d_max = rho + big_h;
    let d = d_min + uv.x * (d_max - d_min);
    var mu = 1.0;
    if (d > 0.0) {
        mu = (big_h * big_h - rho * rho - d * d) / (2.0 * r * d);
    }
    return vec2<f32>(r, clamp(mu, -1.0, 1.0));
}

fn transmittance_r_mu_to_uv(a: AtmosphereParams, r: f32, mu: f32) -> vec2<f32> {
    let big_h = sqrt(a.top_radius * a.top_radius - a.bottom_radius * a.bottom_radius);
    let rho = sqrt(max(r * r - a.bottom_radius * a.bottom_radius, 0.0));
    let disc = r * r * (mu * mu - 1.0) + a.top_radius * a.top_radius;
    let d = max(0.0, -r * mu + sqrt(max(disc, 0.0)));
    let d_min = a.top_radius - r;
    let d_max = rho + big_h;
    return vec2<f32>((d - d_min) / max(d_max - d_min, 1e-6), rho / big_h);
}

fn transmittance_to_top(a: AtmosphereParams, lut: texture_2d<f32>, samp: sampler, r: f32, mu: f32) -> vec3<f32> {
    let uv = transmittance_r_mu_to_uv(a, r, mu);
    return textureSampleLevel(lut, samp, uv, 0.0).rgb;
}

// Multiple-scattering LUT (32x32): x = sun zenith cosine, y = altitude.
fn multiscatter_lookup(a: AtmosphereParams, lut: texture_2d<f32>, samp: sampler, r: f32, sun_mu: f32) -> vec3<f32> {
    let res = vec2<f32>(textureDimensions(lut));
    let u = from_unit_to_sub_uv(clamp(sun_mu * 0.5 + 0.5, 0.0, 1.0), res.x);
    let v = from_unit_to_sub_uv(clamp((r - a.bottom_radius) / (a.top_radius - a.bottom_radius), 0.0, 1.0), res.y);
    return textureSampleLevel(lut, samp, vec2<f32>(u, v), 0.0).rgb;
}

// Sky-view LUT: u = sqrt(azimuth from the sun / pi) (more texels near the sun), v = 0.5 at the horizon,
// squared spacing toward the horizon on both halves (Hillaire 2020 section 5.3).
// Returns (cos zenith, azimuth from the sun).
fn skyview_uv_to_angles(a: AtmosphereParams, uv_in: vec2<f32>, r: f32, res: vec2<f32>) -> vec2<f32> {
    let uv = clamp(vec2<f32>(from_sub_uv_to_unit(uv_in.x, res.x), from_sub_uv_to_unit(uv_in.y, res.y)),
                   vec2<f32>(0.0), vec2<f32>(1.0));
    let v_horizon = sqrt(max(r * r - a.bottom_radius * a.bottom_radius, 0.0));
    let beta = acos(clamp(v_horizon / r, -1.0, 1.0));
    let zenith_horizon = PI - beta;
    var zen: f32;
    if (uv.y < 0.5) {
        var c = 1.0 - 2.0 * uv.y;
        c = 1.0 - c * c;
        zen = zenith_horizon * c;
    } else {
        var c = uv.y * 2.0 - 1.0;
        c = c * c;
        zen = zenith_horizon + beta * c;
    }
    let azimuth = uv.x * uv.x * PI;
    return vec2<f32>(cos(zen), azimuth);
}

// view_dir and sun_dir in the +y-up frame.
fn skyview_dir_to_uv(a: AtmosphereParams, view_dir: vec3<f32>, sun_dir: vec3<f32>, r: f32, res: vec2<f32>) -> vec2<f32> {
    let v_horizon = sqrt(max(r * r - a.bottom_radius * a.bottom_radius, 0.0));
    let beta = acos(clamp(v_horizon / r, -1.0, 1.0));
    let zenith_horizon = PI - beta;
    let zen = acos(clamp(view_dir.y, -1.0, 1.0));
    var v: f32;
    if (zen < zenith_horizon) {
        let c = clamp(zen / zenith_horizon, 0.0, 1.0);
        v = 0.5 * (1.0 - sqrt(1.0 - c));
    } else {
        let c = clamp((zen - zenith_horizon) / max(beta, 1e-6), 0.0, 1.0);
        v = 0.5 * (sqrt(c) + 1.0);
    }
    // Azimuth between the horizontal projections of the view and the sun (0..pi, symmetric).
    let vh = view_dir.xz;
    let sh = sun_dir.xz;
    let lv = length(vh);
    let ls = length(sh);
    var cos_az = 1.0;
    if (lv > 1e-5 && ls > 1e-5) {
        cos_az = clamp(dot(vh, sh) / (lv * ls), -1.0, 1.0);
    }
    let u = sqrt(acos(cos_az) / PI);
    return vec2<f32>(from_unit_to_sub_uv(u, res.x), from_unit_to_sub_uv(v, res.y));
}

// ---- single scattering integrator (shared by the multi-scattering and sky-view LUTs) ------------

struct ScatterResult {
    luminance: vec3<f32>,
    multi_scat_as_1: vec3<f32>, // integral of scattering * transmittance, for f_ms (Hillaire eq. 7)
    transmittance: vec3<f32>,
}

// Ray march from ro (km, planet frame) along rd for one light. use_ms: add the multiple-scattering LUT.
// isotropic: uniform phase (multi-scattering LUT pass). with_ground: add a Lambertian planet hit.
fn integrate_light(
    a: AtmosphereParams,
    t_lut: texture_2d<f32>,
    ms_lut: texture_2d<f32>,
    samp: sampler,
    ro: vec3<f32>,
    rd: vec3<f32>,
    light_dir: vec3<f32>,
    steps: f32,
    use_ms: bool,
    isotropic: bool,
    with_ground: bool,
) -> ScatterResult {
    var res: ScatterResult;
    res.luminance = vec3<f32>(0.0);
    res.multi_scat_as_1 = vec3<f32>(0.0);
    res.transmittance = vec3<f32>(1.0);

    let t_ground = ray_sphere_nearest(ro, rd, a.bottom_radius);
    let t_top = ray_sphere_nearest(ro, rd, a.top_radius);
    var t_max = t_top;
    if (t_ground > 0.0) {
        t_max = t_ground;
    }
    if (t_max <= 0.0) {
        return res;
    }
    let dt = t_max / steps;
    let cos_theta = dot(rd, light_dir);
    var ph_ray = 1.0 / (4.0 * PI);
    var ph_mie = ph_ray;
    if (!isotropic) {
        ph_ray = phase_rayleigh(cos_theta);
        ph_mie = phase_cornette_shanks(cos_theta, a.mie_g);
    }
    var throughput = vec3<f32>(1.0);
    for (var i = 0.0; i < steps; i = i + 1.0) {
        let t = (i + 0.3) * dt;
        let p = ro + rd * t;
        let pr = length(p);
        let up = p / pr;
        let m = sample_medium(a, pr - a.bottom_radius);
        let sun_mu = dot(up, light_dir);
        let t_sun = transmittance_to_top(a, t_lut, samp, pr, sun_mu);
        // Planet shadow on this sample.
        var earth = 1.0;
        if (ray_sphere_nearest(p, light_dir, a.bottom_radius) > 0.0) {
            earth = 0.0;
        }
        var ms = vec3<f32>(0.0);
        if (use_ms) {
            ms = multiscatter_lookup(a, ms_lut, samp, pr, sun_mu);
        }
        let scat = m.scattering_ray + m.scattering_mie;
        let phase_scat = m.scattering_ray * ph_ray + m.scattering_mie * ph_mie;
        let s = earth * t_sun * phase_scat + ms * scat;
        let step_t = exp(-m.extinction * dt);
        let ext = max(m.extinction, vec3<f32>(1e-9));
        // Energy-conserving analytic integration over the segment (Hillaire 2015).
        res.luminance = res.luminance + throughput * (s - s * step_t) / ext;
        res.multi_scat_as_1 = res.multi_scat_as_1 + throughput * (scat - scat * step_t) / ext;
        throughput = throughput * step_t;
    }
    if (with_ground && t_ground > 0.0) {
        let pg = ro + rd * t_ground;
        let pgr = length(pg);
        let upg = pg / pgr;
        let ndl = clamp(dot(upg, light_dir), 0.0, 1.0);
        let t_sun_g = transmittance_to_top(a, t_lut, samp, pgr, dot(upg, light_dir));
        res.luminance = res.luminance + throughput * t_sun_g * ndl * a.ground_albedo / PI;
    }
    res.transmittance = throughput;
    return res;
}
