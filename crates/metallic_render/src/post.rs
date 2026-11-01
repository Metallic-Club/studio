//! The post chain: an ordered list of full-screen passes around the finish pass.
//!
//! A frame runs in named stages:
//!
//! ```text
//! shadows -> scene (sky, opaque, skinned, blended) -> HDR passes -> finish -> LDR passes -> target
//! ```
//!
//! The scene draws linear light into an [`HDR_FORMAT`] target (MSAA resolves into it). Passes in
//! the [`PostSlot::Hdr`] slot read and write that linear light (outlines, fog, bloom, depth of
//! field). The finish pass ([`crate::tonemap`]) applies exposure, grade and tone curve and
//! encodes for display. Passes in the [`PostSlot::Ldr`] slot read and write display-encoded
//! values (screen patterns, quantise and dither, vignette); the last one writes the target.
//! With no LDR pass, the finish pass writes the target directly, so an empty chain costs one
//! full-screen pass in total.
//!
//! Each slot runs its passes in list order. To add a pass, implement [`PostPass`] (most passes
//! wrap a [`FullscreenPass`] and a small parameter struct) and call
//! [`crate::Renderer::post_mut`]`().push(...)`.
use crate::camera::Camera;
use std::any::Any;
use std::borrow::Cow;

/// Format of the scene target and of every HDR pass's input and output, where the adapter can
/// render to it, filter it and (with MSAA) resolve into it. Otherwise (some GL / WebGL2
/// adapters) the renderer falls back to [`LDR_FORMAT`] and stores linear light in 8 bits: the
/// frame still works, but light above 1 clips before the finish pass and the darkest tones
/// band (about 1/255 linear steps). `tests/post_chain.rs` renders through the fallback.
/// [`PostContext::hdr_format`] is the one in use.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The HDR target format for an adapter whose [`HDR_FORMAT`] support is `features` (`None`
/// when unknown, as for a host-provided device) at `samples` MSAA samples.
pub fn hdr_format_for(
    features: Option<wgpu::TextureFormatFeatures>,
    samples: u32,
) -> wgpu::TextureFormat {
    let Some(f) = features else {
        return HDR_FORMAT;
    };
    use wgpu::TextureFormatFeatureFlags as F;
    let msaa = match samples {
        1 => F::empty(),
        2 => F::MULTISAMPLE_X2 | F::MULTISAMPLE_RESOLVE,
        8 => F::MULTISAMPLE_X8 | F::MULTISAMPLE_RESOLVE,
        16 => F::MULTISAMPLE_X16 | F::MULTISAMPLE_RESOLVE,
        _ => F::MULTISAMPLE_X4 | F::MULTISAMPLE_RESOLVE,
    };
    let usages = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
    if f.allowed_usages.contains(usages) && f.flags.contains(F::FILTERABLE | msaa) {
        HDR_FORMAT
    } else {
        LDR_FORMAT
    }
}
/// Format of LDR intermediates. They hold display-encoded (sRGB) values, not linear light.
pub const LDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Where in the frame a pass runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostSlot {
    /// Before the finish pass, on linear HDR light ([`HDR_FORMAT`]).
    Hdr,
    /// After the finish pass, on display-encoded values. The last LDR pass writes the target.
    Ldr,
}

