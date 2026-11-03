//! Data model benchmarks: the hot reads and writes, bulk creation, a renderer-style read of many parts, and a
//! change drain.
//!
//! Run with `cargo bench -p metallic_core --bench data_model`. Issue #14 sets the targets (release, Ryzen 5 5500
//! class): `set(CFrame)` ≤ 100 ns, `get_ref` ≤ 20 ns, creating 1,000 parts with 4 properties set ≤ 1 ms, reading
//! 7 properties of 5,000 parts ≤ 0.3 ms, draining 50 changed of 5,000 ≤ 20 µs, and creating 100,000 instances
//! with one property set each < 50 ms.

#[path = "../tests/support/classes.rs"]
mod classes;

use std::hint::black_box;
use std::time::{Duration, Instant};

use classes::Ids;
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use metallic_core::{CFrame, Color3, DataModel, DirtyBits, DmError, EnumItem, Id, Value, Vector3};

fn ok<T>(result: Result<T, DmError>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{error}"),
    }
}

fn neon(dm: &DataModel) -> EnumItem {
    match dm.registry().enum_item("Material", "Neon") {
        Some(item) => item,
        None => panic!("no Material.Neon"),
    }
}

/// A part under the workspace with four non-default properties, as a scene has them.
fn add_part(dm: &mut DataModel, ids: &Ids, neon: &EnumItem, i: usize) -> Id {
    let p = ok(dm.create(ids.part));
    let x = i as f32;
    ok(dm.set(p, ids.cframe, Value::CFrame(CFrame::new(Vector3::new(x, 0.0, 0.0)))));
    ok(dm.set(p, ids.size, Value::Vector3(Vector3::new(2.0, 1.0, 4.0))));
    ok(dm.set(p, ids.color, Value::Color3(Color3::new(1.0, 0.0, 0.0))));
    ok(dm.set(p, ids.material, Value::Enum(neon.clone())));
    ok(dm.set_parent(p, Some(ids.workspace)));
    p
}

/// A game with `count` parts and the render and physics consumers registered and drained.
fn scene(count: usize) -> (DataModel, Ids, Vec<Id>) {
    let (mut dm, ids) = classes::game();
    let neon = neon(&dm);
    let render = ok(dm.register_consumer(DirtyBits::TRANSFORM | DirtyBits::VISUAL | DirtyBits::STRUCTURE));
    let physics = ok(dm.register_consumer(DirtyBits::TRANSFORM | DirtyBits::PHYSICS | DirtyBits::STRUCTURE));
    let parts = (0..count).map(|i| add_part(&mut dm, &ids, &neon, i)).collect();
    let mut out = Vec::new();
    dm.drain_changes(render, &mut out);
    dm.drain_changes(physics, &mut out);
    (dm, ids, parts)
}

fn single_property(c: &mut Criterion) {
    let (mut dm, ids, parts) = scene(1);
    let p = parts[0];
    let frames = [
        Value::CFrame(CFrame::new(Vector3::new(1.0, 2.0, 3.0))),
        Value::CFrame(CFrame::from_euler_xyz_deg(10.0, 20.0, 30.0)),
    ];
    let mut i = 0usize;
    c.bench_function("set_cframe", |b| {
        b.iter(|| {
            i += 1;
            ok(dm.set(black_box(p), ids.cframe, frames[i & 1].clone()));
        })
    });
    c.bench_function("get_ref_stored", |b| {
        b.iter(|| black_box(ok(dm.get_ref(black_box(p), black_box(ids.cframe)))).clone())
    });
    c.bench_function("get_ref_default", |b| {
        b.iter(|| black_box(ok(dm.get_ref(black_box(p), black_box(ids.transparency)))).clone())
    });
    c.bench_function("cframe_typed", |b| {
        b.iter(|| ok(dm.cframe(black_box(p), black_box(ids.cframe))))
    });
}

fn bulk(c: &mut Criterion) {
    c.bench_function("create_1000_parts_4_props", |b| {
        b.iter_batched(
            classes::game,
            |(mut dm, ids)| {
                let neon = neon(&dm);
                for i in 0..1_000 {
                    add_part(&mut dm, &ids, &neon, i);
                }
                dm
            },
            BatchSize::LargeInput,
        )
    });

    let mut group = c.benchmark_group("large");
    group.sample_size(10);
    group.bench_function("create_100k_set_one", |b| {
        b.iter_batched(
            classes::game,
            |(mut dm, ids)| {
                for i in 0..100_000 {
                    let p = ok(dm.create(ids.part));
                    ok(dm.set(p, ids.transparency, Value::Number((i % 9) as f64 / 10.0 + 0.1)));
                }
                dm
            },
            BatchSize::LargeInput,
        )
    });
    group.finish();
}

fn scene_reads(c: &mut Criterion) {
    let (mut dm, ids, parts) = scene(5_000);
    c.bench_function("read_7_props_5000_parts", |b| {
        b.iter(|| {
            let mut acc = 0.0f32;
            for &p in &parts {
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
            acc
        })
    });

    let consumer = ok(dm.register_consumer(DirtyBits::ALL));
    let mut out = Vec::with_capacity(64);
    dm.drain_changes(consumer, &mut out);
    let mut round = 0u32;
    c.bench_function("drain_50_of_5000", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                round += 1;
                let cf = Value::CFrame(CFrame::new(Vector3::new(round as f32, 0.0, 0.0)));
                for &p in parts.iter().step_by(100) {
                    ok(dm.set(p, ids.cframe, cf.clone()));
                }
                out.clear();
                let start = Instant::now();
                dm.drain_changes(consumer, &mut out);
                total += start.elapsed();
                black_box(&out);
            }
            total
        })
    });
}

criterion_group!(data_model_benches, single_property, bulk, scene_reads);
criterion_main!(data_model_benches);
