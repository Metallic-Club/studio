//! Modeller hot paths on a 100k-face grid: select all, extrude the region, triangulate.
//!
//! Run: `cargo bench -p metallic_modeling --bench edit`.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use glam::Vec3;
use metallic_modeling::{EditMesh, MeshCommand, NormalMode, Op, Primitive, Selection, triangulate};

/// 317 x 317 = 100 489 quads.
fn big_grid() -> EditMesh {
    let mesh = Primitive::Grid {
        x: 317,
        z: 317,
        size: 100.0,
    }
    .build(Vec3::ZERO)
    .unwrap_or_default();
    EditMesh::new(mesh)
}

fn bench(c: &mut Criterion) {
    let base = big_grid();
    let mut group = c.benchmark_group("modeling_100k");
    group.sample_size(10);
    group.bench_function("select_all_faces", |b| {
        b.iter(|| black_box(Selection::from_faces(&base.mesh, base.mesh.face_ids())));
    });
    let all = Selection::from_faces(&base.mesh, base.mesh.face_ids());
    group.bench_function("extrude_region", |b| {
        b.iter_batched(
            || EditMesh {
                mesh: base.mesh.clone(),
                selection: all.clone(),
            },
            |mut edit| {
                black_box(MeshCommand::run(
                    &mut edit,
                    &Op::ExtrudeFaces {
                        distance: 1.0,
                        individual: false,
                    },
                ))
            },
            criterion::BatchSize::LargeInput,
        );
    });
    group.bench_function("triangulate", |b| {
        b.iter(|| black_box(triangulate(&base.mesh, NormalMode::default())));
    });
    group.bench_function("select_extrude_triangulate", |b| {
        b.iter_batched(
            || base.clone(),
            |mut edit| {
                edit.selection = Selection::from_faces(&edit.mesh, edit.mesh.face_ids());
                let cmd = MeshCommand::run(
                    &mut edit,
                    &Op::ExtrudeFaces {
                        distance: 1.0,
                        individual: false,
                    },
                );
                black_box((cmd.is_ok(), triangulate(&edit.mesh, NormalMode::default())))
            },
            criterion::BatchSize::LargeInput,
        );
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
