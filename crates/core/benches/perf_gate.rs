//! The perf gate: instruction counts (Callgrind `Ir`) of `metallic_core`'s hot paths, compared between a pull
//! request's base and head by `.github/workflows/perf.yml`. More than 5% growth on any workload fails the PR.
//!
//! Needs Linux, Valgrind and a `gungraun-runner` of the same version as the `gungraun` crate, so a plain
//! `cargo bench` skips this target (`bench = false` in `Cargo.toml`). Run it by name:
//! `cargo bench -p metallic_core --bench perf_gate`. `docs/perf.md` explains the gate and how to add a workload.
//!
//! Every workload starts from a fixture built by its `setup` function, outside the measured function: 1,000 parts
//! with four non-default properties in a folder under the workspace, with a renderer and a physics consumer
//! registered and drained, as in a running game. Each measured function returns what it changed, so dropping the
//! data model is not measured either. The bodies all differ, so the compiler cannot merge two into one symbol
//! (which would leave the other measuring nothing).
#![allow(
    missing_docs,
    reason = "gungraun's macros generate public modules without doc comments"
)]

#[path = "../tests/support/classes.rs"]
mod classes;

use std::hint::black_box;

use classes::Ids;
use gungraun::{Callgrind, EventKind, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group, main};
use metallic_core::{CFrame, Color3, ConsumerId, DataModel, DirtyBits, DmError, EnumItem, Id, Value, Vector3};

/// Parts in every fixture except `drain_changes`'s.
const PARTS: usize = 1_000;
/// Passes over every part in `reads` and `writes`.
const PASSES: usize = 10;
/// Parts in the `drain_changes` fixture; every hundredth one changes.
const DRAIN_PARTS: usize = 5_000;
const DRAIN_STEP: usize = 100;

/// The growth in `Ir` over which a workload counts as regressed. Changing it is a gate change (`docs/perf.md`).
const IR_LIMIT_PCT: f64 = 5.0;

fn ok<T>(result: Result<T, DmError>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{error}"),
    }
}

/// A game holding `count` parts in a folder under the workspace.
struct Scene {
    dm: DataModel,
    ids: Ids,
    folder: Id,
    parts: Vec<Id>,
}

fn neon(dm: &DataModel) -> EnumItem {
    match dm.registry().enum_item("Material", "Neon") {
        Some(item) => item,
        None => panic!("no Material.Neon"),
    }
}

fn scene(count: usize) -> Scene {
    let (mut dm, ids) = classes::game();
    let neon = neon(&dm);
    let render = ok(dm.register_consumer(DirtyBits::TRANSFORM | DirtyBits::VISUAL | DirtyBits::STRUCTURE));
    let physics = ok(dm.register_consumer(DirtyBits::TRANSFORM | DirtyBits::PHYSICS | DirtyBits::STRUCTURE));
    let folder = ok(dm.create(ids.folder));
    ok(dm.set_parent(folder, Some(ids.workspace)));
    let mut parts = Vec::with_capacity(count);
    for i in 0..count {
        let p = ok(dm.create(ids.part));
        ok(dm.set(
            p,
            ids.cframe,
            Value::CFrame(CFrame::new(Vector3::new(i as f32, 0.0, 0.0))),
        ));
        ok(dm.set(p, ids.size, Value::Vector3(Vector3::new(2.0, 1.0, 4.0))));
        ok(dm.set(p, ids.color, Value::Color3(Color3::new(1.0, 0.0, 0.0))));
        ok(dm.set(p, ids.material, Value::Enum(neon.clone())));
        ok(dm.set_parent(p, Some(folder)));
        parts.push(p);
    }
    let mut out = Vec::new();
    dm.drain_changes(render, &mut out);
    dm.drain_changes(physics, &mut out);
    Scene { dm, ids, folder, parts }
}

