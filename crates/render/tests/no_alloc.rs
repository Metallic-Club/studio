//! The frame path allocates nothing of its own (ADR 0008 §9), measured with a counting global allocator on the
//! offscreen path over 100 frames after a warm-up.
//!
//! wgpu itself allocates while recording (wgpu-core boxes every pass and command buffer: about 18 allocations per
//! render pass and 20 per submit on wgpu 30), and no renderer on top of it can avoid that. So the check is relative:
//! a frame through [`Renderer::render`] allocates no more than the same wgpu calls written out by hand, and three
//! more passes cost it exactly what they cost by hand, so everything the renderer adds (planning, attachment
//! selection, the pass loop) is allocation-free. A second test checks nothing accumulates: the heap does not grow
//! over 100 frames with timing, MSAA and every pass on.
//!
//! The GPU scene (#76) is held to the same rule over 100 frames of 10,000 parts with 1% changing: its CPU side
//! (drain, repack, coalesce) allocates nothing at all, its upload no more than the same wgpu calls by hand, and the
//! heap does not grow across whole frames that sync, upload and render.
//!
//! Drawing parts (#77) too: a frame of `Renderer::render_scene` with a moving camera (so the frame uniform is
//! written and the transparent list sorted and written every frame) allocates no more than the same wgpu calls
//! by hand, and the heap does not grow over 100 such frames with 1% of the parts changing. Since #106 the
//! hand-written frame draws the see-through parts as the renderer does on the storage path: their records written
//! into the staging memory and one draw that pulls the vertices.
//!
//! The debug overlay (#78) too: a frame whose overlay is redrawn (so it is encoded and uploaded) allocates no more
//! than the same write and draw by hand, the same for 35 quads as for 1,300 (beyond the runner noise the parts
//! frames also see), and the heap does not grow over 100 such frames.
//!
//! Culling (#79) too: a cull of 10,000 parts from an orbiting camera allocates nothing at all once warm, the parts
//! frames above cull and upload their visible lists every frame (the hand-written frame makes the same staging
//! write and copies), and the heap does not grow over 100 culled frames on either instance path.
//!
//! Lighting and the sky (#81) too: the parts frames above also hand the renderer a new sun every frame and draw the
//! sky (the hand-written frame draws the same full-screen triangle), and re-reading a `Lighting` service that
//! changes every frame, through the scene's watch on its change consumer, allocates nothing at all.
//!
//! Sun shadows (#82) too: a frame that fits, culls, uploads and draws three cascades allocates no more than the same
//! wgpu calls by hand (the casters of each cascade predicted by a CPU mirror of the scene, as the camera's are), for
//! two workloads of very different size; the cascades' culls (with the camera's, from an orbiting camera) allocate
//! nothing at all once warm; and the heap does not grow over 100 frames with the default shadows on either instance
//! path. The other parts frames above draw no shadows, so they still compare with their hand-written frames.
//!
//! The cached far cascade (#121) too: with the default cascades, a frame that also draws the far cascade's static
//! map and builds its map (the copy of the static map and the moving casters on top) allocates exactly what the same
//! wgpu calls do by hand, for two workloads with different casters and moving casters.
//!
//! Render scale and dynamic resolution (#84) too: a frame drawn into the scene colour at half scale and upscaled, and
//! one drawn into the corner of larger textures (a viewport and scissor in every scene pass), add exactly what the
//! upscale pass and the viewports add by hand; and the heap does not grow while dynamic resolution moves the scale.
//! The other frames here draw at the target's own resolution, which makes the same wgpu calls as before #84.
//!
//! The post chain (#85) too: every hand-written frame that stands for a default frame draws its scene into an HDR
//! scene colour and then records the chain as the renderer does ([`HandPost`]: bloom's downsampling and upsampling
//! passes, the finish pass, with the renderer's own shaders), and the plain frame is also compared with MLAA's pass
//! added and with the scene passes merged into one. A frame without the finish pass makes the calls it made before.
//!
//! Pipelines compiled off the frame (#86) too: every renderer here waits for its background compiles before it is
//! measured, so the frames compare as before; and the default frame drawn while a settings change waits for
//! pipelines still compiling (paused, so they stay pending) allocates exactly what the same wgpu calls do by hand,
//! and keeps nothing over 100 frames. The compiles themselves run on worker threads, whose allocations these
//! per-thread counters never see.
//!
//! `DirectionalLight`s (#123) too: the lit frames' lamps hold four, drawn as extra suns through the frame uniform the
//! frames already write (no wgpu call of their own), and their sync, turning with the moving lamps, allocates nothing.
//!
//! Counters are per thread, so tests running in parallel do not see each other's allocations.
//!
//! The heap-growth tests ("nothing accumulates") sample the heap with the GPU idle and over two windows of frames,
//! so wgpu's bookkeeping for frames in flight and its pools' one-off growth do not decide them; a leak grows in
//! both windows and still fails them ([`HeapGrowth`], #151).
//!
//! Every adapter, software (`DeviceType::Cpu`: WARP, lavapipe) or hardware, is held to the same exact comparison of
//! the renderer's frames with the hand-written ones. The excess CI's runners used to show (up to 6 allocations on the
//! software adapters, the "machine noise" of #77 to #82, and 4 to 8 on macOS's Metal) came from three things in the
//! test setup, not the renderer, and each is now removed at its cause rather than allowed for (#125):
//!
//! - library backtraces, which wgpu-core's debug builds capture on every lock ([`assert_library_backtraces_off`]):
//!   a constant excess on every runner, growing with the depth of the renderer's stack;
//! - the Direct3D 12 debug layer's messages, and wgpu-hal's leaking handler for them ([`debug_output`]): WARP's
//!   run-to-run swings of up to 40;
//! - wgpu-core's per-encoder trackers growing with the order of the buffer indices a frame meets
//!   ([`pad_tracker_indices`]): the ±3 between workloads (lavapipe's 205 against 211 in #125).

// A `#[global_allocator]` needs `unsafe impl GlobalAlloc`. This test binary's allocator only counts, then forwards
// every call unchanged to the system allocator.
#![allow(unsafe_code)]

mod support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use metallic_core::glam::Vec3;
use metallic_core::{CFrame, Color3, DataModel, Id, PropId, Value};
use metallic_render::depth::{DEPTH_CLEAR, DEPTH_FORMAT};
use metallic_render::parts::MeshData;
use metallic_render::resolution::ResolutionSettings;
use metallic_render::scene::{GpuInstance, InstanceBinding, RenderComponents};
use metallic_render::{
    Camera, DrawStats, GpuScene, InstanceScene, LightingReader, MAX_LOD_LEVELS, OffscreenTarget, PassId, PassSet,
    RenderContext, Renderer, RendererSettings, SceneLighting, ShadowSettings, VisibleSet, wgpu,
};

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    static LIVE_BYTES: Cell<i64> = const { Cell::new(0) };
}

fn record(allocations: u64, bytes: i64) {
    // `try_with`: the allocator also runs while a thread's locals are being torn down.
    let _ = ALLOCATIONS.try_with(|c| c.set(c.get() + allocations));
    let _ = LIVE_BYTES.try_with(|c| c.set(c.get() + bytes));
}

fn signed(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

struct Counting;

// SAFETY: every method forwards its arguments unchanged to `System`, which upholds the `GlobalAlloc` contract;
// the counting touches only const-initialised thread-locals without destructors, which never allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(1, signed(layout.size()));
        // SAFETY: the caller's `layout` is passed through unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(1, signed(layout.size()));
        // SAFETY: the caller's `layout` is passed through unchanged.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record(0, -signed(layout.size()));
        // SAFETY: `ptr` came from `System` with this `layout`, as the caller guarantees.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record(1, signed(new_size) - signed(layout.size()));
        // SAFETY: `ptr`, `layout` and `new_size` are the caller's, passed through unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// How many allocations (and reallocations) `f` makes on this thread, and what it returned.
fn allocations_in<R>(f: impl FnOnce() -> R) -> (u64, R) {
    let before = ALLOCATIONS.with(Cell::get);
    let result = f();
    (ALLOCATIONS.with(Cell::get) - before, result)
}

fn live_bytes() -> i64 {
    LIVE_BYTES.with(Cell::get)
}

const SIZE: u32 = 256;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const WARM_UP: usize = 30;
const FRAMES: usize = 100;
/// The size of the frame uniform the parts and the sky read (`Frame` in `src/parts/frame.wgsl`).
const FRAME_UNIFORM_SIZE: u64 = 848;
/// The shadow cascades the shadowed parts frames draw, all of them every frame.
const CASCADES: usize = 3;
/// What the shadowed parts frames draw: three cascades, each holding every caster, so a sun that moves every
/// frame draws all three every frame.
const EVERY_CASCADE: ShadowSettings = ShadowSettings {
    enabled: true,
    cascades: CASCADES as u32,
    dynamic_cascades: CASCADES as u32,
    ..ShadowSettings::OFF
};
/// The default cascades: the far one cached, so its static map is drawn (every frame, the sun moving) and its map
/// built from it and the moving casters (#121).
const CACHED_CASCADE: ShadowSettings = ShadowSettings {
    enabled: true,
    cascades: CASCADES as u32,
    dynamic_cascades: CASCADES as u32 - 1,
    ..ShadowSettings::OFF
};

/// The shadows a parts frame draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShadowWork {
    Off,
    /// [`EVERY_CASCADE`].
    Every,
    /// [`CACHED_CASCADE`], with this many moving balls in the far cascade.
    Cached(usize),
}

impl ShadowWork {
    fn settings(self) -> ShadowSettings {
        match self {
            ShadowWork::Off => ShadowSettings::OFF,
            ShadowWork::Every => EVERY_CASCADE,
            ShadowWork::Cached(_) => CACHED_CASCADE,
        }
    }
}

/// The casters of each kind at each level of detail in one list.
type KindLevels = [[u32; MAX_LOD_LEVELS]; 4];

/// What a shadowed hand-written frame draws: the casters of each kind and level in each cascade's list, and with
/// [`CACHED_CASCADE`] the far cascade's moving casters of each kind and level, drawn on the copy of its static map.
#[derive(Clone, Copy, Debug)]
struct HandCasters {
    cascades: [KindLevels; CASCADES],
    moving: Option<KindLevels>,
}

/// One pass of a hand-written frame: its label, whether it clears the colour, and its depth use (`Some(true)` writes
/// and clears depth, `Some(false)` tests it read-only, `None` has no depth).
type HandPass = (&'static str, bool, Option<bool>);

/// The default frame's scene passes and UI (opaque, sky, transparent, UI), as the renderer plans them. With the
/// post chain ([`HandPost`]) the scene passes draw into its scene colour, and its passes run in UI's place: its
/// finish pass draws UI (#97).
const DEFAULT_PASSES: [HandPass; 4] = [
    ("opaque", true, Some(true)),
    ("sky", false, Some(false)),
    ("transparent", false, Some(false)),
    ("ui", false, None),
];

/// The default frame with its scene passes merged into one ([`PassId::Scene`]): depth cleared and discarded there.
const MERGED_PASSES: [HandPass; 2] = [("scene", true, Some(true)), ("ui", false, None)];

/// A UI-only frame, as the renderer plans it (UI is the first pass, so it clears).
const UI_ONLY: [HandPass; 1] = [("ui", true, None)];

/// The post chain of a hand-written frame as the renderer's defaults draw it (`src/post`): the HDR scene colour,
/// bloom's chain of passes into the levels of one texture, MLAA's pass when asked, and the finish pass into the
/// target, with the renderer's own shaders, pipelines, textures and bind groups like its own.
struct HandPost {
    scene: wgpu::TextureView,
    levels: Vec<wgpu::TextureView>,
    first: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    down_groups: Vec<wgpu::BindGroup>,
    up_groups: Vec<wgpu::BindGroup>,
    mlaa: Option<(wgpu::RenderPipeline, wgpu::BindGroup, wgpu::TextureView)>,
    finish: wgpu::RenderPipeline,
    finish_group: wgpu::BindGroup,
}

/// The renderer's names for bloom's passes (`src/post/bloom.rs`).
const BLOOM_DOWN: [&str; 6] = [
    "bloom_down_0",
    "bloom_down_1",
    "bloom_down_2",
    "bloom_down_3",
    "bloom_down_4",
    "bloom_down_5",
];
const BLOOM_UP: [&str; 6] = [
    "bloom_up_0",
    "bloom_up_1",
    "bloom_up_2",
    "bloom_up_3",
    "bloom_up_4",
    "bloom_up_5",
];

impl HandPost {
    /// A scene colour of `scene` size in the renderer's HDR format, bloom's default levels for a [`SIZE`]-square
    /// output, MLAA's weights and pass with `mlaa`, and the finish pass into [`FORMAT`].
    fn new(context: &RenderContext, scene: (u32, u32), mlaa: bool) -> Self {
        let device = context.device();
        let hdr = metallic_render::post::hdr_format(context);
        let texture = |label, (width, height): (u32, u32), mips, format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let whole = |texture: wgpu::Texture| texture.create_view(&wgpu::TextureViewDescriptor::default());
        let scene_view = whole(texture("hand scene colour", scene, 1, hdr));
        let bloom = metallic_render::PostSettings::default().bloom;
        let count = bloom.levels(SIZE);
        let bloom_texture = texture("hand bloom", (SIZE / 2, SIZE / 2), count as u32, hdr);
        let levels: Vec<_> = (0..count as u32)
            .map(|mip| {
                bloom_texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    ..wgpu::TextureViewDescriptor::default()
                })
            })
            .collect();
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        let module = |label, source: &'static str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        };
        let pipeline = |module: &wgpu::ShaderModule, (vs, fs): (&str, &str), format, blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("hand post"),
                layout: None,
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(vs),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fs),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let uniform = |size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("hand post"),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let bloom_module = module("hand bloom", include_str!("../src/post/bloom.wgsl"));
        let first = pipeline(&bloom_module, ("vs_bloom", "fs_first_wide"), hdr, None);
        let down = pipeline(&bloom_module, ("vs_bloom", "fs_down_wide"), hdr, None);
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        let up = pipeline(&bloom_module, ("vs_bloom", "fs_up"), hdr, Some(add));
        // One slot per pass in one buffer, as the renderer's.
        let stride = u64::from(device.limits().min_uniform_buffer_offset_alignment).max(32);
        let bloom_params = uniform(stride * 12);
        let group = |pipeline: &wgpu::RenderPipeline, slot: u64, source: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("hand bloom"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &bloom_params,
                            offset: slot * stride,
                            size: wgpu::BufferSize::new(32),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        };
        let down_groups = (0..count)
            .map(|i| {
                if i == 0 {
                    group(&first, 0, &scene_view)
                } else {
                    group(&down, i as u64, &levels[i - 1])
                }
            })
            .collect();
        let up_groups = (0..count - 1)
            .map(|i| group(&up, 6 + i as u64, &levels[i + 1]))
            .collect();
        let mlaa = mlaa.then(|| {
            let weights = whole(texture("hand mlaa weights", scene, 1, wgpu::TextureFormat::Rgba8Unorm));
            let module = module("hand mlaa", include_str!("../src/post/mlaa.wgsl"));
            let pipeline = pipeline(&module, ("vs_mlaa", "fs_mlaa"), wgpu::TextureFormat::Rgba8Unorm, None);
            let params = uniform(32);
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("hand mlaa"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&scene_view),
                    },
                ],
            });
            (pipeline, group, weights)
        });
        let finish_module = module("hand finish", include_str!("../src/post/finish.wgsl"));
        let entry = if mlaa.is_some() {
            "fs_finish_bloom_mlaa"
        } else {
            "fs_finish_bloom"
        };
        let finish = pipeline(&finish_module, ("vs_finish", entry), FORMAT, None);
        let finish_params = uniform(80);
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: finish_params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&scene_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&levels[0]),
            },
        ];
        if let Some((_, _, weights)) = mlaa.as_ref() {
            entries.push(wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(weights),
            });
        }
        let finish_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hand finish"),
            layout: &finish.get_bind_group_layout(0),
            entries: &entries,
        });
        Self {
            scene: scene_view,
            levels,
            first,
            down,
            up,
            down_groups,
            up_groups,
            mlaa,
            finish,
            finish_group,
        }
    }

    /// The post chain's passes after the scene, as the renderer records them: MLAA's (into the `corner` when the
    /// scene is drawn into one), bloom's downsamples and upsamples, and the finish pass over `target`.
    fn record(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, corner: Option<(u32, u32)>) {
        fn pass<'e>(
            encoder: &'e mut wgpu::CommandEncoder,
            label: &str,
            view: &wgpu::TextureView,
            load: wgpu::LoadOp<wgpu::Color>,
        ) -> wgpu::RenderPass<'e> {
            let colors = [Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &colors,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
        }
        if let Some((pipeline, group, weights)) = self.mlaa.as_ref() {
            let mut render_pass = pass(encoder, "mlaa", weights, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT));
            if let Some((width, height)) = corner {
                render_pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
                render_pass.set_scissor_rect(0, 0, width, height);
            }
            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(0, group, &[]);
            render_pass.draw(0..3, 0..1);
        }
        for (i, (view, group)) in self.levels.iter().zip(&self.down_groups).enumerate() {
            let mut render_pass = pass(encoder, BLOOM_DOWN[i], view, wgpu::LoadOp::Clear(wgpu::Color::BLACK));
            render_pass.set_pipeline(if i == 0 { &self.first } else { &self.down });
            render_pass.set_bind_group(0, group, &[]);
            render_pass.draw(0..3, 0..1);
        }
        for i in (0..self.levels.len() - 1).rev() {
            let mut render_pass = pass(encoder, BLOOM_UP[i], &self.levels[i], wgpu::LoadOp::Load);
            render_pass.set_pipeline(&self.up);
            render_pass.set_bind_group(0, &self.up_groups[i], &[]);
            render_pass.draw(0..3, 0..1);
        }
        let mut render_pass = pass(encoder, "post", target, wgpu::LoadOp::Clear(wgpu::Color::BLACK));
        render_pass.set_pipeline(&self.finish);
        render_pass.set_bind_group(0, &self.finish_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

/// `passes` as plain wgpu calls (no MSAA, no timing): the floor any renderer on wgpu pays for that frame. With
/// `post`, the passes before UI draw into its scene colour and its passes run in UI's place. A pass labelled `scene` is
/// the merged scene pass, which discards depth at its end.
fn hand_written_frame(
    device: &wgpu::Device,
    context: &RenderContext,
    target: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    passes: &[HandPass],
    post: Option<&HandPost>,
) -> wgpu::SubmissionIndex {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("metallic frame"),
    });
    for &(label, clear, depth_write) in passes {
        let view = match post {
            // The finish pass draws UI.
            Some(post) if label == "ui" => {
                post.record(&mut encoder, target, None);
                continue;
            }
            Some(post) => &post.scene,
            None => target,
        };
        let colors = [Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: if clear {
                    wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                } else {
                    wgpu::LoadOp::Load
                },
                store: wgpu::StoreOp::Store,
            },
        })];
        let depth = depth_write.map(|write| wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: write.then_some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(DEPTH_CLEAR),
                store: if label == "scene" {
                    wgpu::StoreOp::Discard
                } else {
                    wgpu::StoreOp::Store
                },
            }),
            stencil_ops: None,
        });
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &colors,
            depth_stencil_attachment: depth,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }));
    }
    context.submit(std::iter::once(encoder.finish()))
}

