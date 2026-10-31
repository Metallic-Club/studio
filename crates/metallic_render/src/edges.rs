//! Post edge outlines from depth and normals (#65).
//!
//! An HDR pass that reads the geometry prepass (`shaders/prepass.wgsl`: world normal and linear
//! view depth at one sample per pixel) and darkens two kinds of line:
//!
//! - **Silhouettes**: where a neighbour `width_px` away is farther and the reciprocal depth is
//!   not affine across this pixel (planes are affine in 1/z on screen, so a floor seen at a
//!   grazing angle draws no line). Only the near side draws, so a line is `width_px` wide.
//! - **Creases**: where the normal turns by more than the threshold between two pixels of about
//!   the same depth (box edges, wedge ridges), drawn on one side.
//!
//! Surfaces with their own inverted-hull outline (toon characters, #320) are marked in the
//! prepass and get no post line, and the parts behind them see a nearer neighbour, so they draw
//! none either: one line per silhouette. Blended (transparent) parts are not in the prepass.
//! Line colour multiplies the scene colour, so lines keep their weight at any exposure.
use crate::post::{FullscreenPass, PostContext, PostDraw, PostPass, PostSlot};

/// Name of the edge pass in the post chain.
pub const EDGE_PASS: &str = "edges";

const EDGES: &str = "
struct P {
    line: vec4<f32>, // rgb: line colour as a multiplier (linear), w: silhouette strength
    opts: vec4<f32>, // x: width in pixels, y: depth threshold, z: crease threshold (1 - cos), w: crease strength
}
@group(0) @binding(3) var<uniform> params: P;

fn geo(p: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(post_aux));
    return textureLoad(post_aux, clamp(p, vec2<i32>(0), size - 1), 0);
}

// Linear depth of a geometry texel; background is very far.
fn depth_of(t: vec4<f32>) -> f32 {
    return select(1e6, abs(t.w), t.w != 0.0);
}

@fragment fn fs_main(in: PostIn) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.clip.xy);
    let c = textureLoad(post_input, p, 0).rgb;
    let g0 = geo(p);
    // Background, surfaces outlined by their own hull, and a pass with nothing to draw: unchanged.
    if (g0.w <= 0.0 || (params.line.w <= 0.0 && params.opts.w <= 0.0)) {
        return vec4<f32>(post_output(c), 1.0);
    }
    let d0 = g0.w;
    let inv0 = 1.0 / d0;
    let r = i32(max(floor(params.opts.x + 0.5), 1.0));
    let th = params.opts.y;
    var sil = 0.0;
    var crease = 0.0;
    for (var axis = 0; axis < 2; axis = axis + 1) {
        let o = select(vec2<i32>(0, r), vec2<i32>(r, 0), axis == 0);
        let ga = geo(p - o);
        let gb = geo(p + o);
        let da = depth_of(ga);
        let db = depth_of(gb);
        // Second difference of 1/z, relative to this pixel: 0 on any plane.
        let lap = abs((1.0 / da + 1.0 / db - 2.0 * inv0) * d0);
        let jump = smoothstep(th, 2.0 * th, lap);
        // Near side only: a neighbour clearly farther than this pixel.
        let farther = max(smoothstep(th, 2.0 * th, (da - d0) * inv0), smoothstep(th, 2.0 * th, (db - d0) * inv0));
        sil = max(sil, jump * farther);
        // Crease toward the + neighbour, on surfaces of about the same depth (a hull-outlined
        // neighbour already has its own line).
        if (gb.w > 0.0 && abs(db - d0) * inv0 < 4.0 * th) {
            let turn = 1.0 - dot(g0.xyz, gb.xyz);
            crease = max(crease, smoothstep(params.opts.z, 1.5 * params.opts.z, turn));
        }
    }
    let e = clamp(max(sil * params.line.w, crease * params.opts.w), 0.0, 1.0);
    return vec4<f32>(post_output(mix(c, c * params.line.rgb, e)), 1.0);
}";

/// The edge outline pass. The renderer adds one named [`EDGE_PASS`] before bloom, off by
/// default; turn it on with `renderer.post_mut().get_mut::<EdgePass>(EDGE_PASS)`.
pub struct EdgePass {
    /// Draw edges at all.
    pub enabled: bool,
    /// Line width in pixels at 1080p (scaled with the image height, at least 1 pixel).
    pub width_px: f32,
    /// Line colour, sRGB 0..255. It multiplies the scene colour.
    pub color: [f32; 3],
    /// Silhouette line strength, 0..1.
    pub strength: f32,
    /// Crease line strength, 0..1 (0 draws silhouettes only).
    pub crease_strength: f32,
    /// How much the reciprocal depth must bend across a pixel, relative to its own value, to
    /// count as a silhouette. Lower finds more lines.
    pub depth_threshold: f32,
    /// How much the normal must turn between neighbours for a crease, as 1 - cos(angle)
    /// (0.3 is about 46 degrees).
    pub crease_threshold: f32,
    pass: FullscreenPass,
}

impl Default for EdgePass {
    fn default() -> Self {
        EdgePass {
            enabled: false,
            width_px: 1.0,
            color: [40.0, 32.0, 30.0],
            strength: 1.0,
            crease_strength: 0.6,
            depth_threshold: 0.04,
            crease_threshold: 0.3,
            pass: FullscreenPass::new("metallic edges", EDGES, 32),
        }
    }
}

/// The parameter block of the edge shader.
fn params(e: &EdgePass, height: u32) -> [f32; 8] {
    let lin = e
        .color
        .map(|v| crate::scene::srgb_to_linear((v / 255.0).clamp(0.0, 1.0)));
    let width = (e.width_px * height as f32 / 1080.0).max(1.0);
    [
        lin[0],
        lin[1],
        lin[2],
        e.strength.clamp(0.0, 1.0),
        width,
        e.depth_threshold.max(1e-4),
        e.crease_threshold.clamp(1e-3, 2.0),
        e.crease_strength.clamp(0.0, 1.0),
    ]
}

impl PostPass for EdgePass {
    fn name(&self) -> &str {
        EDGE_PASS
    }

    fn slot(&self) -> PostSlot {
        PostSlot::Hdr
    }

    fn enabled(&self) -> bool {
        self.enabled && (self.strength > 0.0 || self.crease_strength > 0.0)
    }

    fn reads_geometry(&self) -> bool {
        true
    }

    fn record(&mut self, ctx: &mut PostContext<'_>) {
        let p = params(self, ctx.size.1);
        let (input, output, output_format) = (ctx.input, ctx.output, ctx.output_format);
        // Without the geometry target (an adapter without float targets) the image passes
        // through unchanged: the shader treats an all-zero geometry texel as background.
        let Some(geometry) = ctx.geometry else {
            self.pass.draw_to(
                ctx,
                PostDraw {
                    input,
                    aux: None,
                    output,
                    output_format,
                },
                bytemuck::cast_slice(&[0.0f32; 8]),
            );
            return;
        };
        self.pass.draw_to(
            ctx,
            PostDraw {
                input,
                aux: Some(geometry),
                output,
                output_format,
            },
            bytemuck::cast_slice(&p),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_validates() {
        EdgePass::default().pass.validate();
    }

    #[test]
    fn width_scales_with_height_and_never_drops_below_a_pixel() {
        let e = EdgePass::default();
        assert_eq!(params(&e, 1080)[4], 1.0);
        assert_eq!(params(&e, 2160)[4], 2.0);
        assert_eq!(params(&e, 360)[4], 1.0);
    }
}
