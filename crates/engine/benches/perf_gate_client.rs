//! The perf gate's engine client suite: instruction counts (Callgrind `Ir`) of the frame loop's steps that only a
//! build with a window has, compared between a pull request's base and head by `.github/workflows/perf.yml`. More
//! than 5% growth on any workload fails the PR.
//!
//! `perf_gate.rs` measures the frame loop as a dedicated server builds it (`--no-default-features --features
//! physics,luau`), which has no input queue. This suite builds with the default features (`render`, so winit is
//! compiled in) to reach the input step, but opens no window and needs no GPU: the engine stays headless.
//!
//! Needs Linux, Valgrind and a `gungraun-runner` of the same version as the `gungraun` crate, so a plain
//! `cargo bench` skips this target (`bench = false`). Run it by name: `cargo bench -p metallic_engine --bench
//! perf_gate_client`. `docs/perf.md` explains the gate.
//!
//! `engine_input_mouse_unwatched` is the input step with nothing listening: 1,000 mouse events (cursor moves and raw
//! motion, alternating) queued with `Engine::push_input` into a started game with no scripts, then one `tick()`.
//! Queuing is counted, as a window's event loop would pay it. The `setup` warms the engine with a few frames of the
//! same input so the queue's buffers have grown, and the measured function returns the engine so dropping it is
//! not counted.
#![allow(
    missing_docs,
    reason = "gungraun's macros generate public modules without doc comments"
)]

use std::hint::black_box;

use gungraun::{Callgrind, EventKind, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group, main};
use metallic_engine::{Engine, EngineConfig, InputEvent, RunContext};
use metallic_platform::{DeviceEvent, Position, WindowEvent, WindowId};
use metallic_script::CollectOutput;

/// The growth in `Ir` over which a workload counts as regressed. Changing it is a gate change (`docs/perf.md`).
const IR_LIMIT_PCT: f64 = 5.0;

/// Mouse events per measured frame: half cursor moves, half raw motion.
const EVENTS: u32 = 1_000;

/// Frames of the same input the `setup` runs before the measured one.
const WARM_UP_FRAMES: u32 = 4;

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

/// A started standalone game with no scripts (so nothing is connected to `UserInputService`), warmed by a few
/// frames of mouse input.
fn unwatched_game(_: ()) -> Engine<CollectOutput> {
    let config = EngineConfig::new(RunContext::Local);
    let mut engine = match Engine::with_output(config, CollectOutput::new()) {
        Ok(engine) => engine,
        Err(error) => panic!("{error}"),
    };
    if let Err(error) = engine.start() {
        panic!("{error}");
    }
    for _ in 0..WARM_UP_FRAMES {
        push_motion(&mut engine);
        engine.tick();
    }
    engine
}

#[library_benchmark]
#[bench::mouse(args = (()), setup = unwatched_game)]
fn engine_input_mouse_unwatched(mut engine: Engine<CollectOutput>) -> Engine<CollectOutput> {
    push_motion(&mut engine);
    black_box(engine.tick());
    engine
}

library_benchmark_group!(name = workloads, benchmarks = [engine_input_mouse_unwatched]);

main!(
    config = LibraryBenchmarkConfig::default().tool(Callgrind::default().soft_limits([(EventKind::Ir, IR_LIMIT_PCT)])),
    library_benchmark_groups = workloads
);