fn median(mut counts: Vec<u64>) -> u64 {
    counts.sort_unstable();
    counts[counts.len() / 2]
}

/// The fewest allocations of a renderer frame and of the same frame by hand over [`FRAMES`] frames each, run in
/// alternating order. wgpu frees finished submissions (allocating while it does) inside whichever call follows a
/// wait, so a fixed order would charge that to one side; the minimum is a frame without that maintenance.
fn renderer_and_floor(
    context: &metallic_render::RenderContext,
    passes: PassSet,
    hand: &[HandPass],
    target: &OffscreenTarget,
    depth: &wgpu::TextureView,
    post: Option<&HandPost>,
) -> (u64, u64) {
    frames_and_floor(context, passes, hand, (target, depth), post, false)
}

/// [`renderer_and_floor`], and with `pending` every renderer frame drawn while compiles are pending (#86): the
/// renderer is asked to drop the post chain, whose frame needs pipelines nothing compiled yet, with its background
/// compiles paused, so it keeps drawing `passes` while the switch waits.
fn frames_and_floor(
    context: &metallic_render::RenderContext,
    passes: PassSet,
    hand: &[HandPass],
    (target, depth): (&OffscreenTarget, &wgpu::TextureView),
    post: Option<&HandPost>,
    pending: bool,
) -> (u64, u64) {
    let device = context.device();
    let mut renderer = Renderer::new(
        context,
        RendererSettings {
            passes,
            gpu_timing: false,
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .expect("a renderer");
    renderer.wait_for_pipelines();
    // Where compiles run inline (OpenGL) nothing is ever pending: the plain frame is measured instead.
    let pending = pending && renderer.pipeline_stats().background;
    if pending {
        renderer.pause_pipeline_compiles(true);
        renderer.set_passes(passes.without(PassId::Post).without(PassId::Bloom));
        let stats = renderer.pipeline_stats();
        assert!(stats.switch_waiting && stats.pending > 0, "{stats:?}");
    }
    let mut pacer = Pacer::new();
    let mut ours = Vec::with_capacity(FRAMES);
    let mut floor = Vec::with_capacity(FRAMES);
    for frame in 0..WARM_UP + FRAMES {
        for renderer_turn in [frame % 2 == 0, frame % 2 != 0] {
            let (count, submission) = if renderer_turn {
                allocations_in(|| renderer.render(target.view()))
            } else {
                allocations_in(|| hand_written_frame(device, context, target.view(), depth, hand, post))
            };
            pacer.submitted(device, submission);
            if frame >= WARM_UP {
                if renderer_turn { &mut ours } else { &mut floor }.push(count);
            }
        }
    }
    if pending {
        // Still waiting: every measured frame was drawn as before the switch, with compiles pending.
        let stats = renderer.pipeline_stats();
        assert!(stats.switch_waiting && stats.pending > 0, "{stats:?}");
        assert!(renderer.plan().post());
        renderer.wait_for_pipelines();
        assert!(!renderer.plan().post() && !renderer.pipeline_stats().switch_waiting);
    }
    eprintln!(
        "{passes:?}{}: allocations per frame over {FRAMES} frames: renderer {}..{} (median {}), the same wgpu calls by \
         hand {}..{} (median {})",
        if pending { " (compiles pending)" } else { "" },
        ours.iter().min().expect("frames"),
        ours.iter().max().expect("frames"),
        median(ours.clone()),
        floor.iter().min().expect("frames"),
        floor.iter().max().expect("frames"),
        median(floor.clone())
    );
    (
        *ours.iter().min().expect("frames"),
        *floor.iter().min().expect("frames"),
    )
}

/// Prints a comparison's counts past the test harness's output capture (which only takes the print macros), so CI's
/// logs keep every run's numbers, passing or not, on every adapter (#125).
fn report(test: &str, context: &RenderContext, workloads: &[(&str, u64, u64)]) {
    use std::fmt::Write as _;
    use std::io::Write as _;
    let mut line = format!("no_alloc {test} on {:?}:", context.capabilities().device_type);
    for (name, ours, floor) in workloads {
        let _ = write!(line, " {name} renderer {ours} / by hand {floor};");
    }
    let _ = writeln!(std::io::stderr().lock(), "{line}");
}

/// A device for these tests (see [`support::gpu`]), on the vertex-buffer instance path if `force_vertex_instances`,
/// in a process whose debug output wgpu-hal never sees ([`debug_output::keep_from_wgpu`]) and whose library
/// backtraces are off ([`assert_library_backtraces_off`]).
fn measuring_gpu(force_vertex_instances: bool) -> Option<RenderContext> {
    assert_library_backtraces_off();
    debug_output::keep_from_wgpu();
    support::gpu_with(&metallic_render::ContextOptions {
        force_vertex_instances,
        ..metallic_render::ContextOptions::default()
    })
}

/// Keeps the Direct3D 12 debug layer's messages from wgpu-hal's handler for them.
///
/// The debug layer (on in debug builds, where wgpu turns on validation) prints a warning through `OutputDebugString`
/// for every clear of a texture made without a clear value, on the thread that records the clear: WARP on CI printed
/// two per parts frame and five per shadowed frame, as many for the renderer as by hand. wgpu-hal's handler for them
/// (`auxil/dxgi/exception.rs`, wgpu 30) decodes the exception's raw parameters instead of the message, so whether it
/// allocates (a 120-byte `String`, grown to 240) depends on the addresses in them, and it is registered again every
/// time a process's count of validating devices goes from zero to one, because its removal passes the function
/// instead of the handle `AddVectoredExceptionHandler` returned (#128). On CI's WARP that came to anything from 0 to
/// 50 allocations per frame, different for the renderer and the hand-written frame and from process to process: the
/// swings of up to 40 in #125 (the dev PC has no debug layer installed, so it never saw them).
///
/// Turning validation off instead removes the messages too, but the layer's checks are worth keeping. (Without it,
/// WARP's shader compiler crashed whenever two tests' devices submitted at once, until `RenderContext::submit` made
/// them take turns: #130.) So validation stays on, and a handler that runs before wgpu-hal's marks every debug-output
/// exception handled, which is what a debugger would do with it: the messages go nowhere (wgpu-hal never logged them
/// anyway), and everything else the layer checks still runs. The hand-written frames submit through
/// `RenderContext::submit` too, as the renderer does.
#[cfg(windows)]
mod debug_output {
    use windows_sys::Win32::Foundation::{DBG_PRINTEXCEPTION_C, DBG_PRINTEXCEPTION_WIDE_C};
    use windows_sys::Win32::System::Diagnostics::Debug::{AddVectoredExceptionHandler, EXCEPTION_POINTERS};

    /// `EXCEPTION_CONTINUE_EXECUTION` and `EXCEPTION_CONTINUE_SEARCH`, a vectored handler's two answers.
    const HANDLED: i32 = -1;
    const NOT_MINE: i32 = 0;

    unsafe extern "system" fn swallow(info: *mut EXCEPTION_POINTERS) -> i32 {
        // SAFETY: the system calls a vectored handler with valid exception pointers and a valid record.
        let code = unsafe { (*(*info).ExceptionRecord).ExceptionCode };
        if code == DBG_PRINTEXCEPTION_C || code == DBG_PRINTEXCEPTION_WIDE_C {
            HANDLED
        } else {
            NOT_MINE
        }
    }

    /// Puts [`swallow`] first in the process's vectored handlers, once.
    pub(crate) fn keep_from_wgpu() {
        static INSTALLED: std::sync::Once = std::sync::Once::new();
        INSTALLED.call_once(|| {
            // SAFETY: `swallow` is a valid handler for the life of the process and is never removed; `1` puts it
            // before every handler already added (wgpu-hal adds its own last).
            unsafe {
                AddVectoredExceptionHandler(1, Some(swallow));
            }
        });
    }
}

/// Only Direct3D 12 prints through debug-output exceptions.
#[cfg(not(windows))]
mod debug_output {
    pub(crate) fn keep_from_wgpu() {}
}

/// Fails unless library backtraces are off, as `.cargo/config.toml` sets them (`RUST_LIB_BACKTRACE=0`).
///
/// Debug builds of wgpu-core capture a `Backtrace` on every snatch-lock acquisition (`snatch.rs`), dozens per frame.
/// With library backtraces on (`RUST_BACKTRACE` set and `RUST_LIB_BACKTRACE` not `0`; CI's toolchain action sets
/// `RUST_BACKTRACE=short`), each capture allocates its frame list, which grows with the depth of the stack that took
/// the lock (the list's growth from 16 to 32 frames was the extra reallocation): a renderer frame, whose wgpu calls
/// sit a few functions deeper than the hand-written frame's, made 1 to 5 more allocations than the same calls by hand
/// on every runner (WARP, lavapipe, macOS's Metal) and on the dev PC once `RUST_BACKTRACE=1` was set there (#125).
/// Release builds never capture. With captures on, the comparisons here would measure stack depth, not the renderer.
fn assert_library_backtraces_off() {
    assert_ne!(
        std::backtrace::Backtrace::capture().status(),
        std::backtrace::BacktraceStatus::Captured,
        "library backtraces are on (RUST_LIB_BACKTRACE or RUST_BACKTRACE), so wgpu-core's debug lock tracing \
         allocates per lock; run with RUST_LIB_BACKTRACE=0, as .cargo/config.toml sets"
    );
}

/// Buffers made before anything else on a test's device, so every buffer a measured frame touches has a high wgpu
/// tracker index: see [`pad_tracker_indices`].
const TRACKER_PADDING: usize = 1024;

/// Makes [`TRACKER_PADDING`] tiny buffers on `context`'s fresh device, to be kept alive for the whole test.
///
/// wgpu-core gives every buffer a tracker index (the lowest free one, from a per-device allocator) and every command
/// encoder a fresh buffer tracker: vectors indexed by those indices (`BufferTracker`'s start and end states and its
/// resource list), grown by `Vec::resize` to the index of each buffer a command touches and, at the first render
/// pass, to the device's index count. How many times they grow depends on the order and size of the indices a frame
/// meets before its first pass (its copies), not on the calls: a renderer frame whose first copy hit index 3, then 8,
/// then a pass reallocated each vector three times, the same calls by hand on buffers 24 and up twice, so the two
/// differed by 3 allocations, more or less with every workload (and lavapipe's "growth with the casters" in #125 was
/// this). With the first [`TRACKER_PADDING`] indices taken, every index a frame meets is at least that, so the first
/// copy allocates each vector at more than half the device's index count, and the next growth (the first copy to a
/// newer buffer, or the first pass) doubles it past every index there is: one allocation and one reallocation each,
/// in both frames, whatever the buffers (only a frame whose first copy went to the device's newest buffer would skip
/// the reallocation; per-call histograms of both frames on every runner show each vector allocated once and
/// reallocated once). The device has a few hundred buffers at most, well under the padding.
fn pad_tracker_indices(context: &RenderContext) -> Vec<wgpu::Buffer> {
    (0..TRACKER_PADDING)
        .map(|_| {
            context.device().create_buffer(&wgpu::BufferDescriptor {
                label: Some("tracker index padding"),
                size: wgpu::COPY_BUFFER_ALIGNMENT,
                usage: wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        })
        .collect()
}

/// On lavapipe (CI's software Vulkan) a renderer frame and the identical hand-written one differed by a constant 2
/// allocations in some processes and by none in others, while the dev PC's Vulkan, DX12 and GL drivers never
/// differ. `render()` only calls wgpu, so this is state inside wgpu (which pooled objects a frame gets), not ours.
const WGPU_POOL_NOISE: u64 = 2;

#[test]
fn a_frame_allocates_nothing_beyond_wgpus_own() {
    let Some(context) = measuring_gpu(false) else { return };
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).unwrap();
    let depth = context
        .device()
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("metallic depth"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default());

    let (ours_one, floor_one) = renderer_and_floor(
        &context,
        PassSet::EMPTY.with(PassId::Ui),
        &UI_ONLY,
        &target,
        &depth,
        None,
    );
    // The frame before the post chain: the scene straight into the target.
    let direct = PassSet::default().without(PassId::Post);
    let (ours_direct, floor_direct) = renderer_and_floor(&context, direct, &DEFAULT_PASSES, &target, &depth, None);
    // The default frame: the scene into the HDR scene colour, bloom's chain and the finish pass (#85); with MLAA's
    // pass; and with the scene passes merged into one.
    let post = HandPost::new(&context, (SIZE, SIZE), false);
    let (ours, floor) = renderer_and_floor(
        &context,
        PassSet::default(),
        &DEFAULT_PASSES,
        &target,
        &depth,
        Some(&post),
    );
    let mlaa = HandPost::new(&context, (SIZE, SIZE), true);
    let (ours_mlaa, floor_mlaa) = renderer_and_floor(
        &context,
        PassSet::default().with(PassId::Mlaa),
        &DEFAULT_PASSES,
        &target,
        &depth,
        Some(&mlaa),
    );
    let (ours_merged, floor_merged) = renderer_and_floor(
        &context,
        PassSet::default().with(PassId::Scene),
        &MERGED_PASSES,
        &target,
        &depth,
        Some(&post),
    );
    // The default frame again, drawn while a settings change waits for its pipelines (#86).
    let (ours_pending, floor_pending) = frames_and_floor(
        &context,
        PassSet::default(),
        &DEFAULT_PASSES,
        (&target, &depth),
        Some(&post),
        true,
    );
    let workloads = [
        ("UI only", ours_one, floor_one),
        ("four passes", ours_direct, floor_direct),
        ("post chain", ours, floor),
        ("post chain with MLAA", ours_mlaa, floor_mlaa),
        ("merged scene", ours_merged, floor_merged),
        ("post chain, compiles pending", ours_pending, floor_pending),
    ];
    report("plain frame", &context, &workloads);
    // Per pass: what the extra passes cost the renderer is exactly what they cost by hand, so nothing in the pass
    // loop allocates. (CI's WARP once showed 2 more: the debug layer and library backtraces, see `debug_output` and
    // `assert_library_backtraces_off`.)
    for (name, ours, floor) in &workloads[1..] {
        assert!(
            ours.saturating_sub(ours_one) <= floor.saturating_sub(floor_one),
            "{name}: its passes cost the renderer {} allocations and the same wgpu calls {} on {:?}",
            ours.saturating_sub(ours_one),
            floor.saturating_sub(floor_one),
            context.capabilities().device_type
        );
        // Per frame: nothing beyond wgpu's own, give or take its pooling.
        assert!(
            *ours <= floor + WGPU_POOL_NOISE,
            "{name}: the renderer adds {} allocation(s) per frame",
            ours - floor
        );
    }
}

#[test]
fn nothing_accumulates_over_frames() {
    let Some(context) = measuring_gpu(false) else { return };
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).unwrap();
    // Every pass (MLAA, bloom, the finish pass, the scene passes merged), with MSAA as well.
    let mut renderer = Renderer::new(
        &context,
        RendererSettings {
            passes: PassSet::ALL,
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .unwrap();
    renderer.wait_for_pipelines();
    let samples = renderer
        .msaa_sample_counts()
        .iter()
        .filter(|&n| n > 1)
        .min()
        .unwrap_or(1);
    renderer.set_msaa_samples(samples).unwrap();
    let mut pacer = Pacer::new();
    // Room for every frame's count, so recording them allocates nothing in the measured frames.
    let mut allocations = Vec::with_capacity(4 * (WARM_UP + FRAMES));
    let mut frame = |_| {
        let (count, submission) = allocations_in(|| renderer.render(target.view()));
        allocations.push(count);
        pacer.submitted(context.device(), submission);
    };
    (0..WARM_UP).for_each(&mut frame);
    let growth = HeapGrowth::measure(context.device(), WARM_UP, frame);
    let measured = &allocations[WARM_UP..];
    eprintln!(
        "frames (every pass, {samples}x MSAA, timing {}): {growth}, allocations per frame {}..{} (median {})",
        renderer.gpu_timing_enabled(),
        measured.iter().min().unwrap(),
        measured.iter().max().unwrap(),
        median(measured.to_vec())
    );
    assert!(renderer.gpu_timings().is_some() || !renderer.gpu_timing_enabled());
    growth.assert_nothing_accumulates();
}

/// The heap check still catches a leak (#151): default frames that also grow a `Vec` by 256 bytes each (doubling its
/// buffer as it fills, so the heap grows in steps) accumulate, and so do frames that each keep a 256-byte box (so it
/// grows evenly). Every heap-growth test here fails the same way with [`LEAK_VAR`] set to 256.
#[test]
fn a_leak_fails_the_heap_check() {
    let Some(context) = measuring_gpu(false) else { return };
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).unwrap();
    let mut renderer = Renderer::new(&context, RendererSettings::new(FORMAT, SIZE, SIZE)).unwrap();
    renderer.wait_for_pipelines();
    let mut pacer = Pacer::new();
    let mut frame = |_| {
        let submission = renderer.render(target.view());
        pacer.submitted(context.device(), submission);
    };
    (0..WARM_UP).for_each(&mut frame);
    let growing = HeapGrowth::measure_leaking(context.device(), WARM_UP, 256, &mut frame);
    // After the warm-up and both windows above; room for every box, so only the boxes grow the heap.
    let first = 2 * (WARM_UP + FRAMES);
    let mut kept = Vec::with_capacity(first + 2 * FRAMES);
    let boxed = HeapGrowth::measure_leaking(context.device(), first, 0, |n| {
        frame(n);
        kept.push(Box::new([0u8; 256]));
    });
    eprintln!("a growing Vec: {growing}; a box per frame: {boxed}");
    assert!(
        growing.accumulates(),
        "a Vec growing 256 bytes per frame went unseen: {growing}"
    );
    assert!(boxed.accumulates(), "a box kept per frame went unseen: {boxed}");
}

/// Frames drawn while compiles are pending (#86) keep nothing either: every frame looks for finished compiles, and
/// finding none must leave nothing behind.
#[test]
fn nothing_accumulates_while_compiles_are_pending() {
    let Some(context) = measuring_gpu(false) else { return };
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).unwrap();
    let mut renderer = Renderer::new(&context, RendererSettings::new(FORMAT, SIZE, SIZE)).unwrap();
    renderer.wait_for_pipelines();
    if !renderer.pipeline_stats().background {
        // OpenGL compiles inline: nothing is ever pending.
        return;
    }
    renderer.pause_pipeline_compiles(true);
    renderer.set_passes(PassSet::default().without(PassId::Post).without(PassId::Bloom));
    assert!(renderer.pipeline_stats().switch_waiting);
    let mut pacer = Pacer::new();
    let mut frame = |_| {
        let submission = renderer.render(target.view());
        pacer.submitted(context.device(), submission);
    };
    (0..WARM_UP).for_each(&mut frame);
    let growth = HeapGrowth::measure(context.device(), WARM_UP, frame);
    let stats = renderer.pipeline_stats();
    eprintln!("frames with {} compiles pending: {growth}", stats.pending);
    assert!(stats.switch_waiting && stats.pending > 0, "{stats:?}");
    growth.assert_nothing_accumulates();
}

/// A hand-written scaled frame: the default passes with the scene drawn into a scene colour of its own size (or into
/// the corner of a larger one) and the post chain, whose finish pass upscales it over the target (#85; before the
/// post chain an upscale pass of its own did).
struct HandScaled {
    post: HandPost,
    depth: wgpu::TextureView,
}

impl HandScaled {
    /// Scene textures of `width` × `height`.
    fn new(context: &RenderContext, (width, height): (u32, u32)) -> Self {
        let depth = context
            .device()
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("hand depth"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            post: HandPost::new(context, (width, height), false),
            depth,
        }
    }

    /// The renderer's default passes drawn into the scene colour (into its top-left `corner` when given: a viewport
    /// and scissor in each), then the post chain over `target`, whose finish pass draws UI: the wgpu calls `Renderer::render`
    /// makes with a render scale.
    fn frame(
        &self,
        device: &wgpu::Device,
        context: &RenderContext,
        target: &wgpu::TextureView,
        corner: Option<(u32, u32)>,
    ) -> wgpu::SubmissionIndex {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("metallic frame"),
        });
        for &(label, clear, depth_write) in &DEFAULT_PASSES[..3] {
            let colors = [Some(wgpu::RenderPassColorAttachment {
                view: &self.post.scene,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })];
            let depth = depth_write.map(|write| wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth,
                depth_ops: write.then_some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(DEPTH_CLEAR),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &colors,
                depth_stencil_attachment: depth,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some((width, height)) = corner {
                pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
                pass.set_scissor_rect(0, 0, width, height);
            }
        }
        // The finish pass draws UI.
        self.post.record(&mut encoder, target, corner);
        context.submit(std::iter::once(encoder.finish()))
    }
}

/// The fewest allocations of a frame of `Renderer::render` at `resolution` and of the same wgpu calls by hand
/// ([`HandScaled::frame`], its scene textures `texture` in size, drawn into `corner`), over [`FRAMES`] frames each in
/// alternating order.
fn scaled_frame_and_floor(
    context: &RenderContext,
    resolution: ResolutionSettings,
    texture: (u32, u32),
    corner: Option<(u32, u32)>,
) -> (u64, u64) {
    let device = context.device();
    let target = OffscreenTarget::new(context, SIZE, SIZE, FORMAT).expect("a target");
    let mut renderer = Renderer::new(
        context,
        RendererSettings {
            gpu_timing: false,
            resolution,
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .expect("a renderer");
    renderer.wait_for_pipelines();
    assert!(renderer.plan().scaled());
    let hand = HandScaled::new(context, texture);
    let mut pacer = Pacer::new();
    let mut ours = Vec::with_capacity(FRAMES);
    let mut floor = Vec::with_capacity(FRAMES);
    for frame in 0..WARM_UP + FRAMES {
        for renderer_turn in [frame % 2 == 0, frame % 2 != 0] {
            let (count, submission) = if renderer_turn {
                allocations_in(|| renderer.render(target.view()))
            } else {
                allocations_in(|| hand.frame(device, context, target.view(), corner))
            };
            pacer.submitted(device, submission);
            if frame >= WARM_UP {
                if renderer_turn { &mut ours } else { &mut floor }.push(count);
            }
        }
    }
    let stats = renderer.resolution_stats();
    assert_eq!(
        corner.unwrap_or(texture),
        (stats.width, stats.height),
        "the frame was drawn at another size"
    );
    (
        *ours.iter().min().expect("frames"),
        *floor.iter().min().expect("frames"),
    )
}

/// Render scale (#84): a frame that draws the scene into a smaller scene colour, and one that draws into the corner
/// of larger textures (dynamic resolution below its highest scale: a viewport and scissor in every scene pass), cost
/// the renderer exactly what the other textures and the viewports cost by hand over the same frame at the target's
/// own resolution. With the post chain (#85) the finish pass upscales, so the frames have the native frame's passes.
#[test]
fn a_scaled_frame_allocates_nothing_beyond_wgpus_own() {
    let Some(context) = measuring_gpu(false) else { return };
    let _padding = pad_tracker_indices(&context);
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).unwrap();
    let native = HandScaled::new(&context, (SIZE, SIZE));
    let (ours_native, floor_native) = renderer_and_floor(
        &context,
        PassSet::default(),
        &DEFAULT_PASSES,
        &target,
        &native.depth,
        Some(&native.post),
    );
    let half = SIZE / 2;
    let corner = ResolutionSettings {
        scale: 0.5,
        ..ResolutionSettings::dynamic(1000.0, 0.5, 1.0)
    };
    let (ours_half, floor_half) = scaled_frame_and_floor(&context, ResolutionSettings::fixed(0.5), (half, half), None);
    let (ours_corner, floor_corner) = scaled_frame_and_floor(&context, corner, (SIZE, SIZE), Some((half, half)));
    report(
        "scaled frame",
        &context,
        &[
            ("native", ours_native, floor_native),
            ("half scale", ours_half, floor_half),
            ("half scale in the corner", ours_corner, floor_corner),
        ],
    );
    for (name, ours, floor) in [
        ("half scale", ours_half, floor_half),
        ("half scale in the corner", ours_corner, floor_corner),
    ] {
        // What the scaled frame adds to the native one costs the renderer exactly what it costs by hand...
        assert_eq!(
            ours.saturating_sub(ours_native),
            floor.saturating_sub(floor_native),
            "{name}: the scaled frame costs the renderer {ours} against {ours_native} at native, and by hand {floor} \
             against {floor_native}, on {:?}",
            context.capabilities().device_type
        );
        // ...and the frame nothing beyond wgpu's own, give or take its pooling.
        assert!(
            ours <= floor + WGPU_POOL_NOISE,
            "{name}: {ours} against {floor} by hand"
        );
    }
}

/// Dynamic resolution (#84) moving the scale from the GPU timings for 100 frames (a target no frame meets, so it
/// steps down to its lowest scale, rewriting the upscale's parameters on the way) does not grow the heap.
#[test]
fn nothing_accumulates_over_dynamic_resolution_frames() {
    let Some(context) = measuring_gpu(false) else { return };
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).unwrap();
    let mut renderer = Renderer::new(
        &context,
        RendererSettings {
            resolution: ResolutionSettings::dynamic(0.0001, 0.5, 1.0),
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .unwrap();
    renderer.wait_for_pipelines();
    let mut pacer = Pacer::new();
    let mut scales = Vec::with_capacity(4 * (WARM_UP + FRAMES));
    let mut frame = |_| {
        let submission = renderer.render(target.view());
        pacer.submitted(context.device(), submission);
        scales.push(renderer.resolution_stats().scale);
    };
    (0..WARM_UP).for_each(&mut frame);
    let growth = HeapGrowth::measure(context.device(), WARM_UP, frame);
    eprintln!(
        "dynamic resolution frames: {growth}, the scale from {:?} to {:?}",
        scales.first(),
        scales.last()
    );
    if renderer.gpu_timing_enabled() {
        assert_eq!(
            *scales.last().unwrap(),
            0.5,
            "no frame meets the target, so the lowest scale"
        );
        assert!(
            scales.windows(2).filter(|w| w[0] != w[1]).count() > 3,
            "the scale stepped"
        );
    }
    growth.assert_nothing_accumulates();
}

/// Waits (outside the frames) until at most two frames are in flight, as a window with
/// `desired_maximum_frame_latency = 2` does. Offscreen nothing else stops the CPU running ahead of the GPU, and wgpu
/// keeps every in-flight frame's command buffers (and grows its pools to match), which the heap check would see.
struct Pacer {
    in_flight: [Option<wgpu::SubmissionIndex>; 2],
    next: usize,
}

impl Pacer {
    fn new() -> Self {
        Self {
            in_flight: [None, None],
            next: 0,
        }
    }

    fn submitted(&mut self, device: &wgpu::Device, submission: wgpu::SubmissionIndex) {
        if let Some(oldest) = self.in_flight[self.next].replace(submission) {
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(oldest),
                    timeout: None,
                })
                .expect("wait for an earlier frame");
        }
        self.next = (self.next + 1) % self.in_flight.len();
    }
}

/// Waits (outside the frames) until the GPU has finished every frame submitted on `device`, and wgpu has retired them.
fn wait_for_idle(device: &wgpu::Device) {
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .expect("wait for every frame");
}

/// What a measured frame may keep on average before [`HeapGrowth`] calls it accumulating: 16,000 bytes over 100
/// frames, the limit these tests have always had.
const KEPT_PER_FRAME: i64 = 160;

/// Set to a number of bytes, every frame [`HeapGrowth::measure`] measures also grows a `Vec` by that many, kept to
/// the end: a deliberate leak, to see that every heap-growth test here fails with one (#151). Unset, nothing leaks.
const LEAK_VAR: &str = "METALLIC_TEST_LEAK_PER_FRAME";

/// How much the heap grew over two consecutive windows of frames, each measured with the GPU idle at both ends.
///
/// A single window of 100 frames, measured with frames in flight, failed at random: on lavapipe (19,248 and 19,392
/// bytes against 16 KiB) and on the dev PC's OpenGL (58 KB on every run), Vulkan and Direct3D 12 (±9 KB) (#151). Two
/// things grow there, and neither accumulates:
///
/// - **wgpu's bookkeeping for frames in flight.** wgpu keeps a submitted frame's command buffers until the frame is
///   retired: on OpenGL wgpu-hal's recorded command list (58,144 bytes for the dynamic resolution frame), elsewhere
///   its submission's bookkeeping (8 to 12 KB on the dev PC's Vulkan and Direct3D 12). With two frames in flight
///   ([`Pacer`]), a sample taken with one more or one fewer frame in flight than the other showed that much growth
///   or shrinkage. Waiting for the GPU to go idle before both samples leaves none in flight at
///   either, and that alone took OpenGL to within 24 bytes of zero.
/// - **High-water marks**, which grow in steps the first time a frame needs more than every frame before it:
///   gpu-allocator's map of a memory block's free-list chunks (5,184 bytes when it doubles, 72 when a smaller one
///   does), which `Queue::write_buffer`'s staging buffers fragment; wgpu-core's pool of command encoders (6,624
///   bytes for a Direct3D 12 frame's two), which grows when the GPU falls a frame further behind; and the scratch
///   lists each pooled Vulkan encoder keeps for render passes and barriers (10,288 and 1,152 bytes on lavapipe), which
///   grow the first time that encoder records the largest frame. Which encoder records which frame, how far behind
///   the GPU falls, and so in which frame a step comes, depend on the GPU's timing: up to 16,368 bytes in one window
///   on the dev PC's Vulkan, still 300 frames in, and the 19,248 and 19,392 bytes of lavapipe's failures came back
///   in one window with the GPU idle at both samples. They stop growing, but not at a frame a test could pick in
///   advance, and letting the warm-up run further ahead of the GPU (on the dev PC's Direct3D 12) did not make them
///   stop sooner.
///
/// A leak grows in every window and a step lands in one, so this calls the frames accumulating only when the heap
/// grew in both windows by more than [`KEPT_PER_FRAME`] per frame. The second window is as long as every frame
/// before it, so a buffer that keeps doubling (whose growth comes in steps twice as far apart each time) also grows
/// in both.
struct HeapGrowth {
    /// The frames in each window and how many bytes the heap grew over them. The second window is measured only if
    /// the heap grew too much in the first: otherwise it could not change the answer.
    windows: [Option<(usize, i64)>; 2],
}

impl HeapGrowth {
    /// Measures `frame`, which draws, submits and paces one frame, numbered from `first` on (the frames before it
    /// warmed up). A deliberate leak is added if [`LEAK_VAR`] asks for one.
    fn measure(device: &wgpu::Device, first: usize, frame: impl FnMut(usize)) -> Self {
        let leak = std::env::var(LEAK_VAR)
            .ok()
            .map(|bytes| bytes.parse().expect("a number of bytes"))
            .unwrap_or(0);
        Self::measure_leaking(device, first, leak, frame)
    }

    /// [`HeapGrowth::measure`] with `leak` bytes added to a `Vec` after every frame.
    fn measure_leaking(device: &wgpu::Device, first: usize, leak: usize, mut frame: impl FnMut(usize)) -> Self {
        let mut leaked: Vec<u8> = Vec::new();
        let mut next = first;
        let mut growth = Self { windows: [None; 2] };
        for (window, frames) in [FRAMES, first + FRAMES].into_iter().enumerate() {
            if window > 0 && !growth.accumulates() {
                break;
            }
            wait_for_idle(device);
            let before = live_bytes();
            for _ in 0..frames {
                frame(next);
                leaked.resize(leaked.len() + leak, 0);
                next += 1;
            }
            wait_for_idle(device);
            growth.windows[window] = Some((frames, live_bytes() - before));
        }
        drop(leaked);
        growth
    }

    /// Whether the heap grew in every window measured so far by more than [`KEPT_PER_FRAME`] per frame.
    fn accumulates(&self) -> bool {
        self.windows
            .iter()
            .flatten()
            .all(|&(frames, grown)| grown > KEPT_PER_FRAME * signed(frames))
    }

    fn assert_nothing_accumulates(&self) {
        assert!(!self.accumulates(), "the heap grew in both windows: {self}");
    }
}

impl std::fmt::Display for HeapGrowth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [first, second] = self.windows;
        if let Some((frames, grown)) = first {
            write!(f, "heap grew {grown} bytes over {frames} frames")?;
        }
        if let Some((frames, grown)) = second {
            write!(f, ", then {grown} over {frames}")?;
        }
        Ok(())
    }
}