/// What a pass gets to record its work.
///
/// A pass must write every pixel of [`PostContext::output`]: its previous content is undefined.
pub struct PostContext<'a> {
    /// The renderer's device.
    pub device: &'a wgpu::Device,
    /// The renderer's queue, for uniform writes before the frame is submitted.
    pub queue: &'a wgpu::Queue,
    /// The frame's encoder; record render passes into it.
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The image so far, single-sampled, `size` pixels.
    pub input: &'a wgpu::TextureView,
    /// Where to write the result, `size` pixels.
    pub output: &'a wgpu::TextureView,
    /// Format of `output`: [`PostContext::hdr_format`], [`LDR_FORMAT`] or, for the last LDR
    /// pass, the renderer's target format.
    pub output_format: wgpu::TextureFormat,
    /// Format of the HDR targets: [`HDR_FORMAT`], or [`LDR_FORMAT`] on adapters that cannot
    /// render to it. Passes that make their own HDR textures use this.
    pub hdr_format: wgpu::TextureFormat,
    /// Changes whenever the renderer rebuilds its targets (resize, quality change). Passes that
    /// cache bind groups of `input` / `output` drop them when it changes.
    pub targets: u64,
    /// Size of `input` and `output` in pixels.
    pub size: (u32, u32),
    /// The scene depth (`Depth32Float`, 0 near .. 1 far), present when any pass in the chain
    /// returns true from [`PostPass::reads_depth`].
    pub depth: Option<&'a wgpu::TextureView>,
    /// Sample count of `depth`: 1, or 4 with MSAA (bind it as `texture_depth_multisampled_2d`).
    pub depth_samples: u32,
    /// The geometry prepass (`Rgba16Float`, one sample per pixel, `size` pixels): world normal
    /// in rgb and linear view depth in a; a is negative on surfaces that draw their own
    /// inverted-hull outline and 0 on background. Present when any pass in the chain returns
    /// true from [`PostPass::reads_geometry`] and the adapter can render `Rgba16Float`.
    pub geometry: Option<&'a wgpu::TextureView>,
    /// The camera of this frame.
    pub camera: &'a Camera,
}

impl PostContext<'_> {
    /// True when the output texture encodes sRGB in hardware, so a pass must write linear
    /// values even in the LDR slot. [`FullscreenPass`] handles this with `post_output`.
    pub fn output_is_srgb(&self) -> bool {
        self.output_format.is_srgb()
    }
}

/// One full-screen pass of the chain.
pub trait PostPass: Any + Send {
    /// A unique name, used to find, reorder or remove the pass.
    fn name(&self) -> &str;
    /// Which slot the pass runs in.
    fn slot(&self) -> PostSlot;
    /// Return true to get [`PostContext::depth`]. The renderer then keeps the depth buffer.
    fn reads_depth(&self) -> bool {
        false
    }
    /// Return true to get [`PostContext::geometry`]. The renderer then draws the geometry
    /// prepass (one extra pass over the opaque geometry) before the scene.
    fn reads_geometry(&self) -> bool {
        false
    }
    /// A disabled pass is skipped without copying its input.
    fn enabled(&self) -> bool {
        true
    }
    /// Records the pass. Called once per frame while enabled.
    fn record(&mut self, ctx: &mut PostContext<'_>);
}

/// The ordered list of post passes. Get it with [`crate::Renderer::post_mut`].
#[derive(Default)]
pub struct PostChain {
    passes: Vec<Box<dyn PostPass>>,
}

impl PostChain {
    /// Appends a pass; it runs after the passes already in its slot. A pass with the same name
    /// is replaced in place.
    pub fn push(&mut self, pass: Box<dyn PostPass>) {
        match self.position(pass.name()) {
            Some(i) => self.passes[i] = pass,
            None => self.passes.push(pass),
        }
    }

    /// Inserts a pass just before the pass named `before`. Returns the pass back when there is
    /// no such pass or the name is taken.
    pub fn insert_before(
        &mut self,
        before: &str,
        pass: Box<dyn PostPass>,
    ) -> Result<(), Box<dyn PostPass>> {
        if self.position(pass.name()).is_some() {
            return Err(pass);
        }
        match self.position(before) {
            Some(i) => {
                self.passes.insert(i, pass);
                Ok(())
            }
            None => Err(pass),
        }
    }

    /// Removes and returns the pass named `name`.
    pub fn remove(&mut self, name: &str) -> Option<Box<dyn PostPass>> {
        self.position(name).map(|i| self.passes.remove(i))
    }

    /// The pass named `name` as its concrete type, to change its parameters.
    pub fn get_mut<T: PostPass>(&mut self, name: &str) -> Option<&mut T> {
        let i = self.position(name)?;
        let pass: &mut dyn Any = self.passes[i].as_mut();
        pass.downcast_mut::<T>()
    }

