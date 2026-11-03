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
//! by hand, and the heap does not grow over 100 such frames with 1% of the parts changing.
//!
//! The debug overlay (#78) too: a frame whose overlay is redrawn (so it is encoded and uploaded) allocates no more
//! than the same write and draw by hand, the same for 35 quads as for 1,300 (beyond the runner noise the parts
//! frames also see), and the heap does not grow over 100 such frames.
//!
//! Culling (#79) too: a cull of 10,000 parts from an orbiting camera allocates nothing at all once warm, the parts
//! frames above cull and upload their visible lists every frame (the hand-written frame makes the same staging
//! write and copies), and the heap does not grow over 100 culled frames on either instance path.
//!
//! Counters are per thread, so tests running in parallel do not see each other's allocations.

// A `#[global_allocator]` needs `unsafe impl GlobalAlloc`. This test binary's allocator only counts, then forwards
// every call unchanged to the system allocator.
#![allow(unsafe_code)]

mod support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use metallic_core::glam::Vec3;
use metallic_core::{CFrame, Color3, DataModel, Id, PropId, Value};
use metallic_render::depth::{DEPTH_CLEAR, DEPTH_FORMAT};
use metallic_render::scene::{GpuInstance, InstanceBinding, RenderComponents};
use metallic_render::{
    Camera, DrawStats, GpuScene, InstanceScene, OffscreenTarget, PassId, PassSet, RenderContext, Renderer,
    RendererSettings, wgpu,
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

/// One pass of a hand-written frame: its label, whether it clears the colour, and its depth use (`Some(true)` writes
/// and clears depth, `Some(false)` tests it read-only, `None` has no depth).
type HandPass = (&'static str, bool, Option<bool>);

/// The default frame's passes (opaque, sky, transparent, UI), as the renderer plans them.
const DEFAULT_PASSES: [HandPass; 4] = [
    ("opaque", true, Some(true)),
    ("sky", false, Some(false)),
    ("transparent", false, Some(false)),
    ("ui", false, None),
];

/// A UI-only frame, as the renderer plans it (UI is the first pass, so it clears).
const UI_ONLY: [HandPass; 1] = [("ui", true, None)];

/// `passes` as plain wgpu calls (no MSAA, no timing): the floor any renderer on wgpu pays for that frame.
fn hand_written_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    passes: &[HandPass],
) -> wgpu::SubmissionIndex {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("metallic frame"),
    });
    for &(label, clear, depth_write) in passes {
        let colors = [Some(wgpu::RenderPassColorAttachment {
            view: target,
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
                store: wgpu::StoreOp::Store,
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
    queue.submit(std::iter::once(encoder.finish()))
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
) -> (u64, u64) {
    let (device, queue) = (context.device(), context.queue());
    let mut renderer = Renderer::new(
        context,
        RendererSettings {
            passes,
            gpu_timing: false,
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .expect("a renderer");
    let mut pacer = Pacer::new();
    let mut ours = Vec::with_capacity(FRAMES);
    let mut floor = Vec::with_capacity(FRAMES);
    for frame in 0..WARM_UP + FRAMES {
        for renderer_turn in [frame % 2 == 0, frame % 2 != 0] {
            let (count, submission) = if renderer_turn {
                allocations_in(|| renderer.render(target.view()))
            } else {
                allocations_in(|| hand_written_frame(device, queue, target.view(), depth, hand))
            };
            pacer.submitted(device, submission);
            if frame >= WARM_UP {
                if renderer_turn { &mut ours } else { &mut floor }.push(count);
            }
        }
    }
    eprintln!(
        "{passes:?}: allocations per frame over {FRAMES} frames: renderer {}..{} (median {}), the same wgpu calls by \
         hand {}..{} (median {})",
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

/// On lavapipe (CI's software Vulkan) a renderer frame and the identical hand-written one differed by a constant 2
/// allocations in some processes and by none in others, while the dev PC's Vulkan, DX12 and GL drivers never
/// differ. `render()` only calls wgpu, so this is state inside wgpu (which pooled objects a frame gets), not ours.
const WGPU_POOL_NOISE: u64 = 2;

#[test]
fn a_frame_allocates_nothing_beyond_wgpus_own() {
    let Some(context) = support::gpu() else { return };
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

    let (ours_one, floor_one) =
        renderer_and_floor(&context, PassSet::EMPTY.with(PassId::Ui), &UI_ONLY, &target, &depth);
    let (ours, floor) = renderer_and_floor(&context, PassSet::default(), &DEFAULT_PASSES, &target, &depth);
    // Per pass: what three more passes cost the renderer is exactly what they cost by hand, so nothing in the pass
    // loop allocates.
    assert!(
        ours.saturating_sub(ours_one) <= floor.saturating_sub(floor_one),
        "three more passes cost the renderer {} allocations and the same wgpu calls {}",
        ours.saturating_sub(ours_one),
        floor.saturating_sub(floor_one)
    );
    // Per frame: nothing beyond wgpu's own, give or take its pooling.
    assert!(
        ours <= floor + WGPU_POOL_NOISE,
        "the renderer adds {} allocation(s) per frame",
        ours - floor
    );
}

#[test]
fn nothing_accumulates_over_frames() {
    let Some(context) = support::gpu() else { return };
    let samples = context.capabilities().msaa.iter().filter(|&n| n > 1).min().unwrap_or(1);
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).unwrap();
    let mut renderer = Renderer::new(
        &context,
        RendererSettings {
            passes: PassSet::ALL,
            msaa_samples: samples,
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .unwrap();
    let mut pacer = Pacer::new();
    let mut allocations = Vec::with_capacity(FRAMES);
    let mut before = 0;
    for frame in 0..WARM_UP + FRAMES {
        if frame == WARM_UP {
            before = live_bytes();
        }
        let (count, submission) = allocations_in(|| renderer.render(target.view()));
        if frame >= WARM_UP {
            allocations.push(count);
        }
        pacer.submitted(context.device(), submission);
    }
    let grown = live_bytes() - before;
    eprintln!(
        "{FRAMES} frames (every pass, {samples}x MSAA, timing {}): heap grew {grown} bytes, allocations per frame {}..{} \
         (median {})",
        renderer.gpu_timing_enabled(),
        allocations.iter().min().unwrap(),
        allocations.iter().max().unwrap(),
        median(allocations.clone())
    );
    assert!(renderer.gpu_timings().is_some() || !renderer.gpu_timing_enabled());
    // Slack for the frames still in flight at either end; anything kept per frame would be far beyond this.
    assert!(
        grown.abs() < 16 * 1024,
        "the heap grew by {grown} bytes over {FRAMES} frames"
    );
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
    context.queue().submit(Some(encoder.finish()));
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
        pacer.submitted(device, queue.submit([encoder.finish(), by_hand.finish()]));
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
    let Some(context) = support::gpu() else { return };
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
    let Some(context) = support::gpu() else { return };
    let mut game = SceneGame::new();
    let mut scene = scene_on_gpu(&context, &mut game);
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).unwrap();
    let mut renderer = Renderer::new(&context, RendererSettings::new(FORMAT, SIZE, SIZE)).unwrap();
    let mut pacer = Pacer::new();
    let mut before = 0;
    for frame in 0..WARM_UP + FRAMES {
        game.edit(frame);
        if frame == WARM_UP {
            before = live_bytes();
        }
        scene.sync(&mut game.dm);
        let submission = renderer.render_with(target.view(), |encoder| {
            scene.upload(encoder);
        });
        pacer.submitted(context.device(), submission);
    }
    let grown = live_bytes() - before;
    eprintln!("{FRAMES} frames of sync, upload and render: heap grew {grown} bytes");
    assert!(
        grown.abs() < 16 * 1024,
        "the heap grew by {grown} bytes over {FRAMES} frames"
    );
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
}

impl HandParts {
    fn new(device: &wgpu::Device, instances: u64) -> Self {
        let source = concat!(
            include_str!("../src/parts/parts.wgsl"),
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
        let sorted = buffer(instances * 4, storage);
        let frame = buffer(256, wgpu::BufferUsages::UNIFORM);
        let materials = buffer(8192, wgpu::BufferUsages::UNIFORM);
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let pipeline = |entry_point: &str, write: bool, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("hand parts"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
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
                depth_stencil: Some(metallic_render::depth::depth_stencil_state(write)),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry_point),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
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
        let groups = |pipeline: &wgpu::RenderPipeline, list: &wgpu::Buffer| {
            let group = |index: u32, buffers: &[&wgpu::Buffer]| {
                let entries: Vec<_> = buffers
                    .iter()
                    .enumerate()
                    .map(|(binding, buffer)| wgpu::BindGroupEntry {
                        binding: binding as u32,
                        resource: buffer.as_entire_binding(),
                    })
                    .collect();
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("hand parts"),
                    layout: &pipeline.get_bind_group_layout(index),
                    entries: &entries,
                })
            };
            [
                group(0, &[&frame, &materials]),
                group(1, &[&regions[0], &regions[1], list]),
            ]
        };
        let opaque_groups = groups(&opaque, &entries);
        let transparent_groups = groups(&transparent, &sorted);
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
        Self {
            opaque,
            transparent,
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
            sorted_bytes: vec![0; (instances * 4) as usize],
        }
    }

    /// The renderer's default passes with `opaque` and `transparent` instanced draws, after writing each kind's
    /// `visible` list entries (one staging write and a copy per list, as a culled frame uploads its lists), the frame
    /// uniform and `sorted` transparent entries: the wgpu calls `Renderer::render_scene` makes.
    fn frame(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        (opaque, transparent, sorted): (u32, u32, usize),
        visible: &[u32],
    ) -> wgpu::SubmissionIndex {
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
        queue.write_buffer(&self.frame, 0, &[0; 160]);
        queue.write_buffer(&self.sorted, 0, &self.sorted_bytes[..sorted * 4]);
        for &(label, clear, depth_write) in &DEFAULT_PASSES {
            let colors = [Some(wgpu::RenderPassColorAttachment {
                view: target,
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
                "transparent" => (&self.transparent, &self.transparent_groups, transparent),
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
        queue.submit(std::iter::once(encoder.finish()))
    }
}

/// The fewest allocations of a parts frame of `parts` parts (every `see_through`th one see-through, blocks and
/// balls in turn) and of the same wgpu calls by hand, over [`FRAMES`] frames each in alternating order, with the
/// frame's draw statistics.
/// What `Renderer::render_scene` draws of `scene` seen from `camera` on the [`SIZE`]-square target with the default
/// culling: (opaque draws, transparent draws, see-through instances) and each kind's visible instances. Culls
/// `scene` (a CPU-only copy of the drawn one) and sorts its see-through instances as the renderer does.
fn expected_calls(
    scene: &mut InstanceScene,
    visible: &mut metallic_render::VisibleSet,
    keys: &mut Vec<(u32, u32)>,
    camera: &Camera,
) -> ((u32, u32, usize), [u32; 4]) {
    let settings = metallic_render::CullSettings::default();
    let view = metallic_render::CullView::for_camera(camera, scene.origin(), SIZE, SIZE, &settings);
    scene.cull(&view, visible);
    let mut lists = [0u32; 4];
    for (kind, n) in lists.iter_mut().enumerate() {
        *n = visible.opaque(metallic_render::scene::MeshKind(kind as u16)).len() as u32;
    }
    let opaque_draws = lists.iter().filter(|&&n| n > 0).count() as u32;
    // Back to front, then one draw per run of the same mesh (`parts/transparent.rs`).
    let eye = camera.position - scene.origin();
    keys.clear();
    keys.extend(visible.transparent().iter().map(|&entry| {
        let distance = scene
            .entry_instance(entry)
            .map_or(0.0, |instance| instance.translation().distance_squared(eye));
        (!distance.max(0.0).to_bits(), entry)
    }));
    keys.sort_unstable();
    let mut runs = 0;
    let mut last = None;
    for &(_, entry) in keys.iter() {
        let kind = scene.entry_instance(entry).map(GpuInstance::mesh);
        if kind != last {
            runs += 1;
            last = kind;
        }
    }
    ((opaque_draws, runs, keys.len()), lists)
}

fn parts_frame_and_floor(context: &RenderContext, parts: usize, see_through: usize) -> (u64, u64, DrawStats) {
    let (device, queue) = (context.device(), context.queue());
    let mut game = support::scenes::Game::new();
    // Every kind opaque, and see-through blocks and balls mixed so the transparent pass makes several draws.
    support::scenes::grid(&mut game, parts, 100, support::scenes::SeeThrough::Mixed(see_through));
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
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .expect("a renderer");
    let hand = HandParts::new(device, parts as u64);
    // The same game culled on the CPU alone, so the hand-written frame makes exactly the calls the renderer makes
    // in the same frame, whichever of the two goes first.
    let mut mirror = InstanceScene::attach(&mut game.dm, Some(game.workspace), &RenderComponents::builtin())
        .expect("a mirror scene");
    mirror.sync(&mut game.dm);
    mirror.coalesce();
    let mut mirror_visible = metallic_render::VisibleSet::new();
    let mut sort_keys = Vec::new();
    let (mut ours, mut floor) = (Vec::with_capacity(FRAMES), Vec::with_capacity(FRAMES));
    for frame in 0..WARM_UP + FRAMES {
        let camera = orbiting_camera(frame);
        // The camera moves every frame, so every frame culls again and uploads each kind's visible list.
        let (calls, visible) = expected_calls(&mut mirror, &mut mirror_visible, &mut sort_keys, &camera);
        for renderer_turn in [frame % 2 == 0, frame % 2 != 0] {
            let (count, submission) = if renderer_turn {
                let drawn = allocations_in(|| renderer.render_scene(target.view(), &mut scene, Some(&camera)));
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
                drawn
            } else {
                allocations_in(|| hand.frame(device, queue, target.view(), calls, &visible))
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
        "parts frame ({} opaque + {} transparent draws, {} instances): allocations {}..{} (median {}); the same wgpu \
         calls by hand {}..{} (median {})",
        stats.opaque_draws,
        stats.transparent_draws,
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

/// What a frame may allocate beyond the same wgpu calls by hand on `context`'s adapter, for the machine rather than
/// for us.
///
/// - **Hardware adapters: nothing.** Every one on the dev PC (Vulkan, DX12, GL) shows 0, and so does WARP there.
/// - **Software adapters** (`DeviceType::Cpu`: lavapipe, llvmpipe, WARP) on CI's runners: up to 6. They showed a
///   constant excess on every frame, whether or not frames overlapped, the same for both workloads of each test:
///   lavapipe 4 and WARP 3 on the parts frames before culling (#77), WARP 5 with it (#79), lavapipe 3 on the
///   overlay frames. The cause is in the runner environment, not found yet (no validation message was reported,
///   and the dev PC's WARP shows 0 for the same calls); that the excess does not change between two very different
///   workloads, which every test also checks, is what shows it is not the renderer's.
fn machine_noise(context: &RenderContext) -> u64 {
    if context.capabilities().device_type == wgpu::DeviceType::Cpu {
        6
    } else {
        0
    }
}

#[test]
fn a_parts_frame_allocates_nothing_beyond_wgpus_own() {
    let Some(context) = support::gpu() else { return };
    if InstanceBinding::for_context(&context) != InstanceBinding::Storage {
        eprintln!("skipped: the hand-written frame is the storage path's");
        return;
    }
    // Two workloads, the second with three times the parts and many more see-through parts, sorted runs and draws.
    let (ours_small, floor_small, small) = parts_frame_and_floor(&context, 2_000, 25);
    let (ours_large, floor_large, large) = parts_frame_and_floor(&context, 6_000, 8);
    assert_eq!((small.opaque_draws, large.opaque_draws), (4, 4));
    assert!(
        small.transparent_draws > 1 && large.transparent_draws > 4 * small.transparent_draws,
        "{small:?} {large:?}"
    );
    let (extra_small, extra_large) = (
        ours_small.saturating_sub(floor_small),
        ours_large.saturating_sub(floor_large),
    );
    // Nothing the renderer does allocates per part, per draw, per sorted instance or per run: whatever a frame
    // allocates beyond wgpu's own is the same for both workloads...
    assert_eq!(
        extra_small, extra_large,
        "the renderer's own allocations grow with the work: {extra_small} for {small:?}, {extra_large} for \
         {large:?}"
    );
    // ...and no more than the machine's noise (none on a hardware adapter).
    assert!(
        extra_small <= machine_noise(&context),
        "drawing parts adds {extra_small} allocation(s) per frame on {:?}",
        context.capabilities().device_type
    );
}

/// Frames of moving parts seen from an orbiting camera, so every frame culls again and uploads the visible lists,
/// on both instance paths (the vertex-buffer fallback's lists hold copies of the visible instances).
#[test]
fn nothing_accumulates_over_parts_frames() {
    for force_vertex_instances in [false, true] {
        let options = metallic_render::ContextOptions {
            force_vertex_instances,
            ..metallic_render::ContextOptions::default()
        };
        let Some(context) = support::gpu_with(&options) else {
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
    let mut pacer = Pacer::new();
    let mut before = 0;
    for frame in 0..WARM_UP + FRAMES {
        // 1% of the parts move, the same ones every frame.
        for (i, &id) in parts.iter().step_by(100).enumerate() {
            let place = CFrame::new(Vec3::new(i as f32 * 4.0, 1.0 + frame as f32 * 0.01, 0.0));
            game.dm.set_by_name(id, "CFrame", Value::CFrame(place)).expect("moved");
        }
        if frame == WARM_UP {
            before = live_bytes();
        }
        scene.sync(&mut game.dm);
        let camera = orbiting_camera(frame);
        let submission = renderer.render_scene(target.view(), &mut scene, Some(&camera));
        pacer.submitted(context.device(), submission);
    }
    let grown = live_bytes() - before;
    eprintln!(
        "{FRAMES} frames on the {:?} path drawing {:?}: heap grew {grown} bytes",
        InstanceBinding::for_context(context),
        renderer.draw_stats()
    );
    assert!(renderer.draw_stats().culled_instances > 0, "nothing was culled");
    assert!(
        grown.abs() < 16 * 1024,
        "the heap grew by {grown} bytes over {FRAMES} frames"
    );
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
        let before = live_bytes();
        let (count, stats) = allocations_in(|| {
            scene.sync(&mut game.dm);
            scene.coalesce();
            let view = metallic_render::CullView::for_camera(&camera, scene.origin(), 1920, 1080, &settings);
            scene.cull(&view, &mut visible)
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
    fn frame(&self, device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::TextureView) -> wgpu::SubmissionIndex {
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
        queue.submit(std::iter::once(encoder.finish()))
    }
}

/// The fewest allocations of a UI-only frame whose overlay is redrawn with `work` (so it is encoded and uploaded
/// every frame) and of the same wgpu calls by hand, over [`FRAMES`] frames each in alternating order, with the
/// overlay's quad count.
fn overlay_frame_and_floor(context: &RenderContext, work: OverlayWork) -> (u64, u64, usize) {
    let (device, queue) = (context.device(), context.queue());
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
                allocations_in(|| hand.frame(device, queue, target.view()))
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

/// Like the parts frame above: CI's lavapipe showed a constant 3 allocations beyond the hand-written overlay frame
/// (65 against 62 for about 260 quads) where every adapter on the dev PC shows 0, the same runner-only excess the
/// parts frames show ([`machine_noise`]). The renderer's overlay frame and the hand-written one make the same wgpu
/// calls, so the check is that the excess does not change with the overlay's size: nothing per quad, per glyph or
/// per line of text allocates.
#[test]
fn an_overlay_frame_allocates_nothing_beyond_wgpus_own() {
    let Some(context) = support::gpu() else { return };
    let (ours_small, floor_small, small) = overlay_frame_and_floor(&context, SMALL_OVERLAY);
    let (ours_large, floor_large, large) = overlay_frame_and_floor(&context, LARGE_OVERLAY);
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
    // ...and no more than the machine's noise (none on a hardware adapter).
    assert!(
        extra_small <= machine_noise(&context),
        "the overlay adds {extra_small} allocation(s) per frame on {:?}",
        context.capabilities().device_type
    );
}

#[test]
fn nothing_accumulates_over_overlay_frames() {
    let Some(context) = support::gpu() else { return };
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).unwrap();
    let mut renderer = Renderer::new(&context, RendererSettings::new(FORMAT, SIZE, SIZE)).unwrap();
    renderer.set_overlay_enabled(true);
    let mut pacer = Pacer::new();
    let mut before = 0;
    for frame in 0..WARM_UP + FRAMES {
        if frame == WARM_UP {
            before = live_bytes();
        }
        fill_overlay(renderer.overlay_mut(), frame, PERF_OVERLAY);
        let submission = renderer.render(target.view());
        pacer.submitted(context.device(), submission);
    }
    let grown = live_bytes() - before;
    eprintln!("{FRAMES} overlay frames: heap grew {grown} bytes");
    assert!(
        grown.abs() < 16 * 1024,
        "the heap grew by {grown} bytes over {FRAMES} frames"
    );
}
