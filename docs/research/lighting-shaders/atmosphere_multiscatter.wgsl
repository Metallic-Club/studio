// Multiple-scattering LUT Psi_ms, 32x32 rgba16float (Hillaire 2020 eq. 10; 0.07 ms on a GTX 1080).
// Prepend atmosphere_common.wgsl. Needs the transmittance LUT. Rebuild when Atmosphere changes.
//
// One workgroup per texel; its 64 threads each integrate one direction of a uniform 8x8 sphere grid
// (20 steps), then a shared-memory tree sums them. x = sun zenith cosine, y = altitude.
// Dispatch: 32 x 32 x 1 workgroups.

@group(0) @binding(0) var<uniform> atmo: AtmosphereParams;
@group(0) @binding(1) var transmittance_lut: texture_2d<f32>;
@group(0) @binding(2) var lut_sampler: sampler; // linear, clamp to edge
@group(0) @binding(3) var multiscatter_out: texture_storage_2d<rgba16float, write>;

const SQRT_DIRS: u32 = 8u;
const DIRS: u32 = 64u;
const MS_STEPS: f32 = 20.0;

var<workgroup> shared_l: array<vec3<f32>, 64>;
var<workgroup> shared_f: array<vec3<f32>, 64>;

@compute @workgroup_size(64, 1, 1)
fn build_multiscatter(
    @builtin(workgroup_id) wg: vec3<u32>,
    @builtin(local_invocation_index) li: u32,
) {
    let size = textureDimensions(multiscatter_out);
    let res = vec2<f32>(size);
    let uv = (vec2<f32>(wg.xy) + 0.5) / res;
    let u = from_sub_uv_to_unit(uv.x, res.x);
    let v = from_sub_uv_to_unit(uv.y, res.y);

    let cos_sun = u * 2.0 - 1.0;
    let sun_dir = vec3<f32>(0.0, cos_sun, sqrt(clamp(1.0 - cos_sun * cos_sun, 0.0, 1.0)));
    let height = atmo.bottom_radius
        + clamp(v + PLANET_RADIUS_OFFSET, 0.0, 1.0) * (atmo.top_radius - atmo.bottom_radius - PLANET_RADIUS_OFFSET);
    let ro = vec3<f32>(0.0, height, 0.0);

    // Direction for this thread: uniform on the sphere.
    let i = f32(li / SQRT_DIRS) + 0.5;
    let j = f32(li % SQRT_DIRS) + 0.5;
    let theta = 2.0 * PI * i / f32(SQRT_DIRS);
    let phi = acos(1.0 - 2.0 * j / f32(SQRT_DIRS));
    let rd = vec3<f32>(cos(theta) * sin(phi), cos(phi), sin(theta) * sin(phi));

    // ms_lut is unused when use_ms is false; pass the transmittance LUT to satisfy the signature.
    let r = integrate_light(atmo, transmittance_lut, transmittance_lut, lut_sampler, ro, rd, sun_dir,
                            MS_STEPS, false, true, true);
    let w = 4.0 * PI / f32(DIRS);
    shared_l[li] = r.luminance * w;
    shared_f[li] = r.multi_scat_as_1 * w;
    workgroupBarrier();

    // Tree reduction. The barrier stays in uniform control flow; only the adds are guarded.
    for (var stride = DIRS / 2u; stride > 0u; stride = stride / 2u) {
        if (li < stride) {
            shared_l[li] = shared_l[li] + shared_l[li + stride];
            shared_f[li] = shared_f[li] + shared_f[li + stride];
        }
        workgroupBarrier();
    }

    if (li == 0u) {
        let isotropic = 1.0 / (4.0 * PI);
        let l2 = shared_l[0] * isotropic;        // second-order luminance (eq. 5)
        let f_ms = shared_f[0] * isotropic;      // transfer factor (eq. 7)
        let psi = l2 / max(vec3<f32>(1.0) - f_ms, vec3<f32>(1e-4)); // geometric series (eq. 9-10)
        textureStore(multiscatter_out, vec2<i32>(wg.xy), vec4<f32>(psi, 1.0));
    }
}