    /// Names of the passes in `slot`, in run order (disabled ones included).
    pub fn names(&self, slot: PostSlot) -> impl Iterator<Item = &str> {
        self.passes
            .iter()
            .filter(move |p| p.slot() == slot)
            .map(|p| p.name())
    }

    /// Number of passes, enabled or not.
    pub fn len(&self) -> usize {
        self.passes.len()
    }

    /// True when the chain holds no pass.
    pub fn is_empty(&self) -> bool {
        self.passes.is_empty()
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.passes.iter().position(|p| p.name() == name)
    }

    pub(crate) fn active(&self, slot: PostSlot) -> usize {
        self.passes
            .iter()
            .filter(|p| p.slot() == slot && p.enabled())
            .count()
    }

    pub(crate) fn reads_depth(&self) -> bool {
        self.passes.iter().any(|p| p.enabled() && p.reads_depth())
    }

    pub(crate) fn reads_geometry(&self) -> bool {
        self.passes
            .iter()
            .any(|p| p.enabled() && p.reads_geometry())
    }

    pub(crate) fn active_mut(&mut self, slot: PostSlot) -> impl Iterator<Item = &mut dyn PostPass> {
        self.passes
            .iter_mut()
            .filter(move |p| p.slot() == slot && p.enabled())
            .map(|p| p.as_mut() as &mut dyn PostPass)
    }
}

const COLOR_WGSL: &str = include_str!("../shaders/color.wgsl");
const PRELUDE_WGSL: &str = include_str!("../shaders/post_prelude.wgsl");

struct FullscreenGpu {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    sampler: wgpu::Sampler,
    params: Option<wgpu::Buffer>,
}

/// Where one [`FullscreenPass::draw_to`] reads and writes, for passes that work on their own
/// textures (a bloom mip chain, a half-resolution blur).
pub struct PostDraw<'a> {
    /// Bound as `post_input`.
    pub input: &'a wgpu::TextureView,
    /// Bound as `post_aux`; `None` binds `input` again.
    pub aux: Option<&'a wgpu::TextureView>,
    /// The texture to fill.
    pub output: &'a wgpu::TextureView,
    /// Format of `output`.
    pub output_format: wgpu::TextureFormat,
}

/// A full-screen fragment shader with the plumbing done: the building block for most passes.
///
/// The fragment source is appended to `shaders/post_prelude.wgsl` (and the sRGB helpers of
/// `color.wgsl`), which declares:
///
/// - `post_input` and `post_aux: texture_2d<f32>` and `post_sampler` (linear, clamp to edge);
///   `post_aux` is the input again unless [`PostDraw::aux`] names another texture,
/// - `PostIn { clip, uv }` for the fragment input (`uv` 0,0 at the top left),
/// - `post_output(c)`, which the fragment must apply to its colour before returning it.
///
/// The source defines `@fragment fn fs_main(in: PostIn) -> @location(0) vec4<f32>` and, when
/// `params_size` is not 0, `@group(0) @binding(3) var<uniform> params: YourParams;`. Sizes come
/// from `textureDimensions`. GPU objects are made on the first draw and a pipeline per output
/// format is cached. A pass may draw several times per frame, but the parameter block holds one
/// value per frame (the last written).
pub struct FullscreenPass {
    label: String,
    source: String,
    params_size: u64,
    gpu: Option<FullscreenGpu>,
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    /// Bind groups by (input, aux), kept across frames; cleared when the targets change.
    groups: Vec<CachedGroup>,
    groups_targets: Option<u64>,
    groups_size: (u32, u32),
}

struct CachedGroup {
    input: wgpu::TextureView,
    aux: wgpu::TextureView,
    group: wgpu::BindGroup,
}

/// Most distinct (input, aux) pairs one pass keeps bind groups for (a bloom chain uses ~6).
const MAX_CACHED_GROUPS: usize = 16;