const SCENE_PARTS: usize = 10_000;
/// 1% of the parts change every frame: half of them move (the same ones every frame, as moving parts do, so they
/// are promoted to the dynamic region), half are recoloured (spread over the whole scene).
const SCENE_CHANGES: usize = SCENE_PARTS / 100;

/// A game with [`SCENE_PARTS`] parts in `Workspace`.
struct SceneGame {
    dm: DataModel,
    workspace: Id,
    parts: Vec<Id>,
    cframe: PropId,
    color: PropId,
}

impl SceneGame {
    fn new() -> Self {
        let mut dm = metallic_classes::new_game().expect("a game");
        let registry = dm.registry().clone();
        let workspace = dm
            .service(registry.class_id("Workspace").expect("Workspace"))
            .expect("a Workspace");
        let class = registry.class_id("Part").expect("Part");
        let cframe = registry.prop_id(class, "CFrame").expect("CFrame");
        let color = registry.prop_id(class, "Color").expect("Color");
        let parts = (0..SCENE_PARTS)
            .map(|i| {
                let id = dm.create(class).expect("a part");
                // Every part gets its own CFrame and Color, so the edits below overwrite stored values and the data
                // model does not grow while the heap is watched.
                let place = CFrame::new(Vec3::new((i % 100) as f32 * 4.0, 1.0, (i / 100) as f32 * 4.0));
                dm.set(id, cframe, Value::CFrame(place)).expect("placed");
                dm.set(id, color, Value::Color3(Color3::new(0.1, 0.2, 0.3)))
                    .expect("coloured");
                dm.set_parent(id, Some(workspace)).expect("parented");
                id
            })
            .collect();
        Self {
            dm,
            workspace,
            parts,
            cframe,
            color,
        }
    }

