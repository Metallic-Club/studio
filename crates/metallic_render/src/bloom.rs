//! Bloom: an HDR post pass driven by `BloomEffect` (`docs/research/2026-10-30-lighting.md` 5.4).
//!
//! A mip chain: a prefilter (soft threshold, firefly clamp) into half resolution, box-filtered
//! downsamples, then tent upsamples that add each level back. The composite blends the scene
//! toward the normalised blur by `intensity` (a lerp, so it keeps the image's energy).
use crate::post::{FullscreenPass, HDR_FORMAT, PostContext, PostDraw, PostPass, PostSlot};

/// Name of the bloom pass in the post chain.
pub const BLOOM_PASS: &str = "bloom";

const PREFILTER: &str = "
struct P { threshold: vec4<f32> }
@group(0) @binding(3) var<uniform> params: P;
@fragment fn fs_main(in: PostIn) -> @location(0) vec4<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(post_input));
    var acc = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var i = 0; i < 4; i = i + 1) {
        let o = vec2<f32>(f32(i & 1) - 0.5, f32(i >> 1u) - 0.5) * 2.0 * t;
        let c = textureSampleLevel(post_input, post_sampler, in.uv + o, 0.0).rgb;
        // Karis average: weight by inverse luminance so single bright pixels do not flicker.
        let w = 1.0 / (1.0 + dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)));
        acc = acc + c * w;
        wsum = wsum + w;
    }
    let c = acc / wsum;
    let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    let knee = params.threshold.x * 0.5 + 1e-4;
    let soft = clamp(l - params.threshold.x + knee, 0.0, 2.0 * knee);
    let contrib = max(soft * soft / (4.0 * knee), l - params.threshold.x) / max(l, 1e-4);
    return vec4<f32>(post_output(c * max(contrib, 0.0)), 1.0);
}";

const DOWNSAMPLE: &str = "
@fragment fn fs_main(in: PostIn) -> @location(0) vec4<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(post_input));
    var acc = vec3<f32>(0.0);
    for (var i = 0; i < 4; i = i + 1) {
        let o = vec2<f32>(f32(i & 1) - 0.5, f32(i >> 1u) - 0.5) * 2.0 * t;
        acc = acc + textureSampleLevel(post_input, post_sampler, in.uv + o, 0.0).rgb;
    }
    return vec4<f32>(post_output(acc * 0.25), 1.0);
}";

// `post_input` is the smaller level (already upsampled), `post_aux` this level's downsample.
const UPSAMPLE: &str = "
@fragment fn fs_main(in: PostIn) -> @location(0) vec4<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(post_input));
    var acc = vec3<f32>(0.0);
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let w = f32((2 - abs(x)) * (2 - abs(y)));
            acc = acc + textureSampleLevel(post_input, post_sampler, in.uv + vec2<f32>(f32(x), f32(y)) * t, 0.0).rgb * w;
        }
    }
    let here = textureSampleLevel(post_aux, post_sampler, in.uv, 0.0).rgb;
    return vec4<f32>(post_output(acc / 16.0 + here), 1.0);
}";

// `post_input` is the scene, `post_aux` the summed blur of `levels` levels.
const COMPOSITE: &str = "
struct P { v: vec4<f32> } // x: intensity, y: 1 / levels
@group(0) @binding(3) var<uniform> params: P;
@fragment fn fs_main(in: PostIn) -> @location(0) vec4<f32> {
    let scene = textureLoad(post_input, vec2<i32>(in.clip.xy), 0).rgb;
    let blur = textureSampleLevel(post_aux, post_sampler, in.uv, 0.0).rgb * params.v.y;
    return vec4<f32>(post_output(mix(scene, blur, params.v.x)), 1.0);
}";

struct Level {
    size: (u32, u32),
    down: wgpu::TextureView,
    up: wgpu::TextureView,
}