impl FullscreenPass {
    /// A pass from WGSL fragment source with a parameter block of `params_size` bytes (0 for
    /// none; otherwise a multiple of 16).
    pub fn new(label: &str, fragment_wgsl: &str, params_size: u64) -> FullscreenPass {
        FullscreenPass {
            label: label.to_owned(),
            source: [COLOR_WGSL, PRELUDE_WGSL, fragment_wgsl].join("\n"),
            params_size,
            gpu: None,
            pipelines: Vec::new(),
            groups: Vec::new(),
            groups_targets: None,
            groups_size: (0, 0),
        }
    }

    /// Draws `ctx.input` through the shader into `ctx.output`. `params` (exactly `params_size`
    /// bytes, or empty to keep the last value) is copied to the parameter block first.
    pub fn draw(&mut self, ctx: &mut PostContext<'_>, params: &[u8]) {
        let (input, output, output_format) = (ctx.input, ctx.output, ctx.output_format);
        self.draw_to(
            ctx,
            PostDraw {
                input,
                aux: None,
                output,
                output_format,
            },
            params,
        );
    }

    /// Draws with explicit textures. `params` as in [`FullscreenPass::draw`].
    pub fn draw_to(&mut self, ctx: &mut PostContext<'_>, d: PostDraw<'_>, params: &[u8]) {
        let device = ctx.device;
        let gpu = self.gpu.get_or_insert_with(|| {
            make_fullscreen(device, &self.label, &self.source, self.params_size)
        });
        let pipeline_index = match self
            .pipelines
            .iter()
            .position(|(f, _)| *f == d.output_format)
        {
            Some(i) => i,
            None => {
                let p = fullscreen_pipeline(device, &self.label, gpu, d.output_format);
                self.pipelines.push((d.output_format, p));
                self.pipelines.len() - 1
            }
        };
        let pipeline = &self.pipelines[pipeline_index].1;
        if let Some(buf) = &gpu.params
            && !params.is_empty()
        {
            debug_assert_eq!(params.len() as u64, self.params_size);
            ctx.queue.write_buffer(buf, 0, params);
        }
        // Drop cached groups (and the textures they hold) when the renderer's targets or the
        // frame size change.
        if self.groups_targets != Some(ctx.targets) || self.groups_size != ctx.size {
            self.groups.clear();
            self.groups_targets = Some(ctx.targets);
            self.groups_size = ctx.size;
        }
        let aux = d.aux.unwrap_or(d.input);
        let group_index = match self
            .groups
            .iter()
            .position(|g| g.input == *d.input && g.aux == *aux)
        {
            Some(i) => i,
            None => {
                if self.groups.len() >= MAX_CACHED_GROUPS {
                    self.groups.remove(0);
                }
                let group = fullscreen_group(device, &self.label, gpu, d.input, aux);
                self.groups.push(CachedGroup {
                    input: d.input.clone(),
                    aux: aux.clone(),
                    group,
                });
                self.groups.len() - 1
            }
        };
        let group = &self.groups[group_index].group;
        let mut pass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(&self.label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: d.output,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
impl FullscreenPass {
    /// Parses and validates the joined source with naga (no GPU).
    pub(crate) fn validate(&self) {
        let module = naga::front::wgsl::parse_str(&self.source)
            .unwrap_or_else(|e| panic!("{}: {}", self.label, e.emit_to_string(&self.source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}: {}", self.label, e.emit_to_string(&self.source)));
    }
}

fn fullscreen_group(
    device: &wgpu::Device,
    label: &str,
    gpu: &FullscreenGpu,
    input: &wgpu::TextureView,
    aux: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let base = [
        wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(input),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::Sampler(&gpu.sampler),
        },
        wgpu::BindGroupEntry {
            binding: 2,
            resource: wgpu::BindingResource::TextureView(aux),
        },
    ];
    let with_params;
    let entries: &[wgpu::BindGroupEntry<'_>] = match &gpu.params {
        Some(buf) => {
            let [a, b, c] = base;
            with_params = [
                a,
                b,
                c,
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buf.as_entire_binding(),
                },
            ];
            &with_params
        }
        None => &base,
    };
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout: &gpu.layout,
        entries,
    })
}

fn make_fullscreen(
    device: &wgpu::Device,
    label: &str,
    source: &str,
    params_size: u64,
) -> FullscreenGpu {
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let mut entries = vec![
        texture(0),
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
        texture(2),
    ];
    if params_size > 0 {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
    }
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &entries,
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(source)),
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(label),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    FullscreenGpu {
        layout,
        pipeline_layout,
        module,
        sampler,
        params: (params_size > 0).then(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: params_size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        }),
    }
}

