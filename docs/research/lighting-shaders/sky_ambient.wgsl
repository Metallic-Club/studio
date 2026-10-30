// Sky ambient: projects the sky-view LUT (plus a ground bounce for the lower hemisphere) onto 9
// spherical-harmonic coefficients already convolved with the cosine lobe, so surfaces get
// E(n) = sum(c_i * Y_i(n)) with one dot-product chain (research prototype, issue #37).
// Prepend atmosphere_common.wgsl. One 64-thread workgroup, 16 directions per thread (1024 total).
// Run after the sky-view LUT, every 4th frame (Low) or every frame (High): about 0.01 ms.
//
// The coefficients are in sky-view LUT units; multiply by frame.light_illuminance (pre-exposed) and by
// Lighting.EnvironmentDiffuseScale when shading. Sky visibility (voxel grid, GTAO) multiplies the result.

struct AmbientParams {
    ground_albedo: vec3<f32>,    // what the lower hemisphere reflects (Parts/terrain average, 0.2 default)
    cloud_cover: f32,            // Clouds.Cover
    color_shift_bottom: vec3<f32>, // 1 + Lighting.ColorShift_Bottom (0..1 sRGB values)
    _pad2: f32,
    sun_on_ground: vec3<f32>,    // sun + moon illuminance on a horizontal plane, LUT units (CPU computes)
    _pad0: f32,
    overcast_zenith: vec3<f32>,  // overcast zenith luminance, LUT units
    _pad1: f32,
}

@group(0) @binding(0) var<uniform> atmo: AtmosphereParams;
@group(0) @binding(1) var<uniform> frame: SkyFrame;
@group(0) @binding(2) var<uniform> amb: AmbientParams;
@group(0) @binding(3) var skyview_lut: texture_2d<f32>;
@group(0) @binding(4) var lut_sampler: sampler;
@group(0) @binding(5) var<storage, read_write> sh_out: array<vec4<f32>, 9>;

var<workgroup> partial: array<array<vec3<f32>, 9>, 64>;

fn sh_basis(d: vec3<f32>) -> array<f32, 9> {
    return array<f32, 9>(
        0.282095,
        0.488603 * d.y,
        0.488603 * d.z,
        0.488603 * d.x,
        1.092548 * d.x * d.y,
        1.092548 * d.y * d.z,
        0.315392 * (3.0 * d.z * d.z - 1.0),
        1.092548 * d.x * d.z,
        0.546274 * (d.x * d.x - d.y * d.y),
    );
}

fn sky_radiance(d: vec3<f32>) -> vec3<f32> {
    let r = atmo.bottom_radius + max(frame.camera_height_km, 0.001);
    let res = vec2<f32>(textureDimensions(skyview_lut));
    let uv = skyview_dir_to_uv(atmo, d, frame.sun_dir, r, res);
    let clear = textureSampleLevel(skyview_lut, lut_sampler, uv, 0.0).rgb;
    let overcast = amb.overcast_zenith * (1.0 + 2.0 * clamp(d.y, 0.0, 1.0)) / 3.0;
    return mix(clear, overcast, pow(clamp(amb.cloud_cover, 0.0, 1.0), 0.7));
}

@compute @workgroup_size(64, 1, 1)
fn project_sky_sh(@builtin(local_invocation_index) li: u32) {
    var acc = array<vec3<f32>, 9>();
    // Ground radiance: Lambertian ground lit by sun/moon (CPU term) plus about 15% for sky light, which is
    // what the reference renders measure at mid-day (docs/agents/37-lighting/sky_table.md).
    let ground = amb.ground_albedo * amb.color_shift_bottom / PI * amb.sun_on_ground * 1.15;
    for (var k = 0u; k < 16u; k = k + 1u) {
        let i = f32(li * 16u + k) + 0.5;
        let n = 1024.0;
        let phi = acos(1.0 - 2.0 * i / n);
        let th = PI * (1.0 + sqrt(5.0)) * i;
        let d = vec3<f32>(cos(th) * sin(phi), cos(phi), sin(th) * sin(phi));
        var l = ground;
        if (d.y > 0.0) {
            l = sky_radiance(d);
        }
        // The basis is written z-up; the world is y-up, so pass (x, z, y).
        let y = sh_basis(vec3<f32>(d.x, d.z, d.y));
        for (var c = 0u; c < 9u; c = c + 1u) {
            acc[c] = acc[c] + l * y[c];
        }
    }
    for (var c = 0u; c < 9u; c = c + 1u) {
        partial[li][c] = acc[c];
    }
    workgroupBarrier();
    for (var stride = 32u; stride > 0u; stride = stride / 2u) {
        if (li < stride) {
            for (var c = 0u; c < 9u; c = c + 1u) {
                partial[li][c] = partial[li][c] + partial[li + stride][c];
            }
        }
        workgroupBarrier();
    }
    if (li == 0u) {
        let w = 4.0 * PI / 1024.0;
        // Cosine-lobe convolution per band: pi, 2pi/3, pi/4.
        let band = array<f32, 9>(PI, 2.0 * PI / 3.0, 2.0 * PI / 3.0, 2.0 * PI / 3.0,
                                 PI / 4.0, PI / 4.0, PI / 4.0, PI / 4.0, PI / 4.0);
        for (var c = 0u; c < 9u; c = c + 1u) {
            sh_out[c] = vec4<f32>(partial[0][c] * w * band[c], 0.0);
        }
    }
}

// Irradiance for a world normal (call from the lighting shader with the same axis swizzle).
fn sh_irradiance(coeffs: array<vec4<f32>, 9>, n: vec3<f32>) -> vec3<f32> {
    let y = sh_basis(vec3<f32>(n.x, n.z, n.y));
    var e = vec3<f32>(0.0);
    for (var c = 0u; c < 9u; c = c + 1u) {
        e = e + coeffs[c].xyz * y[c];
    }
    return max(e, vec3<f32>(0.0));
}