    /// Frame `frame`'s changes, made outside any measurement.
    fn edit(&mut self, frame: usize) {
        let step = 1.0 + frame as f32 * 0.01;
        for i in 0..SCENE_CHANGES / 2 {
            let id = self.parts[i * 97];
            let place = CFrame::new(Vec3::new(i as f32, step, 0.0));
            self.dm.set(id, self.cframe, Value::CFrame(place)).expect("moved");
        }
        let shade = (frame % 200) as f32 / 255.0;
        for i in 0..SCENE_CHANGES / 2 {
            // A different spread of parts every frame.
            let id = self.parts[(i * 7919 + frame * 31) % SCENE_PARTS];
            let colour = Color3::new(shade, 0.5, i as f32 / SCENE_CHANGES as f32);
            self.dm.set(id, self.color, Value::Color3(colour)).expect("recoloured");
        }
    }
}

#[test]
fn a_scene_sync_allocates_nothing_once_warm() {
    let mut game = SceneGame::new();
    let mut scene = InstanceScene::attach(&mut game.dm, Some(game.workspace), &RenderComponents::builtin()).unwrap();
    let mut allocations = 0;
    let mut grown = 0;
    for frame in 0..WARM_UP + FRAMES {
        game.edit(frame);
        let before = live_bytes();
        let (count, ()) = allocations_in(|| {
            scene.sync(&mut game.dm);
            scene.coalesce();
        });
        if frame >= WARM_UP {
            allocations += count;
            grown += live_bytes() - before;
        }
    }
    let dynamic = scene.live(metallic_render::scene::Region::Dynamic);
    eprintln!(
        "{FRAMES} scene syncs of {SCENE_PARTS} parts with {SCENE_CHANGES} changing: {allocations} allocations, heap grew \
         {grown} bytes; {dynamic} dynamic, {} upload ranges in the last frame",
        scene.uploads().len()
    );
    assert_eq!(dynamic, SCENE_CHANGES as u32 / 2);
    assert_eq!(allocations, 0, "the scene sync allocated");
    assert_eq!(grown, 0, "the heap grew");
}

/// `Lighting` changing every frame (#81), as a script animating `ClockTime` makes it: the scene's sync reports the
/// watched service among the parts' changes, and reading it again (its computed `SunDirection` included) allocates
/// nothing. Handing it to the renderer is part of the parts frames below.
#[test]
fn reading_changed_lighting_allocates_nothing() {
    let mut game = SceneGame::new();
    let lighting = game
        .dm
        .service(game.dm.registry().class_id("Lighting").expect("Lighting"))
        .expect("a Lighting");
    let clock = game.dm.prop_id(lighting, "ClockTime").expect("ClockTime");
    let mut scene = InstanceScene::attach(&mut game.dm, Some(game.workspace), &RenderComponents::builtin()).unwrap();
    scene.watch(lighting);
    let mut reader = LightingReader::new();
    let mut lit = SceneLighting::default();
    let (mut allocations, mut reads, mut grown) = (0, 0, 0);
    for frame in 0..WARM_UP + FRAMES {
        game.edit(frame);
        let time = 6.0 + (frame as f64 * 0.1) % 12.0;
        game.dm.set(lighting, clock, Value::Number(time)).expect("ClockTime");
        let before = live_bytes();
        let (count, ()) = allocations_in(|| {
            let stats = scene.sync(&mut game.dm);
            scene.coalesce();
            if stats.watched > 0 {
                lit = reader.read(&game.dm, lighting);
                reads += 1;
            }
        });
        if frame >= WARM_UP {
            allocations += count;
            grown += live_bytes() - before;
        }
    }
    eprintln!("{FRAMES} syncs re-reading Lighting: {allocations} allocations, heap grew {grown} bytes; last {lit:?}");
    assert_eq!(reads, WARM_UP + FRAMES, "Lighting was not reported every frame");
    assert_eq!(allocations, 0, "re-reading Lighting allocated");
    assert_eq!(grown, 0, "the heap grew");
}

/// The lighting of frame `frame`: the sun moving through the day, so every frame resolves new lighting.
fn moving_sun(frame: usize) -> SceneLighting {
    let angle = frame as f32 * 0.02;
    SceneLighting {
        sun_direction: Vec3::new(angle.cos(), angle.sin().abs() + 0.05, 0.3),
        ..SceneLighting::default()
    }
}

/// The wgpu objects a hand-written upload uses: a staging buffer, two region buffers, and the scatter pipeline
/// built from the scene's own shader.
struct HandUpload {
    staging: wgpu::Buffer,
    targets: [wgpu::Buffer; 2],
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
    data: Vec<u8>,
}

impl HandUpload {
    fn new(device: &wgpu::Device) -> Self {
        let size = 2 * SCENE_PARTS as u64 * GpuInstance::SIZE;
        let buffer = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let staging = buffer("hand staging");
        let targets = [buffer("hand static"), buffer("hand dynamic")];
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hand scatter"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../src/scene/scatter.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("hand scatter"),
            layout: None,
            module: &module,
            entry_point: Some("scatter"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hand scatter"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: staging.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: targets[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: targets[1].as_entire_binding(),
                },
            ],
        });
        Self {
            staging,
            targets,
            pipeline,
            bind_group,
            data: vec![0; size as usize],
        }
    }

    /// One staging write of `bytes`, then either a copy per range (target index, offset, size) or one scatter
    /// pass over `instances`: the calls [`GpuScene::upload`] makes.
    fn upload(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        bytes: u64,
        ranges: &[(usize, u64, u64)],
        scatter: Option<u32>,
    ) {
        let Some(size) = wgpu::BufferSize::new(bytes) else {
            return;
        };
        let mut view = queue.write_buffer_with(&self.staging, 0, size).expect("a staging view");
        view.slice(..).copy_from_slice(&self.data[..bytes as usize]);
        drop(view);
        if let Some(instances) = scatter {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("metallic instance scatter"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.dispatch_workgroups(instances.div_ceil(64), 1, 1);
            return;
        }
        let mut source = 0;
        for &(target, offset, size) in ranges {
            encoder.copy_buffer_to_buffer(&self.staging, source, &self.targets[target], offset, size);
            source += size;
        }
    }
}

fn scene_on_gpu(context: &RenderContext, game: &mut SceneGame) -> GpuScene {
    let mut scene = GpuScene::new(
        context,
        &mut game.dm,
        Some(game.workspace),
        &RenderComponents::builtin(),
    )
    .expect("a scene");
    // The first frame places all 10,000 parts (and grows the buffers): not a steady frame, done outside.
    scene.sync(&mut game.dm);
    let mut encoder = context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    scene.upload(&mut encoder);
    context.submit(Some(encoder.finish()));
    scene
}

/// The fewest and the median allocations of a scene frame (sync and upload) and of the same wgpu calls by hand,
/// over [`FRAMES`] frames, with the scatter pass or with copies. `None` when the device cannot scatter.
fn scene_upload_and_floor(context: &RenderContext, scatter: bool) -> Option<((u64, u64), (u64, u64))> {
    let (device, queue) = (context.device(), context.queue());
    let mut game = SceneGame::new();
    let mut scene = scene_on_gpu(context, &mut game);
    if scatter && !scene.can_scatter() {
        return None;
    }
    scene.set_scatter_min_ranges(if scatter { 0 } else { u32::MAX });
    let hand = HandUpload::new(device);
    let mut ranges: Vec<(usize, u64, u64)> = Vec::with_capacity(SCENE_PARTS);
    let mut pacer = Pacer::new();
    let (mut ours, mut floor) = (Vec::with_capacity(FRAMES), Vec::with_capacity(FRAMES));
    for frame in 0..WARM_UP + FRAMES {
        game.edit(frame);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let mut by_hand = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        // The first staging write after a submission costs wgpu two more allocations (whoever makes it), so both
        // measured uploads below come after one, made here outside the measurements.
        let mut primer = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        hand.upload(queue, &mut primer, 64, &[(0, 0, 64)], None);
        drop(primer);
        let (count, stats) = allocations_in(|| {
            scene.sync(&mut game.dm);
            scene.upload(&mut encoder)
        });
        assert_eq!(stats.dispatches, u32::from(scatter));
        ranges.clear();
        ranges.extend(scene.scene().uploads().iter().map(|r| {
            (
                r.region.index(),
                u64::from(r.first) * GpuInstance::SIZE,
                u64::from(r.count) * GpuInstance::SIZE,
            )
        }));
        let instances = scatter.then_some(stats.slots);
        let (floor_count, ()) = allocations_in(|| hand.upload(queue, &mut by_hand, stats.bytes, &ranges, instances));
        pacer.submitted(device, context.submit([encoder.finish(), by_hand.finish()]));
        if frame >= WARM_UP {
            ours.push(count);
            floor.push(floor_count);
        }
    }
    let mins = (
        *ours.iter().min().expect("frames"),
        *floor.iter().min().expect("frames"),
    );
    let medians = (median(ours), median(floor));
    eprintln!(
        "scene sync + upload ({}): {} allocations at least (median {}); the same wgpu calls by hand: {} (median {})",
        if scatter { "scatter" } else { "copies" },
        mins.0,
        medians.0,
        mins.1,
        medians.1
    );
    Some((mins, medians))
}

