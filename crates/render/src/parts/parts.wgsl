// Parts: instanced primitive meshes, shaded PBR-lite (Lambert diffuse, GGX specular) under one directional light
// (the sun, or the moon at night) and a sky/ground hemisphere ambient, all from `Lighting`, plus the local point and
// spot lights (lights.wgsl). One shading model; the material row picks the parameters. frame.wgsl comes before this
// file, lights.wgsl after it, and the vertex entry point last, from instance_storage.wgsl or instance_vertex.wgsl
// (scene/gpu.rs explains the two paths).
//
// Positions are camera-relative: instance translations are relative to the scene origin, and `frame.camera_offset`
// (camera − origin) is subtracted before the rotation-only view-projection, so f32 stays precise far from the world
// origin (ADR 0008 §11).

const PI: f32 = 3.14159265;

// `MaterialRow` in parts/material.rs.
struct MaterialRow {
    // x: roughness, y: metalness, z: emissive, w: 1 for glass.
    surface: vec4<f32>,
    reserved: vec4<f32>,
}

// MAX_MATERIAL_ROWS in parts/material.rs.
const MATERIAL_ROWS: u32 = 256u;

@group(0) @binding(1) var<uniform> materials: array<MaterialRow, 256>;

// The sun's shadow cascades, one layer each, and a comparison sampler that filters four depth tests bilinearly.
@group(0) @binding(2) var shadow_maps: texture_depth_2d_array;
@group(0) @binding(3) var shadow_sampler: sampler_comparison;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // Camera-relative world position.
    @location(0) position: vec3<f32>,
    // World normal, not normalised.
    @location(1) normal: vec3<f32>,
    // Linear colour; alpha is 1 − Transparency.
    @location(2) @interpolate(flat) color: vec4<f32>,
    @location(3) @interpolate(flat) material: u32,
}

// The packed sRGB colour of an instance (red in the lowest byte), as linear light.
fn unpack_color(packed: u32) -> vec3<f32> {
    let srgb = vec3<f32>(f32(packed & 255u), f32((packed >> 8u) & 255u), f32((packed >> 16u) & 255u)) / 255.0;
    return srgb_to_linear(srgb);
}

// Places a mesh vertex with an instance's 3x4 affine (rows r0..r2) and packs what the fragment needs.
fn place(
    position: vec3<f32>,
    normal: vec3<f32>,
    r0: vec4<f32>,
    r1: vec4<f32>,
    r2: vec4<f32>,
    color: u32,
    transparency: f32,
    mesh_material: u32,
) -> VertexOut {
    let p = vec4<f32>(position, 1.0);
    let world = vec3<f32>(dot(r0, p), dot(r1, p), dot(r2, p)) - frame.camera_offset.xyz;
    // The cofactor matrix is the inverse transpose up to a positive scale: correct under non-uniform Size.
    let c0 = vec3<f32>(r0.x, r1.x, r2.x);
    let c1 = vec3<f32>(r0.y, r1.y, r2.y);
    let c2 = vec3<f32>(r0.z, r1.z, r2.z);
    let cofactor = mat3x3<f32>(cross(c1, c2), cross(c2, c0), cross(c0, c1));
    var out: VertexOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.position = world;
    out.normal = cofactor * normal;
    out.color = vec4<f32>(unpack_color(color), 1.0 - transparency);
    out.material = mesh_material & 0xffffu;
    return out;
}

fn material_row(row: u32) -> MaterialRow {
    return materials[select(0u, row, row < MATERIAL_ROWS)];
}

