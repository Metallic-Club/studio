// Metallic style sample: post-process outlines from depth and normals.
//
// Research sample, not engine code. Runs as a full-screen pass after the opaque pass, before tonemapping.
// Inputs: 1x HDR colour, 1x depth, and 1x normal/mask (xyz: view normal * 0.5 + 0.5, w: material mask).
// With scene MSAA > 1, depth and normals come from a separate 1x geometry prepass; only HDR colour resolves.
// With MSAA off, stored opaque depth and its normal/mask MRT supply the inputs. Do not average normal masks.
// All inputs share internal size/camera. See rendering-styles-vfx-animation.md section 4.3 (#109).
// - Depth edges (silhouettes, creases between objects) darken the pixel on the NEAR side of the edge, so the
//   line sits on the object in front. This keeps one-pixel lines clean for 3D pixel art.
// - Normal edges (inner creases) without a depth edge lighten the pixel instead: a highlight line.
// Both are optional per style preset (strength 0 disables a kind).
// Validate: naga docs/research/style-shaders/outline_post.wgsl

struct OutlineParams {
    line_color: vec4<f32>,      // rgb: linear colour of depth lines, a: strength 0..1
    highlight: vec4<f32>,       // rgb: multiplier for normal-edge highlights, a: strength 0..1
    near_far: vec2<f32>,        // camera near and far planes (standard, not reversed, depth)
    depth_threshold: f32,       // relative depth jump that counts as an edge, e.g. 0.05
    normal_threshold: f32,      // 1 - dot(n0, n1) that counts as a crease, e.g. 0.3
    width_px: i32,              // sample distance in pixels (1 for pixel art)
    _pad0: i32,
    _pad1: i32,
    _pad2: i32,
}

@group(0) @binding(0) var<uniform> params: OutlineParams;
@group(0) @binding(1) var color_tex: texture_2d<f32>;
@group(0) @binding(2) var depth_tex: texture_depth_2d;
@group(0) @binding(3) var normal_tex: texture_2d<f32>;

struct FsIn {
    @builtin(position) pos: vec4<f32>,
}

// One triangle that covers the screen; no vertex buffer needed.
@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> FsIn {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: FsIn;
    out.pos = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    return out;
}

fn linear_depth(d: f32) -> f32 {
    let n = params.near_far.x;
    let f = params.near_far.y;
    return n * f / (f - d * (f - n));
}

fn load_depth(p: vec2<i32>, size: vec2<i32>) -> f32 {
    let q = clamp(p, vec2<i32>(0), size - vec2<i32>(1));
    return linear_depth(textureLoad(depth_tex, q, 0));
}

fn load_normal(p: vec2<i32>, size: vec2<i32>) -> vec4<f32> {
    let q = clamp(p, vec2<i32>(0), size - vec2<i32>(1));
    let raw = textureLoad(normal_tex, q, 0);
    return vec4<f32>(raw.xyz * 2.0 - 1.0, raw.w);
}

@fragment
fn fs_main(in: FsIn) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(color_tex));
    let p = vec2<i32>(in.pos.xy);
    let base = textureLoad(color_tex, p, 0);
    let c = load_normal(p, size);
    if (c.w < 0.5) {
        return base; // material opted out of outlines
    }
    let d0 = load_depth(p, size);
    let w = params.width_px;
    let offsets = array<vec2<i32>, 4>(vec2<i32>(w, 0), vec2<i32>(-w, 0), vec2<i32>(0, w), vec2<i32>(0, -w));

    var depth_edge = 0.0;
    var normal_edge = 0.0;
    for (var k = 0; k < 4; k = k + 1) {
        let q = p + offsets[k];
        let d1 = load_depth(q, size);
        // Neighbour clearly farther: this pixel is the near side of a silhouette.
        if ((d1 - d0) > params.depth_threshold * d0) {
            depth_edge = 1.0;
        }
        let n1 = load_normal(q, size);
        // Only count creases against neighbours at about the same depth (not across silhouettes).
        let same_surface = abs(d1 - d0) < params.depth_threshold * d0;
        if (same_surface && (1.0 - dot(c.xyz, n1.xyz)) > params.normal_threshold) {
            normal_edge = 1.0;
        }
    }

    var col = base.rgb;
    col = mix(col, params.line_color.rgb, depth_edge * params.line_color.a);
    col = mix(col, col * params.highlight.rgb, normal_edge * (1.0 - depth_edge) * params.highlight.a);
    return vec4<f32>(col, base.a);
}
