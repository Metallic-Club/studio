//! Allocation guards for the data model (issue #14 performance requirements), measured with a counting global
//! allocator: hot reads and writes allocate nothing, drains allocate nothing once their vectors have grown, and a
//! `Part` with four non-default properties fits in 256 bytes. Signals keep that: a write to an unwatched instance
//! allocates nothing, and a watched one queues its events without allocating once the queue has grown.
//!
//! Counters are per thread, so tests running in parallel do not see each other's allocations.

// A `#[global_allocator]` needs `unsafe impl GlobalAlloc`. This test binary's allocator only counts, then forwards
// every call unchanged to the system allocator.
#![allow(unsafe_code)]

#[path = "support/classes.rs"]
mod classes;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

use metallic_core::{
    CFrame, Color3, ConnectionId, DataModel, Delivery, DirtyBits, EnumItem, Id, ListenerKey, Value, Vector3,
};

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

/// How many allocations (and reallocations) `f` makes on this thread.
fn allocations_in(f: impl FnOnce()) -> u64 {
    let before = ALLOCATIONS.with(Cell::get);
    f();
    ALLOCATIONS.with(Cell::get) - before
}

fn live_bytes() -> i64 {
    LIVE_BYTES.with(Cell::get)
}

#[test]
fn hot_reads_and_writes_do_not_allocate() {
    let (mut dm, ids) = classes::game();
    let render = dm
        .register_consumer(DirtyBits::TRANSFORM | DirtyBits::VISUAL | DirtyBits::STRUCTURE)
        .expect("consumer");
    let physics = dm.register_consumer(DirtyBits::ALL).expect("consumer");
    let p = dm.create(ids.part).expect("part");
    dm.set_parent(p, Some(ids.workspace)).expect("parent");
    let target = dm.create(ids.part).expect("part");
    let neon = dm.registry().enum_item("Material", "Neon").expect("item");
    let wood = dm.registry().enum_item("Material", "Wood").expect("item");
    let mut changes: Vec<(Id, DirtyBits)> = Vec::with_capacity(16);

    let mut round = |dm: &mut DataModel, i: u32| {
        let f = i as f32;
        let set = |dm: &mut DataModel, prop, value| dm.set(p, prop, value).expect("set");
        set(dm, ids.cframe, Value::CFrame(CFrame::new(Vector3::new(f, 1.0, 2.0))));
        set(dm, ids.transparency, Value::Number(f64::from(i % 10) / 10.0));
        // Flips between the default and another value, removing and re-adding the entry.
        set(dm, ids.anchored, Value::Bool(i.is_multiple_of(2)));
        let item = if i.is_multiple_of(2) { &neon } else { &wood };
        set(dm, ids.material, Value::Enum(item.clone()));
        set(dm, ids.target, Value::Ref((i.is_multiple_of(2)).then_some(target)));

        black_box(dm.get_ref(p, ids.cframe).expect("read"));
        black_box(dm.get_ref(p, ids.material).expect("read"));
        black_box(dm.cframe(p, ids.cframe).expect("read"));
        black_box(dm.vector3(p, ids.size).expect("read"));
        black_box(dm.color3(p, ids.color).expect("read"));
        black_box(dm.number(p, ids.transparency).expect("read"));
        black_box(dm.bool(p, ids.anchored).expect("read"));
        black_box(dm.int(p, ids.layer).expect("read"));
        black_box(dm.reference(p, ids.target).expect("read"));
        black_box(dm.enum_item(p, ids.material).expect("read"));
        black_box(dm.get(p, ids.name).expect("computed read"));
        black_box(dm.vector3(p, ids.position).expect("computed read"));
        black_box(dm.name(p).expect("read"));
        black_box(dm.parent(p).expect("read"));
        black_box(dm.children(ids.workspace).expect("read").count());
        black_box(dm.child_count(ids.workspace).expect("read"));
        black_box(dm.descendants(ids.workspace).expect("read").count());

        dm.drain_changes(render, &mut changes);
        changes.clear();
        dm.drain_changes(physics, &mut changes);
        changes.clear();
    };

    // Warm up: the first non-default write of each property adds its entry, and the dirty lists grow.
    round(&mut dm, 0);
    round(&mut dm, 1);
    let allocations = allocations_in(|| {
        for i in 0..1_000 {
            round(&mut dm, i);
        }
    });
    assert_eq!(allocations, 0, "hot reads, writes and drains allocated");
}

