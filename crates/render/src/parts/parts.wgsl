// Parts: instanced primitive meshes, shaded PBR-lite (Lambert diffuse, GGX specular) under one directional light
// and a sky/ground ambient. One shading model; the material row picks the parameters. The vertex entry point is
// in instance_storage.wgsl or instance_vertex.wgsl, appended to this file (scene/gpu.rs explains the two paths).
//
// Positions are camera-relative: instance translations are relative to the scene origin, and `frame.camera_offset`
// (camera − origin) is subtracted before the rotation-only view-projection, so f32 stays precise far from the world
// origin (ADR 0008 §11).

const PI: f32 = 3.14159265;

// `FrameUniforms` in parts/mod.rs.
struct Frame {
    view_proj: mat4x4<f32>,
    // xyz: camera position − scene origin.
    camera_offset: vec4<f32>,
    // xyz: unit vector towards the light.
    light_direction: vec4<f32>,
    // rgb: the light's radiance (linear).
    light_color: vec4<f32>,
    // rgb: ambient light from above (linear).
    sky_ambient: vec4<f32>,
    // rgb: ambient light from below (linear).
    ground_ambient: vec4<f32>,
    // x: 1 when the target is not an sRGB format and the shader must encode.
    output: vec4<f32>,
}

// `MaterialRow` in parts/material.rs.
struct MaterialRow {
    // x: roughness, y: metalness, z: emissive, w: 1 for glass.
    surface: vec4<f32>,
    reserved: vec4<f32>,
}

// MAX_MATERIAL_ROWS in parts/material.rs.
const MATERIAL_ROWS: u32 = 256u;

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<uniform> materials: array<MaterialRow, 256>;

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

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308));
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
    let light = frame.light_color.rgb * n_dot_l;

    var out: Shaded;
    out.body = diffuse / PI * light + diffuse * hemisphere(n.y) + albedo * row.surface.z;
    out.specular = distribution * visibility * fresnel * light + env_brdf(f0, roughness, n_dot_v) * hemisphere(reflect(-v, n).y);
    return out;
}

fn encode(color: vec3<f32>) -> vec3<f32> {
    if frame.output.x > 0.5 {
        return linear_to_srgb(color);
    }
    return color;
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
