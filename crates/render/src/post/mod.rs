//! The HDR post chain (ADR 0008 §7–8, #85): the scene in an HDR colour target, the post effects, and one finish pass
//! into the output.
//!
//! - **The scene colour** is HDR: [`hdr_format`] picks `Rg11b10Ufloat` (4 bytes a pixel) where the adapter can
//!   render, blend and filter it, else `Rgba16Float` (8 bytes; the GLES fallback where it has float colour buffers),
//!   else plain 8-bit sRGB, which still runs the chain but has no headroom above 1. The renderer reports the format
//!   in use ([`PostStats::scene_format`]).
//! - **The effects** ([`crate::PassId::Mlaa`], [`crate::PassId::Bloom`]) read the scene colour and write textures of
//!   their own, in the order of the [`crate::PostChain`]: MLAA's edge weights at the scene's resolution, and bloom's
//!   chain of downsamples and upsamples from half (or a quarter of) the output's resolution down to a thirty-second
//!   ([`BloomSettings`]), one compute pass where the device has compute shaders and a render pass per step elsewhere
//!   ([`BloomSettings::compute`], #145). An effect not in the pass set costs nothing: no pass, no texture, nothing
//!   bound.
//! - **The finish pass** ([`crate::PassId::Post`]) reads the scene colour and writes the output once: exposure, the
//!   colour filter, bloom's composite, the tone curve, the grade (saturation, contrast), MLAA's neighbourhood
//!   blending, the bilinear upscale of a render scale, sRGB encoding and a dither of half an output level. Each of
//!   these would otherwise be a full-screen pass of its own, reading and writing the whole image; on tile-based GPUs
//!   that bandwidth is most of a post chain's cost.
//! - **MSAA** is the scene passes' ([`crate::RendererSettings::msaa_samples`]): resolved into the scene colour at the
//!   end of the last scene pass and discarded ([`crate::FramePlan`]).
//!
//! **The tone curve** ([`tone_curve`]) is the identity up to 0.85 and rolls off smoothly towards 1 above it, per
//! channel. Before the post chain the output clamped at 1, so at the default exposure everything below the knee
//! looks as it did, and highlights keep some detail instead of clipping. **Exposure** is fixed
//! ([`PostSettings::exposure`]), 1 by default; it does not meter the image yet, so nothing is read back from the GPU.
//!
//! Every pipeline (the finish pass's four variants, bloom's and MLAA's) is compiled off the frame
//! ([`crate::pipelines`]): those the first frame runs with the renderer, the others right after in the background.
//! Turning an effect on or changing the effect quality compiles nothing on the frame; it makes the effect's textures,
//! without waiting for the GPU, once the effect's pipelines exist (ADR 0008 §9).

mod bloom;
mod finish;
mod mlaa;

use std::fmt;
use std::str::FromStr;

pub(crate) use bloom::Bloom;
pub(crate) use finish::{Finish, FinishParams};
pub(crate) use mlaa::Mlaa;

use crate::context::in_heap_turn;
use crate::pipelines::{Pipeline, PipelineKey, Recipe};
use crate::{MsaaSampleCounts, PassId, PostChain, RenderContext};

pub use finish::tone_curve;

/// Anti-aliasing (ADR 0008 §8): none, MLAA (a post-process pass), or MSAA with 2, 4 or 8 samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AntiAliasing {
    /// None.
    #[default]
    Off,
    /// Morphological anti-aliasing: one pass finding edges and their blend weights, blended in the finish pass.
    Mlaa,
    /// Multisampling with this many samples (2, 4 or 8), resolved inside the last scene pass.
    Msaa(u32),
}

impl AntiAliasing {
    /// Every choice, cheapest first.
    pub const ALL: [AntiAliasing; 5] = [
        AntiAliasing::Off,
        AntiAliasing::Mlaa,
        AntiAliasing::Msaa(2),
        AntiAliasing::Msaa(4),
        AntiAliasing::Msaa(8),
    ];