#[test]
fn a_scene_upload_allocates_nothing_beyond_wgpus_own() {
    let Some(context) = measuring_gpu(false) else { return };
    for scatter in [false, true] {
        let Some(((ours_min, floor_min), (ours_median, floor_median))) = scene_upload_and_floor(&context, scatter)
        else {
            eprintln!("no scatter pass on this device");
            continue;
        };
        assert!(
            ours_min <= floor_min + WGPU_POOL_NOISE,
            "the scene adds {} allocation(s)",
            ours_min - floor_min
        );
        assert!(ours_median <= floor_median + WGPU_POOL_NOISE);
    }
}

#[test]
fn nothing_accumulates_over_scene_frames() {
    let Some(context) = measuring_gpu(false) else { return };
    let mut game = SceneGame::new();
    let mut scene = scene_on_gpu(&context, &mut game);
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).unwrap();
    let mut renderer = Renderer::new(&context, RendererSettings::new(FORMAT, SIZE, SIZE)).unwrap();
    renderer.wait_for_pipelines();
    let mut pacer = Pacer::new();
    let mut frame = |frame| {
        game.edit(frame);
        scene.sync(&mut game.dm);
        let submission = renderer.render_with(target.view(), |encoder| {
            scene.upload(encoder);
        });
        pacer.submitted(context.device(), submission);
    };
    (0..WARM_UP).for_each(&mut frame);
    let growth = HeapGrowth::measure(context.device(), WARM_UP, frame);
    eprintln!("frames of sync, upload and render: {growth}");
    growth.assert_nothing_accumulates();
}

/// The camera of frame `frame`: circling the grid, so every frame writes the frame uniform and sorts the
/// transparent parts again.
fn orbiting_camera(frame: usize) -> Camera {
    let angle = frame as f32 * 0.05;
    let eye = Vec3::new(angle.cos() * 120.0, 80.0, angle.sin() * 120.0);
    Camera::from_cframe(CFrame::look_at(eye, Vec3::ZERO, Vec3::Y), 70.0)
}

/// The wgpu objects a hand-written parts frame uses: the renderer's own shader on the storage path, with
/// pipelines, bind groups and buffers like the renderer's.
struct HandParts {
    opaque: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
    /// The renderer's sky shader, and its frame group.
    sky: wgpu::RenderPipeline,
    sky_group: wgpu::BindGroup,
    /// Group 0 and 1 of each pipeline (automatic layouts cannot be shared).
    opaque_groups: [wgpu::BindGroup; 2],
    transparent_groups: [wgpu::BindGroup; 2],
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    frame: wgpu::Buffer,
    sorted: wgpu::Buffer,
    /// The opaque draw lists, and the staging buffer their visible entries are written through.
    entries: wgpu::Buffer,
    list_staging: wgpu::Buffer,
    depth: wgpu::TextureView,
    sorted_bytes: Vec<u8>,
    shadows: HandShadows,
    /// The scene colour the parts and the sky draw into, and the post chain after them.
    post: HandPost,
}

/// The shadow side of a hand-written parts frame: the maps (sampled by the parts, and drawn by the caster pass
/// from the renderer's own caster shader when a frame has shadows), their lists and uniforms.
struct HandShadows {
    pipeline: wgpu::RenderPipeline,
    /// Group 0 of each cascade (its uniform) and group 1 (the instances and the cascades' lists).
    cascade_groups: Vec<wgpu::BindGroup>,
    instance_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    lists: wgpu::Buffer,
    staging: wgpu::Buffer,
    layers: Vec<wgpu::TextureView>,
    sample_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    /// The far cascade's static map with [`CACHED_CASCADE`], and the renderer's copy of it into the cascade's map
    /// (`shadow/copy.wgsl`) with its bind group.
    static_map: wgpu::TextureView,
    copy: wgpu::RenderPipeline,
    copy_group: wgpu::BindGroup,
    /// Entries in one cascade's block of `lists`, and where each kind's segment starts in it: the renderer's layout
    /// (`shadow/maps.rs`), so the draws cover the same instances. The far cascade's moving casters have a block
    /// after the cascades'.
    stride: u64,
    offsets: [u64; 4],
    zeros: Vec<u8>,
}

impl HandShadows {
    /// `live` is each mesh kind's live opaque instances, which size its segments as the renderer's do.
    fn new(device: &wgpu::Device, regions: &[wgpu::Buffer; 2], instances: u64, live: [u32; 4]) -> Self {
        let mut offsets = [0u64; 4];
        let mut stride = 0u64;
        for (offset, &len) in offsets.iter_mut().zip(&live) {
            *offset = stride;
            stride += u64::from(len.next_power_of_two().max(256));
        }
        let source = concat!(
            include_str!("../src/shadow/shadow.wgsl"),
            include_str!("../src/shadow/shadow_storage.wgsl")
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hand shadows"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hand shadows"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_shadow"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 24,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                // The renderer's cascades' bias (`shadow/maps.rs`).
                bias: wgpu::DepthBiasState {
                    constant: 0,
                    slope_scale: 3.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hand shadow cascades"),
            size: 1024,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cascade_groups = (0..CASCADES as u64)
            .map(|i| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("hand shadow cascade"),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &uniform,
                            offset: i * 256,
                            size: wgpu::BufferSize::new(64),
                        }),
                    }],
                })
            })
            .collect();
        let blocks = CASCADES as u64 + 1;
        let lists = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hand shadow lists"),
            size: stride * blocks * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hand shadow staging"),
            size: instances * 4 * blocks,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let instance_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hand shadow instances"),
            layout: &pipeline.get_bind_group_layout(1),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: regions[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: regions[1].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: lists.as_entire_binding(),
                },
            ],
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hand shadow maps"),
            size: wgpu::Extent3d {
                width: 2048,
                height: 2048,
                depth_or_array_layers: CASCADES as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let sample_view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..wgpu::TextureViewDescriptor::default()
        });
        let layers = (0..CASCADES as u32)
            .map(|layer| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..wgpu::TextureViewDescriptor::default()
                })
            })
            .collect();
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..wgpu::SamplerDescriptor::default()
        });
        let static_map = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("hand shadow static map"),
                size: wgpu::Extent3d {
                    width: 2048,
                    height: 2048,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let copy_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hand shadow copy"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../src/shadow/copy.wgsl").into()),
        });
        let copy_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hand shadow copy"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let copy = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hand shadow copy"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("hand shadow copy"),
                bind_group_layouts: &[Some(&copy_layout)],
                immediate_size: 0,
            })),
            vertex: wgpu::VertexState {
                module: &copy_module,
                entry_point: Some("vs_copy"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &copy_module,
                entry_point: Some("fs_copy"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[],
            }),
            multiview_mask: None,
            cache: None,
        });
        let copy_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hand shadow copy"),
            layout: &copy_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&static_map),
            }],
        });
        Self {
            pipeline,
            cascade_groups,
            instance_group,
            uniform,
            lists,
            staging,
            layers,
            sample_view,
            sampler,
            static_map,
            copy,
            copy_group,
            stride,
            offsets,
            zeros: vec![0; (instances * 4) as usize * (CASCADES + 1)],
        }
    }

    /// The renderer's shadow work for a frame that draws every cascade, whose lists hold `casters.cascades[cascade]
    /// [kind][level]` casters: one staging write and a copy per non-empty kind's list (its levels back to back; with
    /// [`CACHED_CASCADE`], the far cascade's moving casters' lists after its own), the cascades' uniform, then a depth
    /// pass per cascade with one draw per non-empty kind and level. With [`CACHED_CASCADE`] the far cascade's pass
    /// draws its static map, and a pass after it builds its map: the copy, then one draw per non-empty kind and level
    /// of moving casters.
    fn record(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        (vertices, indices): (&wgpu::Buffer, &wgpu::Buffer),
        casters: &HandCasters,
    ) {
        let far = CASCADES - 1;
        // Each list block in upload order (the moving casters' block after the cascades') and its counts.
        let blocks = || {
            let cascades = casters.cascades.iter().enumerate();
            cascades.chain(casters.moving.iter().map(|kinds| (CASCADES, kinds)))
        };
        let listed: u64 = blocks()
            .flat_map(|(_, kinds)| kinds.iter().flatten())
            .map(|&n| u64::from(n) * 4)
            .sum();
        if let Some(size) = wgpu::BufferSize::new(listed) {
            if let Some(mut view) = queue.write_buffer_with(&self.staging, 0, size) {
                view.slice(..).copy_from_slice(&self.zeros[..listed as usize]);
            }
            let mut source = 0;
            for (block, kinds) in blocks() {
                for (kind, levels) in kinds.iter().enumerate() {
                    let n: u32 = levels.iter().sum();
                    if n == 0 {
                        continue;
                    }
                    let bytes = u64::from(n) * 4;
                    let target = (block as u64 * self.stride + self.offsets[kind]) * 4;
                    encoder.copy_buffer_to_buffer(&self.staging, source, &self.lists, target, bytes);
                    source += bytes;
                }
            }
        }
        queue.write_buffer(&self.uniform, 0, &[0; 1024]);
        let draw = |pass: &mut wgpu::RenderPass<'_>, cascade: usize, block: usize, kinds: &KindLevels| {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.cascade_groups[cascade], &[]);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_bind_group(1, &self.instance_group, &[]);
            for (kind, levels) in kinds.iter().enumerate() {
                let mut first = (block as u64 * self.stride + self.offsets[kind]) as u32;
                for &n in levels {
                    if n > 0 {
                        pass.draw_indexed(0..36, 0, first..first + n);
                    }
                    first += n;
                }
            }
        };
        fn begin<'e>(encoder: &'e mut wgpu::CommandEncoder, view: &wgpu::TextureView) -> wgpu::RenderPass<'e> {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
        }
        for (cascade, kinds) in casters.cascades.iter().enumerate() {
            let view = match casters.moving {
                Some(_) if cascade == far => &self.static_map,
                _ => &self.layers[cascade],
            };
            let mut pass = begin(encoder, view);
            draw(&mut pass, cascade, cascade, kinds);
            drop(pass);
            if let Some(moving) = casters.moving.as_ref().filter(|_| cascade == far) {
                let mut pass = begin(encoder, &self.layers[cascade]);
                pass.set_pipeline(&self.copy);
                pass.set_bind_group(0, &self.copy_group, &[]);
                pass.draw(0..3, 0..1);
                if moving.iter().flatten().any(|&n| n > 0) {
                    draw(&mut pass, cascade, CASCADES, moving);
                }
            }
        }
    }
}

impl HandParts {
    fn new(context: &RenderContext, instances: u64, live: [u32; 4]) -> Self {
        let device = context.device();
        // The scene draws into the post chain's HDR scene colour (#85).
        let scene_format = metallic_render::post::hdr_format(context);
        let source = concat!(
            include_str!("../src/parts/frame.wgsl"),
            include_str!("../src/parts/parts.wgsl"),
            // A frame without local lights draws with lights_off.wgsl (`src/parts/mod.rs`), as the renderer does.
            include_str!("../src/parts/lights_off.wgsl"),
            include_str!("../src/parts/instance_storage.wgsl")
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hand parts"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let buffer = |size: u64, usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("hand parts"),
                size,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let storage = wgpu::BufferUsages::STORAGE;
        let regions = [buffer(instances * 64, storage), buffer(instances * 64, storage)];
        let entries = buffer(instances * 4, storage);
        // The transparent pass's pulled records and chunk table (#106), and the meshes' unindexed copy.
        let sorted = buffer(instances * 64, storage);
        let pulled_vertices = buffer(32 * 4096, storage);
        let frame = buffer(FRAME_UNIFORM_SIZE, wgpu::BufferUsages::UNIFORM);
        let materials = buffer(8192, wgpu::BufferUsages::UNIFORM);
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let mesh_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: 24,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        let pipeline = |entry_point: &str, write: bool, blend: Option<wgpu::BlendState>| {
            // The transparent pass pulls its vertices: no vertex buffer.
            let (vertex, buffers) = match blend {
                Some(_) => ("vs_pulled", &[][..]),
                None => ("vs_main", &mesh_buffers[..]),
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("hand parts"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vertex),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: Some(wgpu::Face::Back),
                    ..wgpu::PrimitiveState::default()
                },
                depth_stencil: Some(metallic_render::depth::depth_stencil_state(write)),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry_point),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: scene_format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque = pipeline("fs_opaque", true, None);
        let transparent = pipeline(
            "fs_transparent",
            false,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let shadows = HandShadows::new(device, &regions, instances, live);
        let groups = |pipeline: &wgpu::RenderPipeline, instances: &[(u32, &wgpu::Buffer)]| {
            let group = |index: u32, buffers: &[(u32, &wgpu::Buffer)]| {
                let mut entries: Vec<_> = buffers
                    .iter()
                    .map(|&(binding, buffer)| wgpu::BindGroupEntry {
                        binding,
                        resource: buffer.as_entire_binding(),
                    })
                    .collect();
                if index == 0 {
                    // The shadow maps and their sampler.
                    entries.push(wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&shadows.sample_view),
                    });
                    entries.push(wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&shadows.sampler),
                    });
                }
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("hand parts"),
                    layout: &pipeline.get_bind_group_layout(index),
                    entries: &entries,
                })
            };
            [group(0, &[(0, &frame), (1, &materials)]), group(1, instances)]
        };
        let opaque_groups = groups(&opaque, &[(0, &regions[0]), (1, &regions[1]), (2, &entries)]);
        let transparent_groups = groups(
            &transparent,
            &[(0, &regions[0]), (1, &regions[1]), (3, &sorted), (4, &pulled_vertices)],
        );
        let depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("hand depth"),
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let sky_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hand sky"),
            source: wgpu::ShaderSource::Wgsl(
                concat!(
                    include_str!("../src/parts/frame.wgsl"),
                    include_str!("../src/parts/sky.wgsl")
                )
                .into(),
            ),
        });
        let sky = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hand sky"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &sky_module,
                entry_point: Some("vs_sky"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                depth_compare: Some(metallic_render::depth::DEPTH_COMPARE_FAR_PLANE),
                ..metallic_render::depth::depth_stencil_state(false)
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &sky_module,
                entry_point: Some("fs_sky"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(scene_format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sky_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hand sky"),
            layout: &sky.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame.as_entire_binding(),
            }],
        });
        Self {
            opaque,
            transparent,
            sky,
            sky_group,
            opaque_groups,
            transparent_groups,
            vertices: buffer(24 * 1024, wgpu::BufferUsages::VERTEX),
            indices: buffer(4 * 4096, wgpu::BufferUsages::INDEX),
            frame,
            sorted,
            list_staging: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("hand list staging"),
                size: instances * 4,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            entries,
            depth,
            sorted_bytes: vec![0; (instances * 64) as usize],
            shadows,
            post: HandPost::new(context, (SIZE, SIZE), false),
        }
    }

    /// The renderer's default passes with `opaque` instanced draws and `transparent.0` pulled vertices (#106: one
    /// draw, none when 0), after writing each kind's `visible` list entries (one staging write and a copy per list, as
    /// a culled frame uploads its lists), the frame uniform and the `transparent.1` bytes of the sorted transparent
    /// records (written into the staging memory, as the renderer does): the wgpu calls `Renderer::render_scene`
    /// makes. With `shadows`, every cascade is drawn first, its lists holding that many casters of each kind
    /// ([`HandShadows::record`]).
    fn frame(
        &self,
        device: &wgpu::Device,
        context: &RenderContext,
        target: &wgpu::TextureView,
        (opaque, transparent): (u32, (u32, usize)),
        visible: &[u32],
        shadows: Option<&HandCasters>,
    ) -> wgpu::SubmissionIndex {
        let queue = context.queue();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("metallic frame"),
        });
        let listed: u64 = visible.iter().map(|&n| u64::from(n) * 4).sum();
        if let Some(size) = wgpu::BufferSize::new(listed) {
            if let Some(mut view) = queue.write_buffer_with(&self.list_staging, 0, size) {
                view.slice(..).copy_from_slice(&self.sorted_bytes[..listed as usize]);
            }
            let mut source = 0;
            for bytes in visible.iter().filter(|&&n| n > 0).map(|&n| u64::from(n) * 4) {
                encoder.copy_buffer_to_buffer(&self.list_staging, source, &self.entries, source, bytes);
                source += bytes;
            }
        }
        if let Some(counts) = shadows {
            self.shadows
                .record(queue, &mut encoder, (&self.vertices, &self.indices), counts);
        }
        queue.write_buffer(&self.frame, 0, &[0; FRAME_UNIFORM_SIZE as usize]);
        let (vertices, sorted) = transparent;
        if vertices > 0
            && let Some(size) = wgpu::BufferSize::new(sorted as u64)
            && let Some(mut view) = queue.write_buffer_with(&self.sorted, 0, size)
        {
            view.slice(..).copy_from_slice(&self.sorted_bytes[..sorted]);
        }
        for &(label, clear, depth_write) in &DEFAULT_PASSES {
            // The post chain's passes come after the scene; its finish pass draws UI.
            if label == "ui" {
                self.post.record(&mut encoder, target, None);
                continue;
            }
            let colors = [Some(wgpu::RenderPassColorAttachment {
                view: &self.post.scene,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })];
            let depth = depth_write.map(|write| wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth,
                depth_ops: write.then_some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(DEPTH_CLEAR),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &colors,
                depth_stencil_attachment: depth,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let (pipeline, groups, draws) = match label {
                "opaque" => (&self.opaque, &self.opaque_groups, opaque),
                "transparent" => {
                    // Every see-through instance in one draw, with no vertex or index buffer.
                    if vertices > 0 {
                        pass.set_pipeline(&self.transparent);
                        pass.set_bind_group(0, &self.transparent_groups[0], &[]);
                        pass.set_bind_group(1, &self.transparent_groups[1], &[]);
                        pass.draw(0..vertices, 0..1);
                    }
                    continue;
                }
                "sky" => {
                    pass.set_pipeline(&self.sky);
                    pass.set_bind_group(0, &self.sky_group, &[]);
                    pass.draw(0..3, 0..1);
                    continue;
                }
                _ => continue,
            };
            if draws == 0 {
                continue;
            }
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &groups[0], &[]);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_bind_group(1, &groups[1], &[]);
            for draw in 0..draws {
                pass.draw_indexed(0..36, 0, draw..draw + 1);
            }
        }
        context.submit(std::iter::once(encoder.finish()))
    }
}

