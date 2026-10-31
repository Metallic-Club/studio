//! Morphological anti-aliasing in the style of SMAA 1x (#65), the AA of renders without MSAA.
//!
//! Three full-screen passes on the display-encoded image (LDR slot, after the finish pass):
//!
//! 1. **Edges**: luma steps above a threshold between a pixel and its left and top neighbours,
//!    kept only where they are at least half the largest step around (SMAA's local contrast
//!    adaptation), so a strong edge does not drag weak ones next to it in.
//! 2. **Weights**: for each edge a pixel touches, the edge is followed in both directions to
//!    its ends (up to 32 pixels). The crossing edges at the ends give the shape (L, Z or U),
//!    which reconstructs the silhouette as a line through the ends' half-pixel steps. The area
//!    between that line and the pixel edge, worked out exactly per pixel rather than read from
//!    a precomputed area texture, is how much of the neighbour across the edge the pixel takes.
//! 3. **Blend**: each pixel mixes in its four neighbours by those weights.
//!
//! Diagonal patterns and corner rounding of full SMAA are not done. Why this and not TAA: no
//! motion vectors or history, so nothing ghosts or smears thin toon lines and pixel looks, a
//! still frame is deterministic (captures and tests), and it runs on every tier including
//! WebGL2. The renderer turns it on when MSAA is off ([`crate::Quality::Low`]).
use crate::post::{FullscreenPass, LDR_FORMAT, PostContext, PostDraw, PostPass, PostSlot};

/// Name of the anti-aliasing pass in the post chain.
pub const SMAA_PASS: &str = "smaa";

const EDGES: &str = "
struct P { v: vec4<f32> } // x: luma threshold
@group(0) @binding(3) var<uniform> params: P;
fn lum(p: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(post_input));
    let c = textureLoad(post_input, clamp(p, vec2<i32>(0), size - 1), 0).rgb;
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}
@fragment fn fs_main(in: PostIn) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.clip.xy);
    let l = lum(p);
    let ll = lum(p - vec2<i32>(1, 0));
    let lt = lum(p - vec2<i32>(0, 1));
    let d = abs(vec2<f32>(l - ll, l - lt));
    // The image border is not an edge.
    var e = step(vec2<f32>(params.v.x), d) * vec2<f32>(f32(p.x > 0), f32(p.y > 0));
    if (e.x + e.y == 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let dr = abs(l - lum(p + vec2<i32>(1, 0)));
    let db = abs(l - lum(p + vec2<i32>(0, 1)));
    let dll = abs(ll - lum(p - vec2<i32>(2, 0)));
    let dtt = abs(lt - lum(p - vec2<i32>(0, 2)));
    let m = max(max(max(d.x, d.y), max(dr, db)), max(dll, dtt));
    e = e * step(vec2<f32>(m), 2.0 * d);
    return vec4<f32>(e, 0.0, 1.0);
}";

