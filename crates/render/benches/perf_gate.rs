//! The perf gate's render suite: instruction counts (Callgrind `Ir`) of the GPU scene's CPU side (#76), compared
//! between a pull request's base and head by `.github/workflows/perf.yml`, next to the other suites. More than 5%
//! growth on any workload fails the PR.
//!
//! Needs Linux, Valgrind and a `gungraun-runner` of the same version as the `gungraun` crate, so a plain
//! `cargo bench` skips this target (`bench = false`). Run it by name: `cargo bench -p metallic_render --bench
//! perf_gate`. `docs/perf.md` explains the gate.
//!
//! No GPU is involved: each workload is one frame of [`InstanceScene`] (drain the change consumer, repack the
//! changed instances, coalesce the dirty slots) followed by what [`metallic_render::GpuScene::upload`] does on the
//! CPU, written into a plain byte buffer instead of a wgpu staging buffer. The fixture is 10,000 parts in
//! `Workspace` with 1% of them changed in the `setup`, so the data model's own write path is not counted (the core
//! suite counts it):
//!
//! - `render_sync_moving`: the same 100 parts move every frame, so they sit in the dynamic region and upload as one
//!   range through the copy path (the instances back to back).
//! - `render_sync_scattered`: 100 parts spread over the scene are recoloured, so they upload as 100 ranges through
//!   the scatter path (each instance's destination, then the instances).
#![allow(
    missing_docs,
    reason = "gungraun's macros generate public modules without doc comments"
)]

use std::hint::black_box;

use gungraun::{Callgrind, EventKind, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group, main};
use metallic_core::glam::Vec3;
use metallic_core::{CFrame, Color3, DataModel, DmError, Id, Value};
use metallic_render::InstanceScene;
use metallic_render::scene::{GpuInstance, Region, RenderComponents};

/// The growth in `Ir` over which a workload counts as regressed. Changing it is a gate change (`docs/perf.md`).
const IR_LIMIT_PCT: f64 = 5.0;

/// Parts in the fixture.
const PARTS: usize = 10_000;
/// Parts changed in the measured frame: 1%.
const CHANGES: usize = PARTS / 100;

fn ok<T>(result: Result<T, DmError>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{error}"),
    }
}

/// A scene over [`PARTS`] parts, the buffer standing in for the staging buffer, and the scratch for destinations.
struct Fixture {
    dm: DataModel,
    scene: InstanceScene,
    parts: Vec<Id>,
    staging: Vec<u8>,
    destinations: Vec<u32>,
}

impl Fixture {
    fn new() -> Self {
        let mut dm = match metallic_classes::new_game() {
            Ok(dm) => dm,
            Err(error) => panic!("{error}"),
        };
        let registry = dm.registry().clone();
        let (Some(workspace_class), Some(part)) = (registry.class_id("Workspace"), registry.class_id("Part")) else {
            panic!("no Workspace or Part class");
        };
        let Some(workspace) = dm.service(workspace_class) else {
            panic!("no Workspace");
        };
        let parts = (0..PARTS)
            .map(|i| {
                let id = ok(dm.create(part));
                let place = CFrame::new(Vec3::new((i % 100) as f32 * 4.0, 1.0, (i / 100) as f32 * 4.0));
                ok(dm.set_by_name(id, "CFrame", Value::CFrame(place)));
                ok(dm.set_parent(id, Some(workspace)));
                id
            })
            .collect();
        let scene = ok(InstanceScene::attach(
            &mut dm,
            Some(workspace),
            &RenderComponents::builtin(),
        ));
        let mut fixture = Self {
            dm,
            scene,
            parts,
            staging: Vec::with_capacity(PARTS * GpuInstance::SIZE as usize * 2),
            destinations: Vec::with_capacity(PARTS),
        };
        fixture.scene.sync(&mut fixture.dm);
        fixture.scene.coalesce();
        fixture
    }

