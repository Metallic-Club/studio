//! Wall-time benches of the Luau bindings (issue #46). Each `*_1m` bench runs one script that does the operation a
//! million times; divide the reported time by 1,000,000 for the cost of one. `vm/hot_loop` runs 20 million loop
//! iterations with the interrupt (the time budget) installed, `vm/hot_loop_no_interrupt` the same without it.
//!
//! Run with `cargo bench -p metallic_luau --bench api`.
#![allow(missing_docs, reason = "criterion's macros generate undocumented items")]

use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use metallic_core::{DataModel, Id};
use metallic_luau::LuauLanguage;
use metallic_script::{Resume, ScriptHost};

/// A host with the Luau backend, a script instance and a part in the workspace named `Part`.
struct Bench {
    dm: DataModel,
    host: ScriptHost,
    script: Id,
}

fn bench(interrupt: bool) -> Bench {
    let mut dm = metallic_classes::new_game().unwrap_or_else(|e| panic!("{e}"));
    let mut luau = LuauLanguage::new(dm.registry().clone()).expect("Luau VM");
    luau.set_time_budget_enabled(interrupt).expect("interrupt");
    let mut host = ScriptHost::new();
    host.register_language(Box::new(luau)).expect("register");
    host.set_resume_budget(Duration::from_secs(60));
    let registry = dm.registry().clone();
    let workspace = dm
        .service(registry.class_id("Workspace").expect("Workspace"))
        .expect("workspace");
    let part = dm.create(registry.class_id("Part").expect("Part")).expect("part");
    dm.set_name(part, "Part".into()).expect("name");
    dm.set_parent(part, Some(workspace)).expect("parent");
    let script = dm.create(registry.class_id("Script").expect("Script")).expect("script");
    dm.set_parent(script, Some(workspace)).expect("parent");
    Bench { dm, host, script }
}

impl Bench {
    /// Runs `source` to completion (the chunk is compiled once and cached by the host).
    fn run(&mut self, source: &str) {
        let run = self
            .host
            .run_script(&mut self.dm, self.script, "Bench.luau", source)
            .expect("runs");
        assert!(matches!(run.resume, Resume::Finished(_)), "{:?}", run.resume);
    }
}

const PROPERTY_GET: &str = r#"
local part = workspace.Part
local p
for i = 1, 1000000 do
    p = part.Position
end
return p
"#;

/// A stored property, to tell the binding's cost from the computed `Position` getter's.
const PROPERTY_GET_STORED: &str = r#"
local part = workspace.Part
local t
for i = 1, 1000000 do
    t = part.Transparency
end
return t
"#;

const PROPERTY_SET: &str = r#"
local part = workspace.Part
for i = 1, 1000000 do
    part.Transparency = (i % 100) / 100
end
"#;

const METHOD_CALL: &str = r#"
local part = workspace.Part
local n = 0
for i = 1, 1000000 do
    if part:IsA("BasePart") then n += 1 end
end
return n
"#;

/// A loop with a little arithmetic and a table access per iteration, as game code has.
const HOT_LOOP: &str = r#"
local t = {1, 2, 3, 4}
local sum = 0
for i = 1, 20000000 do
    sum += t[i % 4 + 1] * 0.5
end
return sum
"#;

fn api(c: &mut Criterion) {
    let mut b = bench(true);
    let mut group = c.benchmark_group("api");
    group.sample_size(10);
    group.bench_function("part_property_get_1m", |bench| bench.iter(|| b.run(PROPERTY_GET)));
    group.bench_function("part_property_get_stored_1m", |bench| {
        bench.iter(|| b.run(PROPERTY_GET_STORED))
    });
    group.bench_function("part_property_set_1m", |bench| bench.iter(|| b.run(PROPERTY_SET)));
    group.bench_function("method_call_1m", |bench| bench.iter(|| b.run(METHOD_CALL)));
    group.finish();
}

fn vm(c: &mut Criterion) {
    let mut with = bench(true);
    let mut without = bench(false);
    let mut group = c.benchmark_group("vm");
    group.sample_size(10);
    group.bench_function("hot_loop", |bench| bench.iter(|| with.run(HOT_LOOP)));
    group.bench_function("hot_loop_no_interrupt", |bench| bench.iter(|| without.run(HOT_LOOP)));
    group.finish();
}

criterion_group!(benches, api, vm);
criterion_main!(benches);