const WEIGHTS: &str = "
const MAX_SEARCH: i32 = 32;
fn edge(p: vec2<i32>) -> vec2<f32> {
    let size = vec2<i32>(textureDimensions(post_input));
    if (any(p < vec2<i32>(0)) || any(p >= size)) {
        return vec2<f32>(0.0);
    }
    return textureLoad(post_input, p, 0).rg;
}
// Height of the reconstructed line at an end of an edge: half a pixel into the side the
// crossing edge is on (+ is the second pixel of the pair), 0 with no or both crossings.
fn end_height(second: f32, first: f32) -> f32 {
    if (second > 0.5 && first < 0.5) {
        return 0.5;
    }
    if (first > 0.5 && second < 0.5) {
        return -0.5;
    }
    return 0.0;
}
// Positive and negative area under a line from height ha to hb over width w.
fn seg(w: f32, ha: f32, hb: f32) -> vec2<f32> {
    if (ha >= 0.0 && hb >= 0.0) {
        return vec2<f32>(w * (ha + hb) * 0.5, 0.0);
    }
    if (ha <= 0.0 && hb <= 0.0) {
        return vec2<f32>(0.0, -w * (ha + hb) * 0.5);
    }
    let t = ha / (ha - hb);
    if (ha > 0.0) {
        return vec2<f32>(w * t * ha * 0.5, -w * (1.0 - t) * hb * 0.5);
    }
    return vec2<f32>(w * (1.0 - t) * hb * 0.5, -w * t * ha * 0.5);
}
// Line height at x along an edge of length len: straight from hl to hr (L and Z shapes), or
// down to 0 in the middle and back (U shapes).
fn line_h(x: f32, len: f32, hl: f32, hr: f32) -> f32 {
    if (hl * hr > 0.0) {
        let m = 0.5 * len;
        return select(hr * (x - m) / m, hl * (1.0 - x / m), x < m);
    }
    return hl + (hr - hl) * x / len;
}
// Areas for column i of an edge of length len: x for the second pixel, y for the first.
fn column_area(i: f32, len: f32, hl: f32, hr: f32) -> vec2<f32> {
    if (hl == 0.0 && hr == 0.0) {
        return vec2<f32>(0.0);
    }
    let a = i;
    let b = i + 1.0;
    let m = 0.5 * len;
    if (hl * hr > 0.0 && a < m && b > m) {
        return seg(m - a, line_h(a, len, hl, hr), 0.0) + seg(b - m, 0.0, line_h(b, len, hl, hr));
    }
    return seg(1.0, line_h(a, len, hl, hr), line_h(b, len, hl, hr));
}
// The horizontal edge on the top side of p (between rows p.y - 1 and p.y).
fn horizontal(p: vec2<i32>) -> vec2<f32> {
    var dl = 0;
    for (var k = 1; k <= MAX_SEARCH; k = k + 1) {
        if (edge(p - vec2<i32>(k, 0)).y < 0.5) {
            break;
        }
        dl = k;
    }
    var dr = 0;
    for (var k = 1; k <= MAX_SEARCH; k = k + 1) {
        if (edge(p + vec2<i32>(k, 0)).y < 0.5) {
            break;
        }
        dr = k;
    }
    let x0 = p.x - dl;
    let x1 = p.x + dr;
    var hl = end_height(edge(vec2<i32>(x0, p.y)).x, edge(vec2<i32>(x0, p.y - 1)).x);
    var hr = end_height(edge(vec2<i32>(x1 + 1, p.y)).x, edge(vec2<i32>(x1 + 1, p.y - 1)).x);
    hl = select(hl, 0.0, dl == MAX_SEARCH);
    hr = select(hr, 0.0, dr == MAX_SEARCH);
    return column_area(f32(dl), f32(dl + dr + 1), hl, hr);
}
// The vertical edge on the left side of p (between columns p.x - 1 and p.x).
fn vertical(p: vec2<i32>) -> vec2<f32> {
    var du = 0;
    for (var k = 1; k <= MAX_SEARCH; k = k + 1) {
        if (edge(p - vec2<i32>(0, k)).x < 0.5) {
            break;
        }
        du = k;
    }
    var dd = 0;
    for (var k = 1; k <= MAX_SEARCH; k = k + 1) {
        if (edge(p + vec2<i32>(0, k)).x < 0.5) {
            break;
        }
        dd = k;
    }
    let y0 = p.y - du;
    let y1 = p.y + dd;
    var ht = end_height(edge(vec2<i32>(p.x, y0)).y, edge(vec2<i32>(p.x - 1, y0)).y);
    var hb = end_height(edge(vec2<i32>(p.x, y1 + 1)).y, edge(vec2<i32>(p.x - 1, y1 + 1)).y);
    ht = select(ht, 0.0, du == MAX_SEARCH);
    hb = select(hb, 0.0, dd == MAX_SEARCH);
    return column_area(f32(du), f32(du + dd + 1), ht, hb);
}
@fragment fn fs_main(in: PostIn) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.clip.xy);
    let here = edge(p);
    var w = vec4<f32>(0.0); // top, bottom, left, right
    if (here.y > 0.5) {
        w.x = horizontal(p).x;
    }
    if (edge(p + vec2<i32>(0, 1)).y > 0.5) {
        w.y = horizontal(p + vec2<i32>(0, 1)).y;
    }
    if (here.x > 0.5) {
        w.z = vertical(p).x;
    }
    if (edge(p + vec2<i32>(1, 0)).x > 0.5) {
        w.w = vertical(p + vec2<i32>(1, 0)).y;
    }
    return w;
}";