    /// The name the settings file and the command line take: `off`, `mlaa`, `msaa2`, `msaa4`, `msaa8`.
    pub fn name(self) -> &'static str {
        match self {
            AntiAliasing::Off => "off",
            AntiAliasing::Mlaa => "mlaa",
            AntiAliasing::Msaa(2) => "msaa2",
            AntiAliasing::Msaa(4) => "msaa4",
            AntiAliasing::Msaa(_) => "msaa8",
        }
    }

    /// The MSAA samples per pixel (1 without MSAA).
    pub fn msaa_samples(self) -> u32 {
        match self {
            AntiAliasing::Msaa(samples) => samples.max(1),
            AntiAliasing::Off | AntiAliasing::Mlaa => 1,
        }
    }

    /// Whether MLAA runs.
    pub fn mlaa(self) -> bool {
        self == AntiAliasing::Mlaa
    }

    /// What a device with `counts` MSAA sample counts draws for this choice: MSAA at the largest count it has up to
    /// the one asked for, or MLAA where it has none above 1. Off and MLAA work everywhere.
    #[must_use]
    pub fn supported(self, counts: MsaaSampleCounts) -> Self {
        match self {
            AntiAliasing::Msaa(samples) => match counts.iter().filter(|&n| n > 1 && n <= samples).last() {
                Some(n) => AntiAliasing::Msaa(n),
                None => AntiAliasing::Mlaa,
            },
            other => other,
        }
    }
}

impl fmt::Display for AntiAliasing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A name that is not an anti-aliasing choice.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown anti-aliasing `{0}` (off, mlaa, msaa2, msaa4, msaa8)")]
pub struct UnknownAntiAliasing(pub String);

impl FromStr for AntiAliasing {
    type Err = UnknownAntiAliasing;

    /// Parses a name ([`AntiAliasing::name`]), whatever its case.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        AntiAliasing::ALL
            .into_iter()
            .find(|aa| aa.name().eq_ignore_ascii_case(name))
            .ok_or_else(|| UnknownAntiAliasing(name.to_owned()))
    }
}

/// How much smaller than the scene colour bloom's first level is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BloomStart {
    /// Half the output's size: the finest glow.
    Half,
    /// A quarter: a quarter of the texels of [`BloomStart::Half`] in every pass, the glow a little softer.
    Quarter,
}

impl BloomStart {
    /// The divisor of the output's size.
    pub fn divisor(self) -> u32 {
        match self {
            BloomStart::Half => 2,
            BloomStart::Quarter => 4,
        }
    }
}

/// The smallest bloom level is this many times smaller than the output, whatever the start: so every effect
/// quality blurs as far, and lower ones only skip the finest level.
pub const BLOOM_SPAN: u32 = 32;
/// The most levels bloom's chain has.
pub const MAX_BLOOM_LEVELS: usize = 6;

/// Bloom: a share of a wide blur of the image mixed back into it, so bright things glow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomSettings {
    /// The blur's share of the result, 0 to 1. Without a threshold every pixel is mixed with its surroundings by
    /// this much, so it is kept small; what stands out is light much brighter than its neighbours.
    pub intensity: f32,
    /// Only what is brighter than this (after exposure, in linear light) blooms, with a soft knee half as wide; 0
    /// for no threshold (the default: physically, every light scatters).
    pub threshold: f32,
    /// The first level's size.
    pub start: BloomStart,
    /// Downsample with Jimenez's 13-tap filter instead of the dual filter's 5 taps: steadier on small bright
    /// details, at about twice the taps.
    pub wide_filter: bool,
    /// Draw the chain as one compute pass where the device has compute shaders (the default; #145), rather than a
    /// render pass per step: the same filters, for one pass's CPU cost instead of 2 × levels − 1. Devices without
    /// compute shaders (WebGL 2, GLES 3.0) and OpenGL draw render passes either way.
    pub compute: bool,
}

impl BloomSettings {
    /// The Low effect quality: from a quarter of the output's size, 5-tap downsampling.
    pub const LOW: Self = Self {
        intensity: 0.1,
        threshold: 0.0,
        start: BloomStart::Quarter,
        wide_filter: false,
        compute: true,
    };
    /// Medium: from half the output's size, 5-tap downsampling.
    pub const MEDIUM: Self = Self {
        start: BloomStart::Half,
        ..Self::LOW
    };
    /// High (and Ultra): from half the output's size, 13-tap downsampling.
    pub const HIGH: Self = Self {
        wide_filter: true,
        ..Self::MEDIUM
    };

