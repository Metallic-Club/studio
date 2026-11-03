//! The frame path allocates nothing of its own (ADR 0008 §9), measured with a counting global allocator on the
//! offscreen path over 100 frames after a warm-up.
//!
//! wgpu itself allocates while recording (wgpu-core boxes every pass and command buffer: about 18 allocations per
//! render pass and 20 per submit on wgpu 30), and no renderer on top of it can avoid that. So the check is relative:
//! a frame through [`Renderer::render`] allocates no more than the same wgpu calls written out by hand, which means
//! everything the renderer adds (planning, attachment selection, timing bookkeeping) is allocation-free. A second
//! test checks nothing accumulates: the heap does not grow over 100 frames with timing, MSAA and every pass on.
//!
//! Counters are per thread, so tests running in parallel do not see each other's allocations.

// A `#[global_allocator]` needs `unsafe impl GlobalAlloc`. This test binary's allocator only counts, then forwards
// every call unchanged to the system allocator.
#![allow(unsafe_code)]

mod support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use metallic_render::depth::{DEPTH_CLEAR, DEPTH_FORMAT};
use metallic_render::{OffscreenTarget, PassSet, Renderer, RendererSettings, wgpu};

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

/// The default frame (opaque, sky, transparent, UI; no MSAA, no timing) as plain wgpu calls: the floor any renderer
/// on wgpu pays.
fn hand_written_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &wgpu::TextureView,
    depth: &wgpu::TextureView,
) -> wgpu::SubmissionIndex {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("metallic frame"),
    });
    let passes: [(&str, bool, Option<bool>); 4] = [
        ("opaque", true, Some(true)),
        ("sky", false, Some(false)),
        ("transparent", false, Some(false)),
        ("ui", false, None),
    ];
    for (label, clear, depth_write) in passes {
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

#[test]
fn a_frame_allocates_nothing_beyond_wgpus_own() {
    let Some(context) = support::gpu() else { return };
    let (device, queue) = (context.device(), context.queue());
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).unwrap();
    let mut renderer = Renderer::new(
        &context,
        RendererSettings {
            gpu_timing: false,
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .unwrap();
    assert_eq!(renderer.settings().passes, PassSet::default());
    let depth = device
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

    let mut pacer = Pacer::new();
    let mut ours = Vec::with_capacity(FRAMES);
    let mut floor = Vec::with_capacity(FRAMES);
    for frame in 0..WARM_UP + FRAMES {
        // Alternate which goes first: wgpu frees finished submissions (and allocates while doing so) inside whichever
        // call comes after a wait, so a fixed order would charge that to one side.
        for renderer_turn in [frame % 2 == 0, frame % 2 != 0] {
            if renderer_turn {
                let (count, submission) = allocations_in(|| renderer.render(target.view()));
                pacer.submitted(device, submission);
                if frame >= WARM_UP {
                    ours.push(count);
                }
            } else {
                let (count, submission) = allocations_in(|| hand_written_frame(device, queue, target.view(), &depth));
                pacer.submitted(device, submission);
                if frame >= WARM_UP {
                    floor.push(count);
                }
            }
        }
    }
    eprintln!(
        "allocations per frame over {FRAMES} frames: renderer {}..{} (median {}), the same wgpu calls by hand {}..{} \
         (median {})",
        ours.iter().min().unwrap(),
        ours.iter().max().unwrap(),
        median(ours.clone()),
        floor.iter().min().unwrap(),
        floor.iter().max().unwrap(),
        median(floor.clone())
    );
    // DIAGNOSTIC (temporary): which input makes lavapipe differ.
    let measure = |f: &mut dyn FnMut() -> wgpu::SubmissionIndex, pacer: &mut Pacer| {
        let mut counts = Vec::with_capacity(40);
        for i in 0..40 {
            let (count, submission) = allocations_in(&mut *f);
            pacer.submitted(device, submission);
            if i >= 10 {
                counts.push(count);
            }
        }
        (*counts.iter().min().unwrap(), *counts.iter().max().unwrap())
    };
    let renderer_view = target.view();
    eprintln!(
        "DIAG renderer {:?}",
        measure(&mut || renderer.render(renderer_view), &mut pacer)
    );
    eprintln!(
        "DIAG hand {:?}",
        measure(
            &mut || hand_written_frame(device, queue, renderer_view, &depth),
            &mut pacer
        )
    );
    let mut no_timing_settings = *renderer.settings();
    no_timing_settings.passes = PassSet::default();
    let mut second = Renderer::new(&context, no_timing_settings).unwrap();
    eprintln!(
        "DIAG second renderer {:?}",
        measure(&mut || second.render(renderer_view), &mut pacer)
    );
    let mut single = Renderer::new(
        &context,
        RendererSettings {
            gpu_timing: false,
            passes: PassSet::EMPTY.with(metallic_render::PassId::Ui),
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .unwrap();
    eprintln!(
        "DIAG ui-only renderer {:?}",
        measure(&mut || single.render(renderer_view), &mut pacer)
    );
    eprintln!(
        "DIAG empty submit {:?}",
        measure(
            &mut || {
                let encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("metallic frame"),
                });
                queue.submit(std::iter::once(encoder.finish()))
            },
            &mut pacer
        )
    );
    eprintln!(
        "DIAG ui-only hand {:?}",
        measure(
            &mut || {
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("metallic frame"),
                });
                let colors = [Some(wgpu::RenderPassColorAttachment {
                    view: renderer_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })];
                drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ui"),
                    color_attachments: &colors,
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                }));
                queue.submit(std::iter::once(encoder.finish()))
            },
            &mut pacer
        )
    );
    // The minimum is a frame with no maintenance by wgpu in it: exactly the recording cost.
    let (ours, floor) = (*ours.iter().min().unwrap(), *floor.iter().min().unwrap());
    assert!(
        ours <= floor,
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
