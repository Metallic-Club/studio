// Transmittance LUT, 256x64 rgba16float, 40 steps (Hillaire 2020 table 2: 0.01 ms on a GTX 1080).
// Prepend atmosphere_common.wgsl. Rebuild only when Atmosphere properties change, not per frame.
//
// Dispatch: ceil(256/8) x ceil(64/8) workgroups.

@group(0) @binding(0) var<uniform> atmo: AtmosphereParams;
@group(0) @binding(1) var transmittance_out: texture_storage_2d<rgba16float, write>;

const TRANSMITTANCE_STEPS: f32 = 40.0;

@compute @workgroup_size(8, 8, 1)
fn build_transmittance(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(transmittance_out);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(size);
    let r_mu = transmittance_uv_to_r_mu(atmo, uv);
    let r = r_mu.x;
    let mu = r_mu.y;
    let ro = vec3<f32>(0.0, r, 0.0);
    let rd = vec3<f32>(sqrt(max(1.0 - mu * mu, 0.0)), mu, 0.0);
    let t_max = ray_sphere_nearest(ro, rd, atmo.top_radius);
    var optical_depth = vec3<f32>(0.0);
    if (t_max > 0.0) {
        let dt = t_max / TRANSMITTANCE_STEPS;
        for (var i = 0.0; i < TRANSMITTANCE_STEPS; i = i + 1.0) {
            let p = ro + rd * ((i + 0.5) * dt);
            let m = sample_medium(atmo, length(p) - atmo.bottom_radius);
            optical_depth = optical_depth + m.extinction * dt;
        }
    }
    textureStore(transmittance_out, vec2<i32>(id.xy), vec4<f32>(exp(-optical_depth), 1.0));
}