    /// How many levels bloom has for an output `height` pixels tall: from the start down to
    /// [`BLOOM_SPAN`] times smaller, none smaller than 2 texels, at least 1.
    pub fn levels(&self, height: u32) -> usize {
        let mut levels = 0;
        let mut divisor = self.start.divisor();
        while divisor <= BLOOM_SPAN && height / divisor >= 2 && levels < MAX_BLOOM_LEVELS {
            levels += 1;
            divisor *= 2;
        }
        levels.max(1)
    }
}

impl Default for BloomSettings {
    fn default() -> Self {
        Self::HIGH
    }
}

/// MLAA's edge detection and search.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MlaaSettings {
    /// The step in perceptual luma (0 to 1) that counts as an edge.
    pub threshold: f32,
    /// How far along an edge the search walks each way, in pixels: longer edges get smoother gradients, at more
    /// taps on edge pixels.
    pub search: u32,
}

impl MlaaSettings {
    /// Low: 8 pixels each way.
    pub const LOW: Self = Self {
        threshold: 0.1,
        search: 8,
    };
    /// Medium: 16 pixels each way.
    pub const MEDIUM: Self = Self {
        search: 16,
        ..Self::LOW
    };
    /// High: 32 pixels each way.
    pub const HIGH: Self = Self {
        search: 32,
        ..Self::LOW
    };
}

impl Default for MlaaSettings {
    fn default() -> Self {
        Self::HIGH
    }
}

/// How the post chain finishes the image: the exposure, the grade, and how its effects are drawn. Which effects run
/// is the pass set's choice ([`crate::RendererSettings::passes`]); their order is [`PostSettings::chain`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostSettings {
    /// What the scene's light is multiplied by before the tone curve (1: as lit).
    pub exposure: f32,
    /// A colour filter multiplying the image before the tone curve (white keeps it).
    pub tint: [f32; 3],
    /// Saturation after the tone curve (1 keeps it, 0 is grey).
    pub saturation: f32,
    /// Contrast around mid-grey after the tone curve (1 keeps it; taken as at least 0.001).
    pub contrast: f32,
    /// Bloom, when [`crate::PassId::Bloom`] runs.
    pub bloom: BloomSettings,
    /// MLAA, when [`crate::PassId::Mlaa`] runs.
    pub mlaa: MlaaSettings,
    /// The order the effects run in.
    pub chain: PostChain,
}

impl PostSettings {
    /// The effects of `quality` (bloom and MLAA as cheap as that tier), with no grade.
    pub const fn effects(bloom: BloomSettings, mlaa: MlaaSettings) -> Self {
        Self {
            exposure: 1.0,
            tint: [1.0; 3],
            saturation: 1.0,
            contrast: 1.0,
            bloom,
            mlaa,
            chain: PostChain::DEFAULT,
        }
    }

    /// The Low effect quality: bloom from a quarter of the output's size, MLAA searching 8 pixels.
    pub const LOW: Self = Self::effects(BloomSettings::LOW, MlaaSettings::LOW);
    /// Medium: bloom from half the output's size, MLAA searching 16 pixels.
    pub const MEDIUM: Self = Self::effects(BloomSettings::MEDIUM, MlaaSettings::MEDIUM);
    /// High (and Ultra): bloom from half the output's size with 13-tap downsampling, MLAA searching 32 pixels.
    pub const HIGH: Self = Self::effects(BloomSettings::HIGH, MlaaSettings::HIGH);
}

impl Default for PostSettings {
    fn default() -> Self {
        Self::HIGH
    }
}

/// What the post chain draws with, as the renderer last planned it: for the overlay and reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PostStats {
    /// The scene colour's format: `Rg11b10Ufloat` or `Rgba16Float` with the finish pass, the output's without it.
    pub scene_format: wgpu::TextureFormat,
    /// Whether the scene is drawn into an HDR target and finished (the finish pass runs).
    pub finish: bool,
    /// MSAA samples per pixel (1: none).
    pub msaa_samples: u32,
    /// Whether MLAA runs.
    pub mlaa: bool,
    /// Bloom's levels (0: no bloom).
    pub bloom_levels: u32,
    /// Render passes the frame begins after the shadows: the planned passes, with bloom's chain counted pass by
    /// pass where it draws render passes, and not at all where it is a compute pass.
    pub render_passes: u32,
    /// Compute passes the frame begins after the shadows: 1 for bloom's chain where it runs as compute dispatches
    /// ([`BloomSettings::compute`]), else 0.
    pub compute_passes: u32,
    /// Passes reading or writing a whole target at the scene's or output's resolution
    /// ([`crate::FramePlan::full_screen_passes`]).
    pub full_screen_passes: u32,
    /// The planned passes in order (bloom's chain as one), then `None`s.
    pub planned: [Option<PassId>; PassId::COUNT],
}

