// Metallic style sample: 3D-rendered pixel art. Palette quantise + ordered dither, then a clean upscale.
//
// Research sample, not engine code. Pipeline for the "Pixel 3D" look:
//   1. Render the scene into a low-res target (for example 480 x 270) with the camera snapped to whole
//      low-res texels in its own view plane, so static geometry does not shimmer when the camera moves.
//   2. outline_post.wgsl at width_px = 1, then tonemap.wgsl (curve 2 or 1) at low res.
//   3. fs_quantize (this file) at low res: posterise per channel or snap to a palette, with Bayer dither
//      evaluated per low-res pixel so the pattern stays locked to the art pixels.
//   4. fs_upscale (this file) to the window: integer or "sharp bilinear" scale, plus the sub-texel offset
//      the camera snap removed, so camera motion stays smooth while art pixels stay square.
// Validate: naga docs/research/style-shaders/pixel_art.wgsl

struct Quantize {
    mode: u32,               // 0 off, 1 levels per channel, 2 palette
    levels: f32,             // mode 1: steps per channel, e.g. 32 for 15-bit colour, 4 for a harsh look
    dither: f32,             // 0..1 dither strength
    matrix_size: u32,        // 4 or 8 (Bayer)
    palette_count: u32,      // mode 2: entries used, up to 64
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    palette: array<vec4<f32>, 64>, // display-space (sRGB-encoded) colours
}

struct Upscale {
    src_size: vec2<f32>,     // low-res target size in pixels
    dst_size: vec2<f32>,     // window size in pixels
    subpixel: vec2<f32>,     // camera snap remainder, in low-res texels (-0.5..0.5)
    sharpness: f32,          // 0 = nearest, 1 = sharp bilinear (for non-integer scales)
    _pad: f32,
}

@group(0) @binding(0) var<uniform> quant: Quantize;
@group(0) @binding(1) var<uniform> up: Upscale;
@group(0) @binding(2) var src_tex: texture_2d<f32>;
@group(0) @binding(3) var lin_samp: sampler;

struct FsIn {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> FsIn {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: FsIn;
    out.pos = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

// Bayer matrices built from bits, so no lookup table is needed. Values are in [0, 1).
fn bayer2(x: u32, y: u32) -> u32 {
    return ((x ^ y) << 1u) | y;
}

fn bayer4(p: vec2<u32>) -> f32 {
    let m = 4u * bayer2(p.x & 1u, p.y & 1u) + bayer2((p.x >> 1u) & 1u, (p.y >> 1u) & 1u);
    return (f32(m) + 0.5) / 16.0;
}

fn bayer8(p: vec2<u32>) -> f32 {
    let m4 = 4u * bayer2(p.x & 1u, p.y & 1u) + bayer2((p.x >> 1u) & 1u, (p.y >> 1u) & 1u);
    let m = 4u * m4 + bayer2((p.x >> 2u) & 1u, (p.y >> 2u) & 1u);
    return (f32(m) + 0.5) / 64.0;
}

// OKLab gives perceptual distances, so palette matches look right to the eye.
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3<f32>(0.0)) + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn oklab(srgb: vec3<f32>) -> vec3<f32> {
    let c = srgb_to_linear(srgb);
    let l = 0.4122214708 * c.r + 0.5363325363 * c.g + 0.0514459929 * c.b;
    let m = 0.2119034982 * c.r + 0.6806995451 * c.g + 0.1073969566 * c.b;
    let s = 0.0883024619 * c.r + 0.2817188376 * c.g + 0.6299787005 * c.b;
    let lc = pow(max(vec3<f32>(l, m, s), vec3<f32>(0.0)), vec3<f32>(1.0 / 3.0));
    return vec3<f32>(
        0.2104542553 * lc.x + 0.7936177850 * lc.y - 0.0040720468 * lc.z,
        1.9779984951 * lc.x - 2.4285922050 * lc.y + 0.4505937099 * lc.z,
        0.0259040371 * lc.x + 0.7827717662 * lc.y - 0.8086757660 * lc.z,
    );
}

fn nearest_palette(c: vec3<f32>) -> vec3<f32> {
    let target_lab = oklab(c);
    var best = quant.palette[0].rgb;
    var best_d = 1e9;
    let count = min(quant.palette_count, 64u);
    for (var i = 0u; i < count; i = i + 1u) {
        let p = quant.palette[i].rgb;
        let e = oklab(p) - target_lab;
        let d = dot(e, e);
        if (d < best_d) {
            best_d = d;
            best = p;
        }
    }
    return best;
}

// Runs at low resolution. Input and output are display-space colours (after tonemap.wgsl).
@fragment
fn fs_quantize(in: FsIn) -> @location(0) vec4<f32> {
    let p = vec2<u32>(in.pos.xy);
    let c = textureLoad(src_tex, vec2<i32>(p), 0);
    if (quant.mode == 0u) {
        return c;
    }
    var t = bayer4(p);
    if (quant.matrix_size == 8u) {
        t = bayer8(p);
    }
    if (quant.mode == 1u) {
        let steps = max(quant.levels - 1.0, 1.0);
        // Threshold dither: t replaces the 0.5 rounding offset, scaled by strength.
        let offset = mix(0.5, t, quant.dither);
        let q = floor(c.rgb * steps + offset) / steps;
        return vec4<f32>(clamp(q, vec3<f32>(0.0), vec3<f32>(1.0)), c.a);
    }
    // Palette: nudge the colour by the dither threshold, then take the nearest entry.
    let spread = quant.dither / max(f32(quant.palette_count), 2.0) * 4.0;
    let nudged = c.rgb + (t - 0.5) * spread;
    return vec4<f32>(nearest_palette(clamp(nudged, vec3<f32>(0.0), vec3<f32>(1.0))), c.a);
}

// Runs at window resolution. Square art pixels, smooth camera motion.
@fragment
fn fs_upscale(in: FsIn) -> @location(0) vec4<f32> {
    let scale = up.dst_size / up.src_size;
    // Position in low-res texel space, shifted by the camera's sub-texel remainder.
    let px = in.uv * up.src_size + up.subpixel - 0.5;
    let i = floor(px);
    let f = px - i;
    // Sharp bilinear: blend only within the last screen pixel at each texel border.
    let sharp = clamp((f - 0.5) * scale + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
    let nearest = step(vec2<f32>(0.5), f);
    let w = mix(nearest, sharp, up.sharpness);
    let uv = (i + w + 0.5) / up.src_size;
    return textureSampleLevel(src_tex, lin_samp, uv, 0.0);
}
