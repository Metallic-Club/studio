//! CPU time of the GPU scene (#76, ADR 0008 §2) on 100,000 parts: a frame's sync with 1% of the parts changing
//! (target ≤ 0.5 ms on the dev PC), the same with its upload on a real device, a full rebuild, and a cull (#79).
//!
//! ```text
//! cargo bench -p metallic_render --bench gpu_scene
//! ```
//!
//! Two kinds of change are measured, each 1,000 parts per frame. `moving`: the same parts move every frame, as
//! moving parts do, so they sit together in the dynamic region after their second frame. `scattered`: a different
//! spread of parts is recoloured every frame, each a lone slot in the static region: the worst case for
//! coalescing. Edits to the data model happen outside the timed part. The upload benches need an adapter and are
//! skipped without one; `upload/per_range_write_buffer` is the alternative the scene's single staging write was
//! chosen over: one `queue.write_buffer` per range.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use metallic_core::glam::Vec3;
use metallic_core::{CFrame, Color3, DataModel, Id, PropId, Value};
use metallic_render::scene::{GpuInstance, RenderComponents, SCATTER_MIN_RANGES};
use metallic_render::{
    Camera, ContextOptions, CullSettings, CullView, GpuScene, InstanceScene, RenderContext, VisibleSet, wgpu,
};

const PARTS: usize = 100_000;
const CHANGES: usize = PARTS / 100;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Change {
    Moving,
    Scattered,
}

struct Game {
    dm: DataModel,
    workspace: Id,
    parts: Vec<Id>,
    cframe: PropId,
    color: PropId,
    frame: usize,
}

impl Game {
    fn new() -> Self {
        let mut dm = metallic_classes::new_game().expect("a game");
        let registry = dm.registry().clone();
        let workspace = dm
            .service(registry.class_id("Workspace").expect("Workspace"))
            .expect("a Workspace");
        let class = registry.class_id("Part").expect("Part");
        let cframe = registry.prop_id(class, "CFrame").expect("CFrame");
        let color = registry.prop_id(class, "Color").expect("Color");
        dm.reserve(PARTS);
        let parts = (0..PARTS)
            .map(|i| {
                let id = dm.create(class).expect("a part");
                let place = CFrame::new(Vec3::new((i % 300) as f32 * 4.0, 1.0, (i / 300) as f32 * 4.0));
                dm.set(id, cframe, Value::CFrame(place)).expect("placed");
                dm.set_parent(id, Some(workspace)).expect("parented");
                id
            })
            .collect();
        Self {
            dm,
            workspace,
            parts,
            cframe,
            color,
            frame: 0,
        }
    }

    /// One frame's 1% of changes.
    fn edit(&mut self, change: Change) {
        self.frame += 1;
        let frame = self.frame;
        match change {
            Change::Moving => {
                let y = 1.0 + frame as f32 * 0.001;
                for i in 0..CHANGES {
                    let id = self.parts[i * 97];
                    let place = CFrame::new(Vec3::new(i as f32, y, 0.0));
                    self.dm.set(id, self.cframe, Value::CFrame(place)).expect("moved");
                }
            }
            Change::Scattered => {
                let shade = (frame % 250) as f32 / 255.0;
                for i in 0..CHANGES {
                    let id = self.parts[(i * 7919 + frame * 31) % PARTS];
                    let colour = Color3::new(shade, 0.5, 0.5);
                    self.dm.set(id, self.color, Value::Color3(colour)).expect("recoloured");
                }
            }
        }
    }
}

/// Times `run` once per iteration after an untimed `edit`.
fn timed(iters: u64, game: &mut Game, change: Change, mut run: impl FnMut(&mut Game)) -> Duration {
    let mut total = Duration::ZERO;
    for _ in 0..iters {
        game.edit(change);
        let start = Instant::now();
        run(game);
        total += start.elapsed();
    }
    total
}

fn sync(c: &mut Criterion) {
    let mut group = c.benchmark_group("sync");
    for (name, change) in [
        ("100k_1pct_moving", Change::Moving),
        ("100k_1pct_scattered", Change::Scattered),
    ] {
        let mut game = Game::new();
        let mut scene =
            InstanceScene::attach(&mut game.dm, Some(game.workspace), &RenderComponents::builtin()).expect("a scene");
        scene.sync(&mut game.dm);
        scene.coalesce();
        // Settle: the movers are promoted and every vector has grown.
        for _ in 0..10 {
            game.edit(change);
            scene.sync(&mut game.dm);
            scene.coalesce();
        }
        group.bench_function(name, |b| {
            b.iter_custom(|iters| {
                timed(iters, &mut game, change, |game| {
                    black_box(scene.sync(&mut game.dm));
                    scene.coalesce();
                    black_box(scene.uploads().len());
                })
            });
        });
        eprintln!(
            "sync/{name}: {} upload ranges per frame, {} dynamic",
            scene.uploads().len(),
            scene.live(metallic_render::scene::Region::Dynamic)
        );
    }
    group.finish();
}