impl PostStats {
    /// The planned passes, in order.
    pub fn passes(&self) -> impl Iterator<Item = PassId> + '_ {
        self.planned.iter().map_while(|pass| *pass)
    }

    /// The anti-aliasing drawn.
    pub fn anti_aliasing(&self) -> AntiAliasing {
        if self.msaa_samples > 1 {
            AntiAliasing::Msaa(self.msaa_samples)
        } else if self.mlaa {
            AntiAliasing::Mlaa
        } else {
            AntiAliasing::Off
        }
    }

    /// The scene colour's format as a short name: `rg11b10f`, `rgba16f`, or the output's (`rgba8`, `bgra8`, with
    /// `-srgb` when it encodes).
    pub fn format_name(&self) -> &'static str {
        match self.scene_format {
            wgpu::TextureFormat::Rg11b10Ufloat => "rg11b10f",
            wgpu::TextureFormat::Rgba16Float => "rgba16f",
            wgpu::TextureFormat::Rgba8Unorm => "rgba8",
            wgpu::TextureFormat::Rgba8UnormSrgb => "rgba8-srgb",
            wgpu::TextureFormat::Bgra8Unorm => "bgra8",
            wgpu::TextureFormat::Bgra8UnormSrgb => "bgra8-srgb",
            _ => "other",
        }
    }
}

impl Default for PostStats {
    fn default() -> Self {
        Self {
            scene_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            finish: false,
            msaa_samples: 1,
            mlaa: false,
            bloom_levels: 0,
            render_passes: 0,
            compute_passes: 0,
            full_screen_passes: 0,
            planned: [None; PassId::COUNT],
        }
    }
}

/// The scene colour's format with the post chain on `context`'s adapter: the first of `Rg11b10Ufloat` and
/// `Rgba16Float` it can render to, blend, filter and sample, else 8-bit sRGB.
pub fn hdr_format(context: &RenderContext) -> wgpu::TextureFormat {
    [wgpu::TextureFormat::Rg11b10Ufloat, wgpu::TextureFormat::Rgba16Float]
        .into_iter()
        .find(|&format| {
            let features = context.format_features(format);
            features
                .allowed_usages
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING)
                && features
                    .flags
                    .contains(wgpu::TextureFormatFeatureFlags::FILTERABLE | wgpu::TextureFormatFeatureFlags::BLENDABLE)
        })
        .unwrap_or(wgpu::TextureFormat::Rgba8UnormSrgb)
}

/// The format of bloom's levels where its chain runs as one compute pass on `context` ([`BloomSettings::compute`]):
/// `Rg11b10Ufloat` where the adapter writes it as storage, else `Rgba16Float`; `None` without compute shaders
/// (WebGL 2, GLES 3.0) and on OpenGL (#145), which draw a render pass per step. With `Rgba16Float` levels wider than
/// the scene colour's, outputs above about 6 megapixels draw the render passes too (their levels would be large
/// textures).
pub fn bloom_compute_format(context: &RenderContext) -> Option<wgpu::TextureFormat> {
    bloom::compute_format(context)
}

/// Whether `format` holds light above 1.
pub fn is_hdr(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Rg11b10Ufloat | wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
    )
}

/// A full-screen pass's uniform slice of `size` bytes at `offset` in `buffer`.
fn uniform_slice(buffer: &wgpu::Buffer, offset: u64, size: u64) -> wgpu::BindingResource<'_> {
    wgpu::BindingResource::Buffer(wgpu::BufferBinding {
        buffer,
        offset,
        size: wgpu::BufferSize::new(size),
    })
}

/// A bind group layout entry for a fragment-stage uniform of `size` bytes.
fn uniform_entry(binding: u32, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size),
        },
        count: None,
    }
}

