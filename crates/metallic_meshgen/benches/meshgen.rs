//! Generation time per shape and level of detail (default size, default bevel, AO on).
//!
//! Run with `cargo bench -p metallic_meshgen -j 4`. Numbers are recorded in `docs/agents/35-meshgen.md`.
use criterion::{Criterion, criterion_group, criterion_main};
use metallic_meshgen::{Lod, MeshDesc, Shape, generate};
use std::hint::black_box;

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("generate");
    group.sample_size(20);
    for shape in Shape::ALL {
        for lod in Lod::ALL {
            if lod != Lod::High && !shape.is_curved() {
                continue; // LOD does not change flat shapes much
            }
            let desc = MeshDesc::new(shape, shape.default_size()).with_lod(lod);
            group.bench_function(format!("{}/{:?}", shape.name(), lod), |b| {
                b.iter(|| generate(black_box(&desc)))
            });
        }
    }
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