/// The fewest allocations of a parts frame of `parts` parts (every `see_through`th one see-through, blocks and
/// balls in turn) and of the same wgpu calls by hand, over [`FRAMES`] frames each in alternating order, with the
/// frame's draw statistics.
/// The main view `Renderer::render_scene` culls `scene` with from `camera` on the [`SIZE`]-square target: the
/// default culling and levels of detail.
fn main_view(scene: &InstanceScene, camera: &Camera) -> metallic_render::CullView {
    let settings = metallic_render::CullSettings::default();
    metallic_render::CullView::for_camera(camera, scene.origin(), SIZE, SIZE, &settings)
}

/// What `Renderer::render_scene` draws of `scene` seen from `camera` on the [`SIZE`]-square target with the default
/// culling and levels of detail: (opaque draws, transparent draws, see-through instances), the transparent pass's
/// (pulled vertices, upload bytes) and each kind's visible instances, every level. Culls `scene` (a CPU-only copy of
/// the drawn one, which so chooses the same levels).
fn expected_calls(
    scene: &mut InstanceScene,
    visible: &mut metallic_render::VisibleSet,
    camera: &Camera,
) -> ((u32, u32, usize), (u32, usize), [u32; 4]) {
    let view = main_view(scene, camera);
    scene.cull(&view, visible);
    let mut lists = [0u32; 4];
    let mut opaque_draws = 0;
    for (kind, n) in lists.iter_mut().enumerate() {
        let kind = metallic_render::scene::MeshKind(kind as u16);
        *n = visible.opaque_len(kind) as u32;
        // One draw per level with instances.
        opaque_draws += (0..MAX_LOD_LEVELS)
            .filter(|&level| !visible.opaque(kind, level).is_empty())
            .count() as u32;
    }
    // Every see-through instance in one draw of its mesh's vertices, one instance after another, and an upload of
    // the record and chunk counts, a chunk table entry per 64 vertices and a 12-byte record per instance
    // (`parts/transparent.rs`, #106). The order they are sorted in changes neither.
    let (entries, levels) = (visible.transparent(), visible.transparent_levels());
    let mut vertices = 0u32;
    for (&entry, &level) in entries.iter().zip(levels) {
        let kind = scene.entry_instance(entry).expect("a drawn entry").mesh();
        let mesh = MeshData::primitive_level(kind, usize::from(level)).expect("a built-in mesh");
        vertices += mesh.indices.len() as u32;
    }
    let bytes = 8 + vertices.div_ceil(64) as usize * 4 + entries.len() * 12;
    let draws = u32::from(vertices > 0);
    ((opaque_draws, draws, entries.len()), (vertices, bytes), lists)
}

/// The radius of a packed instance's bounding sphere: half the diagonal of its stretched, turned unit cube.
fn bounding_radius(instance: &GpuInstance) -> f32 {
    let squared: f32 = instance
        .rows
        .iter()
        .map(|r| r[0] * r[0] + r[1] * r[1] + r[2] * r[2])
        .sum();
    0.5 * squared.sqrt()
}

/// The casters of each kind and level the renderer draws into each cascade of `scene` (a CPU-only copy of the drawn
/// one) seen from `camera` and lit by `lighting`, with `work`'s settings: the culls the renderer makes. With
/// [`ShadowWork::Cached`], also the far cascade's moving casters: every caster of the dynamic region (the test
/// checks the renderer finds all of them in it), each at the level the far cascade draws it at.
fn expected_shadows(
    scene: &mut InstanceScene,
    sets: &mut [VisibleSet; CASCADES],
    camera: &Camera,
    lighting: &SceneLighting,
    work: ShadowWork,
) -> HandCasters {
    let settings = work.settings();
    let lod = main_view(scene, camera).lod;
    let (views, count) = metallic_render::shadow::cascade_views(
        &settings,
        camera,
        1.0,
        scene.origin(),
        lighting.light_direction(),
        &lod,
    );
    assert_eq!(count, CASCADES);
    let mut counts = [[[0u32; MAX_LOD_LEVELS]; 4]; CASCADES];
    for ((view, casters), (set, kinds)) in views.iter().zip(sets.iter_mut().zip(&mut counts)) {
        scene.cull_casters(view, *casters, set);
        for (kind, levels) in kinds.iter_mut().enumerate() {
            for (level, n) in levels.iter_mut().enumerate() {
                *n = set.opaque(metallic_render::scene::MeshKind(kind as u16), level).len() as u32;
            }
        }
    }
    let far = views[CASCADES - 1].0;
    let moving = matches!(work, ShadowWork::Cached(_)).then(|| {
        let mut kinds = [[0u32; MAX_LOD_LEVELS]; 4];
        let dynamic = scene.instances(metallic_render::scene::Region::Dynamic);
        for instance in dynamic.iter().filter(|i| metallic_render::scene::is_shadow_caster(i)) {
            let level = metallic_render::lod::caster_level(&far, instance.mesh(), bounding_radius(instance));
            kinds[usize::from(instance.mesh().0)][level] += 1;
        }
        kinds
    });
    HandCasters {
        cascades: counts,
        moving,
    }
}

/// [`parts_frame_and_floor`] without shadows.
fn parts_frame_and_floor(context: &RenderContext, parts: usize, see_through: usize) -> (u64, u64, DrawStats) {
    frame_and_floor(context, parts, see_through, ShadowWork::Off)
}

/// The fewest allocations of a parts frame (as [`parts_frame_and_floor`]) that also draws every shadow cascade of
/// `work` (not [`ShadowWork::Off`]), the sun moving every frame, and of the same wgpu calls by hand.
fn shadowed_frame_and_floor(
    context: &RenderContext,
    parts: usize,
    see_through: usize,
    work: ShadowWork,
) -> (u64, u64, DrawStats) {
    frame_and_floor(context, parts, see_through, work)
}

/// The frames before this one move [`ShadowWork::Cached`]'s balls (so they go to the dynamic region and stay
/// there: nothing syncs the scene again, so nothing returns them to the static region).
const MOVES: usize = 4;

fn frame_and_floor(
    context: &RenderContext,
    parts: usize,
    see_through: usize,
    work: ShadowWork,
) -> (u64, u64, DrawStats) {
    let shadows = work != ShadowWork::Off;
    let device = context.device();
    let mut game = support::scenes::Game::new();
    // Every kind opaque, and see-through blocks and balls mixed so the transparent pass makes several draws.
    support::scenes::grid(&mut game, parts, 100, support::scenes::SeeThrough::Mixed(see_through));
    // Balls in the middle of the grid, where the camera looks: always inside the far cascade.
    let movers = match work {
        ShadowWork::Cached(movers) => movers,
        _ => 0,
    };
    let at = |i: usize, frame: usize| Vec3::new(i as f32 * 3.0 - movers as f32 * 1.5, 6.0 + frame as f32 * 0.1, 2.0);
    let balls: Vec<Id> = (0..movers)
        .map(|i| {
            game.part(
                support::scenes::Look::new("Ball", "Plastic", [200, 160, 40]),
                Vec3::splat(2.0),
                CFrame::new(at(i, 0)),
            )
        })
        .collect();
    let mut scene = GpuScene::new(
        context,
        &mut game.dm,
        Some(game.workspace),
        &RenderComponents::builtin(),
    )
    .expect("a scene");
    scene.sync(&mut game.dm);
    let target = OffscreenTarget::new(context, SIZE, SIZE, FORMAT).expect("a target");
    let mut renderer = Renderer::new(
        context,
        RendererSettings {
            gpu_timing: false,
            shadows: work.settings(),
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .expect("a renderer");
    renderer.wait_for_pipelines();
    // The same game culled on the CPU alone, so the hand-written frame makes exactly the calls the renderer makes
    // in the same frame, whichever of the two goes first.
    let mut mirror = InstanceScene::attach(&mut game.dm, Some(game.workspace), &RenderComponents::builtin())
        .expect("a mirror scene");
    mirror.sync(&mut game.dm);
    mirror.coalesce();
    let live = [0u16, 1, 2, 3].map(|kind| mirror.draw_list(metallic_render::scene::MeshKind(kind)).len() as u32);
    let hand = HandParts::new(context, parts as u64, live);
    let mut mirror_visible = metallic_render::VisibleSet::new();
    let mut mirror_cascades: [VisibleSet; CASCADES] = Default::default();
    let (mut ours, mut floor) = (Vec::with_capacity(FRAMES), Vec::with_capacity(FRAMES));
    for frame in 0..WARM_UP + FRAMES {
        if frame < MOVES && !balls.is_empty() {
            for (i, &ball) in balls.iter().enumerate() {
                game.dm
                    .set_by_name(ball, "CFrame", Value::CFrame(CFrame::new(at(i, frame + 1))))
                    .expect("moved");
            }
            scene.sync(&mut game.dm);
            mirror.sync(&mut game.dm);
            mirror.coalesce();
        }
        let camera = orbiting_camera(frame);
        // The camera moves every frame, so every frame culls again and uploads each kind's visible list.
        let (calls, pulled, visible) = expected_calls(&mut mirror, &mut mirror_visible, &camera);
        // The sun moves every frame too, so every cascade is drawn again.
        let casters =
            shadows.then(|| expected_shadows(&mut mirror, &mut mirror_cascades, &camera, &moving_sun(frame), work));
        for renderer_turn in [frame % 2 == 0, frame % 2 != 0] {
            let (count, submission) = if renderer_turn {
                let drawn = allocations_in(|| {
                    renderer.set_lighting(moving_sun(frame));
                    renderer.render_scene(target.view(), &mut scene, Some(&camera))
                });
                let stats = renderer.draw_stats();
                assert_eq!(
                    (
                        stats.opaque_draws,
                        stats.transparent_draws,
                        stats.transparent_instances as usize
                    ),
                    calls,
                    "the mirror predicted another frame"
                );
                if let Some(casters) = casters.as_ref() {
                    assert_eq!(stats.shadow_redrawn, 0b111, "every cascade is drawn");
                    let expected: Vec<u32> = casters
                        .cascades
                        .iter()
                        .map(|kinds| kinds.iter().flatten().sum())
                        .collect();
                    assert_eq!(&stats.shadow_casters[..CASCADES], expected, "the mirror's casters");
                    if let Some(moving) = casters.moving {
                        assert_eq!(stats.shadow_built, 0b100, "the far cascade's map is built");
                        assert_eq!(
                            stats.shadow_moving[2],
                            moving.iter().flatten().sum::<u32>(),
                            "the mirror's moving casters"
                        );
                    }
                }
                drawn
            } else {
                allocations_in(|| {
                    hand.frame(
                        device,
                        context,
                        target.view(),
                        (calls.0, pulled),
                        &visible,
                        casters.as_ref(),
                    )
                })
            };
            // Each frame finishes before the next starts. The hand-written draws are degenerate, so on a software
            // rasteriser the renderer's frames take far longer, and with frames in flight wgpu's bookkeeping of
            // pending submissions (and what it frees when they finish) would differ between the two sides.
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: None,
                })
                .expect("wait for the frame");
            if frame >= WARM_UP {
                if renderer_turn { &mut ours } else { &mut floor }.push(count);
            }
        }
    }
    let stats = renderer.draw_stats();
    eprintln!(
        "parts frame ({} opaque + {} transparent + {} shadow draws, {} instances): allocations {}..{} (median {}); \
         the same wgpu calls by hand {}..{} (median {})",
        stats.opaque_draws,
        stats.transparent_draws,
        stats.shadow_draws,
        stats.instances,
        ours.iter().min().expect("frames"),
        ours.iter().max().expect("frames"),
        median(ours.clone()),
        floor.iter().min().expect("frames"),
        floor.iter().max().expect("frames"),
        median(floor.clone())
    );
    (
        *ours.iter().min().expect("frames"),
        *floor.iter().min().expect("frames"),
        stats,
    )
}

