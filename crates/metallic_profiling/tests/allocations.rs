#![cfg(not(any(feature = "puffin", feature = "tracy")))]

use metallic_profiling::{BudgetTable, Clock, FrameProfiler, Subsystem};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::Duration;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

struct CountingAllocator;

fn count_allocation() {
    let _ = ALLOCATIONS.try_with(|counter| {
        if let Some(count) = counter.get() {
            counter.set(Some(count + 1));
        }
    });
}

// Forward every allocation unchanged; only the current test thread's enabled window is counted.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count_allocation();
        unsafe { System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

struct ManualClock(Cell<u64>);
impl Clock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_micros(self.0.get())
    }
}

#[test]
fn default_recording_and_budget_assessment_allocate_nothing() {
    let clock = ManualClock(Cell::new(0));
    let mut profiler = FrameProfiler::<_, 4>::new(&clock);
    let budgets = BudgetTable::proposed_dev_60();
    ALLOCATIONS.with(|counter| counter.set(Some(0)));
    let frame = profiler.begin_frame();
    for _ in 0..100 {
        metallic_profiling::cpu_scope!(frame, Subsystem::Scripts, "script tick");
        clock.0.set(clock.0.get() + 1);
    }
    let report = frame.finish();
    let checks = report.check_budgets(&budgets);
    std::hint::black_box(checks.overall_state());
    std::hint::black_box(checks);
    std::hint::black_box(report.samples().count());
    let allocations = ALLOCATIONS.with(|counter| counter.replace(None));
    assert_eq!(allocations, Some(0));
    assert_eq!(report.scope_count(Subsystem::Scripts), 100);
    assert_eq!(report.dropped_samples(), 96);
}
