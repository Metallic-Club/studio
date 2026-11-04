//! Bloom ([`crate::PassId::Bloom`]): a chain of downsampling passes from the scene colour into the levels of one
//! texture, then upsampling passes adding each level onto the next larger (`bloom.wgsl`). Every pass works at half
//! the output's resolution or below; the chain is timed as one.

use super::{BloomSettings, BloomStart, FullScreen, MAX_BLOOM_LEVELS, post_texture, sampler_entry};
use super::{texture_entry, uniform_entry, uniform_slice};
use crate::pipelines::PipelineKey;

/// Bytes of one pass's parameters (`BloomPass` in `bloom.wgsl`).
const PARAMS_SIZE: u64 = 32;

/// The passes' names in GPU debuggers.
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

/// Which downsampling filter a pass uses.
#[derive(Clone, Copy)]
enum Filter {
    Dual,
    Wide,
    Quarter,
}

/// Bloom's levels while it runs: the texture, a view and size per level, and each pass's bind group.
struct Levels {
    views: Vec<wgpu::TextureView>,
    /// The pass writing level `i` reads the scene colour (`i == 0`) or level `i - 1`.
    down: Vec<wgpu::BindGroup>,
    /// The pass adding level `i + 1` onto level `i`.
    up: Vec<wgpu::BindGroup>,
    first: Filter,
    rest: Filter,
    threshold: f32,
}

/// The first downsample's entry points: dual filter, 13 taps, from a quarter.
const FIRST_ENTRY_POINTS: [&str; 3] = ["fs_first_dual", "fs_first_wide", "fs_first_quarter"];
/// The later downsamples' entry points: dual filter, 13 taps.
const DOWN_ENTRY_POINTS: [&str; 2] = ["fs_down_dual", "fs_down_wide"];

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
    /// One slot per pass, [`MAX_BLOOM_LEVELS`] downsamples then as many upsamples, `stride` bytes apart.
    params: wgpu::Buffer,
    stride: u64,
    levels: Option<Levels>,
    /// The first pass's parameters as last written (they follow the drawn corner of the scene colour).
    first_written: Option<[f32; 8]>,
}

impl Bloom {
    /// Bloom for levels in `format` (the scene colour's), before any pipeline is compiled.
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
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

    /// The pipelines bloom draws with as `settings` say: the first downsample's, the later downsamples' and the
    /// upsample's.
    pub(crate) fn keys(settings: &BloomSettings) -> [PipelineKey; 3] {
        let first = match (settings.start, settings.wide_filter) {
            (BloomStart::Quarter, _) => 2,
            (BloomStart::Half, true) => 1,
            (BloomStart::Half, false) => 0,
        };
        [
            PipelineKey::BloomFirst(first),
            PipelineKey::BloomDown(u8::from(settings.wide_filter)),
            PipelineKey::BloomUp,
        ]
    }

    /// What bloom's pipeline `key` is made from; `None` for a key that is not bloom's.
    pub(crate) fn full_screen(&self, key: PipelineKey) -> Option<FullScreen> {
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

    /// The slot of bloom's pipeline `key`.
    fn slot(&mut self, key: PipelineKey) -> Option<&mut Option<wgpu::RenderPipeline>> {
        match key {
            PipelineKey::BloomFirst(i) => self.first.get_mut(usize::from(i)),
            PipelineKey::BloomDown(i) => self.down.get_mut(usize::from(i)),
            PipelineKey::BloomUp => Some(&mut self.up),
            _ => None,
        }
    }

    /// Puts the compiled pipeline `key` in place.
    pub(crate) fn install(&mut self, key: PipelineKey, pipeline: wgpu::RenderPipeline) {
        if let Some(slot) = self.slot(key) {
            *slot = Some(pipeline);
        }
    }

    /// Whether bloom's pipeline `key` is compiled.
    pub(crate) fn has(&self, key: PipelineKey) -> bool {
        let slot = match key {
            PipelineKey::BloomFirst(i) => self.first.get(usize::from(i)),
            PipelineKey::BloomDown(i) => self.down.get(usize::from(i)),
            PipelineKey::BloomUp => Some(&self.up),
            _ => None,
        };
        slot.is_some_and(Option::is_some)
    }

    /// Makes the levels for an output of `size`, reading the scene colour through `scene`, as `settings` say, or drops them when bloom
    /// does not run (`None`). Writes every pass's parameters but the first's, which follow the drawn corner
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
        let count = settings.levels(size.1);
        let divisor = settings.start.divisor();
        let mut sizes = Vec::with_capacity(count);
        let mut level = ((size.0 / divisor).max(1), (size.1 / divisor).max(1));
        for _ in 0..count {
            sizes.push(level);
            level = ((level.0 / 2).max(1), (level.1 / 2).max(1));
        }
        let texture = post_texture(device, "metallic bloom", sizes[0], count as u32, self.format);
        let views: Vec<_> = (0..count as u32)
            .map(|mip| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("metallic bloom level"),
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    ..wgpu::TextureViewDescriptor::default()
                })
            })
            .collect();
        let group = |slot: usize, source: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("metallic bloom"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform_slice(&self.params, slot as u64 * self.stride, PARAMS_SIZE),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            })
        };
        let down = (0..count)
            .map(|i| group(i, if i == 0 { scene } else { &views[i - 1] }))
            .collect();
        let up = (0..count.saturating_sub(1))
            .map(|i| group(MAX_BLOOM_LEVELS + i, &views[i + 1]))
            .collect();
        // A level's passes read the whole of the level before (or after) it.
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

    /// The render passes its chain begins (0 when it does not run).
    pub(crate) fn passes(&self) -> usize {
        (2 * self.levels()).saturating_sub(1)
    }

    /// Level 0, which ends up holding the sum of every level's blur: what the finish pass mixes in.
    pub(crate) fn output(&self) -> Option<&wgpu::TextureView> {
        self.levels.as_ref().map(|levels| &levels.views[0])
    }

    /// Reads the top-left `render` corner of a `texture`-sized scene colour in the next frames, with the threshold
    /// scaled by `exposure`. Writes the first pass's parameters only when they change.
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
    /// end) go on its first and last pass.
    pub(crate) fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let Some(levels) = self.levels.as_ref() else {
            return;
        };
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