// Split-sum environment BRDF, analytic fit (Karis 2014): how much of the ambient light a surface reflects.
fn env_brdf(f0: vec3<f32>, roughness: f32, n_dot_v: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

fn hemisphere(direction_y: f32) -> vec3<f32> {
    return mix(frame.ground_ambient.rgb, frame.sky_ambient.rgb, clamp(direction_y * 0.5 + 0.5, 0.0, 1.0));
}

// How much of the sun (or moon) reaches a point: 1 lit, 0 in shadow. `position` is camera-relative, `n` the unit
// normal and `n_dot_l` its cosine to the light. The cascade is the first whose slice of the view the point is in;
// the point moves along its normal by the cascade's offset, scaled by 1 − N·L (only grazing receivers need it), and
// is compared with its bias, through a tent of bilinear comparison taps (`shadow_params.y` texels each way). Past
// the last cascade there is no shadow, faded in over the last tenth.
fn sun_visibility(position: vec3<f32>, n: vec3<f32>, n_dot_l: f32) -> f32 {
    let count = u32(frame.shadow_params.x);
    if count == 0u {
        return 1.0;
    }
    let depth = dot(position, frame.view_forward.xyz);
    var cascade = 0u;
    loop {
        if cascade >= count || depth < frame.shadow_splits[cascade] {
            break;
        }
        cascade += 1u;
    }
    if cascade >= count {
        return 1.0;
    }
    let p = position + n * (frame.shadow_offsets[cascade] * (1.0 - n_dot_l));
    let clip = frame.shadow_matrices[cascade] * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
    let reference = clip.z - frame.shadow_biases[cascade];
    let texel = frame.shadow_params.z;
    let reach = i32(frame.shadow_params.y);
    var lit = 0.0;
    var total = 0.0;
    for (var y = -reach; y <= reach; y += 1) {
        for (var x = -reach; x <= reach; x += 1) {
            let weight = f32(reach + 1 - abs(x)) * f32(reach + 1 - abs(y));
            let at = uv + vec2<f32>(f32(x), f32(y)) * texel;
            lit += weight * textureSampleCompareLevel(shadow_maps, shadow_sampler, at, cascade, reference);
            total += weight;
        }
    }
    let fade = smoothstep(frame.shadow_params.w, frame.shadow_splits[count - 1u], depth);
    return mix(lit / total, 1.0, fade);
}

struct Shaded {
    // Diffuse, ambient and emitted light: what transparency fades.
    body: vec3<f32>,
    // Specular reflection (direct and ambient).
    specular: vec3<f32>,
}

fn shade(in: VertexOut) -> Shaded {
    let row = material_row(in.material);
    let roughness = clamp(row.surface.x, 0.045, 1.0);
    let metalness = row.surface.y;
    let albedo = in.color.rgb;
    let n = normalize(in.normal);
    let v = normalize(-in.position);
    let l = frame.light_direction.xyz;

    let f0 = mix(vec3<f32>(0.04), albedo, metalness);
    let diffuse = albedo * (1.0 - metalness);
    let n_dot_l = max(dot(n, l), 0.0);
    let n_dot_v = max(dot(n, v), 1e-4);
    let h = normalize(l + v);
    let n_dot_h = max(dot(n, h), 0.0);
    let v_dot_h = max(dot(v, h), 0.0);

    // GGX distribution, height-correlated Smith visibility, Schlick Fresnel.
    let a = roughness * roughness;
    let a2 = a * a;
    let d_denominator = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    let distribution = a2 / (PI * d_denominator * d_denominator);
    let visibility = 0.5 / max(
        n_dot_l * sqrt(n_dot_v * n_dot_v * (1.0 - a2) + a2) + n_dot_v * sqrt(n_dot_l * n_dot_l * (1.0 - a2) + a2),
        1e-5,
    );
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - v_dot_h, 5.0);
    var shadow = 1.0;
    if n_dot_l > 0.0 {
        shadow = sun_visibility(in.position, n, n_dot_l);
    }
    let light = frame.light_color.rgb * n_dot_l * shadow;

    var out: Shaded;
    out.body = diffuse / PI * light + diffuse * hemisphere(n.y) + albedo * row.surface.z;
    out.specular = distribution * visibility * fresnel * light + env_brdf(f0, roughness, n_dot_v) * hemisphere(reflect(-v, n).y);
    // Frames that draw no local light use pipelines made with lights_off.wgsl, whose `local_lighting` adds nothing
    // and binds nothing (parts/mod.rs).
    let local = local_lighting(in.clip.xy, in.position, n, v, n_dot_v, diffuse, f0, a2);
    out.body += local.body;
    out.specular += local.specular;
    return out;
}

@fragment
fn fs_opaque(in: VertexOut) -> @location(0) vec4<f32> {
    let s = shade(in);
    return vec4<f32>(encode(s.body + s.specular), 1.0);
}

// Premultiplied alpha. Glass keeps its reflections at full strength while its body fades.
@fragment
fn fs_transparent(in: VertexOut) -> @location(0) vec4<f32> {
    let s = shade(in);
    let alpha = in.color.a;
    let glass = material_row(in.material).surface.w > 0.5;
    let specular = select(s.specular * alpha, s.specular, glass);
    return vec4<f32>(encode(s.body * alpha + specular), alpha);
}