#[test]
fn drains_do_not_allocate_once_grown() {
    let (mut dm, ids) = classes::game();
    let consumer = dm.register_consumer(DirtyBits::ALL).expect("consumer");
    let parts: Vec<Id> = (0..5_000)
        .map(|_| {
            let p = dm.create(ids.part).expect("part");
            dm.set_parent(p, Some(ids.workspace)).expect("parent");
            dm.set(p, ids.cframe, Value::CFrame(CFrame::new(Vector3::ONE)))
                .expect("set");
            p
        })
        .collect();
    let mut changes = Vec::new();
    let mut removed = Vec::new();
    dm.drain_changes(consumer, &mut changes);
    assert_eq!(changes.len(), parts.len());
    changes.clear();

    let move_some = |dm: &mut DataModel, round: u32| {
        for &p in parts.iter().step_by(100) {
            let cf = CFrame::new(Vector3::new(round as f32, 0.0, 0.0));
            dm.set(p, ids.cframe, Value::CFrame(cf)).expect("set");
        }
    };
    move_some(&mut dm, 1);
    let allocations = allocations_in(|| {
        dm.drain_changes(consumer, &mut changes);
        dm.drain_removed(consumer, &mut removed);
    });
    assert_eq!(changes.len(), 50);
    assert_eq!(allocations, 0, "drain allocated");

    for round in 2..10 {
        changes.clear();
        move_some(&mut dm, round);
        assert_eq!(allocations_in(|| dm.drain_changes(consumer, &mut changes)), 0);
        assert_eq!(changes.len(), 50);
    }

    for &p in &parts[..10] {
        dm.destroy(p).expect("destroy");
    }
    removed.reserve(10);
    assert_eq!(allocations_in(|| dm.drain_removed(consumer, &mut removed)), 0);
    assert_eq!(removed, parts[..10].to_vec());
}

#[test]
fn writes_and_drains_with_every_consumer_registered_do_not_allocate() {
    let (mut dm, ids) = classes::game();
    let consumers: Vec<_> = (0..metallic_core::MAX_CONSUMERS)
        .map(|_| dm.register_consumer(DirtyBits::ALL).expect("consumer"))
        .collect();
    let parts: Vec<Id> = (0..100)
        .map(|_| {
            let p = dm.create(ids.part).expect("part");
            dm.set_parent(p, Some(ids.workspace)).expect("parent");
            p
        })
        .collect();
    let mut changes = Vec::new();
    let mut round = |dm: &mut DataModel, i: u32| {
        for &p in &parts {
            // Never the default, so every write stores into the entry the first one added.
            let cf = CFrame::new(Vector3::new(i as f32, 1.0, 0.0));
            dm.set(p, ids.cframe, Value::CFrame(cf)).expect("set");
        }
        for &c in &consumers {
            dm.drain_changes(c, &mut changes);
            assert_eq!(changes.len(), parts.len());
            changes.clear();
        }
    };
    // Warm up: the dirty lists and `changes` grow to their working size.
    round(&mut dm, 0);
    let allocations = allocations_in(|| {
        for i in 1..100 {
            round(&mut dm, i);
        }
    });
    assert_eq!(
        allocations, 0,
        "writes or drains allocated with every consumer registered"
    );
}

/// Live bytes per part, inline and heap together, for `count` parts with four non-default properties under the
/// workspace. With `reserve`, the slot table is sized up front as a scene loader does.
fn bytes_per_part(count: usize, reserve: bool) -> f64 {
    let (mut dm, ids) = classes::game();
    let neon: EnumItem = dm.registry().enum_item("Material", "Neon").expect("item");
    let before = live_bytes();
    if reserve {
        dm.reserve(count);
    }
    for i in 0..count {
        let p = dm.create(ids.part).expect("part");
        let f = i as f32;
        dm.set(p, ids.cframe, Value::CFrame(CFrame::new(Vector3::new(f, 0.0, 0.0))))
            .expect("set");
        dm.set(p, ids.size, Value::Vector3(Vector3::new(2.0, 2.0, 2.0)))
            .expect("set");
        dm.set(p, ids.color, Value::Color3(Color3::new(1.0, 0.0, 0.0)))
            .expect("set");
        dm.set(p, ids.material, Value::Enum(neon.clone())).expect("set");
        dm.set_parent(p, Some(ids.workspace)).expect("parent");
    }
    let bytes = live_bytes() - before;
    black_box(&dm);
    bytes as f64 / count as f64
}

#[test]
fn a_part_fits_the_memory_budget() {
    const COUNT: usize = 10_000;
    let reserved = bytes_per_part(COUNT, true);
    let grown = bytes_per_part(COUNT, false);
    println!("bytes per Part with 4 non-default properties: {reserved:.1} (slot table reserved), {grown:.1} (grown)");
    assert!(reserved <= 256.0, "{reserved:.1} bytes per part, budget 256");
}

