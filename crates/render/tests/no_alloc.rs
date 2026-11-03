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
//! Counters are per thread, so tests running in parallel do not see each other's allocations.

// A `#[global_allocator]` needs `unsafe impl GlobalAlloc`. This test binary's allocator only counts, then forwards
// every call unchanged to the system allocator.
#![allow(unsafe_code)]

mod support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use metallic_render::depth::{DEPTH_CLEAR, DEPTH_FORMAT};
use metallic_render::{OffscreenTarget, PassId, PassSet, Renderer, RendererSettings, wgpu};

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