/// A bind group layout entry for a fragment-stage 2D float texture.
fn texture_entry(binding: u32, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// A bind group layout entry for a fragment-stage filtering sampler.
fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

/// What a full-screen pipeline is made from: `module`'s `entry_points` (vertex, fragment) drawing one triangle into
/// `format`, blended with `blend`. Owned, so its compile runs on any thread ([`crate::pipelines`]).
#[derive(Clone, Debug)]
pub(crate) struct FullScreen {
    pub(crate) label: &'static str,
    pub(crate) layout: wgpu::BindGroupLayout,
    pub(crate) module: wgpu::ShaderModule,
    pub(crate) entry_points: (&'static str, &'static str),
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) blend: Option<wgpu::BlendState>,
}

impl FullScreen {
    /// The compile of this pipeline.
    pub(crate) fn recipe(self) -> Recipe {
        Box::new(move |device, cache| self.create(device, cache).into())
    }

    /// Makes the pipeline now.
    pub(crate) fn create(&self, device: &wgpu::Device, cache: Option<&wgpu::PipelineCache>) -> wgpu::RenderPipeline {
        let (vertex, fragment) = self.entry_points;
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(self.label),
            bind_group_layouts: &[Some(&self.layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(self.label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some(vertex),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.module,
                entry_point: Some(fragment),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.format,
                    blend: self.blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache,
        })
    }
}

/// The linear, edge-clamped sampler the post passes read with.
fn linear_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("metallic post"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..wgpu::SamplerDescriptor::default()
    })
}

/// A 2D texture of `size` with `mips` levels in `format`, sampled and used as `usage` says (drawn into, or written
/// as storage).
fn post_texture(
    device: &wgpu::Device,
    label: &str,
    (width, height): (u32, u32),
    mips: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    in_heap_turn(|| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: usage | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    })
}

/// The post chain's GPU side: the finish pass, bloom and MLAA, with the sampler they share.
pub(crate) struct Post {
    sampler: wgpu::Sampler,
    pub(crate) finish: Finish,
    pub(crate) bloom: Bloom,
    pub(crate) mlaa: Mlaa,
}

impl Post {
    /// Every pipeline of the chain, in the order a frame is likeliest to need them: the finish pass's variants, then
    /// bloom's (its render passes', then its compute chain's), then MLAA's.
    pub(crate) const KEYS: [PipelineKey; 17] = [
        PipelineKey::Finish(1),
        PipelineKey::Finish(0),
        PipelineKey::Finish(3),
        PipelineKey::Finish(2),
        PipelineKey::BloomFirst(1),
        PipelineKey::BloomDown(1),
        PipelineKey::BloomUp,
        PipelineKey::BloomFirst(0),
        PipelineKey::BloomDown(0),
        PipelineKey::BloomFirst(2),
        PipelineKey::BloomCompute(1),
        PipelineKey::BloomCompute(4),
        PipelineKey::BloomCompute(5),
        PipelineKey::BloomCompute(0),
        PipelineKey::BloomCompute(3),
        PipelineKey::BloomCompute(2),
        PipelineKey::Mlaa,
    ];

    /// The chain for the finish pass writing `output` and the effects reading and writing `scene`, on `context`
    /// (bloom's compute chain where it has compute shaders). Its pipelines are compiled apart ([`Post::recipe`]) and
    /// installed as they finish.
    pub(crate) fn new(context: &RenderContext, output: wgpu::TextureFormat, scene: wgpu::TextureFormat) -> Self {
        let device = context.device();
        let sampler = linear_sampler(device);
        Self {
            finish: Finish::new(device, output),
            bloom: Bloom::new(context, scene),
            mlaa: Mlaa::new(device),
            sampler,
        }
    }

    /// Whether `key` is compiled ahead of any switch: every key of the chain but bloom's for the path this device
    /// does not draw by default ([`Bloom::prewarms`]).
    pub(crate) fn prewarms(&self, key: PipelineKey) -> bool {
        self.bloom.prewarms(key)
    }

