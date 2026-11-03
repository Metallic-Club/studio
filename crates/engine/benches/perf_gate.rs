//! The perf gate's engine suite: instruction counts (Callgrind `Ir`) of the frame loop's own overhead, compared
//! between a pull request's base and head by `.github/workflows/perf.yml`, next to `metallic_core`'s and
//! `metallic_luau`'s suites. More than 5% growth on any workload fails the PR.
//!
//! Needs Linux, Valgrind and a `gungraun-runner` of the same version as the `gungraun` crate, so a plain
//! `cargo bench` skips this target (`bench = false`). Run it by name, with the dedicated server's features:
//! `cargo bench -p metallic_engine --bench perf_gate --no-default-features --features physics,luau`.
//! `docs/perf.md` explains the gate.
//!
//! The workload is an empty frame: a started server game with no scripts (the Luau backend registered, as in a
//! real server), so what is counted is the loop itself: the clock, the four script steps with nothing due, the
//! `RunService` watch checks and the empty simulation and render steps. The `setup` warms the engine with a few
//! frames so every reused buffer has grown, and the measured function returns the engine so dropping it is not
//! counted.
#![allow(
    missing_docs,
    reason = "gungraun's macros generate public modules without doc comments"
)]

use std::hint::black_box;

use gungraun::{Callgrind, EventKind, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group, main};
use metallic_engine::{Engine, EngineConfig, RunContext};
use metallic_script::CollectOutput;

/// The growth in `Ir` over which a workload counts as regressed. Changing it is a gate change (`docs/perf.md`).
const IR_LIMIT_PCT: f64 = 5.0;

/// Frames per measured run, so the count is per-frame work times 100 rather than one small number.
const FRAMES: u32 = 100;

/// A started server game with no scripts, warmed by a few frames.
fn empty_game(_: ()) -> Engine<CollectOutput> {
    let config = EngineConfig::new(RunContext::Server);
    let mut engine = match Engine::with_output(config, CollectOutput::new()) {
        Ok(engine) => engine,
        Err(error) => panic!("{error}"),
    };
    if let Err(error) = engine.start() {
        panic!("{error}");
    }
    engine.run_headless(10);
    engine
}

#[library_benchmark]
#[bench::frames(args = (()), setup = empty_game)]
fn engine_empty_frames(mut engine: Engine<CollectOutput>) -> Engine<CollectOutput> {
    for _ in 0..FRAMES {
        black_box(engine.tick());
    }
    engine
}

library_benchmark_group!(name = workloads, benchmarks = [engine_empty_frames]);

main!(
    config = LibraryBenchmarkConfig::default().tool(Callgrind::default().soft_limits([(EventKind::Ir, IR_LIMIT_PCT)])),
    library_benchmark_groups = workloads
);
