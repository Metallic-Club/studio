use metallic_core::classes::default_value_ref;
use metallic_core::{Changes, CoreError, InstanceId, Value, Vec3, World};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

const WRITES: usize = 1_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    allocations: usize,
    requested_bytes: usize,
}

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static COUNTS: Cell<Counts> = const { Cell::new(Counts { allocations: 0, requested_bytes: 0 }) };
}

struct CountingAllocator;

fn record(bytes: usize) {
    if ENABLED.try_with(Cell::get).unwrap_or(false) {
        let _ = COUNTS.try_with(|counts| {
            let old = counts.get();
            counts.set(Counts {
                allocations: old.allocations + 1,
                requested_bytes: old.requested_bytes + bytes,
            });
        });
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forward the allocator contract unchanged to System.
        let result = unsafe { System.alloc(layout) };
        if !result.is_null() {
            record(layout.size());
        }
        result
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forward the allocator contract unchanged to System.
        let result = unsafe { System.alloc_zeroed(layout) };
        if !result.is_null() {
            record(layout.size());
        }
        result
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: pointer and layout are supplied by the GlobalAlloc caller.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: forward the allocator contract unchanged to System.
        let result = unsafe { System.realloc(pointer, layout, size) };
        if !result.is_null() {
            record(size);
        }
        result
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

struct CountingScope;

impl Drop for CountingScope {
    fn drop(&mut self) {
        ENABLED.set(false);
    }
}

fn measure(operation: impl FnOnce()) -> Counts {
    assert!(!ENABLED.get(), "allocation measurements cannot nest");
    COUNTS.set(Counts::default());
    ENABLED.set(true);
    let scope = CountingScope;
    operation();
    drop(scope);
    COUNTS.get()
}

fn position(x: f64) -> Value {
    Value::Vector3(Vec3::new(x, 1.0, 0.0))
}

fn fixture() -> (World, InstanceId) {
    // Initialize the shared default registry and grow the journal through its first eviction.
    // Neither fixture allocation nor first-insertion/tree capacity is part of a warmed update.
    black_box(default_value_ref("Part", "Position").unwrap());
    let mut world = World::new_empty();
    let part = world.create("Part", None).unwrap();
    for index in 0..20_000 {
        world.set(part, "Position", position(index as f64)).unwrap();
    }
    world.set(part, "Position", position(1.0)).unwrap();
    (world, part)
}

#[test]
fn counting_scope_stops_during_unwind() {
    let result = std::panic::catch_unwind(|| measure(|| panic!("probe unwind")));
    assert!(result.is_err());
    assert!(!ENABLED.get());
    assert_eq!(measure(|| {}), Counts::default());
}

#[test]
fn warmed_property_write_allocation_matrix() {
    let (mut world, part) = fixture();
    let before = world.change_cursor();
    let changed = measure(|| {
        for index in 0..WRITES {
            world
                .set(part, "Position", position(index as f64 + 2.0))
                .unwrap();
        }
    });
    assert_eq!(world.get(part, "Position").unwrap(), position(1_001.0));
    assert_eq!(world.changes_since(before).0, Changes::Ids(vec![part]));
    assert_ne!(world.change_cursor(), before);

    let before = world.change_cursor();
    let noop = measure(|| {
        for _ in 0..WRITES {
            world.set(part, "Position", position(1_001.0)).unwrap();
        }
    });
    assert_eq!(world.change_cursor(), before);
    assert_eq!(world.changes_since(before).0, Changes::Ids(vec![]));
    assert_eq!(world.get(part, "Position").unwrap(), position(1_001.0));

    let (mut resets, warm_part) = fixture();
    let mut reset_ids = vec![warm_part];
    for _ in 1..WRITES {
        let id = resets.create("Part", None).unwrap();
        resets.set(id, "Position", position(1.0)).unwrap();
        reset_ids.push(id);
    }
    let before = resets.change_cursor();
    let default_reset = measure(|| {
        for &id in &reset_ids {
            resets
                .set(id, "Position", Value::Vector3(Vec3::new(0.0, 0.0, 0.0)))
                .unwrap();
        }
    });
    assert_eq!(
        resets.changes_since(before).0,
        Changes::Ids(reset_ids.clone())
    );
    let default = default_value_ref("Part", "Position").unwrap();
    for id in reset_ids {
        assert!(std::ptr::eq(
            resets.get_ref(id, "Position").unwrap(),
            default
        ));
    }

    let unrelated_calls = Arc::new(AtomicUsize::new(0));
    let calls = Arc::clone(&unrelated_calls);
    world.property_changed(part, "Anchored").connect(move |_| {
        calls.fetch_add(1, Ordering::Relaxed);
    });
    let before = world.change_cursor();
    let unrelated_listener = measure(|| {
        for index in 0..WRITES {
            world
                .set(part, "Position", position(index as f64 + 2.0))
                .unwrap();
        }
    });
    assert_eq!(unrelated_calls.load(Ordering::Relaxed), 0);
    assert_eq!(world.changes_since(before).0, Changes::Ids(vec![part]));
    assert_eq!(world.get(part, "Position").unwrap(), position(1_001.0));

    let matching_calls = Arc::new(AtomicUsize::new(0));
    let matching_last_x = Arc::new(AtomicU64::new(0));
    let calls = Arc::clone(&matching_calls);
    let last_x = Arc::clone(&matching_last_x);
    world
        .property_changed(part, "Position")
        .connect(move |value| {
            let Value::Vector3(vector) = value else {
                panic!("wrong callback type")
            };
            calls.fetch_add(1, Ordering::Relaxed);
            last_x.store(vector.x.to_bits(), Ordering::Relaxed);
        });
    let matching_listener = measure(|| {
        for index in 0..WRITES {
            world
                .set(part, "Position", position(index as f64 + 2.0))
                .unwrap();
        }
    });
    assert_eq!(matching_calls.load(Ordering::Relaxed), WRITES);
    assert_eq!(
        matching_last_x.load(Ordering::Relaxed),
        1_001.0_f64.to_bits()
    );
    assert_eq!(unrelated_calls.load(Ordering::Relaxed), 0);

    // Print every baseline row before asserting the desired warmed-storage contract.
    eprintln!(
        "property_writes count={WRITES}: changed={changed:?}; noop={noop:?}; default_reset={default_reset:?}; unrelated_listener={unrelated_listener:?}; matching_listener={matching_listener:?}"
    );
    assert_eq!(changed, Counts::default());
    assert_eq!(noop, Counts::default());
    assert_eq!(default_reset, Counts::default());
    // Signal lookup still builds its temporary key. This PR does not change signal storage.
    let signal_key_cost = Counts {
        allocations: WRITES,
        requested_bytes: WRITES * "Position".len(),
    };
    assert_eq!(unrelated_listener, signal_key_cost);
    assert_eq!(matching_listener, signal_key_cost);
}

#[test]
fn validation_clamping_and_callback_order_stay_unchanged() {
    let mut world = World::new_with_services();
    let part = world.create("Part", None).unwrap();
    world.set(part, "Position", position(1.0)).unwrap();
    let lighting = world.root("Lighting").unwrap();
    let values = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = Arc::clone(&values);
    world
        .property_changed(lighting, "ExposureCompensation")
        .connect(move |value| {
            seen.lock().unwrap().push(value.clone());
        });
    let before = world.change_cursor();
    assert!(matches!(
        world.set(part, "Position", Value::Bool(true)),
        Err(CoreError::TypeMismatch { .. })
    ));
    assert!(matches!(
        world.set(part, "Position", position(f64::NAN)),
        Err(CoreError::InvalidValue { .. })
    ));
    assert!(matches!(
        world.set(part, "Material", Value::String("invalid material".into())),
        Err(CoreError::InvalidValue { .. })
    ));
    assert!(matches!(
        world.set(part, "unknown", Value::Bool(false)),
        Err(CoreError::UnknownProperty { .. })
    ));
    assert_eq!(world.change_cursor(), before);
    assert_eq!(world.get(part, "Position").unwrap(), position(1.0));
    assert!(values.lock().unwrap().is_empty());
    world
        .set(lighting, "ExposureCompensation", Value::Number(10.0))
        .unwrap();
    assert_eq!(
        world.get(lighting, "ExposureCompensation").unwrap(),
        Value::Number(4.0)
    );
    let clamped = world.change_cursor();
    world
        .set(lighting, "ExposureCompensation", Value::Number(20.0))
        .unwrap();
    assert_eq!(world.change_cursor(), clamped);
    world
        .set(lighting, "ExposureCompensation", Value::Number(0.0))
        .unwrap();
    assert!(std::ptr::eq(
        world.get_ref(lighting, "ExposureCompensation").unwrap(),
        default_value_ref("Lighting", "ExposureCompensation").unwrap()
    ));
    assert_eq!(
        *values.lock().unwrap(),
        vec![Value::Number(4.0), Value::Number(0.0)]
    );
}
