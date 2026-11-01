//! Isolate signal-name work by removing warmed stored values, not replacing their keys.
use metallic_core::classes::default_value_ref;
use metallic_core::{Changes, InstanceId, Value, Vec3, World};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    calls: usize,
    requested_bytes: usize,
}

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static COUNTS: Cell<Counts> = const { Cell::new(Counts { calls: 0, requested_bytes: 0 }) };
}

fn record(bytes: usize) {
    if ENABLED.try_with(Cell::get).unwrap_or(false) {
        let _ = COUNTS.try_with(|counts| {
            let previous = counts.get();
            counts.set(Counts {
                calls: previous.calls + 1,
                requested_bytes: previous.requested_bytes + bytes,
            });
        });
    }
}

struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forward the caller's allocator contract unchanged to System.
        let result = unsafe { System.alloc(layout) };
        if !result.is_null() {
            record(layout.size());
        }
        result
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forward the caller's allocator contract unchanged to System.
        let result = unsafe { System.alloc_zeroed(layout) };
        if !result.is_null() {
            record(layout.size());
        }
        result
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: pointer/layout/size come from the GlobalAlloc caller.
        let result = unsafe { System.realloc(pointer, layout, size) };
        if !result.is_null() {
            record(size);
        }
        result
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: pointer and layout come from the GlobalAlloc caller.
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

struct Scope;
impl Drop for Scope {
    fn drop(&mut self) {
        ENABLED.set(false);
    }
}

fn measure<T>(operation: impl FnOnce() -> T) -> (T, Counts) {
    assert!(!ENABLED.get(), "nested counting scope");
    COUNTS.set(Counts::default());
    ENABLED.set(true);
    let scope = Scope;
    let result = operation();
    drop(scope);
    (result, COUNTS.get())
}

const PARTS: usize = 1_000;

#[derive(Clone, Copy, Debug)]
enum Listeners {
    None,
    Unrelated,
    Matching,
}

fn fixture(
    mode: Listeners,
) -> (
    World,
    Vec<InstanceId>,
    Value,
    Arc<AtomicUsize>,
    Arc<AtomicBool>,
) {
    let mut world = World::new_empty();
    let reset = default_value_ref("Part", "Position").unwrap().clone();
    let parts: Vec<_> = (0..PARTS)
        .map(|index| {
            let part = world.create("Part", None).unwrap();
            world
                .set(
                    part,
                    "Position",
                    Value::Vector3(Vec3::new(index as f64 + 1.0, 2.0, 3.0)),
                )
                .unwrap();
            assert_ne!(world.get(part, "Position").unwrap(), reset);
            part
        })
        .collect();
    // Public writes warm journal capacity beyond this fixture's later 1,000 records.
    // The no-listener row independently checks that no unrelated allocation remains.
    for index in 0..3_000 {
        world
            .set(parts[0], "Anchored", Value::Bool(index % 2 == 0))
            .unwrap();
    }
    let callbacks = Arc::new(AtomicUsize::new(0));
    let wrong_value = Arc::new(AtomicBool::new(false));
    let property = match mode {
        Listeners::None => return (world, parts, reset, callbacks, wrong_value),
        Listeners::Unrelated => "Color",
        Listeners::Matching => "Position",
    };
    for &part in &parts {
        let callbacks = callbacks.clone();
        let wrong_value = wrong_value.clone();
        let expected = reset.clone();
        world
            .property_changed(part, property)
            .connect(move |value| {
                callbacks.fetch_add(1, Ordering::Relaxed);
                if value != &expected {
                    wrong_value.store(true, Ordering::Relaxed);
                }
            });
    }
    (world, parts, reset, callbacks, wrong_value)
}

#[test]
fn default_removal_isolates_temporary_signal_lookup_names() {
    let mut rows = Vec::new();
    for mode in [Listeners::None, Listeners::Unrelated, Listeners::Matching] {
        let (mut world, parts, reset, callbacks, wrong_value) = fixture(mode);
        let cursor = world.change_cursor();
        let (_, counts) = measure(|| {
            for &part in &parts {
                world
                    .set(black_box(part), black_box("Position"), reset.clone())
                    .unwrap();
            }
        });
        assert_eq!(
            callbacks.load(Ordering::Relaxed),
            if matches!(mode, Listeners::Matching) {
                PARTS
            } else {
                0
            }
        );
        assert!(!wrong_value.load(Ordering::Relaxed));
        let (changes, _) = world.changes_since(cursor);
        assert_eq!(changes, Changes::Ids(parts.clone()));
        for &part in &parts {
            assert_eq!(world.get(part, "Position").unwrap(), reset);
            assert!(
                std::ptr::eq(
                    world.get_ref(part, "Position").unwrap(),
                    default_value_ref("Part", "Position").unwrap()
                ),
                "default writes must remove storage, not replace its String key"
            );
        }
        eprintln!("default_reset {mode:?}: {counts:?}");
        rows.push(counts);
    }
    // All behavioral controls and all rows are observed before the allocation contract.
    assert_eq!(rows[0], Counts::default(), "fixture/control allocation");
    assert_eq!(
        rows[1],
        Counts::default(),
        "unrelated signal name lookup allocated"
    );
    assert_eq!(
        rows[2],
        Counts::default(),
        "matching signal name lookup allocated"
    );
}

#[test]
fn warmed_known_registration_does_not_allocate_a_temporary_name() {
    let mut world = World::new_empty();
    let part = world.create("Part", None).unwrap();
    let name = String::from("Position");
    world.property_changed(part, &name).connect(|_| {});
    let cursor = world.change_cursor();
    let (_, counts) = measure(|| {
        for _ in 0..PARTS {
            black_box(world.property_changed(part, black_box(name.as_str())).len());
        }
    });
    assert_eq!(world.property_changed(part, "Position").len(), 1);
    assert_eq!(world.change_cursor(), cursor);
    eprintln!("known_registration: {counts:?}");
    assert_eq!(counts, Counts::default());
}

#[test]
fn counting_scope_recovers_after_unwind() {
    assert!(std::panic::catch_unwind(|| measure(|| panic!("counting probe unwind"))).is_err());
    assert!(!ENABLED.get());
    let (owned, counts) = measure(|| black_box(Box::new(17_u64)));
    assert_eq!(*owned, 17);
    assert_eq!(
        counts,
        Counts {
            calls: 1,
            requested_bytes: 8
        }
    );
    assert!(!ENABLED.get());
}