const BLEND: &str = "
fn col(p: vec2<i32>) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(post_input));
    return textureLoad(post_input, clamp(p, vec2<i32>(0), size - 1), 0).rgb;
}
@fragment fn fs_main(in: PostIn) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.clip.xy);
    let c = col(p);
    var w = textureLoad(post_aux, p, 0);
    let sum = w.x + w.y + w.z + w.w;
    if (sum <= 0.0) {
        return vec4<f32>(post_output(c), 1.0);
    }
    w = w / max(sum, 1.0);
    let mixed = c * (1.0 - min(sum, 1.0))
        + col(p - vec2<i32>(0, 1)) * w.x
        + col(p + vec2<i32>(0, 1)) * w.y
        + col(p - vec2<i32>(1, 0)) * w.z
        + col(p + vec2<i32>(1, 0)) * w.w;
    return vec4<f32>(post_output(mixed), 1.0);
}";

/// The anti-aliasing pass. The renderer adds one named [`SMAA_PASS`] at the end of the chain
/// and turns it on while MSAA is off; Looks without AA (pixel art) turn it off.
pub struct SmaaPass {
    /// Run the pass.
    pub enabled: bool,
    /// Smallest luma step (display-encoded, 0..1) treated as an edge.
    pub threshold: f32,
    edges: FullscreenPass,
    weights: FullscreenPass,
    blend: FullscreenPass,
    targets: Option<(u64, (u32, u32), wgpu::TextureView, wgpu::TextureView)>,
}

impl Default for SmaaPass {
    fn default() -> Self {
        SmaaPass {
            enabled: false,
            threshold: 0.1,
            edges: FullscreenPass::new("metallic smaa edges", EDGES, 16),
            weights: FullscreenPass::new("metallic smaa weights", WEIGHTS, 0),
            blend: FullscreenPass::new("metallic smaa blend", BLEND, 0),
            targets: None,
        }
    }
}

fn target(device: &wgpu::Device, label: &str, size: (u32, u32)) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: LDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

impl PostPass for SmaaPass {
    fn name(&self) -> &str {
        SMAA_PASS
    }

    fn slot(&self) -> PostSlot {
        PostSlot::Ldr
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn record(&mut self, ctx: &mut PostContext<'_>) {
        let fresh =
            matches!(&self.targets, Some((t, s, _, _)) if *t == ctx.targets && *s == ctx.size);
        if !fresh {
            self.targets = Some((
                ctx.targets,
                ctx.size,
                target(ctx.device, "metallic smaa edges", ctx.size),
                target(ctx.device, "metallic smaa weights", ctx.size),
            ));
        }
        let Some((_, _, edges, weights)) = self.targets.take() else {
            return;
        };
        let (input, output, output_format) = (ctx.input, ctx.output, ctx.output_format);
        let threshold = [self.threshold.clamp(0.01, 1.0), 0.0, 0.0, 0.0];
        self.edges.draw_to(
            ctx,
            PostDraw {
                input,
                aux: None,
                output: &edges,
                output_format: LDR_FORMAT,
            },
            bytemuck::cast_slice(&threshold),
        );
        self.weights.draw_to(
            ctx,
            PostDraw {
                input: &edges,
                aux: None,
                output: &weights,
                output_format: LDR_FORMAT,
            },
            &[],
        );
        self.blend.draw_to(
            ctx,
            PostDraw {
                input,
                aux: Some(&weights),
                output,
                output_format,
            },
            &[],
        );
        self.targets = Some((ctx.targets, ctx.size, edges, weights));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shaders_validate() {
        let p = SmaaPass::default();
        p.edges.validate();
        p.weights.validate();
        p.blend.validate();
    }
}
