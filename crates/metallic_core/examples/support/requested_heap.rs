//! Thread-local requested-heap accounting for standalone measurement programs.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default)]
/// Successful allocation/reallocation requests and released layout sizes in one interval.
pub struct Counts {
    /// Number of allocation or reallocation calls that succeeded.
    pub allocations: usize,
    /// Requested bytes, counting the new size of a successful reallocation.
    pub allocated: usize,
    /// Released requested bytes, including the old size of successful reallocations.
    pub freed: usize,
}

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static COUNTS: Cell<Counts> = const { Cell::new(Counts { allocations: 0, allocated: 0, freed: 0 }) };
}

struct CountingAllocator;

fn record(allocated: usize, freed: usize, allocations: usize) {
    if ENABLED.try_with(Cell::get).unwrap_or(false) {
        let _ = COUNTS.try_with(|counts| {
            let old = counts.get();
            counts.set(Counts {
                allocations: old.allocations + allocations,
                allocated: old.allocated + allocated,
                freed: old.freed + freed,
            });
        });
    }
}

// SAFETY: every operation forwards its pointer/layout contract unchanged to System.
// The bookkeeping uses allocation-free thread-local Cells and never dereferences user memory.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies a valid allocation layout.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record(layout.size(), 0, 1);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies a valid allocation layout.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            record(layout.size(), 0, 1);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        record(0, layout.size(), 0);
        // SAFETY: the caller supplies the original pointer and layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: all arguments are forwarded under GlobalAlloc's contract.
        let result = unsafe { System.realloc(pointer, layout, new_size) };
        if !result.is_null() {
            record(new_size, layout.size(), 1);
        }
        result
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

struct StopCounting;
impl Drop for StopCounting {
    fn drop(&mut self) {
        ENABLED.set(false);
    }
}

/// Count only allocations made on this thread during the closure; retain its returned objects.
pub fn measure<T>(operation: impl FnOnce() -> T) -> (T, Counts) {
    assert!(
        !ENABLED.get(),
        "nested measurement intervals are unsupported"
    );
    COUNTS.set(Counts::default());
    ENABLED.set(true);
    let guard = StopCounting;
    let result = operation();
    drop(guard);
    (result, COUNTS.get())
}