    /// The compile of the chain's pipeline `key`; `None` for a key that is not the chain's.
    pub(crate) fn recipe(&self, key: PipelineKey) -> Option<Recipe> {
        match key {
            PipelineKey::Finish(variant) => Some(self.finish.full_screen(usize::from(variant)).recipe()),
            PipelineKey::BloomFirst(_)
            | PipelineKey::BloomDown(_)
            | PipelineKey::BloomUp
            | PipelineKey::BloomCompute(_) => self.bloom.recipe(key),
            PipelineKey::Mlaa => Some(self.mlaa.full_screen().recipe()),
            _ => None,
        }
    }

    /// Puts the compiled pipeline `key` in place.
    pub(crate) fn install(&mut self, key: PipelineKey, pipeline: Pipeline) {
        match key {
            PipelineKey::Finish(variant) => {
                if let Some(pipeline) = pipeline.render() {
                    self.finish.install(usize::from(variant), pipeline);
                }
            }
            PipelineKey::Mlaa => {
                if let Some(pipeline) = pipeline.render() {
                    self.mlaa.install(pipeline);
                }
            }
            _ => self.bloom.install(key, pipeline),
        }
    }

    /// Whether the chain's pipeline `key` is compiled (true for a key that is not the chain's).
    pub(crate) fn has(&self, key: PipelineKey) -> bool {
        match key {
            PipelineKey::Finish(variant) => self.finish.has(usize::from(variant)),
            PipelineKey::BloomFirst(_)
            | PipelineKey::BloomDown(_)
            | PipelineKey::BloomUp
            | PipelineKey::BloomCompute(_) => self.bloom.has(key),
            PipelineKey::Mlaa => self.mlaa.has(),
            _ => true,
        }
    }

    /// Makes the effects' textures and binds them, or drops them: `bloom` and `mlaa` say which run. MLAA's weights
    /// are the scene colour's `scene` size; bloom's levels follow the `output` size, so a render scale (or dynamic
    /// resolution moving it) leaves the glow as it is. A settings change, never per frame.
    pub(crate) fn rebuild(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: Option<&wgpu::TextureView>,
        (size, output): ((u32, u32), (u32, u32)),
        bloom: Option<&BloomSettings>,
        mlaa: bool,
    ) {
        self.bloom.rebuild(
            device,
            queue,
            &self.sampler,
            scene.filter(|_| bloom.is_some()),
            output,
            bloom,
        );
        self.mlaa.rebuild(device, scene.filter(|_| mlaa), size);
        self.finish
            .set_sources(device, &self.sampler, scene, self.bloom.output(), self.mlaa.output());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anti_aliasing_names_round_trip() {
        for aa in AntiAliasing::ALL {
            assert_eq!(aa.name().parse(), Ok(aa));
        }
        assert_eq!("MSAA4".parse(), Ok(AntiAliasing::Msaa(4)));
        assert!("fxaa".parse::<AntiAliasing>().is_err());
        assert_eq!(AntiAliasing::Msaa(4).msaa_samples(), 4);
        assert_eq!(AntiAliasing::Mlaa.msaa_samples(), 1);
        assert!(AntiAliasing::Mlaa.mlaa() && !AntiAliasing::Msaa(2).mlaa());
    }

    #[test]
    fn anti_aliasing_falls_back_to_what_the_device_has() {
        let four = MsaaSampleCounts::from_counts(&[1, 4]);
        assert_eq!(AntiAliasing::Msaa(8).supported(four), AntiAliasing::Msaa(4));
        assert_eq!(AntiAliasing::Msaa(4).supported(four), AntiAliasing::Msaa(4));
        assert_eq!(AntiAliasing::Msaa(2).supported(four), AntiAliasing::Mlaa);
        assert_eq!(
            AntiAliasing::Msaa(4).supported(MsaaSampleCounts::from_counts(&[1])),
            AntiAliasing::Mlaa
        );
        assert_eq!(AntiAliasing::Off.supported(four), AntiAliasing::Off);
    }

    #[test]
    fn bloom_blurs_as_far_at_every_quality() {
        // 1080 rows: from half (540) to a thirty-second (33), or from a quarter.
        assert_eq!(BloomSettings::HIGH.levels(1080), 5);
        assert_eq!(BloomSettings::LOW.levels(1080), 4);
        // A tiny target keeps every level at least 2 texels.
        assert_eq!(BloomSettings::HIGH.levels(16), 3);
        assert_eq!(BloomSettings::HIGH.levels(2), 1);
    }
}