fn rebuild(c: &mut Criterion) {
    let mut game = Game::new();
    let mut scene =
        InstanceScene::attach(&mut game.dm, Some(game.workspace), &RenderComponents::builtin()).expect("a scene");
    scene.sync(&mut game.dm);
    c.bench_function("rebuild/100k", |b| {
        b.iter(|| {
            scene.rebuild_all(&mut game.dm);
            black_box(scene.sync(&mut game.dm));
            scene.coalesce();
        });
    });
}

/// One cull of the 100,000 parts (#79) for a 1920 × 1080 camera in the middle of the grid at head height, looking
/// along it: 70° sees about 30% of the parts, 26° about 10%.
fn cull(c: &mut Criterion) {
    let mut game = Game::new();
    let mut scene =
        InstanceScene::attach(&mut game.dm, Some(game.workspace), &RenderComponents::builtin()).expect("a scene");
    scene.sync(&mut game.dm);
    scene.coalesce();
    let mut visible = VisibleSet::new();
    let mut group = c.benchmark_group("cull");
    for (name, fov) in [("100k_30pct", 70.0), ("100k_10pct", 26.0)] {
        let eye = Vec3::new(600.0, 6.0, 668.0);
        let camera = Camera::from_cframe(CFrame::look_at(eye, eye + Vec3::X, Vec3::Y), fov);
        let view = CullView::for_camera(&camera, scene.origin(), 1920, 1080, &CullSettings::default());
        let stats = scene.cull(&view, &mut visible);
        eprintln!("cull/{name}: {stats:?}");
        group.bench_function(name, |b| b.iter(|| black_box(scene.cull(&view, &mut visible))));
    }
    group.finish();
}

fn device() -> Option<RenderContext> {
    match RenderContext::new_offscreen(&ContextOptions::default()) {
        Ok(context) => Some(context),
        Err(error) => {
            eprintln!("upload benches skipped: {error}");
            None
        }
    }
}

/// Waits for `submission` (outside the timing), so the GPU keeps up and wgpu's queues do not grow.
fn wait(context: &RenderContext, submission: wgpu::SubmissionIndex) {
    let _ = context.device().poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    });
}

fn upload(c: &mut Criterion) {
    let Some(context) = device() else { return };
    let mut group = c.benchmark_group("upload");
    let cases = [
        ("100k_1pct_moving", Change::Moving, SCATTER_MIN_RANGES),
        ("100k_1pct_moving_scatter", Change::Moving, 0),
        ("100k_1pct_scattered", Change::Scattered, SCATTER_MIN_RANGES),
        ("100k_1pct_scattered_copies", Change::Scattered, u32::MAX),
    ];
    for (name, change, scatter_min_ranges) in cases {
        let mut game = Game::new();
        let mut scene = GpuScene::new(
            &context,
            &mut game.dm,
            Some(game.workspace),
            &RenderComponents::builtin(),
        )
        .expect("a scene");
        scene.set_scatter_min_ranges(scatter_min_ranges);
        for _ in 0..10 {
            game.edit(change);
            scene.sync(&mut game.dm);
            let mut encoder = context
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            scene.upload(&mut encoder);
            wait(&context, context.queue().submit(Some(encoder.finish())));
        }
        // Sync, upload and the submission that carries it: the scene's whole CPU cost in a frame.
        group.bench_function(format!("sync_upload_{name}"), |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    game.edit(change);
                    let start = Instant::now();
                    scene.sync(&mut game.dm);
                    let mut encoder = context
                        .device()
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                    black_box(scene.upload(&mut encoder));
                    let submission = context.queue().submit(Some(encoder.finish()));
                    total += start.elapsed();
                    wait(&context, submission);
                }
                total
            });
        });
    }

    // The alternative: the same scattered ranges, each written with its own `queue.write_buffer`.
    let mut game = Game::new();
    let mut scene =
        InstanceScene::attach(&mut game.dm, Some(game.workspace), &RenderComponents::builtin()).expect("a scene");
    scene.sync(&mut game.dm);
    scene.coalesce();
    let size = scene.instances(metallic_render::scene::Region::Static).len() as u64 * GpuInstance::SIZE;
    let buffer = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("bench instances"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    group.bench_function("per_range_write_buffer_100k_1pct_scattered", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                game.edit(Change::Scattered);
                let start = Instant::now();
                scene.sync(&mut game.dm);
                scene.coalesce();
                for range in scene.uploads() {
                    let first = range.first as usize;
                    let instances = &scene.instances(range.region)[first..first + range.count as usize];
                    context.queue().write_buffer(
                        &buffer,
                        u64::from(range.first) * GpuInstance::SIZE,
                        bytemuck::cast_slice(instances),
                    );
                }
                let submission = context.queue().submit(None);
                total += start.elapsed();
                wait(&context, submission);
            }
            total
        });
    });
    group.finish();
}

criterion_group!(benches, sync, rebuild, cull, upload);
criterion_main!(benches);
