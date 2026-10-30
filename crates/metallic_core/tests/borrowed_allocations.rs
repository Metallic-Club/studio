use metallic_core::classes::default_value_ref;
use metallic_core::{Color3, InstanceId, Value, Vec3, World};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

#[derive(Clone, Copy, Debug, Default)]
struct Counts {
    allocations: usize,
    allocated: usize,
    freed: usize,
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

// Counting is thread-local so unrelated test-runner activity cannot make zero-allocation reads flaky.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forward the allocator contract unchanged to System.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record(layout.size(), 0, 1);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forward the allocator contract unchanged to System.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            record(layout.size(), 0, 1);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        record(0, layout.size(), 0);
        // SAFETY: pointer and layout are supplied by the GlobalAlloc caller.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: forward the allocator contract unchanged to System.
        let result = unsafe { System.realloc(pointer, layout, new_size) };
        if !result.is_null() {
            record(new_size, layout.size(), 1);
        }
        result
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn measure<T>(operation: impl FnOnce() -> T) -> (T, Counts) {
    COUNTS.set(Counts::default());
    ENABLED.set(true);
    let result = operation();
    ENABLED.set(false);
    (result, COUNTS.get())
}

fn level(count: usize) -> (World, Vec<InstanceId>) {
    let mut world = World::new_with_services();
    let model = world.create("Model", world.root("Workspace")).unwrap();
    let mut parts = Vec::with_capacity(count);
    for index in 0..count {
        let part = world.create("Part", Some(model)).unwrap();
        world
            .set(
                part,
                "Position",
                Value::Vector3(Vec3::new(index as f64, 1.0, 0.0)),
            )
            .unwrap();
        world
            .set(part, "Size", Value::Vector3(Vec3::new(4.0, 1.0, 8.0)))
            .unwrap();
        world
            .set(
                part,
                "Color",
                Value::Color3(Color3::new(120.0, 90.0, 200.0)),
            )
            .unwrap();
        world.set(part, "Anchored", Value::Bool(true)).unwrap();
        parts.push(part);
    }
    (world, parts)
}

#[test]
fn borrowed_hot_reads_allocate_nothing_and_owned_reads_keep_independent_strings() {
    let (_, cache) = measure(|| black_box(default_value_ref("Part", "Material")));
    let ((mut world, parts), fixture) = measure(|| level(5_000));
    let properties = [
        "Position",
        "Orientation",
        "Size",
        "Anchored",
        "CanCollide",
        "Material",
        "Shape",
    ];
    let (_, borrowed) = measure(|| {
        for &part in &parts {
            for property in properties {
                black_box(world.get_ref(black_box(part), black_box(property)).unwrap());
            }
        }
    });
    let (_, owned) = measure(|| {
        for &part in &parts {
            for property in properties {
                black_box(world.get(black_box(part), black_box(property)).unwrap());
            }
        }
    });
    assert_eq!(borrowed.allocations, 0);
    assert_eq!(borrowed.allocated, 0);
    assert_eq!(owned.allocations, 2 * parts.len());

    let script = world.create("Script", None).unwrap();
    world
        .set(
            script,
            "Source",
            Value::String("stored source ".repeat(128)),
        )
        .unwrap();
    let (_, stored_and_default_strings) = measure(|| {
        for _ in 0..10_000 {
            black_box(world.get_ref(script, "Source").unwrap());
            black_box(world.get_ref(script, "Name").unwrap());
        }
    });
    assert_eq!(stored_and_default_strings.allocations, 0);
    assert_eq!(stored_and_default_strings.allocated, 0);

    eprintln!("shared_default_cache: {cache:?}");
    eprintln!(
        "fixture_5000_parts: {fixture:?}; live_requested_bytes_per_part={:.3}",
        (fixture.allocated - fixture.freed) as f64 / parts.len() as f64
    );
    eprintln!("seven_property_reads_5000_parts: borrowed={borrowed:?}; owned={owned:?}");
    eprintln!("stored_and_default_string_reads_20000: {stored_and_default_strings:?}");
}
