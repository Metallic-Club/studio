// Full-screen sky draw: sky-view LUT + sun disk + moon + stars + overcast blend, into the HDR target.
// Prepend atmosphere_common.wgsl. Drawn after opaque geometry with depth test "equal to far plane"
// (reverse-Z: depth == 0) so only sky pixels shade. Output is pre-exposed linear HDR.

struct SkyView {
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>, // world, studs
}

@group(0) @binding(0) var<uniform> atmo: AtmosphereParams;
@group(0) @binding(1) var<uniform> frame: SkyFrame;
@group(0) @binding(2) var<uniform> view: SkyView;
@group(0) @binding(3) var transmittance_lut: texture_2d<f32>;
@group(0) @binding(4) var skyview_lut: texture_2d<f32>;
@group(0) @binding(5) var lut_sampler: sampler;

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    let p = uv * 2.0 - 1.0;
    var out: SkyOut;
    out.clip = vec4<f32>(p, 0.0, 1.0); // reverse-Z far plane
    out.ndc = p;
    return out;
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    var q = fract(vec3<f32>(p.xyx) * vec3<f32>(0.1031, 0.1030, 0.0973));
    q = q + dot(q, q.yzx + 33.33);
    return fract((q.xx + q.yz) * q.zy);
}

// Sun or moon disk with limb darkening (sun) and a 1-pixel-wide soft edge from the angular radius.
fn disk(view_dir: vec3<f32>, body_dir: vec3<f32>, radius: f32, limb: f32) -> f32 {
    let c = dot(view_dir, body_dir);
    let ang = acos(clamp(c, -1.0, 1.0));
    let x = clamp(ang / radius, 0.0, 1.0);
    let mu = sqrt(max(1.0 - x * x, 0.0));
    let darkening = 1.0 - limb * (1.0 - mu);
    let edge = clamp((radius - ang) / (radius * 0.08), 0.0, 1.0);
    return darkening * edge;
}

// Stars: one candidate per cell of an equal-angle grid; density from StarCount; twinkle near the horizon.
fn stars(dir: vec3<f32>) -> f32 {
    if (frame.star_intensity <= 0.0 || dir.y < 0.0) {
        return 0.0;
    }
    let cells = 900.0;
    let lon = atan2(dir.x, dir.z);
    let lat = asin(clamp(dir.y, -1.0, 1.0));
    let cell = floor(vec2<f32>((lon + PI) / (2.0 * PI) * cells, (lat + 0.5 * PI) / PI * cells * 0.5));
    let h = hash22(cell);
    if (h.x >= frame.star_density) {
        return 0.0;
    }
    let mag = 0.5 + h.y * 1.5;
    let twinkle = 1.0 + 0.25 * sin(frame.time_seconds * (3.0 + 5.0 * h.y) + h.x * 100.0) * (1.0 - dir.y);
    return mag * twinkle;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let far_h = view.inv_view_proj * vec4<f32>(in.ndc, 0.0, 1.0);
    let dir = normalize(far_h.xyz / far_h.w - view.camera_pos.xyz);
    let r = atmo.bottom_radius + max(frame.camera_height_km, 0.001);
    let res = vec2<f32>(textureDimensions(skyview_lut));
    let uv = skyview_dir_to_uv(atmo, dir, frame.sun_dir, r, res);
    var l = textureSampleLevel(skyview_lut, lut_sampler, uv, 0.0).rgb * frame.light_illuminance;

    // Overcast: CIE overcast distribution L = Lz (1 + 2 sin(elevation)) / 3 (CPU sets Lz so the
    // horizontal illuminance is 35% of the clear sky's). Blend weight cover^0.7.
    let k = pow(clamp(frame.cloud_cover, 0.0, 1.0), 0.7);
    let overcast = frame.overcast_zenith * (1.0 + 2.0 * clamp(dir.y, 0.0, 1.0)) / 3.0;
    l = mix(l, overcast, k);

    let clear = pow(1.0 - clamp(frame.cloud_cover, 0.0, 1.0), 3.0);
    // Sun disk: luminance of the real sun (0.2666 deg radius) attenuated by the atmosphere, drawn at the
    // creator's visual size. It saturates the tonemapper and feeds bloom, so the glow comes from bloom.
    let sun_solid_angle = 2.0 * PI * (1.0 - cos(0.004654));
    let t_sun = transmittance_to_top(atmo, transmittance_lut, lut_sampler, r, frame.sun_dir.y);
    var horizon_cut = 1.0;
    if (ray_sphere_nearest(vec3<f32>(0.0, r, 0.0), dir, atmo.bottom_radius) > 0.0) {
        horizon_cut = 0.0;
    }
    let sun_l = frame.light_illuminance / sun_solid_angle * t_sun;
    l = l + sun_l * disk(dir, frame.sun_dir, frame.sun_angular_radius, 0.6) * clear * horizon_cut;

    // Moon: a lit grey disk (albedo 0.12) with the same transmittance model; no phase shading.
    let t_moon = transmittance_to_top(atmo, transmittance_lut, lut_sampler, r, frame.moon_dir.y);
    let moon_l = frame.light_illuminance * frame.moon_relative * 0.12 / sun_solid_angle * t_moon;
    l = l + moon_l * disk(dir, frame.moon_dir, frame.moon_angular_radius, 0.2) * clear * horizon_cut;

    // Stars fade out as the sky brightens: hide them where the sky is brighter than 50x a star.
    let sky_lum = dot(l, vec3<f32>(0.2126, 0.7152, 0.0722));
    let star_vis = clamp(1.0 - sky_lum / (frame.star_intensity * 50.0 + 1e-12), 0.0, 1.0);
    l = l + vec3<f32>(frame.star_intensity * stars(dir) * star_vis * clear * horizon_cut);

    return vec4<f32>(l, 1.0);
}