#[test]
fn a_parts_frame_allocates_nothing_beyond_wgpus_own() {
    let Some(context) = measuring_gpu(false) else { return };
    if InstanceBinding::for_context(&context) != InstanceBinding::Storage {
        eprintln!("skipped: the hand-written frame is the storage path's");
        return;
    }
    let _padding = pad_tracker_indices(&context);
    // Two workloads, the second with three times the parts and many more see-through parts, sorted and pulled.
    let (ours_small, floor_small, small) = parts_frame_and_floor(&context, 2_000, 25);
    let (ours_large, floor_large, large) = parts_frame_and_floor(&context, 6_000, 8);
    report(
        "parts frame",
        &context,
        &[
            ("2,000 parts", ours_small, floor_small),
            ("6,000 parts", ours_large, floor_large),
        ],
    );
    // One draw per mesh kind and level of detail in use: every kind, and the balls and cylinders at more than one
    // level (#80), at most nine.
    for stats in [&small, &large] {
        assert!((5..=9).contains(&stats.opaque_draws), "{stats:?}");
    }
    // Every see-through instance in one draw (#106), however many there are.
    assert!(
        small.transparent_draws == 1
            && large.transparent_draws == 1
            && large.transparent_instances > 4 * small.transparent_instances,
        "{small:?} {large:?}"
    );
    let (extra_small, extra_large) = (
        ours_small.saturating_sub(floor_small),
        ours_large.saturating_sub(floor_large),
    );
    // Nothing the renderer does allocates per part, per draw or per sorted instance: whatever a frame
    // allocates beyond wgpu's own is the same for both workloads...
    assert_eq!(
        extra_small, extra_large,
        "the renderer's own allocations grow with the work: {extra_small} for {small:?}, {extra_large} for \
         {large:?}"
    );
    // ...and nothing at all.
    assert_eq!(
        extra_small,
        0,
        "drawing parts adds {extra_small} allocation(s) per frame on {:?}",
        context.capabilities().device_type
    );
}

/// Sun shadows (#82): a frame that also fits, culls, uploads and draws three cascades (the sun moving, so all three
/// every frame) allocates exactly what the same wgpu calls do by hand, for two workloads with very different casters.
#[test]
fn a_shadowed_frame_allocates_nothing_beyond_wgpus_own() {
    let Some(context) = measuring_gpu(false) else { return };
    if InstanceBinding::for_context(&context) != InstanceBinding::Storage {
        eprintln!("skipped: the hand-written frame is the storage path's");
        return;
    }
    let _padding = pad_tracker_indices(&context);
    let (ours_small, floor_small, small) = shadowed_frame_and_floor(&context, 2_000, 25, ShadowWork::Every);
    let (ours_large, floor_large, large) = shadowed_frame_and_floor(&context, 6_000, 8, ShadowWork::Every);
    report(
        "shadowed frame",
        &context,
        &[
            ("2,000 parts", ours_small, floor_small),
            ("6,000 parts", ours_large, floor_large),
        ],
    );
    assert_eq!((small.shadow_cascades, large.shadow_cascades), (3, 3));
    assert!(
        large.shadow_instances > 2 * small.shadow_instances,
        "the workloads differ enough: {small:?} {large:?}"
    );
    let (extra_small, extra_large) = (
        ours_small.saturating_sub(floor_small),
        ours_large.saturating_sub(floor_large),
    );
    // Nothing the shadows add allocates per caster, per cascade draw or per list: whatever a frame allocates beyond
    // wgpu's own is the same for both workloads... (That the culls, the fitting and the lists allocate nothing at
    // all is checked on its own by `a_cull_allocates_nothing_once_warm`.)
    assert_eq!(
        extra_small, extra_large,
        "the shadows' own allocations grow with the casters: {extra_small} for {small:?}, {extra_large} for \
         {large:?}"
    );
    // ...and nothing at all.
    assert_eq!(
        extra_small,
        0,
        "a shadowed frame allocates {ours_small} against {floor_small} by hand ({small:?}) on {:?}",
        context.capabilities().device_type
    );
}

/// The cached far cascade (#121): with the default cascades and the sun moving, a frame draws the near cascades,
/// the far cascade's static map, and builds its map from a copy of the static map and the moving casters. It
/// allocates exactly what the same wgpu calls do by hand, for two workloads with different casters and moving
/// casters.
#[test]
fn a_cached_cascade_frame_allocates_nothing_beyond_wgpus_own() {
    let Some(context) = measuring_gpu(false) else { return };
    if InstanceBinding::for_context(&context) != InstanceBinding::Storage {
        eprintln!("skipped: the hand-written frame is the storage path's");
        return;
    }
    let _padding = pad_tracker_indices(&context);
    let (ours_small, floor_small, small) = shadowed_frame_and_floor(&context, 2_000, 25, ShadowWork::Cached(4));
    let (ours_large, floor_large, large) = shadowed_frame_and_floor(&context, 6_000, 8, ShadowWork::Cached(24));
    report(
        "cached cascade frame",
        &context,
        &[
            ("2,000 parts, 4 moving", ours_small, floor_small),
            ("6,000 parts, 24 moving", ours_large, floor_large),
        ],
    );
    assert_eq!((small.shadow_moving[2], large.shadow_moving[2]), (4, 24));
    assert_eq!((small.shadow_built, large.shadow_built), (0b100, 0b100));
    let (extra_small, extra_large) = (
        ours_small.saturating_sub(floor_small),
        ours_large.saturating_sub(floor_large),
    );
    assert_eq!(
        extra_small, extra_large,
        "the cached cascade's own allocations grow with the casters: {extra_small} for {small:?}, {extra_large} for \
         {large:?}"
    );
    assert_eq!(
        extra_small,
        0,
        "a cached cascade frame allocates {ours_small} against {floor_small} by hand ({small:?}) on {:?}",
        context.capabilities().device_type
    );
}

/// How many allocations wgpu's own work in a frame can differ by between two frames that make the same calls with
/// different resources: wgpu-core's per-encoder buffer trackers grow with the order and size of the tracker indices
/// the frame's copies meet ([`pad_tracker_indices`]). Measured on the dev PC (#82): +3, 0, +3, −3 and 0 for five
/// shadowed workloads against their hand-written frames. The lit frame compares two renderers with different
/// buffers, so it allows this; the parts and shadowed frames pad the indices and compare exactly.
const WGPU_DATA_JITTER: u64 = 3;

/// Frames of moving parts seen from an orbiting camera, so every frame culls again and uploads the visible lists,
/// on both instance paths (the vertex-buffer fallback's lists hold copies of the visible instances).
#[test]
fn nothing_accumulates_over_parts_frames() {
    for force_vertex_instances in [false, true] {
        let Some(context) = measuring_gpu(force_vertex_instances) else {
            return;
        };
        parts_frames_do_not_grow_the_heap(&context);
    }
}

fn parts_frames_do_not_grow_the_heap(context: &RenderContext) {
    let mut game = support::scenes::Game::new();
    let parts = support::scenes::grid(&mut game, 2_000, 10, support::scenes::SeeThrough::Mixed(25));
    let mut scene = GpuScene::new(
        context,
        &mut game.dm,
        Some(game.workspace),
        &RenderComponents::builtin(),
    )
    .expect("a scene");
    let target = OffscreenTarget::new(context, SIZE, SIZE, FORMAT).expect("a target");
    let mut renderer = Renderer::new(context, RendererSettings::new(FORMAT, SIZE, SIZE)).expect("a renderer");
    renderer.wait_for_pipelines();
    let mut pacer = Pacer::new();
    let mut frame = |frame| {
        // 1% of the parts move, the same ones every frame.
        for (i, &id) in parts.iter().step_by(100).enumerate() {
            let place = CFrame::new(Vec3::new(i as f32 * 4.0, 1.0 + frame as f32 * 0.01, 0.0));
            game.dm.set_by_name(id, "CFrame", Value::CFrame(place)).expect("moved");
        }
        scene.sync(&mut game.dm);
        let camera = orbiting_camera(frame);
        let submission = renderer.render_scene(target.view(), &mut scene, Some(&camera));
        pacer.submitted(context.device(), submission);
    };
    (0..WARM_UP).for_each(&mut frame);
    let growth = HeapGrowth::measure(context.device(), WARM_UP, frame);
    eprintln!(
        "frames on the {:?} path drawing {:?}: {growth}",
        InstanceBinding::for_context(context),
        renderer.draw_stats()
    );
    assert!(renderer.draw_stats().culled_instances > 0, "nothing was culled");
    // With the default shadows: the near cascades follow the camera, so they are drawn every frame.
    assert_eq!(renderer.draw_stats().shadow_cascades, 3);
    assert!(renderer.draw_stats().shadow_draws > 0, "no shadows were drawn");
    growth.assert_nothing_accumulates();
}

/// Allocations and heap growth of [`FRAMES`] frames of sync, coalesce and cull (the CPU side of a culled frame,
/// `GpuScene::upload_view` without its wgpu calls) of a grid of `parts` parts, every 25th see-through, 1% moving
/// (the same ones, which also cross grid cells) and 1% recoloured, seen from an orbiting camera. Returns them with
/// the parts culled over the measured frames.
fn culled_frames(parts: usize) -> (u64, i64, u64) {
    let mut game = support::scenes::Game::new();
    let ids = support::scenes::grid(&mut game, parts, 10, support::scenes::SeeThrough::Mixed(25));
    let mut scene =
        InstanceScene::attach(&mut game.dm, Some(game.workspace), &RenderComponents::builtin()).expect("a scene");
    let mut visible = metallic_render::VisibleSet::new();
    let mut cascades: [VisibleSet; 3] = Default::default();
    let settings = metallic_render::CullSettings {
        min_screen_size: 2.0,
        ..metallic_render::CullSettings::default()
    };
    let (mut allocations, mut grown, mut culled) = (0, 0, 0);
    for frame in 0..WARM_UP + FRAMES {
        for (i, &id) in ids.iter().step_by(100).enumerate() {
            // Up and down through a cell boundary at y = 0.
            let y = (frame as f32 * 0.7 + i as f32).sin() * 3.0;
            let place = CFrame::new(Vec3::new(i as f32 * 4.0 - 40.0, y, 0.0));
            game.dm.set_by_name(id, "CFrame", Value::CFrame(place)).expect("moved");
        }
        let shade = (frame % 200) as f32 / 255.0;
        for i in 0..parts / 100 {
            let id = ids[(i * 7919 + frame * 31) % parts];
            game.dm
                .set_by_name(id, "Color", Value::Color3(Color3::new(shade, 0.5, 0.5)))
                .expect("recoloured");
        }
        let camera = orbiting_camera(frame);
        let sun = moving_sun(frame).light_direction();
        let before = live_bytes();
        let (count, stats) = allocations_in(|| {
            scene.sync(&mut game.dm);
            scene.coalesce();
            let view = metallic_render::CullView::for_camera(&camera, scene.origin(), 1920, 1080, &settings);
            let stats = scene.cull(&view, &mut visible);
            // The default shadows' cascades: each culls its own casters (the far one the static region's only),
            // and the scene's noted shadow-caster changes are taken, as a frame does.
            let (views, count) = metallic_render::shadow::cascade_views(
                &ShadowSettings::default(),
                &camera,
                16.0 / 9.0,
                scene.origin(),
                sun,
                &view.lod,
            );
            for ((view, casters), set) in views.iter().zip(&mut cascades).take(count) {
                scene.cull_casters(view, *casters, set);
            }
            scene.clear_shadow_changes();
            stats
        });
        if frame >= WARM_UP {
            allocations += count;
            grown += live_bytes() - before;
            culled += u64::from(stats.culled());
        }
    }
    eprintln!(
        "{FRAMES} culled frames of {parts} parts from an orbiting camera: {allocations} allocations, heap grew \
         {grown} bytes, {culled} parts culled; the last: {:?}",
        visible.stats()
    );
    (allocations, grown, culled)
}

#[test]
fn a_cull_allocates_nothing_once_warm() {
    for parts in [2_000, 8_000] {
        let (allocations, grown, culled) = culled_frames(parts);
        assert!(culled > 0, "nothing was culled");
        assert_eq!(allocations, 0, "the sync, coalesce and cull of {parts} parts allocated");
        assert_eq!(grown, 0, "the heap grew");
    }
}

/// What an overlay frame draws: `lines` lines of formatted numbers that change every frame and a graph of `bars`
/// bars, over a panel.
#[derive(Clone, Copy, Debug)]
struct OverlayWork {
    lines: usize,
    bars: usize,
}

/// The performance overlay's frame: a panel, a line of numbers and a 240-frame graph (about 260 quads).
const PERF_OVERLAY: OverlayWork = OverlayWork { lines: 1, bars: 240 };
/// A small overlay: one line, a short graph (about 30 quads).
const SMALL_OVERLAY: OverlayWork = OverlayWork { lines: 1, bars: 8 };
/// A large one: 40 lines of text and the full graph (about 1,300 quads).
const LARGE_OVERLAY: OverlayWork = OverlayWork { lines: 40, bars: 240 };

/// Redraws the overlay as the performance overlay does every frame.
fn fill_overlay(overlay: &mut metallic_render::Overlay, frame: usize, work: OverlayWork) {
    overlay.clear();
    overlay.rect(4.0, 4.0, 248.0, 120.0, [8, 10, 14, 200]);
    for line in 0..work.lines {
        overlay.text_fmt(
            8.0,
            8.0 + line as f32 * 6.0,
            1.0,
            [235, 238, 242, 255],
            format_args!(
                "FRAME {frame} {:.2} MS  P99 {:.2}  LINE {line}",
                frame as f32 * 0.01,
                4.5
            ),
        );
    }
    for i in 0..work.bars {
        let h = ((i + frame) % 50) as f32;
        overlay.rect(8.0 + i as f32, 120.0 - h, 1.0, h.max(1.0), [96, 214, 120, 255]);
    }
}

/// The overlay's pipeline and buffer by hand, from its own shader, for the hand-written floor.
struct HandOverlay {
    pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    data: Vec<u8>,
}

impl HandOverlay {
    /// One quad is 32 bytes: a clip-space rectangle, a colour, two words of glyph pixels and a kind.
    const QUAD: u64 = 32;

    fn new(device: &wgpu::Device, quads: usize) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hand overlay"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../src/overlay/overlay.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x4, 1 => Unorm8x4, 2 => Uint32x2, 3 => Uint32];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: Self::QUAD,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_encoded"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: Self::QUAD * quads as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            buffer,
            data: vec![0; quads * Self::QUAD as usize],
        }
    }

    /// A UI-only frame that writes the quads and draws them: the same wgpu calls as the renderer's overlay frame.
    fn frame(
        &self,
        device: &wgpu::Device,
        context: &RenderContext,
        target: &wgpu::TextureView,
    ) -> wgpu::SubmissionIndex {
        let queue = context.queue();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("metallic frame"),
        });
        queue.write_buffer(&self.buffer, 0, &self.data);
        let colors = [Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui"),
            color_attachments: &colors,
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let quads = (self.data.len() as u64 / Self::QUAD) as u32;
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.buffer.slice(..u64::from(quads) * Self::QUAD));
        pass.draw(0..6, 0..quads);
        drop(pass);
        context.submit(std::iter::once(encoder.finish()))
    }
}

