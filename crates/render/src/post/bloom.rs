//! Bloom ([`crate::PassId::Bloom`]): downsamples from the scene colour into the levels of one texture, then upsamples
//! adding each level onto the next larger (`bloom.wgsl`). Every step works at half the output's resolution or below;
//! the chain is timed as one.
//!
//! **One compute pass** where the device has compute shaders, but on OpenGL (#145, [`compute_format`]): every step is
//! a dispatch of `bloom_compute.wgsl` in the same pass, so the chain of 2N − 1 steps costs one pass's recording and
//! submit (about 18 µs of render CPU a render pass on DX12) instead of one each. The upsamples write each level's sum
//! into a texture of its own (a dispatch cannot blend onto what it reads), the smallest level's sum being the level,
//! and every level is a texture of its own too (a backend that sets the mips a view samples on the texture, as
//! OpenGL does, cannot sample one mip and write another in one dispatch). The levels are `Rg11b10Ufloat`, as the
//! render passes' are, where the adapter writes it as storage (DX12, Vulkan and Metal on most GPUs), else
//! `Rgba16Float`, which every device with compute shaders writes. **Elsewhere** (OpenGL, WebGL 2), when
//! [`BloomSettings::compute`] is off, and where `Rgba16Float` levels would be large textures that the render passes'
//! narrower ones are not (outputs above about 6 megapixels at Medium and above; [`crate::textures`] makes large
//! textures off the frame, which a settings change of bloom's does not wait for), the chain is a render pass per step
//! into levels in the scene colour's format, the upsamples blending onto them.

use super::{BloomSettings, BloomStart, FullScreen, MAX_BLOOM_LEVELS, post_texture, sampler_entry};
use super::{texture_entry, uniform_entry, uniform_slice};
use crate::RenderContext;
use crate::pipelines::{Pipeline, PipelineKey, Recipe};
use crate::textures::BACKGROUND_BYTES;

/// Bytes of one step's parameters (`BloomPass` in `bloom.wgsl`).
const PARAMS_SIZE: u64 = 32;

/// The compute chain's level formats, best first: the render passes' compact one where the adapter writes it as
/// storage, then the one every device with compute shaders writes.
const COMPUTE_FORMATS: [wgpu::TextureFormat; 2] =
    [wgpu::TextureFormat::Rg11b10Ufloat, wgpu::TextureFormat::Rgba16Float];
/// The storage format `bloom_compute.wgsl` declares, replaced by [`Compute::new`] for another.
const SHADER_FORMAT: &str = "rgba16float";
/// The compute entry points' workgroup size in each direction (`bloom_compute.wgsl`).
const WORKGROUP: u32 = 8;

/// The render passes' names in GPU debuggers.
const DOWN_NAMES: [&str; MAX_BLOOM_LEVELS] = [
    "bloom_down_0",
    "bloom_down_1",
    "bloom_down_2",
    "bloom_down_3",
    "bloom_down_4",
    "bloom_down_5",
];
const UP_NAMES: [&str; MAX_BLOOM_LEVELS] = [
    "bloom_up_0",
    "bloom_up_1",
    "bloom_up_2",
    "bloom_up_3",
    "bloom_up_4",
    "bloom_up_5",
];

/// Which downsampling filter a step uses.
#[derive(Clone, Copy)]
enum Filter {
    Dual,
    Wide,
    Quarter,
}

/// Bloom's levels while it runs: the texture, a view and size per level, and each step's bind group.
struct Levels {
    /// The level each downsample writes.
    views: Vec<wgpu::TextureView>,
    /// The step writing level `i` reads the scene colour (`i == 0`) or level `i - 1`.
    down: Vec<wgpu::BindGroup>,
    /// The step adding level `i + 1` (its sum, on the compute chain) onto level `i`.
    up: Vec<wgpu::BindGroup>,
    /// The compute chain's workgroups per level; empty on the render passes.
    workgroups: Vec<(u32, u32)>,
    /// The compute chain's sums of each level and every one below it (what the upsamples write; the smallest level's
    /// is the level itself, which its downsample writes here, and `views` holds too); empty on the render passes.
    sums: Vec<wgpu::TextureView>,
    /// Whether the chain is one compute pass.
    compute: bool,
    first: Filter,
    rest: Filter,
    threshold: f32,
}