#[test]
fn unwatched_writes_do_not_allocate_while_others_listen() {
    let (mut dm, ids) = classes::game();
    let watched = dm.create(ids.part).expect("part");
    dm.set_parent(watched, Some(ids.workspace)).expect("parent");
    // Connections of every kind elsewhere, so the whole-model counters are non-zero.
    dm.connect(watched, ids.changed, ListenerKey(1)).expect("connect");
    dm.connect(watched, ids.ancestry_changed, ListenerKey(2))
        .expect("connect");
    dm.connect(ids.workspace, ids.descendant_added, ListenerKey(3))
        .expect("connect");
    dm.connect_property_changed(watched, ids.cframe, ListenerKey(4))
        .expect("connect");
    let p = dm.create(ids.part).expect("part");
    dm.set_parent(p, Some(ids.workspace)).expect("parent");
    let mut out = Vec::new();
    dm.drain_deliveries(&mut out);
    assert!(!dm.is_watched(p));

    let round = |dm: &mut DataModel, i: u32| {
        let f = i as f32;
        dm.set(p, ids.cframe, Value::CFrame(CFrame::new(Vector3::new(f, 1.0, 2.0))))
            .expect("set");
        dm.set(p, ids.anchored, Value::Bool(i.is_multiple_of(2))).expect("set");
    };
    round(&mut dm, 0);
    round(&mut dm, 1);
    let allocations = allocations_in(|| {
        for i in 0..1_000 {
            round(&mut dm, i);
        }
    });
    assert_eq!(allocations, 0, "writes to an unwatched instance allocated");
    assert_eq!(dm.pending_events(), 0);
}

#[test]
fn watched_writes_queue_without_allocating_once_grown() {
    const WRITES: u32 = 100;
    let (mut dm, ids) = classes::game();
    let p = dm.create(ids.part).expect("part");
    dm.connect(p, ids.changed, ListenerKey(1)).expect("connect");
    dm.connect_property_changed(p, ids.cframe, ListenerKey(2))
        .expect("connect");
    let write = |dm: &mut DataModel, i: u32| {
        let cf = CFrame::new(Vector3::new(i as f32, 0.0, 0.0));
        dm.set(p, ids.cframe, Value::CFrame(cf)).expect("set");
    };
    let mut out: Vec<Delivery> = Vec::new();
    // Warm up: the queue, the argument list, the shared property name and `out` grow to their working size.
    for i in 0..WRITES {
        write(&mut dm, i + 1);
    }
    dm.drain_deliveries(&mut out);
    out.clear();

    let queued = allocations_in(|| {
        for i in 0..WRITES {
            write(&mut dm, i + 1_000);
        }
    });
    assert_eq!(queued, 0, "queueing allocated");
    assert_eq!(dm.pending_events(), 2 * WRITES as usize);
    let drained = allocations_in(|| dm.drain_deliveries(&mut out));
    assert_eq!(out.len(), 2 * WRITES as usize);
    // Only each `Changed` delivery's own argument list; the property signal has none.
    assert_eq!(drained, u64::from(WRITES), "the drain allocated beyond the arguments");
}

#[test]
fn reporting_dropped_connections_does_not_allocate_once_grown() {
    const PARTS: usize = 50;
    let (mut dm, ids) = classes::game();
    let mut out: Vec<Delivery> = Vec::new();
    let mut dropped: Vec<(ConnectionId, ListenerKey)> = Vec::new();
    // The first round grows the queue, `out` and both dropped-connection lists; the later ones must not allocate.
    for round in 0..4 {
        for i in 0..PARTS {
            let p = dm.create(ids.part).expect("part");
            dm.connect(p, ids.destroying, ListenerKey(i as u64)).expect("connect");
            dm.connect_property_changed(p, ids.cframe, ListenerKey(i as u64))
                .expect("connect");
            dm.destroy(p).expect("destroy");
        }
        out.clear();
        dropped.clear();
        let allocations = allocations_in(|| {
            dm.drain_deliveries(&mut out);
            dm.drain_dropped_connections(&mut dropped);
        });
        assert_eq!(out.len(), PARTS, "one Destroying each");
        assert_eq!(dropped.len(), 2 * PARTS);
        if round > 0 {
            assert_eq!(
                allocations, 0,
                "round {round}: dropping and reporting connections allocated"
            );
        }
    }
}
