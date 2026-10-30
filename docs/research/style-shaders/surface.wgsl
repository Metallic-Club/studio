// Metallic style sample: forward surface shader with a per-material lighting model id.
//
// Research sample, not engine code. It shows the hook the M1 renderer can adopt cheaply:
// - Vertex layout matches the Task 3 renderer: mesh position/normal at locations 0-1, instance model
//   matrix at 2-5, colour at 6, material at 7 = (roughness, metalness, material id, lighting model id).
//   Task 3 leaves material.w unused; this sample reads it as the lighting model id.
// - Output is linear HDR (target Rgba16Float). Tonemapping and every style pass run later (tonemap.wgsl).
// - A second target carries the view-space normal and a per-material outline mask for post passes.
//
// Lighting models: 0 PBR (GGX), 1 Toon (bands + rim + specular blob), 2 Unlit, 3 Flat (faceted Lambert).
// Validate: naga docs/research/style-shaders/surface.wgsl

struct Frame {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    camera_pos: vec4<f32>,
    sun_dir: vec4<f32>,      // xyz: unit direction toward the sun
    sun_color: vec4<f32>,    // rgb: colour times intensity
    sky_color: vec4<f32>,    // ambient light from above
    ground_color: vec4<f32>, // ambient light from below
    toon: vec4<f32>,         // x: band count, y: band edge softness, z: rim width, w: specular size
    shadow_tint: vec4<f32>,  // rgb: colour multiplier in toon shadow (never pure black)
}

@group(0) @binding(0) var<uniform> frame: Frame;

const LM_PBR: u32 = 0u;
const LM_TOON: u32 = 1u;
const LM_UNLIT: u32 = 2u;
const LM_FLAT: u32 = 3u;
const PI: f32 = 3.14159265;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) m0: vec4<f32>,
    @location(3) m1: vec4<f32>,
    @location(4) m2: vec4<f32>,
    @location(5) m3: vec4<f32>,
    @location(6) color: vec4<f32>,
    @location(7) material: vec4<f32>,
}

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_n: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) @interpolate(flat) material: vec4<f32>,
}

struct FOut {
    @location(0) color: vec4<f32>,
    // xyz: view-space normal * 0.5 + 0.5; w: 1 if the material wants post-process outlines.
    @location(1) normal: vec4<f32>,
}

@vertex
fn vs_main(v: VIn) -> VOut {
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
    let wp = model * vec4<f32>(v.pos, 1.0);
    // Cofactor matrix: inverse transpose up to a positive scale, correct under non-uniform scale.
    let c0 = v.m0.xyz;
    let c1 = v.m1.xyz;
    let c2 = v.m2.xyz;
    let cof = mat3x3<f32>(cross(c1, c2), cross(c2, c0), cross(c0, c1));
    var out: VOut;
    out.clip = frame.view_proj * wp;
    out.world_pos = wp.xyz;
    out.world_n = normalize(cof * v.normal);
    out.color = v.color;
    out.material = v.material;
    return out;
}

fn hemi_ambient(n: vec3<f32>) -> vec3<f32> {
    return mix(frame.ground_color.rgb, frame.sky_color.rgb, n.y * 0.5 + 0.5);
}

fn shade_pbr(albedo: vec3<f32>, rough_in: f32, metal: f32, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    let rough = clamp(rough_in, 0.04, 1.0);
    let f0 = mix(vec3<f32>(0.04), albedo, metal);
    let diffuse = albedo * (1.0 - metal);
    let h = normalize(l + v);
    let ndl = max(dot(n, l), 0.0);
    let ndv = max(dot(n, v), 1e-4);
    let ndh = max(dot(n, h), 0.0);
    let vdh = max(dot(v, h), 0.0);
    let a = rough * rough;
    let a2 = a * a;
    let dd = ndh * ndh * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * dd * dd);
    let k = (rough + 1.0) * (rough + 1.0) / 8.0;
    let g = (ndl / (ndl * (1.0 - k) + k)) * (ndv / (ndv * (1.0 - k) + k));
    let f = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vdh, 5.0);
    let spec = d * g * f / max(4.0 * ndl * ndv, 1e-3);
    let direct = (diffuse / PI + spec) * frame.sun_color.rgb * ndl;
    let ambient = diffuse * hemi_ambient(n) + f0 * hemi_ambient(reflect(-v, n)) * (1.0 - rough);
    return direct + ambient;
}

