// Metallic style sample: inverted-hull outline (the classic cel-shading outline).
//
// Research sample, not engine code. The mesh is drawn a second time, pushed out along its normals,
// with FRONT faces culled (wgpu::Face::Front) so only the back shell shows around the silhouette.
// The push happens in clip space so the line keeps a constant width in pixels at any distance.
// Per-vertex colour alpha can scale the width (artists paint thin/thick lines, Guilty Gear style).
// For smooth lines on hard-edged meshes, bake smoothed normals into a separate attribute (location 2).
// Validate: naga docs/research/style-shaders/outline_hull.wgsl

struct Frame {
    view_proj: mat4x4<f32>,
    viewport: vec4<f32>,      // x, y: target size in pixels
}

struct Outline {
    color: vec4<f32>,         // linear rgb, a unused
    width_px: f32,            // line width in pixels at the base resolution
    max_clip_width: f32,      // cap in clip units (about world units times the projection scale)
    _pad0: f32,
    _pad1: f32,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> outline: Outline;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) smooth_normal: vec3<f32>, // averaged normal; equals `normal` on smooth meshes
    @location(3) color: vec4<f32>,         // a: per-vertex width scale (0 hides the line)
}

@vertex
fn vs_main(v: VIn) -> @builtin(position) vec4<f32> {
    let clip = frame.view_proj * vec4<f32>(v.pos, 1.0);
    let clip_n = (frame.view_proj * vec4<f32>(v.smooth_normal, 0.0)).xy;
    let len = length(clip_n);
    if (len < 1e-6) {
        return clip;
    }
    // Pixels -> NDC is 2 / size; multiplying by w undoes the perspective divide.
    let ndc_offset = (clip_n / len) * outline.width_px * v.color.a * 2.0 / frame.viewport.xy;
    var offset = ndc_offset * clip.w;
    // Cap the clip-space push so far objects do not get lines thicker than themselves.
    let cap = outline.max_clip_width;
    offset = offset * min(1.0, cap / max(length(offset), 1e-6));
    return vec4<f32>(clip.xy + offset, clip.z, clip.w);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(outline.color.rgb, 1.0);
}
