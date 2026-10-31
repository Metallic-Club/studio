//! Cost of one `InputRuntime::evaluate` on the reference load of the input spec (section 13):
//! 4 contexts, 32 actions, 96 bindings and 40 raw events a frame. Budget: 0.1 ms (the perf lane
//! fails above 2x the Dev number of 0.05 ms).
//!
//! Run: `cargo bench -p metallic_runtime --bench input_eval -j 4`.
use criterion::{Criterion, criterion_group, criterion_main};
use metallic_runtime::InputRuntime;
use std::hint::black_box;

#[path = "../tests/common/reference_load.rs"]
mod reference_load;

fn bench(c: &mut Criterion) {
    let mut rt = InputRuntime::new(&reference_load::config());
    let dt = 1.0 / 60.0;
    let mut f = 0u64;
    // Pushing the 40 events, evaluating, and draining the action events: one frame of input.
    c.bench_function("input_eval/96_bindings", |b| {
        b.iter(|| {
            f += 1;
            let t = 10.0 + f as f64 * dt;
            reference_load::push_frame(&mut rt, f, t);
            rt.evaluate(t, dt as f32);
            black_box(rt.drain_events().count());
        })
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
