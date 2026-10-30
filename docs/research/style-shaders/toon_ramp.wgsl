// Metallic style sample: ramp-texture toon lighting (anime / cel look).
//
// Research sample, not engine code. The artist paints a small ramp texture (for example 256 x 8):
// x = half-Lambert light amount times shadow, y = ramp row picked per material. Hard edges, soft edges,
// coloured shadow bands and day/night variants are all just different ramp pixels, no shader change.
// Pair it with outline_hull.wgsl (inverted hull) or outline_post.wgsl (depth/normal edges).
// Output is linear HDR; tonemap.wgsl runs later.
// Validate: naga docs/research/style-shaders/toon_ramp.wgsl

struct Frame {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    ambient: vec4<f32>,
}

struct ToonMaterial {
    base_color: vec4<f32>,
    rim_color: vec4<f32>,  // rgb colour, a: rim width (0..1)
    ramp_row: f32,         // which row of the ramp texture this material uses, 0..rows-1
    ramp_rows: f32,
    spec_size: f32,        // 0 disables the highlight
    spec_strength: f32,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> mat: ToonMaterial;
@group(1) @binding(1) var ramp_tex: texture_2d<f32>;
@group(1) @binding(2) var albedo_tex: texture_2d<f32>;
@group(1) @binding(3) var clamp_samp: sampler;
@group(1) @binding(4) var repeat_samp: sampler;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_n: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

@vertex
fn vs_main(v: VIn) -> VOut {
    var out: VOut;
    out.clip = frame.view_proj * vec4<f32>(v.pos, 1.0);
    out.world_pos = v.pos;
    out.world_n = v.normal;
    out.uv = v.uv;
    return out;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let albedo = textureSample(albedo_tex, repeat_samp, in.uv).rgb * mat.base_color.rgb;
    let n = normalize(in.world_n);
    let v = normalize(frame.camera_pos.xyz - in.world_pos);
    let l = frame.sun_dir.xyz;
    let ndh = dot(n, normalize(l + v));
    let spec_aa = max(fwidth(ndh), 1e-4);
    let ndv = max(dot(n, v), 0.0);
    let rim_aa = max(fwidth(ndv), 1e-4);

    // A real renderer multiplies the cascade shadow term into `x` here.
    let x = clamp(dot(n, l) * 0.5 + 0.5, 0.0, 1.0);
    let row = (mat.ramp_row + 0.5) / max(mat.ramp_rows, 1.0);
    let ramp = textureSampleLevel(ramp_tex, clamp_samp, vec2<f32>(x, row), 0.0).rgb;

    let t = 1.0 - mat.spec_size;
    let spec = select(0.0, smoothstep(t - spec_aa, t + spec_aa, ndh), mat.spec_size > 0.0);
    let rt = 1.0 - mat.rim_color.a;
    let rim = smoothstep(rt - rim_aa, rt + rim_aa, 1.0 - ndv) * x;

    let col = albedo * (ramp * frame.sun_color.rgb + frame.ambient.rgb)
        + spec * mat.spec_strength * frame.sun_color.rgb
        + rim * mat.rim_color.rgb;
    return vec4<f32>(col, 1.0);
}