/// The first downsample's entry points: dual filter, 13 taps, from a quarter.
const FIRST_ENTRY_POINTS: [&str; 3] = ["fs_first_dual", "fs_first_wide", "fs_first_quarter"];
/// The later downsamples' entry points: dual filter, 13 taps.
const DOWN_ENTRY_POINTS: [&str; 2] = ["fs_down_dual", "fs_down_wide"];
/// The compute chain's entry points, as [`PipelineKey::BloomCompute`] numbers them: the first downsample's three
/// filters, the later downsamples' two, the upsample.
const COMPUTE_ENTRY_POINTS: [&str; 6] = [
    "cs_first_dual",
    "cs_first_wide",
    "cs_first_quarter",
    "cs_down_dual",
    "cs_down_wide",
    "cs_up",
];
/// [`COMPUTE_ENTRY_POINTS`]' upsample.
const COMPUTE_UP: u8 = 5;

/// The compute chain's shader, bind group layouts (the downsamples', and the upsample's with the level it adds onto)
/// and pipelines, where the device has compute shaders, writing levels in `format`.
struct Compute {
    format: wgpu::TextureFormat,
    module: wgpu::ShaderModule,
    down_layout: wgpu::BindGroupLayout,
    up_layout: wgpu::BindGroupLayout,
    pipelines: [Option<wgpu::ComputePipeline>; 6],
    /// The device is WARP, whose compiler makes a compute pipeline's machine code as the pipeline is made: then each
    /// compile takes its turn with every WARP device's ([`crate::context::warp_turn`]).
    warp: bool,
}

impl Compute {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, warp: bool) -> Self {
        let source = concat!(include_str!("bloom.wgsl"), include_str!("bloom_compute.wgsl"));
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("metallic bloom compute"),
            source: wgpu::ShaderSource::Wgsl(source.replace(SHADER_FORMAT, storage_format_name(format)).into()),
        });
        let compute = |entry: wgpu::BindGroupLayoutEntry| wgpu::BindGroupLayoutEntry {
            visibility: wgpu::ShaderStages::COMPUTE,
            ..entry
        };
        let storage = wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let down = [
            compute(uniform_entry(0, PARAMS_SIZE)),
            compute(texture_entry(1, true)),
            compute(sampler_entry(2)),
            storage,
        ];
        let down_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("metallic bloom down"),
            entries: &down,
        });
        let up_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("metallic bloom up"),
            entries: &[down[0], down[1], down[2], down[3], compute(texture_entry(4, false))],
        });
        Self {
            format,
            module,
            down_layout,
            up_layout,
            pipelines: Default::default(),
            warp,
        }
    }

    /// The compile of entry point `index`.
    fn recipe(&self, index: u8) -> Option<Recipe> {
        let entry = *COMPUTE_ENTRY_POINTS.get(usize::from(index))?;
        let layout = if index == COMPUTE_UP {
            self.up_layout.clone()
        } else {
            self.down_layout.clone()
        };
        let module = self.module.clone();
        let warp = self.warp;
        Some(Box::new(move |device, cache| {
            let _turn = warp.then(crate::context::warp_turn);
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            Pipeline::Compute(device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache,
            }))
        }))
    }
}

/// `format`'s name in a WGSL storage texture type.
fn storage_format_name(format: wgpu::TextureFormat) -> &'static str {
    match format {
        wgpu::TextureFormat::Rg11b10Ufloat => "rg11b10ufloat",
        _ => SHADER_FORMAT,
    }
}