// Quantises half-Lambert light into `bands` steps with soft, anti-aliased edges.
fn toon_bands(ndl: f32, shadow: f32, bands: f32, softness: f32) -> f32 {
    let x = clamp(ndl * 0.5 + 0.5, 0.0, 1.0) * shadow;
    let b = max(bands, 1.0);
    let scaled = x * b;
    let base = floor(scaled);
    let edge = smoothstep(0.5 - softness, 0.5 + softness, scaled - base);
    return min((base + edge) / b, 1.0);
}

// `spec_aa` and `rim_aa` are screen-space derivative widths computed in uniform control flow.
fn shade_toon(albedo: vec3<f32>, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, spec_aa: f32, rim_aa: f32) -> vec3<f32> {
    let ndl = dot(n, l);
    let lit = toon_bands(ndl, 1.0, frame.toon.x, frame.toon.y);
    let base = albedo * mix(frame.shadow_tint.rgb, vec3<f32>(1.0), lit);
    // Specular blob: hard-edged Blinn-Phong highlight.
    let ndh = dot(n, normalize(l + v));
    let t = 1.0 - frame.toon.w;
    let spec = smoothstep(t - spec_aa, t + spec_aa, ndh) * lit;
    // Rim light on the lit side only.
    let rim_x = 1.0 - max(dot(n, v), 0.0);
    let rt = 1.0 - frame.toon.z;
    let rim = smoothstep(rt - rim_aa, rt + rim_aa, rim_x) * lit;
    // Divided by PI like the PBR diffuse term, so one sun intensity works for every lighting model.
    let direct = (base + vec3<f32>(spec * 0.6 + rim * 0.4)) * frame.sun_color.rgb / PI;
    // Flat ambient keeps toon shadows readable and coloured.
    return direct + albedo * hemi_ambient(vec3<f32>(0.0, 1.0, 0.0)) * 0.5;
}

fn shade_flat(albedo: vec3<f32>, faceted_n: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    let ndl = max(dot(faceted_n, l), 0.0);
    return albedo * (frame.sun_color.rgb * ndl / PI + hemi_ambient(faceted_n));
}

@fragment
fn fs_main(in: VOut) -> FOut {
    let n = normalize(in.world_n);
    let v = normalize(frame.camera_pos.xyz - in.world_pos);
    let l = frame.sun_dir.xyz;

    // Derivatives must run in uniform control flow, so compute them before branching on the model id.
    var faceted = normalize(cross(dpdx(in.world_pos), dpdy(in.world_pos)));
    faceted = select(faceted, -faceted, dot(faceted, v) < 0.0);
    let ndh = dot(n, normalize(l + v));
    let spec_aa = max(fwidth(ndh), 1e-4);
    let rim_aa = max(fwidth(dot(n, v)), 1e-4);

    let albedo = in.color.rgb;
    let model = u32(in.material.w + 0.5);
    var col: vec3<f32>;
    var outline = 0.0;
    switch model {
        case LM_TOON: {
            col = shade_toon(albedo, n, v, l, spec_aa, rim_aa);
            outline = 1.0;
        }
        case LM_UNLIT: {
            col = albedo;
        }
        case LM_FLAT: {
            col = shade_flat(albedo, faceted, l);
            outline = 1.0;
        }
        default: {
            col = shade_pbr(albedo, in.material.x, in.material.y, n, v, l);
        }
    }

    let vn = normalize((frame.view * vec4<f32>(n, 0.0)).xyz);
    var out: FOut;
    out.color = vec4<f32>(col, in.color.a);
    out.normal = vec4<f32>(vn * 0.5 + 0.5, outline);
    return out;
}
