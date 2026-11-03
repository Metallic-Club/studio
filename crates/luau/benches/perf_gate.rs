//! The perf gate's Luau suite: instruction counts (Callgrind `Ir`) of the Luau bindings' hot paths, compared
//! between a pull request's base and head by `.github/workflows/perf.yml`, next to `metallic_core`'s suite. More
//! than 5% growth on any workload fails the PR.
//!
//! Needs Linux, Valgrind and a `gungraun-runner` of the same version as the `gungraun` crate, so a plain
//! `cargo bench` skips this target (`bench = false`). Run it by name:
//! `cargo bench -p metallic_luau --bench perf_gate`. `docs/perf.md` explains the gate.
//!
//! Each workload's `setup` builds a game with one part in the workspace, a host with the Luau backend, and runs the
//! workload's script once, so the chunk is compiled and cached and the per-class member cache is warm, as in a
//! running game. The measured function runs the script once more: one script start plus `OPS` operations, so
//! the operations dominate. The scripts all differ, so no two workloads measure the same thing.
#![allow(
    missing_docs,
    reason = "gungraun's macros generate public modules without doc comments"
)]

use std::hint::black_box;

use gungraun::{Callgrind, EventKind, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group, main};
use metallic_core::{DataModel, Id};
use metallic_luau::LuauLanguage;
use metallic_script::{Resume, ScriptHost};

/// The growth in `Ir` over which a workload counts as regressed. Changing it is a gate change (`docs/perf.md`).
const IR_LIMIT_PCT: f64 = 5.0;

/// `part.Position` 10,000 times.
const READS: &str = "local part = workspace.Part\nlocal p\nfor i = 1, 10000 do\n    p = part.Position\nend\nreturn p\n";

/// `part.Transparency = x` 10,000 times; every write changes the value.
const WRITES: &str = "local part = workspace.Part\nfor i = 1, 10000 do\n    part.Transparency = (i % 100) / 100\nend\n";

/// `part:IsA("BasePart")` 10,000 times.
const METHOD_CALLS: &str = "local part = workspace.Part\nlocal n = 0\nfor i = 1, 10000 do\n    if part:IsA(\"BasePart\") then n += 1 end\nend\nreturn n\n";

/// A game, a host with the Luau backend, the script instance and its source.
struct Fixture {
    dm: DataModel,
    host: ScriptHost,
    script: Id,
    source: &'static str,
}

fn fixture(source: &'static str) -> Fixture {
    let mut dm = match metallic_classes::new_game() {
        Ok(dm) => dm,
        Err(error) => panic!("{error}"),
    };
    let luau = match LuauLanguage::new(dm.registry().clone()) {
        Ok(luau) => luau,
        Err(error) => panic!("{error}"),
    };
    let mut host = ScriptHost::new();
    if let Err(error) = host.register_language(Box::new(luau)) {
        panic!("{error}");
    }
    let registry = dm.registry().clone();
    let class = |name: &str| match registry.class_id(name) {
        Some(class) => class,
        None => panic!("no {name} class"),
    };
    let workspace = dm.service(class("Workspace")).unwrap_or_else(|| panic!("no Workspace"));
    let part = dm.create(class("Part")).unwrap_or_else(|e| panic!("{e}"));
    dm.set_name(part, "Part".into()).unwrap_or_else(|e| panic!("{e}"));
    dm.set_parent(part, Some(workspace)).unwrap_or_else(|e| panic!("{e}"));
    let script = dm.create(class("Script")).unwrap_or_else(|e| panic!("{e}"));
    dm.set_parent(script, Some(workspace)).unwrap_or_else(|e| panic!("{e}"));
    let mut fixture = Fixture {
        dm,
        host,
        script,
        source,
    };
    // Warm: compile and cache the chunk, fill the member cache.
    run(&mut fixture);
    fixture
}

fn reads_fixture(_: ()) -> Fixture {
    fixture(READS)
}

fn writes_fixture(_: ()) -> Fixture {
    fixture(WRITES)
}

fn method_calls_fixture(_: ()) -> Fixture {
    fixture(METHOD_CALLS)
}

/// Runs the fixture's script to completion.
fn run(fixture: &mut Fixture) -> Resume {
    match fixture
        .host
        .run_script(&mut fixture.dm, fixture.script, "Gate.luau", fixture.source)
    {
        Ok(run) => run.resume,
        Err(error) => panic!("{error}"),
    }
}

#[library_benchmark]
#[bench::ops(args = (()), setup = reads_fixture)]
fn luau_reads(mut fixture: Fixture) -> (Fixture, Resume) {
    let resume = black_box(run(&mut fixture));
    (fixture, resume)
}

#[library_benchmark]
#[bench::ops(args = (()), setup = writes_fixture)]
fn luau_writes(mut fixture: Fixture) -> (Fixture, Resume) {
    let resume = black_box(run(&mut fixture));
    (fixture, resume)
}

#[library_benchmark]
#[bench::ops(args = (()), setup = method_calls_fixture)]
fn luau_method_calls(mut fixture: Fixture) -> (Fixture, Resume) {
    let resume = black_box(run(&mut fixture));
    (fixture, resume)
}

library_benchmark_group!(
    name = workloads,
    benchmarks = [luau_reads, luau_writes, luau_method_calls]
);

main!(
    config = LibraryBenchmarkConfig::default().tool(Callgrind::default().soft_limits([(EventKind::Ir, IR_LIMIT_PCT)])),
    library_benchmark_groups = workloads
);