/// The fewest allocations of a UI-only frame whose overlay is redrawn with `work` (so it is encoded and uploaded
/// every frame) and of the same wgpu calls by hand, over [`FRAMES`] frames each in alternating order, with the
/// overlay's quad count.
fn overlay_frame_and_floor(context: &RenderContext, work: OverlayWork) -> (u64, u64, usize) {
    let device = context.device();
    let target = OffscreenTarget::new(context, SIZE, SIZE, FORMAT).expect("a target");
    let mut renderer = Renderer::new(
        context,
        RendererSettings {
            passes: PassSet::EMPTY.with(PassId::Ui),
            gpu_timing: false,
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .expect("a renderer");
    renderer.wait_for_pipelines();
    renderer.set_overlay_enabled(true);
    fill_overlay(renderer.overlay_mut(), 0, work);
    let hand = HandOverlay::new(device, renderer.overlay().len());
    let mut pacer = Pacer::new();
    let (mut ours, mut floor) = (Vec::with_capacity(FRAMES), Vec::with_capacity(FRAMES));
    for frame in 0..WARM_UP + FRAMES {
        for renderer_turn in [frame % 2 == 0, frame % 2 != 0] {
            let (count, submission) = if renderer_turn {
                // The overlay's contents change every frame, so every frame encodes and uploads them.
                allocations_in(|| {
                    fill_overlay(renderer.overlay_mut(), frame, work);
                    renderer.render(target.view())
                })
            } else {
                allocations_in(|| hand.frame(device, context, target.view()))
            };
            pacer.submitted(device, submission);
            if frame >= WARM_UP {
                if renderer_turn { &mut ours } else { &mut floor }.push(count);
            }
        }
    }
    let (ours, floor) = (
        *ours.iter().min().expect("frames"),
        *floor.iter().min().expect("frames"),
    );
    let quads = renderer.overlay().len();
    eprintln!("overlay frame ({quads} quads): renderer {ours} allocations, the same wgpu calls by hand {floor}");
    (ours, floor, quads)
}

/// The renderer's overlay frame and the hand-written one make the same wgpu calls, so they allocate the same, and the
/// same for 35 quads as for 1,300: nothing per quad, per glyph or per line of text allocates. (CI's runners used to
/// show 1 to 3 more for the renderer: library backtraces, see [`assert_library_backtraces_off`].)
#[test]
fn an_overlay_frame_allocates_nothing_beyond_wgpus_own() {
    let Some(context) = measuring_gpu(false) else { return };
    let _padding = pad_tracker_indices(&context);
    let (ours_small, floor_small, small) = overlay_frame_and_floor(&context, SMALL_OVERLAY);
    let (ours_large, floor_large, large) = overlay_frame_and_floor(&context, LARGE_OVERLAY);
    report(
        "overlay frame",
        &context,
        &[("small", ours_small, floor_small), ("large", ours_large, floor_large)],
    );
    assert!(
        large > 30 * small,
        "the workloads differ enough: {small} and {large} quads"
    );
    let (extra_small, extra_large) = (
        ours_small.saturating_sub(floor_small),
        ours_large.saturating_sub(floor_large),
    );
    // Nothing the overlay does allocates per quad: whatever a frame allocates beyond wgpu's own is the same for
    // both workloads...
    assert_eq!(
        extra_small, extra_large,
        "the overlay's own allocations grow with its size: {extra_small} for {small} quads, {extra_large} for \
         {large}"
    );
    // ...and nothing at all.
    assert_eq!(
        extra_small,
        0,
        "the overlay adds {extra_small} allocation(s) per frame on {:?}",
        context.capabilities().device_type
    );
}

#[test]
fn nothing_accumulates_over_overlay_frames() {
    let Some(context) = measuring_gpu(false) else { return };
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).unwrap();
    let mut renderer = Renderer::new(&context, RendererSettings::new(FORMAT, SIZE, SIZE)).unwrap();
    renderer.wait_for_pipelines();
    renderer.set_overlay_enabled(true);
    let mut pacer = Pacer::new();
    let mut frame = |frame| {
        fill_overlay(renderer.overlay_mut(), frame, PERF_OVERLAY);
        let submission = renderer.render(target.view());
        pacer.submitted(context.device(), submission);
    };
    (0..WARM_UP).for_each(&mut frame);
    let growth = HeapGrowth::measure(context.device(), WARM_UP, frame);
    eprintln!("overlay frames: {growth}");
    growth.assert_nothing_accumulates();
}

/// Lamps on a grid of parts: 64 neon balls 2 m up, each holding a `PointLight` (every ninth a `SpotLight` pointing
/// down), every eighth point light with `Shadows`, and every sixteenth lamp a `DirectionalLight` too (#123: four
/// extra suns, which turn with their lamps). Returns the lamps.
fn lit_grid(game: &mut support::scenes::Game) -> Vec<Id> {
    support::scenes::grid(game, 2_000, 10, support::scenes::SeeThrough::Mixed(25));
    let registry = game.dm.registry().clone();
    let (point, spot, directional) = (
        registry.class_id("PointLight").expect("PointLight"),
        registry.class_id("SpotLight").expect("SpotLight"),
        registry.class_id("DirectionalLight").expect("DirectionalLight"),
    );
    (0..64)
        .map(|i| {
            let at = Vec3::new((i % 8) as f32 * 22.0 - 77.0, 2.0, (i / 8) as f32 * 22.0 - 77.0);
            let lamp = game.part(
                support::scenes::Look::new("Ball", "Neon", [255, 220, 180]),
                Vec3::splat(0.5),
                CFrame::new(at),
            );
            let is_spot = i % 9 == 8;
            let light = game.dm.create(if is_spot { spot } else { point }).expect("a light");
            game.dm.set_by_name(light, "Range", Value::Number(14.0)).expect("Range");
            if is_spot {
                let down = CFrame::from_parts(
                    Vec3::ZERO,
                    metallic_core::glam::Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
                );
                game.dm
                    .set_by_name(light, "CFrame", Value::CFrame(down))
                    .expect("CFrame");
            } else if i % 8 == 0 {
                game.dm
                    .set_by_name(light, "Shadows", Value::Bool(true))
                    .expect("Shadows");
            }
            game.dm.set_parent(light, Some(lamp)).expect("parented");
            if i % 16 == 0 {
                let sun = game.dm.create(directional).expect("a directional light");
                let slanted = CFrame::look_at(Vec3::ZERO, Vec3::new(i as f32 - 30.0, -40.0, 10.0), Vec3::Y);
                game.dm
                    .set_by_name(sun, "CFrame", Value::CFrame(slanted))
                    .expect("CFrame");
                game.dm
                    .set_by_name(sun, "Brightness", Value::Number(0.2 + i as f64 / 100.0))
                    .expect("Brightness");
                game.dm.set_parent(sun, Some(lamp)).expect("parented");
            }
            lamp
        })
        .collect()
}

/// Local lights (#83): the sync of lights in lamps that move every frame (each move places the lamp's light again,
/// directional lights among them, #123) allocates nothing at all once warm.
#[test]
fn a_sync_of_moving_lamps_allocates_nothing() {
    let mut game = support::scenes::Game::new();
    let lamps = lit_grid(&mut game);
    let mut scene =
        InstanceScene::attach(&mut game.dm, Some(game.workspace), &RenderComponents::builtin()).expect("a scene");
    let mut total = 0;
    for frame in 0..WARM_UP + FRAMES {
        for (i, &lamp) in lamps.iter().enumerate().step_by(4) {
            let at = Vec3::new(i as f32, 2.0 + frame as f32 * 0.01, 0.0);
            game.dm
                .set_by_name(lamp, "CFrame", Value::CFrame(CFrame::new(at)))
                .expect("moved");
        }
        let (count, stats) = allocations_in(|| scene.sync(&mut game.dm));
        if frame >= WARM_UP {
            assert!(stats.lights >= 16, "{stats:?}");
            total += count;
        }
        scene.coalesce();
    }
    assert_eq!(scene.light_count(), 64);
    assert_eq!(scene.directional_lights().len(), 4);
    assert_eq!(total, 0, "syncing moving lamps allocated");
}

/// Local lights (#83): a frame that selects, fades, clusters and packs 64 lights (a moving camera changes all of it
/// every frame) allocates no more than the same frame without local lights plus the two writes it adds (the lights'
/// uniform and the cluster texture) by hand. The shadowed lights' tiles are kept (nothing moves), so no atlas pass.
#[test]
fn a_lit_frame_allocates_only_its_writes() {
    let Some(context) = measuring_gpu(false) else { return };
    let (device, queue) = (context.device(), context.queue());
    let mut game = support::scenes::Game::new();
    lit_grid(&mut game);
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).expect("a target");
    let mut renderers = [
        metallic_render::LightSettings::default(),
        metallic_render::LightSettings::OFF,
    ]
    .map(|lights| {
        let mut renderer = Renderer::new(
            &context,
            RendererSettings {
                gpu_timing: false,
                lights,
                ..RendererSettings::new(FORMAT, SIZE, SIZE)
            },
        )
        .expect("a renderer");
        renderer.wait_for_pipelines();
        let mut scene = GpuScene::new(
            &context,
            &mut game.dm,
            Some(game.workspace),
            &RenderComponents::builtin(),
        )
        .expect("a scene");
        scene.sync(&mut game.dm);
        // The first frame queues the extra suns' pipelines (the lamps hold `DirectionalLight`s, #123), compiled in
        // the background like the rest: wait for them too before anything is measured.
        let first = Camera::from_cframe(CFrame::look_at(Vec3::new(20.0, 150.0, 0.0), Vec3::ZERO, Vec3::Y), 70.0);
        renderer.render_scene(target.view(), &mut scene, Some(&first));
        renderer.wait_for_pipelines();
        (renderer, scene)
    });
    // The same writes by hand, into resources like the renderer's.
    let light_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("hand lights"),
        size: 16384,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let clusters = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hand clusters"),
        size: wgpu::Extent3d {
            width: 1024,
            height: 58,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let data = vec![1u8; 1024 * 58 * 4];
    let (mut lit, mut unlit, mut hand) = (Vec::new(), Vec::new(), Vec::new());
    for frame in 0..WARM_UP + FRAMES {
        let camera = Camera::from_cframe(
            CFrame::look_at(
                // High above the lamps, so every one stays in view as the camera circles.
                Vec3::new(
                    (frame as f32 * 0.03).cos() * 20.0,
                    150.0,
                    (frame as f32 * 0.03).sin() * 20.0,
                ),
                Vec3::ZERO,
                Vec3::Y,
            ),
            70.0,
        );
        for (index, (renderer, scene)) in renderers.iter_mut().enumerate() {
            let (count, submission) = allocations_in(|| renderer.render_scene(target.view(), scene, Some(&camera)));
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: None,
                })
                .expect("wait for the frame");
            if frame >= WARM_UP {
                if index == 0 { &mut lit } else { &mut unlit }.push(count);
            }
        }
        let stats = renderers[0].0.light_stats();
        let rows = (3456 + stats.cluster_entries.div_ceil(4)).div_ceil(1024);
        let (count, ()) = allocations_in(|| {
            queue.write_buffer(&light_buffer, 0, &data[..stats.drawn as usize * 64]);
            queue.write_texture(
                clusters.as_image_copy(),
                &data[..(rows * 4096) as usize],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4096),
                    rows_per_image: Some(rows),
                },
                wgpu::Extent3d {
                    width: 1024,
                    height: rows,
                    depth_or_array_layers: 1,
                },
            );
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        if frame >= WARM_UP {
            hand.push(count);
            assert_eq!(stats.tiles_redrawn, 0, "frame {frame} drew shadow tiles: {stats:?}");
        }
    }
    let stats = renderers[0].0.light_stats();
    let (lit, unlit, hand) = (
        *lit.iter().min().expect("frames"),
        *unlit.iter().min().expect("frames"),
        *hand.iter().min().expect("frames"),
    );
    eprintln!("lit frame {lit} allocations, unlit {unlit}, the two writes by hand {hand} ({stats:?})");
    assert!(
        stats.drawn > 8 && stats.shadowed > 0 && stats.cluster_entries > 0,
        "{stats:?}"
    );
    assert!(
        lit <= unlit + hand + WGPU_DATA_JITTER,
        "a lit frame allocates {lit} against {unlit} unlit and {hand} for its writes"
    );
}

/// Local lights (#83): the heap does not grow over frames whose lamps move (so their lights move, and the moving
/// shadowed ones draw their atlas tiles again every frame) seen from an orbiting camera, on both instance paths.
#[test]
fn nothing_accumulates_over_lit_frames() {
    for force_vertex_instances in [false, true] {
        let Some(context) = measuring_gpu(force_vertex_instances) else {
            return;
        };
        let mut game = support::scenes::Game::new();
        let lamps = lit_grid(&mut game);
        let mut scene = GpuScene::new(
            &context,
            &mut game.dm,
            Some(game.workspace),
            &RenderComponents::builtin(),
        )
        .expect("a scene");
        let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).expect("a target");
        let mut renderer = Renderer::new(&context, RendererSettings::new(FORMAT, SIZE, SIZE)).expect("a renderer");
        renderer.wait_for_pipelines();
        // The first frame queues the extra suns' pipelines (the lamps hold `DirectionalLight`s, #123), compiled in the
        // background like the rest: wait for them too before anything is measured.
        scene.sync(&mut game.dm);
        renderer.render_scene(target.view(), &mut scene, Some(&orbiting_camera(0)));
        renderer.wait_for_pipelines();
        // Fades by a fixed step, so which lights hold shadows in which frame does not depend on how fast the machine
        // draws.
        renderer.set_light_fade_step(Some(1.0 / 60.0));
        let mut pacer = Pacer::new();
        let mut frame = 0;
        let mut draw = |frame: usize| {
            // A quarter of the lamps move, shadowed ones among them.
            for (i, &lamp) in lamps.iter().enumerate().step_by(4) {
                let at = Vec3::new(
                    (i % 8) as f32 * 22.0 - 77.0,
                    2.0 + (frame % 10) as f32 * 0.05,
                    (i / 8) as f32 * 22.0 - 77.0,
                );
                game.dm
                    .set_by_name(lamp, "CFrame", Value::CFrame(CFrame::new(at)))
                    .expect("moved");
            }
            scene.sync(&mut game.dm);
            let camera = orbiting_camera(frame);
            let submission = renderer.render_scene(target.view(), &mut scene, Some(&camera));
            pacer.submitted(context.device(), submission);
            renderer.light_stats()
        };
        // Warm up until the lights' GPU resources exist, the shadow atlas included: they are made when first
        // needed (the first light drawn, the first shadow), and that must not land in the measured frames.
        let atlas = u64::from(metallic_render::LightSettings::default().atlas_resolution).pow(2) * 4;
        loop {
            let stats = draw(frame);
            frame += 1;
            if frame >= WARM_UP && stats.gpu_bytes > atlas {
                break;
            }
            assert!(
                frame < 10 * WARM_UP,
                "no shadowed light was drawn in {frame} frames: {stats:?}"
            );
        }
        let mut redrawn = 0;
        let growth = HeapGrowth::measure(context.device(), frame, |frame| {
            redrawn += draw(frame).tiles_redrawn;
        });
        eprintln!(
            "lit frames on the {:?} path ({:?}, {redrawn} tiles drawn): {growth}",
            InstanceBinding::for_context(&context),
            renderer.light_stats()
        );
        assert!(redrawn > 0, "no shadow tile was drawn");
        growth.assert_nothing_accumulates();
    }
}
