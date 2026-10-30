// Metallic style sample: PS1-style retro surface.
//
// Research sample, not engine code. Three tricks, each a parameter of the "Retro 32-bit" preset:
// - Vertex snapping: clip-space positions round to a coarse screen grid (the PS1 had no sub-pixel vertices).
// - Affine texture mapping: UVs interpolate without perspective correction (@interpolate(linear)), which
//   gives the warping you see on large polygons. Strength blends between affine and correct mapping.
// - Per-vertex (Gouraud) lighting with nearest-neighbour textures.
// Finish with pixel_art.wgsl fs_quantize in mode 1 (levels 32 = 15-bit colour) and 4x4 dither.
// Validate: naga docs/research/style-shaders/ps1_retro.wgsl

struct Frame {
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    ambient: vec4<f32>,
    snap_grid: vec2<f32>,     // grid size in pixels for vertex snapping, e.g. 320 x 240
    affine: f32,              // 0 = perspective-correct UVs, 1 = full affine warp
    fog_far: f32,             // distance where vertex fog reaches full strength
    fog_color: vec4<f32>,
    camera_pos: vec4<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var point_samp: sampler;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
}

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(linear) uv_affine: vec2<f32>,
    @location(1) uv_persp: vec2<f32>,
    @location(2) light: vec3<f32>,
    @location(3) fog: f32,
}

@vertex
fn vs_main(v: VIn) -> VOut {
    var clip = frame.view_proj * vec4<f32>(v.pos, 1.0);
    // Snap in NDC, then return to clip space so depth and clipping still work.
    let w = max(clip.w, 1e-4);
    let half_grid = frame.snap_grid * 0.5;
    let ndc = round(clip.xy / w * half_grid) / half_grid;
    clip = vec4<f32>(ndc * w, clip.z, clip.w);

    let ndl = max(dot(normalize(v.normal), frame.sun_dir.xyz), 0.0);
    var out: VOut;
    out.clip = clip;
    out.uv_affine = v.uv;
    out.uv_persp = v.uv;
    out.light = v.color.rgb * (frame.sun_color.rgb * ndl + frame.ambient.rgb);
    out.fog = clamp(distance(v.pos, frame.camera_pos.xyz) / frame.fog_far, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let uv = mix(in.uv_persp, in.uv_affine, frame.affine);
    let texel = textureSampleLevel(tex, point_samp, uv, 0.0);
    if (texel.a < 0.5) {
        discard; // cut-out alpha, no blending, like the original hardware
    }
    let col = mix(texel.rgb * in.light, frame.fog_color.rgb, in.fog);
    return vec4<f32>(col, 1.0);
}
