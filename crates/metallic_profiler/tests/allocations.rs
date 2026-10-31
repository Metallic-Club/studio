//! Recording a frame never allocates once the profiler is enabled.
use metallic_profiler::{GpuPass, Profiler, Subsystem};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;

thread_local! {
    static COUNT: Cell<u64> = const { Cell::new(0) };
}

// SAFETY: forwards every call to the system allocator unchanged; it only counts.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = COUNT.try_with(|c| c.set(c.get() + 1));
        // SAFETY: the caller's layout is passed through unchanged.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `System.alloc` with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = COUNT.try_with(|c| c.set(c.get() + 1));
        // SAFETY: as for `alloc` and `dealloc`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn frame() {
    Profiler::begin_frame();
    {
        let _a = Profiler::scope(Subsystem::Physics, "physics");
        let _b = Profiler::scope(Subsystem::Script, "scripts");
        Profiler::charge_script(3, 10);
    }
    let frame = Profiler::current_frame().unwrap_or(0);
    Profiler::end_frame();
    Profiler::record_gpu(
        frame,
        &[GpuPass {
            name: "scene",
            ns: 5,
        }],
    );
}

#[test]
fn recording_frames_does_not_allocate() {
    // Off: nothing at all, not even the profiler's state.
    let before = COUNT.with(Cell::get);
    for _ in 0..100 {
        frame();
    }
    assert_eq!(COUNT.with(Cell::get), before, "disabled profiler allocated");

    Profiler::set_enabled(true);
    frame();
    let before = COUNT.with(Cell::get);
    // More than the history, so the ring wraps.
    for _ in 0..1500 {
        frame();
    }
    let latest = Profiler::latest();
    assert_eq!(COUNT.with(Cell::get), before, "recording allocated");
    assert_eq!(latest.map(|f| f.scopes().len()), Some(2));
}
