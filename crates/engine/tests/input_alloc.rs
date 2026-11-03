//! Allocation guard for `UserInputService`'s hot path, measured with a counting global allocator: once warm, a
//! frame of 1,000 mouse-motion events with nothing connected to `InputChanged` allocates nothing and creates no
//! instance, queuing the events included.
//!
//! Counters are per thread, so tests running in parallel do not see each other's allocations.
#![cfg(all(feature = "luau", feature = "render"))]
// A `#[global_allocator]` needs `unsafe impl GlobalAlloc`. This test binary's allocator only counts, then forwards
// every call unchanged to the system allocator.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use metallic_core::Value;
use metallic_engine::{Engine, EngineConfig, InputEvent, MemorySources, RunContext};
use metallic_platform::{DeviceEvent, Position, WindowEvent, WindowId};
use metallic_script::CollectOutput;

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

const EVENTS: u32 = 1_000;

/// Queues one frame of mouse input: the cursor and raw motion, alternating, `EVENTS` in all.
fn push_motion(engine: &mut Engine<CollectOutput>) {
    for i in 0..EVENTS / 2 {
        let x = f64::from(i % 100);
        engine.push_input(InputEvent::Window {
            window: WindowId::from_raw(1),
            event: WindowEvent::CursorMoved(Position { x, y: x * 0.5 }),
        });
        engine.push_input(InputEvent::Device(DeviceEvent::MouseMotion { dx: 1.0, dy: -1.0 }));
    }
}

/// A started standalone game with one script, `source` (or none).
fn started(source: Option<&str>) -> Engine<CollectOutput> {
    let mut sources = MemorySources::new();
    if let Some(source) = source {
        sources.insert("Main.luau", source);
    }
    let config = EngineConfig::new(RunContext::Local).with_sources(sources);
    let mut engine = Engine::with_output(config, CollectOutput::new()).expect("engine");
    if source.is_some() {
        let dm = engine.data_model_mut();
        let class = dm.registry().class_id("Script").expect("Script");
        let service = dm.registry().class_id("ServerScriptService").expect("service class");
        let service = dm.service(service).expect("ServerScriptService");
        let script = dm.create(class).expect("create");
        dm.set_by_name(script, "Source", Value::String("Main.luau".into()))
            .expect("source");
        dm.set_parent(script, Some(service)).expect("parent");
    }
    engine.start().expect("start");
    engine
}

/// Warms `engine` with a few frames of motion, then measures one more.
fn assert_motion_is_free(mut engine: Engine<CollectOutput>) {
    for _ in 0..4 {
        push_motion(&mut engine);
        engine.tick();
    }
    let instances = engine.data_model().len();
    let allocations = allocations_in(|| {
        push_motion(&mut engine);
        engine.tick();
    });
    assert_eq!(allocations, 0, "a frame of {EVENTS} mouse events allocated");
    assert_eq!(engine.data_model().len(), instances, "no instance was created");
    let delta = engine
        .data_model()
        .get_by_name(engine.user_input_service(), "MouseDelta")
        .expect("MouseDelta");
    assert_eq!(
        delta,
        Value::Vector2(metallic_core::Vector2::new(500.0, -500.0)),
        "the frame's movement was still taken"
    );
}

#[test]
fn unwatched_mouse_motion_allocates_nothing() {
    assert_motion_is_free(started(None));
}

#[test]
fn mouse_motion_allocates_nothing_while_only_other_input_events_are_watched() {
    let engine = started(Some(
        r#"
        local UserInputService = game:GetService("UserInputService")
        UserInputService.InputBegan:Connect(function() end)
        UserInputService.InputEnded:Connect(function() end)
        "#,
    ));
    assert_motion_is_free(engine);
}