    /// Moves the same [`CHANGES`] parts to height `y`.
    fn move_movers(&mut self, y: f32) {
        for i in 0..CHANGES {
            let id = self.parts[i * 97];
            ok(self
                .dm
                .set_by_name(id, "CFrame", Value::CFrame(CFrame::new(Vec3::new(i as f32, y, 0.0)))));
        }
    }

    /// Runs a frame outside the measurement.
    fn settle(&mut self) {
        self.scene.sync(&mut self.dm);
        self.scene.coalesce();
    }
}

/// The movers moved for a few frames (so they are in the dynamic region), then once more for the measured frame.
fn moving(_: ()) -> Fixture {
    let mut fixture = Fixture::new();
    for frame in 0..4 {
        fixture.move_movers(2.0 + frame as f32);
        fixture.settle();
    }
    fixture.move_movers(10.0);
    fixture
}

/// [`CHANGES`] parts spread over the scene recoloured, after one warm-up frame of the same kind.
fn scattered(_: ()) -> Fixture {
    let mut fixture = Fixture::new();
    for (frame, shade) in [(1, 0.25), (2, 0.75)] {
        for i in 0..CHANGES {
            let id = fixture.parts[(i * 7919 + frame * 31) % PARTS];
            ok(fixture
                .dm
                .set_by_name(id, "Color", Value::Color3(Color3::new(shade, 0.5, 0.5))));
        }
        if frame == 1 {
            fixture.settle();
        }
    }
    fixture
}

/// The copy path's CPU work: every range's instances back to back.
fn write_ranges(scene: &InstanceScene, staging: &mut Vec<u8>) {
    staging.clear();
    for range in scene.uploads() {
        let first = range.first as usize;
        let instances = &scene.instances(range.region)[first..first + range.count as usize];
        staging.extend_from_slice(bytemuck::cast_slice(instances));
    }
}

/// The scatter path's CPU work: a header, every instance's destination, then the instances.
fn write_scatter(scene: &InstanceScene, destinations: &mut Vec<u32>, staging: &mut Vec<u8>) {
    destinations.clear();
    for range in scene.uploads() {
        let bit = if range.region == Region::Dynamic { 1 << 31 } else { 0 };
        destinations.extend((range.first..range.first + range.count).map(|slot| slot | bit));
    }
    staging.clear();
    let count = u32::try_from(destinations.len()).unwrap_or(u32::MAX);
    staging.extend_from_slice(bytemuck::bytes_of(&[count, 0, 0, 0]));
    staging.extend_from_slice(bytemuck::cast_slice(destinations));
    staging.resize(16 + destinations.len().div_ceil(4) * 16, 0);
    write_ranges_appending(scene, staging);
}

fn write_ranges_appending(scene: &InstanceScene, staging: &mut Vec<u8>) {
    for range in scene.uploads() {
        let first = range.first as usize;
        let instances = &scene.instances(range.region)[first..first + range.count as usize];
        staging.extend_from_slice(bytemuck::cast_slice(instances));
    }
}

#[library_benchmark]
#[bench::frame(args = (()), setup = moving)]
fn render_sync_moving(mut fixture: Fixture) -> Fixture {
    black_box(fixture.scene.sync(&mut fixture.dm));
    fixture.scene.coalesce();
    write_ranges(&fixture.scene, &mut fixture.staging);
    black_box(fixture.staging.len());
    fixture
}

#[library_benchmark]
#[bench::frame(args = (()), setup = scattered)]
fn render_sync_scattered(mut fixture: Fixture) -> Fixture {
    black_box(fixture.scene.sync(&mut fixture.dm));
    fixture.scene.coalesce();
    write_scatter(&fixture.scene, &mut fixture.destinations, &mut fixture.staging);
    black_box(fixture.staging.len());
    fixture
}

library_benchmark_group!(
    name = workloads,
    benchmarks = [render_sync_moving, render_sync_scattered]
);

main!(
    config = LibraryBenchmarkConfig::default().tool(Callgrind::default().soft_limits([(EventKind::Ir, IR_LIMIT_PCT)])),
    library_benchmark_groups = workloads
);
