//! Allocation guard for the frame statistics (#78, ADR 0008 §9), measured with a counting global allocator: once
//! made, recording a frame, summarising the window and drawing the performance overlay allocate nothing, with the
//! window filling and then wrapping. (GPU timings only come from a device; recording them copies a fixed-size
//! array into the same kind of ring.)
//!
//! Counters are per thread, so tests running in parallel do not see each other's allocations.
#![cfg(feature = "render")]
// A `#[global_allocator]` needs `unsafe impl GlobalAlloc`. This test binary's allocator only counts, then forwards
// every call unchanged to the system allocator.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use metallic_engine::perf::{FrameSample, PerfStats, draw_overlay};
use metallic_render::{DrawStats, Overlay};

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

fn record() {
    // `try_with`: the allocator also runs while a thread's locals are being torn down.
    let _ = ALLOCATIONS.try_with(|c| c.set(c.get() + 1));
}

struct Counting;

// SAFETY: every method forwards its arguments unchanged to `System`, which upholds the `GlobalAlloc` contract;
// the counting touches only a const-initialised thread-local without a destructor, which never allocates.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record();
        // SAFETY: the caller's `layout` is passed through unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record();
        // SAFETY: the caller's `layout` is passed through unchanged.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `System` with this `layout`, as the caller guarantees.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record();
        // SAFETY: `ptr`, `layout` and `new_size` are the caller's, passed through unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// How many allocations (and reallocations) `f` makes on this thread.
fn allocations_in(f: impl FnOnce()) -> u64 {
    let before = ALLOCATIONS.with(Cell::get);
    f();
    ALLOCATIONS.with(Cell::get) - before
}

const WINDOW: usize = 600;

fn sample(frame: usize) -> FrameSample {
    let ms = 2.0 + (frame % 37) as f32 * 0.25;
    FrameSample {
        frame_ms: ms,
        script_ms: ms * 0.05,
        physics_ms: ms * 0.02,
        render_ms: ms * 0.3,
        cull_ms: ms * 0.05,
        shadow_redrawn: 0,
    }
}

#[test]
fn recording_summarising_and_drawing_the_overlay_allocate_nothing() {
    let mut stats = PerfStats::new(WINDOW);
    let mut overlay = Overlay::new();
    let draws = DrawStats {
        opaque_draws: 4,
        transparent_draws: 12,
        instances: 10_100,
        triangles: 1_200_000,
        opaque_instances: 10_000,
        opaque_triangles: 1_190_000,
        transparent_instances: 100,
        transparent_triangles: 10_000,
        shadow_cascades: 3,
        shadow_redrawn: 0b011,
        shadow_draws: 8,
        shadow_instances: 4_000,
        shadow_casters: [1_000, 3_000, 20_000, 0],
        ..DrawStats::default()
    };
    // Twice the window: the rings fill, then wrap.
    let mut counts = Vec::with_capacity(2 * WINDOW);
    for frame in 0..2 * WINDOW {
        let count = allocations_in(|| {
            stats.record(sample(frame), None, draws);
            let summary = stats.summary();
            draw_overlay(&mut overlay, &stats, &summary);
        });
        counts.push(count);
    }
    assert_eq!(stats.summary().frames, WINDOW);
    assert_eq!(overlay.dropped(), 0);
    let total: u64 = counts.iter().sum();
    assert_eq!(total, 0, "allocations per frame: {:?}", &counts[..16]);
}