/// The format of bloom's levels as compute dispatches on `context` ([`COMPUTE_FORMATS`]: the first it writes as
/// storage and filters); `None` without compute shaders or storage textures (WebGL 2, GLES 3.0), and on OpenGL.
///
/// OpenGL draws the render passes even with compute shaders: Mesa's llvmpipe (CI's OpenGL) ran the first
/// downsample's dispatch before it had rasterised the scene pass before it, so bloom blurred the frame before
/// (`tests/post.rs`, `bloom_reads_the_scene_of_its_own_frame`, failed in 29 of 30 runs there; never with the render
/// passes, nor on lavapipe). OpenGL orders a render pass's writes before a later texture fetch without a barrier, and
/// wgpu emits none there, so nothing a frame records can fix it; and OpenGL is the fallback path, whose drivers are
/// the least tested with compute.
pub(crate) fn compute_format(context: &RenderContext) -> Option<wgpu::TextureFormat> {
    let capabilities = context.capabilities();
    if !capabilities.compute
        || capabilities.backend == wgpu::Backend::Gl
        || context.device().limits().max_storage_textures_per_shader_stage == 0
    {
        return None;
    }
    COMPUTE_FORMATS.into_iter().find(|&format| {
        let features = context.format_features(format);
        features
            .allowed_usages
            .contains(wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING)
            && features.flags.contains(
                wgpu::TextureFormatFeatureFlags::STORAGE_WRITE_ONLY | wgpu::TextureFormatFeatureFlags::FILTERABLE,
            )
    })
}

/// The sizes of bloom's levels for an output of `size` as `settings` say.
fn level_sizes(size: (u32, u32), settings: &BloomSettings) -> Vec<(u32, u32)> {
    let divisor = settings.start.divisor();
    let mut level = ((size.0 / divisor).max(1), (size.1 / divisor).max(1));
    (0..settings.levels(size.1))
        .map(|_| {
            let this = level;
            level = ((level.0 / 2).max(1), (level.1 / 2).max(1));
            this
        })
        .collect()
}

/// The bytes of one texel of `format` (8 for a format without a fixed size, which bloom never uses).
fn texel_bytes(format: wgpu::TextureFormat) -> u64 {
    u64::from(format.block_copy_size(None).unwrap_or(8))
}

/// The bytes of bloom's levels, `texel` bytes a texel, for an output of `size` as `settings` say.
fn levels_bytes(size: (u32, u32), settings: &BloomSettings, texel: u64) -> u64 {
    let divisor = settings.start.divisor();
    let (mut width, mut height) = ((size.0 / divisor).max(1), (size.1 / divisor).max(1));
    let mut bytes = 0;
    for _ in 0..settings.levels(size.1) {
        bytes += u64::from(width) * u64::from(height) * texel;
        (width, height) = ((width / 2).max(1), (height / 2).max(1));
    }
    bytes
}

/// Whether the compute chain's levels, `compute` bytes a texel against the render passes' `render`, are made at once
/// for an output of `size` as the render passes' would be: no wider, or not a large texture ([`BACKGROUND_BYTES`]).
fn compute_fits(size: (u32, u32), settings: &BloomSettings, compute: u64, render: u64) -> bool {
    compute <= render || levels_bytes(size, settings, compute) < BACKGROUND_BYTES
}