/// The bloom pass. The renderer adds one named [`BLOOM_PASS`] and sets it from the lighting.
pub struct BloomPass {
    /// Blend toward the blur, 0 (off) .. 1.
    pub intensity: f32,
    /// Blur size in pixels at 1080p; picks how many mip levels are used (2 to 6).
    pub size: f32,
    /// Scene luminance (after exposure) below which nothing blooms; 0 blooms everything a little.
    pub threshold: f32,
    prefilter: FullscreenPass,
    down: FullscreenPass,
    up: FullscreenPass,
    composite: FullscreenPass,
    levels: Vec<Level>,
}

impl Default for BloomPass {
    fn default() -> Self {
        BloomPass {
            intensity: 0.04,
            size: 24.0,
            threshold: 0.0,
            prefilter: FullscreenPass::new("metallic bloom prefilter", PREFILTER, 16),
            down: FullscreenPass::new("metallic bloom down", DOWNSAMPLE, 0),
            up: FullscreenPass::new("metallic bloom up", UPSAMPLE, 0),
            composite: FullscreenPass::new("metallic bloom composite", COMPOSITE, 16),
            levels: Vec::new(),
        }
    }
}

impl BloomPass {
    /// Mip levels for a `size` and an image `height`.
    fn level_count(&self, height: u32) -> usize {
        let px = self.size.max(1.0) * height as f32 / 1080.0;
        (px.log2().round() as i32 - 1).clamp(2, 6) as usize
    }

    fn ensure_levels(&mut self, device: &wgpu::Device, size: (u32, u32), count: usize) {
        let mut want = Vec::with_capacity(count);
        let mut s = size;
        for _ in 0..count {
            s = ((s.0 / 2).max(1), (s.1 / 2).max(1));
            want.push(s);
        }
        let same =
            self.levels.len() == count && self.levels.iter().zip(&want).all(|(l, w)| l.size == *w);
        if same {
            return;
        }
        let tex = |label: &str, size: (u32, u32)| {
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
                    format: HDR_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        self.levels = want
            .into_iter()
            .map(|size| Level {
                size,
                down: tex("metallic bloom down", size),
                up: tex("metallic bloom up", size),
            })
            .collect();
    }
}

impl PostPass for BloomPass {
    fn name(&self) -> &str {
        BLOOM_PASS
    }

    fn slot(&self) -> PostSlot {
        PostSlot::Hdr
    }

    fn enabled(&self) -> bool {
        self.intensity > 0.0
    }

    fn record(&mut self, ctx: &mut PostContext<'_>) {
        let count = self.level_count(ctx.size.1);
        self.ensure_levels(ctx.device, ctx.size, count);
        let threshold = [self.threshold.max(0.0), 0.0, 0.0, 0.0];
        let input = ctx.input;
        let levels = std::mem::take(&mut self.levels);
        self.prefilter.draw_to(
            ctx,
            PostDraw {
                input,
                aux: None,
                output: &levels[0].down,
                output_format: HDR_FORMAT,
            },
            bytemuck::cast_slice(&threshold),
        );
        for i in 1..levels.len() {
            self.down.draw_to(
                ctx,
                PostDraw {
                    input: &levels[i - 1].down,
                    aux: None,
                    output: &levels[i].down,
                    output_format: HDR_FORMAT,
                },
                &[],
            );
        }
        // The smallest level's "up" is its own downsample: start the sum from the next one.
        let last = levels.len() - 1;
        for i in (0..last).rev() {
            let smaller = if i + 1 == last {
                &levels[last].down
            } else {
                &levels[i + 1].up
            };
            self.up.draw_to(
                ctx,
                PostDraw {
                    input: smaller,
                    aux: Some(&levels[i].down),
                    output: &levels[i].up,
                    output_format: HDR_FORMAT,
                },
                &[],
            );
        }
        let v = [
            self.intensity.clamp(0.0, 1.0),
            1.0 / levels.len() as f32,
            0.0,
            0.0,
        ];
        let (output, output_format) = (ctx.output, ctx.output_format);
        self.composite.draw_to(
            ctx,
            PostDraw {
                input,
                aux: Some(&levels[0].up),
                output,
                output_format,
            },
            bytemuck::cast_slice(&v),
        );
        self.levels = levels;
    }
}
