//! Step cost with 2,000 dynamic bodies (boxes and circles) piled in a bin, plus a character.
use criterion::{Criterion, criterion_group, criterion_main};
use metallic_physics2d::*;
use std::hint::black_box;

const BODIES: u64 = 2_000;

fn pile() -> Result<(PhysicsWorld2d, Scene2d), Physics2dError> {
    let mut bodies = vec![
        BodyDesc::new(BodyId(1), BodyKind::Static, Vec2::new(0.0, -2.0))
            .with_collider(ColliderDesc::rect(240.0, 4.0)),
        BodyDesc::new(BodyId(2), BodyKind::Static, Vec2::new(-120.0, 100.0))
            .with_collider(ColliderDesc::rect(4.0, 200.0)),
        BodyDesc::new(BodyId(3), BodyKind::Static, Vec2::new(120.0, 100.0))
            .with_collider(ColliderDesc::rect(4.0, 200.0)),
    ];
    for i in 0..BODIES {
        let x = -110.0 + (i % 80) as f64 * 2.75;
        let y = 2.0 + (i / 80) as f64 * 2.75;
        let collider = if i % 2 == 0 {
            ColliderDesc::rect(2.0, 2.0)
        } else {
            ColliderDesc::circle(1.0)
        };
        bodies.push(
            BodyDesc::new(BodyId(100 + i), BodyKind::Dynamic, Vec2::new(x, y))
                .with_collider(collider),
        );
    }
    let scene = Scene2d {
        bodies,
        joints: vec![],
    };
    let mut world = PhysicsWorld2d::new(WorldConfig::default())?;
    world.sync_from(&scene)?;
    world.add_character(
        BodyId(50),
        CharacterConfig::default(),
        Vec2::new(0.0, 120.0),
    )?;
    Ok((world, scene))
}

fn step_2000(c: &mut Criterion) {
    let Ok((mut world, mut scene)) = pile() else {
        return;
    };
    // Let the pile fall and settle into a dense, awake stack of contacts.
    for _ in 0..60 {
        let _ = world.step(&mut scene, 1.0 / 60.0);
    }
    c.bench_function("step 2000 bodies (1 tick, 4 substeps)", |b| {
        b.iter(|| {
            let _ = world.set_character_input(
                BodyId(50),
                CharacterInput {
                    move_x: 1.0,
                    jump: false,
                    drop_through: false,
                },
            );
            black_box(world.step(&mut scene, 1.0 / 60.0))
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(30);
    targets = step_2000
}
criterion_main!(benches);