fn fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    gpu: &FullscreenGpu,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let constants = [("POST_OUTPUT_SRGB", if format.is_srgb() { 1.0 } else { 0.0 })];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&gpu.pipeline_layout),
        vertex: wgpu::VertexState {
            module: &gpu.module,
            entry_point: Some("post_vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &gpu.module,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            },
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fullscreen_prelude_validates() {
        let source = FullscreenPass::new(
            "test",
            "@fragment fn fs_main(in: PostIn) -> @location(0) vec4<f32> {
                let a = textureSampleLevel(post_aux, post_sampler, in.uv, 0.0);
                return vec4<f32>(post_output(textureLoad(post_input, vec2<i32>(in.clip.xy), 0).rgb + a.rgb), 1.0);
            }",
            0,
        )
        .source;
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    }

    #[test]
    fn hdr_format_falls_back_without_float_targets() {
        use wgpu::{TextureFormatFeatureFlags as F, TextureFormatFeatures, TextureUsages as U};
        let full = TextureFormatFeatures {
            allowed_usages: U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
            flags: F::FILTERABLE | F::MULTISAMPLE_X4 | F::MULTISAMPLE_RESOLVE,
        };
        assert_eq!(hdr_format_for(None, 4), HDR_FORMAT);
        assert_eq!(hdr_format_for(Some(full), 4), HDR_FORMAT);
        assert_eq!(hdr_format_for(Some(full), 1), HDR_FORMAT);
        let no_render = TextureFormatFeatures {
            allowed_usages: U::TEXTURE_BINDING,
            ..full
        };
        assert_eq!(hdr_format_for(Some(no_render), 1), LDR_FORMAT);
        let no_msaa = TextureFormatFeatures {
            flags: F::FILTERABLE,
            ..full
        };
        assert_eq!(hdr_format_for(Some(no_msaa), 4), LDR_FORMAT);
        assert_eq!(hdr_format_for(Some(no_msaa), 1), HDR_FORMAT);
    }

    struct Named(&'static str, PostSlot);

    impl PostPass for Named {
        fn name(&self) -> &str {
            self.0
        }
        fn slot(&self) -> PostSlot {
            self.1
        }
        fn record(&mut self, _: &mut PostContext<'_>) {}
    }

    #[test]
    fn chain_keeps_order_per_slot_and_replaces_by_name() {
        let mut c = PostChain::default();
        c.push(Box::new(Named("edges", PostSlot::Hdr)));
        c.push(Box::new(Named("dither", PostSlot::Ldr)));
        c.push(Box::new(Named("bloom", PostSlot::Hdr)));
        assert!(
            c.insert_before("bloom", Box::new(Named("fog", PostSlot::Hdr)))
                .is_ok()
        );
        assert!(
            c.insert_before("nope", Box::new(Named("x", PostSlot::Hdr)))
                .is_err()
        );
        c.push(Box::new(Named("edges", PostSlot::Hdr)));
        assert_eq!(
            c.names(PostSlot::Hdr).collect::<Vec<_>>(),
            ["edges", "fog", "bloom"]
        );
        assert_eq!(c.names(PostSlot::Ldr).collect::<Vec<_>>(), ["dither"]);
        assert!(c.get_mut::<Named>("fog").is_some());
        assert!(c.remove("fog").is_some());
        assert_eq!(c.len(), 3);
        assert_eq!(c.active(PostSlot::Hdr), 2);
    }
}
