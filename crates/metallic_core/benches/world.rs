//! Baseline benchmarks for the Instance tree hot paths.
//!
//! Run with `cargo bench -p metallic_core --bench world`. The numbers are recorded in
//! `docs/research/2026-10-30-performance-and-scale.md`. The `frame_read` group copies what the
//! renderer and the physics sync do each frame: read the drawable properties of every part.
use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use metallic_core::{Color3, InstanceId, Value, Vec3, World, load_scene, save_scene};
use std::hint::black_box;

/// Properties a renderer or physics sync reads per BasePart.
const PART_PROPS: [&str; 7] = [
    "Position",
    "Orientation",
    "Size",
    "Anchored",
    "CanCollide",
    "Material",
    "Shape",
];

/// A world with `n` Parts in one Model under Workspace, each with a few non-default properties,
/// like a hand-built level.
fn level(n: usize) -> (World, Vec<InstanceId>) {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").expect("Workspace exists");
    let model = w.create("Model", Some(ws)).expect("create Model");
    let mut parts = Vec::with_capacity(n);
    for i in 0..n {
        let p = w.create("Part", Some(model)).expect("create Part");
        let f = i as f64;
        w.set(p, "Position", Value::Vector3(Vec3::new(f, 1.0, -f)))
            .expect("set Position");
        w.set(p, "Size", Value::Vector3(Vec3::new(4.0, 1.0, 8.0)))
            .expect("set Size");
        w.set(p, "Color", Value::Color3(Color3::new(120.0, 90.0, 200.0)))
            .expect("set Color");
        w.set(p, "Anchored", Value::Bool(true))
            .expect("set Anchored");
        parts.push(p);
    }
    (w, parts)
}

fn property_access(c: &mut Criterion) {
    let (mut w, parts) = level(100);
    let p = parts[50];
    let mut g = c.benchmark_group("property");
    g.bench_function("get_set_value", |b| {
        b.iter(|| black_box(w.get(black_box(p), "Position")))
    });
    g.bench_function("get_default_value", |b| {
        b.iter(|| black_box(w.get(black_box(p), "Transparency")))
    });
    g.bench_function("get_name", |b| {
        b.iter(|| black_box(w.get(black_box(p), "Name")))
    });
    g.bench_function("get_ref_set_value", |b| {
        b.iter(|| black_box(w.get_ref(black_box(p), "Position")))
    });
    g.bench_function("get_ref_default_value", |b| {
        b.iter(|| black_box(w.get_ref(black_box(p), "Transparency")))
    });
    g.bench_function("get_ref_name", |b| {
        b.iter(|| black_box(w.get_ref(black_box(p), "Name")))
    });
    let mut x = 0.0;
    g.bench_function("set_position", |b| {
        b.iter(|| {
            x += 1.0;
            w.set(
                black_box(p),
                "Position",
                Value::Vector3(Vec3::new(x, 1.0, 0.0)),
            )
        })
    });
    g.bench_function("is_a_basepart", |b| {
        b.iter(|| black_box(w.is_a(black_box(p), "BasePart")))
    });
    g.finish();
}

fn frame_read(c: &mut Criterion) {
    let mut g = c.benchmark_group("frame_read");
    for n in [300usize, 5_000] {
        let (w, parts) = level(n);
        g.bench_with_input(BenchmarkId::new("seven_props_per_part", n), &n, |b, _| {
            b.iter(|| {
                let mut sum = 0usize;
                for &p in &parts {
                    if !w.is_a(p, "BasePart") {
                        continue;
                    }
                    for prop in PART_PROPS {
                        if black_box(w.get(p, prop)).is_ok() {
                            sum += 1;
                        }
                    }
                }
                black_box(sum)
            })
        });
        g.bench_with_input(
            BenchmarkId::new("borrowed_seven_props_per_part", n),
            &n,
            |b, _| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &p in &parts {
                        if !w.is_a(p, "BasePart") {
                            continue;
                        }
                        for prop in PART_PROPS {
                            if black_box(w.get_ref(p, prop)).is_ok() {
                                sum += 1;
                            }
                        }
                    }
                    black_box(sum)
                })
            },
        );
    }
    g.finish();
}

fn tree_ops(c: &mut Criterion) {
    let mut g = c.benchmark_group("tree");
    let (w, parts) = level(1_000);
    let model = w.parent(parts[0]).expect("parts have a parent");
    g.bench_function("find_first_child_miss_1000", |b| {
        b.iter(|| black_box(w.find_first_child(model, "NoSuchChild")))
    });
    g.bench_function("full_name", |b| {
        b.iter(|| black_box(w.full_name(black_box(parts[999]))))
    });
    g.bench_function("create_1000_parts", |b| b.iter(|| black_box(level(1_000))));
    g.bench_function("destroy_1000_siblings_one_by_one", |b| {
        b.iter_batched(
            || level(1_000),
            |(mut w, parts)| {
                for p in parts {
                    w.destroy(p);
                }
                w
            },
            BatchSize::LargeInput,
        )
    });
    let (big, _) = level(5_000);
    g.bench_function("deep_clone_5000", |b| {
        b.iter(|| black_box(big.deep_clone()))
    });
    g.finish();
}

fn scene_files(c: &mut Criterion) {
    let mut g = c.benchmark_group("scene");
    g.sample_size(20);
    // One folder per process, so two benchmark runs at once do not write the same file.
    let dir = std::env::temp_dir().join(format!("metallic_core_bench-{}", std::process::id()));
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("level_5000.ron");
    let (w, _) = level(5_000);
    g.bench_function("save_5000", |b| b.iter(|| save_scene(&w, &path)));
    if save_scene(&w, &path).is_ok() {
        g.bench_function("load_5000", |b| b.iter(|| black_box(load_scene(&path))));
    }
    g.finish();
    let _ = std::fs::remove_dir_all(&dir);
}

criterion_group!(benches, property_access, frame_read, tree_ops, scene_files);
criterion_main!(benches);