/// A bind group entry for `view` at `binding`.
fn texture_at(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

/// Bloom's pipelines (compiled apart, [`crate::pipelines`]), its parameters, and its levels while it runs.
pub(crate) struct Bloom {
    module: wgpu::ShaderModule,
    layout: wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    /// The first downsample from the scene colour: dual filter, 13 taps, a quarter.
    first: [Option<wgpu::RenderPipeline>; 3],
    /// Later downsamples: dual filter, 13 taps.
    down: [Option<wgpu::RenderPipeline>; 2],
    up: Option<wgpu::RenderPipeline>,
    /// The compute chain, where the device has compute shaders.
    compute: Option<Compute>,
    /// One slot per step, [`MAX_BLOOM_LEVELS`] downsamples then as many upsamples, `stride` bytes apart.
    params: wgpu::Buffer,
    stride: u64,
    levels: Option<Levels>,
    /// The first step's parameters as last written (they follow the drawn corner of the scene colour).
    first_written: Option<[f32; 8]>,
}

impl Bloom {
    /// Bloom for levels in `format` (the scene colour's) on its render passes, and on `context`'s compute chain where
    /// it has one ([`compute_format`]), before any pipeline is compiled.
    pub(crate) fn new(context: &RenderContext, format: wgpu::TextureFormat) -> Self {
        let device = context.device();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("metallic bloom"),
            source: wgpu::ShaderSource::Wgsl(include_str!("bloom.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("metallic bloom"),
            entries: &[uniform_entry(0, PARAMS_SIZE), texture_entry(1, true), sampler_entry(2)],
        });
        let stride = u64::from(device.limits().min_uniform_buffer_offset_alignment).max(PARAMS_SIZE);
        Self {
            module,
            first: Default::default(),
            down: Default::default(),
            up: None,
            compute: compute_format(context)
                .map(|format| Compute::new(device, format, crate::context::is_warp(context.capabilities()))),
            params: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("metallic bloom"),
                size: stride * 2 * MAX_BLOOM_LEVELS as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            stride,
            layout,
            format,
            levels: None,
            first_written: None,
        }
    }

    /// Whether the compute chain's levels are wider than the render passes' (`Rgba16Float` against the scene
    /// colour's `Rg11b10Ufloat`), so may make a large texture where the render passes make none.
    fn wider(&self) -> bool {
        self.compute
            .as_ref()
            .is_some_and(|compute| texel_bytes(compute.format) > texel_bytes(self.format))
    }

    /// Whether `settings` draw the chain as compute dispatches for an output of `size`: where the device has the
    /// compute chain, unless its levels would be a large texture ([`BACKGROUND_BYTES`]) the render passes' are not.
    /// Allocates nothing (a waiting settings change asks every frame).
    fn computes(&self, settings: &BloomSettings, size: (u32, u32)) -> bool {
        self.compute
            .as_ref()
            .filter(|_| settings.compute)
            .is_some_and(|compute| compute_fits(size, settings, texel_bytes(compute.format), texel_bytes(self.format)))
    }

    /// The pipelines bloom draws with as `settings` say, for an output of `size`: the first downsample's, the later
    /// downsamples' and the upsample's.
    pub(crate) fn keys(&self, settings: &BloomSettings, size: (u32, u32)) -> [PipelineKey; 3] {
        let first = match (settings.start, settings.wide_filter) {
            (BloomStart::Quarter, _) => 2,
            (BloomStart::Half, true) => 1,
            (BloomStart::Half, false) => 0,
        };
        let wide = u8::from(settings.wide_filter);
        if self.computes(settings, size) {
            [
                PipelineKey::BloomCompute(first),
                PipelineKey::BloomCompute(3 + wide),
                PipelineKey::BloomCompute(COMPUTE_UP),
            ]
        } else {
            [
                PipelineKey::BloomFirst(first),
                PipelineKey::BloomDown(wide),
                PipelineKey::BloomUp,
            ]
        }
    }

    /// Whether `key` is one of a chain this device may draw without a settings change, so compiled ahead of any:
    /// the compute chain's where there is one; the render passes' where there is none, or where the compute chain's
    /// levels are wider (a larger window may draw them, [`Bloom::computes`]). True for a key that is not bloom's.
    pub(crate) fn prewarms(&self, key: PipelineKey) -> bool {
        match key {
            PipelineKey::BloomCompute(_) => self.compute.is_some(),
            PipelineKey::BloomFirst(_) | PipelineKey::BloomDown(_) | PipelineKey::BloomUp => {
                self.compute.is_none() || self.wider()
            }
            _ => true,
        }
    }

    /// The compile of bloom's pipeline `key`; `None` for a key that is not bloom's, or a compute one without compute
    /// shaders.
    pub(crate) fn recipe(&self, key: PipelineKey) -> Option<Recipe> {
        match key {
            PipelineKey::BloomCompute(index) => self.compute.as_ref()?.recipe(index),
            _ => self.full_screen(key).map(FullScreen::recipe),
        }
    }

    /// What bloom's render pipeline `key` is made from; `None` for a key that is not one.
    fn full_screen(&self, key: PipelineKey) -> Option<FullScreen> {
        let (entry, blend) = match key {
            PipelineKey::BloomFirst(i) => (*FIRST_ENTRY_POINTS.get(usize::from(i))?, None),
            PipelineKey::BloomDown(i) => (*DOWN_ENTRY_POINTS.get(usize::from(i))?, None),
            PipelineKey::BloomUp => (
                "fs_up",
                Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent::REPLACE,
                }),
            ),
            _ => return None,
        };
        Some(FullScreen {
            label: entry,
            layout: self.layout.clone(),
            module: self.module.clone(),
            entry_points: ("vs_bloom", entry),
            format: self.format,
            blend,
        })
    }

    /// The slot of bloom's render pipeline `key`.
    fn slot(&mut self, key: PipelineKey) -> Option<&mut Option<wgpu::RenderPipeline>> {
        match key {
            PipelineKey::BloomFirst(i) => self.first.get_mut(usize::from(i)),
            PipelineKey::BloomDown(i) => self.down.get_mut(usize::from(i)),
            PipelineKey::BloomUp => Some(&mut self.up),
            _ => None,
        }
    }

    /// Puts the compiled pipeline `key` in place.
    pub(crate) fn install(&mut self, key: PipelineKey, pipeline: Pipeline) {
        match (key, pipeline) {
            (PipelineKey::BloomCompute(i), Pipeline::Compute(pipeline)) => {
                if let Some(slot) = self
                    .compute
                    .as_mut()
                    .and_then(|compute| compute.pipelines.get_mut(usize::from(i)))
                {
                    *slot = Some(pipeline);
                }
            }
            (key, Pipeline::Render(pipeline)) => {
                if let Some(slot) = self.slot(key) {
                    *slot = Some(pipeline);
                }
            }
            (_, Pipeline::Compute(_)) => {}
        }
    }

    /// Whether bloom's pipeline `key` is compiled.
    pub(crate) fn has(&self, key: PipelineKey) -> bool {
        match key {
            PipelineKey::BloomFirst(i) => self.first.get(usize::from(i)).is_some_and(Option::is_some),
            PipelineKey::BloomDown(i) => self.down.get(usize::from(i)).is_some_and(Option::is_some),
            PipelineKey::BloomUp => self.up.is_some(),
            PipelineKey::BloomCompute(i) => self
                .compute
                .as_ref()
                .and_then(|compute| compute.pipelines.get(usize::from(i)))
                .is_some_and(Option::is_some),
            _ => false,
        }
    }

    /// Makes the levels for an output of `size`, reading the scene colour through `scene`, as `settings` say, or drops them when bloom
    /// does not run (`None`). Writes every step's parameters but the first's, which follow the drawn corner
    /// ([`Bloom::prepare`]). A settings change, never per frame.
    pub(crate) fn rebuild(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sampler: &wgpu::Sampler,
        scene: Option<&wgpu::TextureView>,
        size: (u32, u32),
        settings: Option<&BloomSettings>,
    ) {
        self.first_written = None;
        let (Some(scene), Some(settings)) = (scene, settings) else {
            self.levels = None;
            return;
        };
        let compute = self.computes(settings, size);
        let sizes = level_sizes(size, settings);
        let count = sizes.len();
        let (format, usage) = match self.compute.as_ref().filter(|_| compute) {
            Some(chain) => (chain.format, wgpu::TextureUsages::STORAGE_BINDING),
            None => (self.format, wgpu::TextureUsages::RENDER_ATTACHMENT),
        };
        let mips = |texture: &wgpu::Texture, count: usize| -> Vec<wgpu::TextureView> {
            (0..count as u32)
                .map(|mip| {
                    texture.create_view(&wgpu::TextureViewDescriptor {
                        label: Some("metallic bloom level"),
                        base_mip_level: mip,
                        mip_level_count: Some(1),
                        ..wgpu::TextureViewDescriptor::default()
                    })
                })
                .collect()
        };
        // The levels each downsample writes. The compute chain gives every level and every sum a texture of its own:
        // OpenGL (which draws the render passes now) sets the mips a view samples on the texture, not the view, and
        // llvmpipe dropped the image stores of a dispatch into a mip outside them, so no dispatch samples one mip of a
        // texture and writes another. The smallest level is its own sum, so its downsample writes it among the sums.
        let (views, sums) = if compute {
            let single = |label, size| {
                post_texture(device, label, size, 1, format, usage).create_view(&wgpu::TextureViewDescriptor::default())
            };
            let sums: Vec<_> = sizes.iter().map(|&size| single("metallic bloom sum", size)).collect();
            let mut views: Vec<_> = sizes[..count - 1]
                .iter()
                .map(|&size| single("metallic bloom level", size))
                .collect();
            views.push(sums[count - 1].clone());
            (views, sums)
        } else {
            let texture = post_texture(device, "metallic bloom", sizes[0], count as u32, format, usage);
            (mips(&texture, count), Vec::new())
        };
        let buffer = |slot: usize| wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform_slice(&self.params, slot as u64 * self.stride, PARAMS_SIZE),
        };
        let sampler_entry = wgpu::BindGroupEntry {
            binding: 2,
            resource: wgpu::BindingResource::Sampler(sampler),
        };
        let group = |layout: &wgpu::BindGroupLayout, entries: &[wgpu::BindGroupEntry<'_>]| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("metallic bloom"),
                layout,
                entries,
            })
        };
        let down_source = |i: usize| if i == 0 { scene } else { &views[i - 1] };
        // The upsample onto level `i` reads the sum of the levels below it.
        let up_source = |i: usize| &sums[i + 1];
        let (down, up) = match self.compute.as_ref().filter(|_| compute) {
            Some(chain) => (
                (0..count)
                    .map(|i| {
                        let entries = [
                            buffer(i),
                            texture_at(1, down_source(i)),
                            sampler_entry.clone(),
                            texture_at(3, &views[i]),
                        ];
                        group(&chain.down_layout, &entries)
                    })
                    .collect(),
                (0..count - 1)
                    .map(|i| {
                        let entries = [
                            buffer(MAX_BLOOM_LEVELS + i),
                            texture_at(1, up_source(i)),
                            sampler_entry.clone(),
                            texture_at(3, &sums[i]),
                            texture_at(4, &views[i]),
                        ];
                        group(&chain.up_layout, &entries)
                    })
                    .collect(),
            ),
            None => (
                (0..count)
                    .map(|i| {
                        group(
                            &self.layout,
                            &[buffer(i), texture_at(1, down_source(i)), sampler_entry.clone()],
                        )
                    })
                    .collect(),
                (0..count - 1)
                    .map(|i| {
                        let entries = [
                            buffer(MAX_BLOOM_LEVELS + i),
                            texture_at(1, &views[i + 1]),
                            sampler_entry.clone(),
                        ];
                        group(&self.layout, &entries)
                    })
                    .collect(),
            ),
        };
        let workgroups = if compute {
            sizes
                .iter()
                .map(|&(w, h)| (w.div_ceil(WORKGROUP), h.div_ceil(WORKGROUP)))
                .collect()
        } else {
            Vec::new()
        };
        // A level's steps read the whole of the level before (or after) it.
        let whole = |(w, h): (u32, u32)| {
            let (w, h) = (w as f32, h as f32);
            [1.0 / w, 1.0 / h, 1.0, 1.0, 1.0 - 0.5 / w, 1.0 - 0.5 / h, 0.0, 0.0]
        };
        for i in 1..count {
            self.write(queue, i, whole(sizes[i - 1]));
        }
        for i in 0..count.saturating_sub(1) {
            self.write(queue, MAX_BLOOM_LEVELS + i, whole(sizes[i + 1]));
        }
        self.levels = Some(Levels {
            views,
            down,
            up,
            workgroups,
            sums,
            compute,
            first: match (settings.start, settings.wide_filter) {
                (BloomStart::Quarter, _) => Filter::Quarter,
                (BloomStart::Half, true) => Filter::Wide,
                (BloomStart::Half, false) => Filter::Dual,
            },
            rest: if settings.wide_filter {
                Filter::Wide
            } else {
                Filter::Dual
            },
            threshold: settings.threshold.max(0.0),
        });
    }

    fn write(&self, queue: &wgpu::Queue, slot: usize, params: [f32; 8]) {
        queue.write_buffer(&self.params, slot as u64 * self.stride, bytemuck::cast_slice(&params));
    }

    /// How many levels bloom has (0 when it does not run).
    pub(crate) fn levels(&self) -> usize {
        self.levels.as_ref().map_or(0, |levels| levels.views.len())
    }

    /// The render passes its chain begins (0 when it does not run or runs as compute dispatches).
    pub(crate) fn render_passes(&self) -> usize {
        match self.levels.as_ref() {
            Some(levels) if !levels.compute => 2 * levels.views.len() - 1,
            _ => 0,
        }
    }

    /// The compute passes its chain begins: 1 when it runs as compute dispatches, else 0.
    pub(crate) fn compute_passes(&self) -> usize {
        usize::from(self.levels.as_ref().is_some_and(|levels| levels.compute))
    }

    /// The sum of every level's blur, at the first level's size: what the finish pass mixes in.
    pub(crate) fn output(&self) -> Option<&wgpu::TextureView> {
        self.levels
            .as_ref()
            .map(|levels| levels.sums.first().unwrap_or(&levels.views[0]))
    }

    /// Reads the top-left `render` corner of a `texture`-sized scene colour in the next frames, with the threshold
    /// scaled by `exposure`. Writes the first step's parameters only when they change.
    pub(crate) fn prepare(&mut self, queue: &wgpu::Queue, render: (u32, u32), texture: (u32, u32), exposure: f32) {
        let Some(threshold) = self.levels.as_ref().map(|levels| levels.threshold) else {
            return;
        };
        let (tw, th) = (texture.0.max(1) as f32, texture.1.max(1) as f32);
        let (rw, rh) = (render.0.min(texture.0) as f32, render.1.min(texture.1) as f32);
        // The threshold is on the exposed image: on the scene colour it is that much higher or lower.
        let threshold = threshold / exposure.max(1e-4);
        let params = [
            1.0 / tw,
            1.0 / th,
            rw / tw,
            rh / th,
            (rw - 0.5) / tw,
            (rh - 0.5) / th,
            threshold,
            threshold * 0.5,
        ];
        if self.first_written != Some(params) {
            self.write(queue, 0, params);
            self.first_written = Some(params);
        }
    }

    /// Records the chain into `encoder`: every downsample, then every upsample. `timestamps` (the chain's start and
    /// end) go on its compute pass, or its first and last render pass.
    pub(crate) fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let Some(levels) = self.levels.as_ref() else {
            return;
        };
        match self.compute.as_ref().filter(|_| levels.compute) {
            Some(compute) => Self::dispatch(compute, levels, encoder, timestamps),
            None => self.draw(levels, encoder, timestamps),
        }
    }

    /// The chain as one compute pass: a dispatch per step.
    fn dispatch(
        compute: &Compute,
        levels: &Levels,
        encoder: &mut wgpu::CommandEncoder,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let timestamp_writes = timestamps.map(|writes| wgpu::ComputePassTimestampWrites {
            query_set: writes.query_set,
            beginning_of_pass_write_index: writes.beginning_of_pass_write_index,
            end_of_pass_write_index: writes.end_of_pass_write_index,
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("bloom"),
            timestamp_writes,
        });
        let first = match levels.first {
            Filter::Dual => 0,
            Filter::Wide => 1,
            Filter::Quarter => 2,
        };
        let rest = match levels.rest {
            Filter::Wide => 4,
            Filter::Dual | Filter::Quarter => 3,
        };
        let pipelines = &compute.pipelines;
        // The renderer runs bloom only once its pipelines are compiled; the pass stays either way, so the timestamps
        // keep their places.
        let (Some(first), Some(rest), Some(up)) =
            (&pipelines[first], &pipelines[rest], &pipelines[usize::from(COMPUTE_UP)])
        else {
            return;
        };
        for (i, (group, &(x, y))) in levels.down.iter().zip(&levels.workgroups).enumerate() {
            if i <= 1 {
                pass.set_pipeline(if i == 0 { first } else { rest });
            }
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(x, y, 1);
        }
        pass.set_pipeline(up);
        for i in (0..levels.up.len()).rev() {
            let (x, y) = levels.workgroups[i];
            pass.set_bind_group(0, &levels.up[i], &[]);
            pass.dispatch_workgroups(x, y, 1);
        }
    }

    /// The chain as render passes: one per step.
    fn draw(
        &self,
        levels: &Levels,
        encoder: &mut wgpu::CommandEncoder,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let count = levels.views.len();
        let last = 2 * count - 2;
        let mut index = 0;
        let mut pass = |encoder: &mut wgpu::CommandEncoder,
                        label: &str,
                        view: &wgpu::TextureView,
                        load: wgpu::LoadOp<wgpu::Color>,
                        pipeline: Option<&wgpu::RenderPipeline>,
                        group: &wgpu::BindGroup| {
            let timestamp_writes = timestamps.as_ref().map(|writes| wgpu::RenderPassTimestampWrites {
                query_set: writes.query_set,
                beginning_of_pass_write_index: if index == 0 {
                    writes.beginning_of_pass_write_index
                } else {
                    None
                },
                end_of_pass_write_index: if index == last {
                    writes.end_of_pass_write_index
                } else {
                    None
                },
            });
            let timestamp_writes = timestamp_writes.filter(|writes| {
                writes.beginning_of_pass_write_index.is_some() || writes.end_of_pass_write_index.is_some()
            });
            index += 1;
            let colors = [Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &colors,
                depth_stencil_attachment: None,
                timestamp_writes,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // The renderer runs bloom only once its pipelines are compiled; the pass stays either way, so the
            // timestamps keep their places.
            if let Some(pipeline) = pipeline {
                render_pass.set_pipeline(pipeline);
                render_pass.set_bind_group(0, group, &[]);
                render_pass.draw(0..3, 0..1);
            }
        };
        for (i, (view, group)) in levels.views.iter().zip(&levels.down).enumerate() {
            let filter = if i == 0 { levels.first } else { levels.rest };
            let pipeline = match (i, filter) {
                (0, Filter::Dual) => &self.first[0],
                (0, Filter::Wide) => &self.first[1],
                (0, Filter::Quarter) => &self.first[2],
                (_, Filter::Wide) => &self.down[1],
                (_, Filter::Dual | Filter::Quarter) => &self.down[0],
            }
            .as_ref();
            // Every texel is written, so nothing is loaded.
            pass(
                encoder,
                DOWN_NAMES[i.min(MAX_BLOOM_LEVELS - 1)],
                view,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                pipeline,
                group,
            );
        }
        for i in (0..count - 1).rev() {
            pass(
                encoder,
                UP_NAMES[i],
                &levels.views[i],
                wgpu::LoadOp::Load,
                self.up.as_ref(),
                &levels.up[i],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_sizes_halve_from_the_start() {
        let high = BloomSettings::HIGH;
        assert_eq!(
            level_sizes((1920, 1080), &high),
            [(960, 540), (480, 270), (240, 135), (120, 67), (60, 33)]
        );
        assert_eq!(level_sizes((1280, 720), &BloomSettings::LOW).len(), 4);
        let bytes: u64 = level_sizes((1920, 1080), &high)
            .iter()
            .map(|&(w, h)| u64::from(w) * u64::from(h) * 4)
            .sum();
        assert_eq!(levels_bytes((1920, 1080), &high, 4), bytes);
    }

    #[test]
    fn wider_compute_levels_give_way_to_the_render_passes_only_where_they_would_be_large() {
        let (high, low) = (BloomSettings::HIGH, BloomSettings::LOW);
        // As narrow as the render passes' levels: the compute chain at any size.
        assert!(compute_fits((7680, 4320), &high, 4, 4));
        // Twice as wide (`Rgba16Float` against `Rg11b10Ufloat`): under 16 MiB up to 1440p and ultra-wide, not at 4K.
        assert!(compute_fits((2560, 1440), &high, 8, 4));
        assert!(compute_fits((3440, 1440), &high, 8, 4));
        assert!(!compute_fits((3840, 2160), &high, 8, 4));
        // Low starts at a quarter: a sixteenth of the texels.
        assert!(compute_fits((3840, 2160), &low, 8, 4));
    }
}
