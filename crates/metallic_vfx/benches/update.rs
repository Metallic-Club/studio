//! Criterion benchmark: one simulation step of 100k live particles.
//!
//! Two emitters cases: the cheap path (gravity, drag, over-life curves) and the same plus curl noise,
//! which dominates the cost on the CPU. Results are recorded in `docs/agents/41-vfx.md`.

use criterion::{Criterion, criterion_group, criterion_main};
use metallic_vfx::{
    Curve, Direction, Effect, Emitter, Gradient, InitModule, Range, Shape, Simulation,
    SpawnModule, UpdateModule, Vec3,
};
use std::hint::black_box;

fn effect(curl: bool) -> Effect {
    let mut update = vec![
        UpdateModule::Acceleration {
            accel: Vec3::new(0.0, -9.8, 0.0),
            disabled: false,
        },
        UpdateModule::Drag {
            drag: 0.5,
            disabled: false,
        },
        UpdateModule::SizeOverLife {
            curve: Curve::smooth(&[(0.0, 0.2), (0.3, 1.0), (1.0, 0.0)]),
            disabled: false,
        },
        UpdateModule::ColorOverLife {
            gradient: Gradient::hex(&[(0.0, 0xFFF3C4), (0.5, 0xF25C19), (1.0, 0x3A3330)]),
            disabled: false,
        },
        UpdateModule::TransparencyOverLife {
            curve: Curve::linear(&[(0.0, 0.0), (1.0, 1.0)]),
            disabled: false,
        },
    ];
    if curl {
        update.push(UpdateModule::CurlNoise {
            strength: 2.0,
            scale: 3.0,
            scroll: Vec3::ZERO,
            octaves: 1,
            disabled: false,
        });
    }
    Effect {
        name: "bench".into(),
        emitters: vec![Emitter {
            name: "bench".into(),
            capacity: 100_000,
            spawn: vec![SpawnModule::Burst {
                time: 0.0,
                count: 100_000,
                cycles: 1,
                interval: 0.0,
                disabled: false,
            }],
            init: vec![
                InitModule::Shape {
                    shape: Shape::Sphere {
                        radius: 5.0,
                        partial: 1.0,
                    },
                    surface: false,
                    rotation: Vec3::ZERO,
                    offset: Vec3::ZERO,
                    disabled: false,
                },
                InitModule::Lifetime {
                    seconds: Range::new(19.0, 20.0),
                    disabled: false,
                },
                InitModule::Velocity {
                    speed: Range::new(1.0, 3.0),
                    direction: Direction::Axis(Vec3::Y),
                    spread: (360.0, 360.0),
                    disabled: false,
                },
            ],
            update,
            ..Emitter::default()
        }],
        ..Effect::default()
    }
}

fn bench_update(c: &mut Criterion) {
    for (label, curl) in [("update_100k", false), ("update_100k_curl", true)] {
        let Ok(mut sim) = Simulation::new(effect(curl), 1) else {
            return;
        };
        sim.step(1.0 / 60.0);
        assert_eq!(sim.particle_count(), 100_000);
        c.bench_function(label, |b| {
            b.iter_batched_ref(
                || sim.clone(),
                |s| s.step(black_box(1.0 / 60.0)),
                criterion::BatchSize::LargeInput,
            );
        });
    }
}

criterion_group!(benches, bench_update);
criterion_main!(benches);
