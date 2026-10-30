//! `InputRuntime::evaluate` (and `push`, `drain_events`) must not allocate once the runtime is
//! built (input spec section 13). A counting global allocator checks it on the reference load:
//! 4 contexts, 32 actions, 96 bindings, 40 raw events a frame.
use metallic_runtime::InputRuntime;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

#[path = "common/reference_load.rs"]
mod reference_load;

struct Counting;

thread_local! {
    static ON: Cell<bool> = const { Cell::new(false) };
    static COUNT: Cell<u64> = const { Cell::new(0) };
}

fn note() {
    let _ = ON.try_with(|on| {
        if on.get() {
            let _ = COUNT.try_with(|c| c.set(c.get() + 1));
        }
    });
}

// SAFETY: forwards every call to the system allocator; it only counts calls on the thread that
// switched counting on.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[test]
fn evaluate_makes_no_heap_allocation() {
    let mut rt = InputRuntime::new(&reference_load::config());
    let dt = 1.0 / 60.0;
    // Warm up so every path has run once.
    for f in 0..120u64 {
        let t = 10.0 + f as f64 * dt;
        reference_load::push_frame(&mut rt, f, t);
        rt.evaluate(t, dt as f32);
        rt.drain_events().for_each(drop);
    }
    COUNT.with(|c| c.set(0));
    ON.with(|on| on.set(true));
    let mut events = 0;
    for f in 120..720u64 {
        let t = 10.0 + f as f64 * dt;
        reference_load::push_frame(&mut rt, f, t);
        rt.evaluate(t, dt as f32);
        events += rt.drain_events().count();
    }
    let allocations = COUNT.with(Cell::get);
    // The counter works: a plain allocation is seen.
    drop(std::hint::black_box(vec![0u8; 64]));
    assert_eq!(
        COUNT.with(Cell::get),
        allocations + 1,
        "the counting allocator is live"
    );
    ON.with(|on| on.set(false));
    assert!(events > 1000, "the load did something: {events} events");
    assert_eq!(allocations, 0, "push, evaluate and drain_events allocated");
}