/// The `drain_changes` fixture: a consumer of everything, drained, then 50 of the parts moved.
fn drain_scene(count: usize) -> (DataModel, ConsumerId, Vec<(Id, DirtyBits)>) {
    let Scene { mut dm, ids, parts, .. } = scene(count);
    let consumer = ok(dm.register_consumer(DirtyBits::ALL));
    // Room for the 50 entries, so the drain does not allocate (as in a game, where `out` is reused).
    let out = Vec::with_capacity(count / DRAIN_STEP + 1);
    let moved = Value::CFrame(CFrame::new(Vector3::new(0.0, 5.0, 0.0)));
    for &p in parts.iter().step_by(DRAIN_STEP) {
        ok(dm.set(p, ids.cframe, moved.clone()));
    }
    (dm, consumer, out)
}

// Seven properties of every part, ten times: what a renderer and physics read each frame.
#[library_benchmark]
#[bench::parts(args = (PARTS), setup = scene)]
fn reads(scene: Scene) -> f32 {
    let Scene { dm, ids, parts, .. } = &scene;
    let mut acc = 0.0f32;
    for _ in 0..PASSES {
        for &p in parts {
            let cf = ok(dm.cframe(p, ids.cframe));
            let size = ok(dm.vector3(p, ids.size));
            let color = ok(dm.color3(p, ids.color));
            let transparency = ok(dm.number(p, ids.transparency));
            let anchored = ok(dm.bool(p, ids.anchored));
            let collide = ok(dm.bool(p, ids.can_collide));
            let material = ok(dm.enum_item(p, ids.material));
            acc += cf.position().x + size.y + color.r + transparency as f32;
            acc += f32::from(u8::from(anchored) + u8::from(collide));
            black_box(material);
        }
    }
    acc
}

// A new `CFrame` for every part, ten times; each write changes the value.
#[library_benchmark]
#[bench::parts(args = (PARTS), setup = scene)]
fn writes(mut scene: Scene) -> Scene {
    for pass in 1..=PASSES {
        let y = pass as f32;
        for (i, &p) in scene.parts.iter().enumerate() {
            let cf = Value::CFrame(CFrame::new(Vector3::new(i as f32, y, 0.0)));
            ok(scene.dm.set(p, scene.ids.cframe, cf));
        }
    }
    scene
}

// 1,000 more parts, each created and parented into the folder.
#[library_benchmark]
#[bench::parts(args = (PARTS), setup = scene)]
fn create(mut scene: Scene) -> Scene {
    for _ in 0..PARTS {
        let p = ok(scene.dm.create(scene.ids.part));
        ok(scene.dm.set_parent(p, Some(scene.folder)));
    }
    scene
}

// Every part destroyed on its own, in creation order.
#[library_benchmark]
#[bench::parts(args = (PARTS), setup = scene)]
fn destroy(mut scene: Scene) -> Scene {
    for &p in &scene.parts {
        ok(scene.dm.destroy(p));
    }
    scene
}

// A deep copy of the folder and its 1,000 parts.
#[library_benchmark]
#[bench::parts(args = (PARTS), setup = scene)]
fn clone(mut scene: Scene) -> (Scene, Option<Id>) {
    let copy = ok(scene.dm.clone_tree(scene.folder));
    (scene, copy)
}

// The 50 changed parts of 5,000, drained by one consumer.
#[library_benchmark]
#[bench::parts(args = (DRAIN_PARTS), setup = drain_scene)]
fn drain_changes(fixture: (DataModel, ConsumerId, Vec<(Id, DirtyBits)>)) -> (DataModel, Vec<(Id, DirtyBits)>) {
    let (mut dm, consumer, mut out) = fixture;
    dm.drain_changes(consumer, &mut out);
    (dm, out)
}

library_benchmark_group!(
    name = workloads,
    benchmarks = [reads, writes, create, destroy, clone, drain_changes]
);

main!(
    config = LibraryBenchmarkConfig::default().tool(Callgrind::default().soft_limits([(EventKind::Ir, IR_LIMIT_PCT)])),
    library_benchmark_groups = workloads
);
