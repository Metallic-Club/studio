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
//! - `render_cull` (#114): [`InstanceScene::cull`] of the same 10,000 parts for a fixed camera that sees part of the
//!   grid, with `CullSettings::default()`, into a warmed [`VisibleSet`]. 1% of the parts moved since the last cull
//!   (half across grid cells, half within their cell), so the measured call pays for moving their grid items
//!   (the sync only queues them) as well as for the walk of the view.
#![allow(
    missing_docs,
    reason = "gungraun's macros generate public modules without doc comments"
)]

use std::hint::black_box;

use gungraun::{Callgrind, EventKind, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group, main};
use metallic_core::glam::Vec3;
use metallic_core::{CFrame, Color3, DataModel, DmError, Id, Value};
use metallic_render::scene::{GpuInstance, Region, RenderComponents};
use metallic_render::{Camera, CullSettings, CullView, InstanceScene, VisibleSet};

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

/// A scene for the cull workload: the fixture, the view to cull for and the set the culls write into.
struct CullFixture {
    fixture: Fixture,
    view: CullView,
    visible: VisibleSet,
}

/// Moves the [`CHANGES`] cull movers for move number `frame`. Even movers cross into another grid cell (cells are
/// 32 m) and back, odd movers stay inside their cell and shift by half a metre in height. A mover's position only
/// depends on `frame`'s parity, so every frame changes all of them.
fn cull_movers(fixture: &mut Fixture, frame: usize) {
    let shifted = frame % 2 == 1;
    for i in 0..CHANGES {
        let part = i * 97;
        let (mut x, mut y, z) = ((part % 100) as f32 * 4.0, 1.0, (part / 100) as f32 * 4.0);
        if shifted {
            if i % 2 == 0 {
                x += 40.0;
            } else {
                y += 0.5;
            }
        }
        let id = fixture.parts[part];
        ok(fixture
            .dm
            .set_by_name(id, "CFrame", Value::CFrame(CFrame::new(Vec3::new(x, y, z)))));
    }
}

/// The parts (a 400 m square) seen from above the grid's south-west corner, looking across it: part of the cells
/// are wholly in view, part cross the frustum's edges and the rest (about two fifths of the parts) are outside.
fn cull_view(scene: &InstanceScene) -> CullView {
    let place = CFrame::look_at(Vec3::new(60.0, 30.0, 330.0), Vec3::new(330.0, 0.0, 120.0), Vec3::Y);
    let camera = Camera::from_cframe(place, 45.0);
    CullView::for_camera(&camera, scene.origin(), 1920, 1080, &CullSettings::default())
}

/// The movers moved and culled for a few frames (so they sit in the dynamic region and the visible set has grown
/// to its size), then moved once more, synced and coalesced: their grid items are left for the measured cull.
fn culling(_: ()) -> CullFixture {
    let mut fixture = Fixture::new();
    let view = cull_view(&fixture.scene);
    let mut visible = VisibleSet::new();
    fixture.scene.cull(&view, &mut visible);
    for frame in 1..=4 {
        cull_movers(&mut fixture, frame);
        fixture.settle();
        fixture.scene.cull(&view, &mut visible);
    }
    cull_movers(&mut fixture, 5);
    fixture.settle();
    CullFixture { fixture, view, visible }
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

#[library_benchmark]
#[bench::frame(args = (()), setup = culling)]
fn render_cull(mut state: CullFixture) -> CullFixture {
    black_box(state.fixture.scene.cull(&state.view, &mut state.visible));
    black_box(state.visible.len());
    state
}

/// TEMPORARY (#80 diagnosis, to be removed): the cull with nothing left to move, the walk alone.
fn culling_walk(_: ()) -> CullFixture {
    let mut state = culling(());
    state.fixture.scene.cull(&state.view, &mut state.visible);
    state
}

#[library_benchmark]
#[bench::frame(args = (()), setup = culling_walk)]
fn render_cull_walk(mut state: CullFixture) -> CullFixture {
    black_box(state.fixture.scene.cull(&state.view, &mut state.visible));
    black_box(state.visible.len());
    state
}

library_benchmark_group!(
    name = workloads,
    benchmarks = [render_sync_moving, render_sync_scattered, render_cull, render_cull_walk]
);

main!(
    config = LibraryBenchmarkConfig::default().tool(Callgrind::default().soft_limits([(EventKind::Ir, IR_LIMIT_PCT)])),
    library_benchmark_groups = workloads
);
