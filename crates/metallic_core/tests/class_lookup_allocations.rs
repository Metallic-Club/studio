//! Registry lookups allocate nothing, on a hit or a miss (design section 4.2).
use metallic_core::classes::{self, registry};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

struct CountingAllocator;

fn count() {
    if COUNTING.try_with(Cell::get).unwrap_or(false) {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
    }
}

// Counting is thread-local so unrelated test-runner activity cannot make this flaky.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        // SAFETY: forward the allocator contract unchanged to System.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: pointer and layout are supplied by the GlobalAlloc caller.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        // SAFETY: forward the allocator contract unchanged to System.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn member_and_class_lookups_do_not_allocate() {
    let part = registry().class_by_name("Part").unwrap();
    COUNTING.with(|counting| counting.set(true));
    for name in ["Size", "Name", "GetPivot", "NotAMember", ""] {
        black_box(part.member(black_box(name)));
        black_box(part.property(black_box(name)));
        black_box(part.default_of(black_box(name)));
    }
    for name in ["Part", "SpawnLocation", "NotAClass", ""] {
        black_box(registry().class_id(black_box(name)));
        black_box(classes::find_property(black_box(name), "Size"));
        black_box(classes::default_value_ref(black_box(name), "Name"));
        black_box(classes::property_index(black_box(name), "Size"));
        black_box(classes::is_subclass(black_box(name), "BasePart"));
    }
    COUNTING.with(|counting| counting.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}
