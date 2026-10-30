// Sky-view LUT, 192x108 rgba16float, 30 steps per texel (Hillaire 2020 section 5.3; 0.05 ms at
// 200x100 on a GTX 1080). Low tier: 96x54 and 12 steps, rebuilt every 2nd frame.
// Prepend atmosphere_common.wgsl. Needs the transmittance and multiple-scattering LUTs.
//
// The LUT holds sky luminance for a sun of illuminance 1 plus the moon (scaled by moon_relative), so
// sky radiance = sample * frame.light_illuminance. Twilight works because the integrator keeps
// lighting the upper atmosphere after the sun sets (planet shadow per sample).
// Dispatch: ceil(192/8) x ceil(108/8) workgroups, every frame (or when the sun moves > 0.1 deg).

@group(0) @binding(0) var<uniform> atmo: AtmosphereParams;
@group(0) @binding(1) var<uniform> frame: SkyFrame;
@group(0) @binding(2) var transmittance_lut: texture_2d<f32>;
@group(0) @binding(3) var multiscatter_lut: texture_2d<f32>;
@group(0) @binding(4) var lut_sampler: sampler; // linear, clamp to edge
@group(0) @binding(5) var skyview_out: texture_storage_2d<rgba16float, write>;

const SKYVIEW_STEPS: f32 = 30.0;

@compute @workgroup_size(8, 8, 1)
fn build_skyview(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(skyview_out);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let res = vec2<f32>(size);
    let uv = (vec2<f32>(id.xy) + 0.5) / res;
    let r = atmo.bottom_radius + max(frame.camera_height_km, 0.001);
    let ro = vec3<f32>(0.0, r, 0.0);
    let angles = skyview_uv_to_angles(atmo, uv, r, res);
    let cos_zen = angles.x;
    let sin_zen = sqrt(max(1.0 - cos_zen * cos_zen, 0.0));

    // The LUT frame puts the sun at azimuth 0 along +x; the moon keeps its azimuth relative to the sun.
    let sun_h = normalize(vec2<f32>(frame.sun_dir.x, frame.sun_dir.z) + vec2<f32>(1e-6, 0.0));
    let rd = vec3<f32>(sin_zen * cos(angles.y), cos_zen, sin_zen * sin(angles.y));
    let sun_local = vec3<f32>(sqrt(max(1.0 - frame.sun_dir.y * frame.sun_dir.y, 0.0)), frame.sun_dir.y, 0.0);
    // Moon in the same local frame: rotate its horizontal part by the sun's azimuth.
    let moon_h = vec2<f32>(frame.moon_dir.x, frame.moon_dir.z);
    let moon_local = vec3<f32>(dot(moon_h, sun_h), frame.moon_dir.y, sun_h.x * moon_h.y - sun_h.y * moon_h.x);

    let s = integrate_light(atmo, transmittance_lut, multiscatter_lut, lut_sampler, ro, rd, sun_local,
                            SKYVIEW_STEPS, true, false, false);
    var l = s.luminance;
    if (frame.moon_relative > 0.0) {
        let m = integrate_light(atmo, transmittance_lut, multiscatter_lut, lut_sampler, ro, rd, moon_local,
                                SKYVIEW_STEPS * 0.5, true, false, false);
        l = l + m.luminance * frame.moon_relative;
    }
    textureStore(skyview_out, vec2<i32>(id.xy), vec4<f32>(l, 1.0));
}
