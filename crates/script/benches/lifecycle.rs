//! What following the scripts' lifecycle costs a game that creates and destroys many instances (issue #68).
//!
//! Run with `cargo bench -p metallic_script --bench lifecycle`. Each iteration is one "frame": 1,000 `Part`s are
//! created and parented to `Workspace`, then destroyed one by one, and then the frame's signals are dispatched. The
//! target is that a running game's host adds no more than noise to the same frame without one.
//!
//! - `create_destroy_1000/no_host`: the data model alone, nothing connected.
//! - `create_destroy_1000/no_host_connected`: the data model with one unrelated connection, which is what a running
//!   script's host watch costs the data model (destroyed instances stay readable until their events are delivered
//!   while anything is connected); its deliveries are drained each frame.
//! - `create_destroy_1000/host_empty_game`: a started game with no scripts, and a whole frame (four `step`s).
//! - `create_destroy_1000/host_one_script`: a started game running one script that sleeps, and a whole frame.

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use metallic_core::{ClassId, DataModel, Delivery, Id, ListenerKey, ScriptValue, Value};
use metallic_script::{
    ChunkId, CompileError, HostContext, MemorySources, ModuleKey, Phase, Resume, RunContext, ScriptFailure, ScriptHost,
    ScriptLanguage, ThreadId, YieldReason,
};

/// Parts created and destroyed per frame.
const PARTS: usize = 1000;
const FRAME: f64 = 1.0 / 60.0;

/// A backend whose every script sleeps for good, so the numbers are the host's.
#[derive(Default)]
struct SleepLanguage {
    next: u64,
}

impl ScriptLanguage for SleepLanguage {
    fn name(&self) -> &str {
        "sleep"
    }

    fn extensions(&self) -> &[&str] {
        &["sleep"]
    }

    fn compile(&mut self, _source: &str, _chunk_name: &str) -> Result<ChunkId, CompileError> {
        Ok(ChunkId(0))
    }

    fn spawn(&mut self, _chunk: ChunkId, _ctx: &mut HostContext<'_>) -> Result<ThreadId, ScriptFailure> {
        self.next += 1;
        Ok(ThreadId(self.next))
    }

    fn spawn_listener(
        &mut self,
        _listener: ListenerKey,
        _ctx: &mut HostContext<'_>,
    ) -> Result<ThreadId, ScriptFailure> {
        Err(ScriptFailure::runtime("the bench backend has no listeners", None))
    }

    fn spawn_module(
        &mut self,
        _chunk: ChunkId,
        _key: ModuleKey,
        _ctx: &mut HostContext<'_>,
    ) -> Result<ThreadId, ScriptFailure> {
        Err(ScriptFailure::runtime("the bench backend has no modules", None))
    }

    fn release_module(&mut self, _key: ModuleKey) {}

    fn resume_error(&mut self, _thread: ThreadId, message: &str, _ctx: &mut HostContext<'_>) -> Resume {
        Resume::Errored(ScriptFailure::runtime(message, None))
    }

    fn resume(&mut self, _thread: ThreadId, _args: &[ScriptValue], _ctx: &mut HostContext<'_>) -> Resume {
        Resume::Yielded(YieldReason::Wait { seconds: 1e9 })
    }

    fn release_thread(&mut self, _thread: ThreadId) {}

    fn release_listener(&mut self, _listener: ListenerKey) {}

    fn memory_used(&self) -> usize {
        0
    }

    fn set_memory_limit(&mut self, _bytes: Option<usize>) {}
}

/// A new game, its `Workspace` and the `Part` class.
fn game() -> (DataModel, Id, ClassId) {
    let dm = match metallic_classes::new_game() {
        Ok(dm) => dm,
        Err(error) => panic!("{error}"),
    };
    let registry = dm.registry();
    let (Some(workspace), Some(part)) = (registry.class_id("Workspace"), registry.class_id("Part")) else {
        panic!("no Workspace or Part class");
    };
    let Some(workspace) = dm.service(workspace) else {
        panic!("no Workspace");
    };
    (dm, workspace, part)
}

/// A host with the sleeping backend running a game, with one sleeping script in `Workspace` when `script` is set.
fn host_game(dm: &mut DataModel, workspace: Id, script: bool) -> ScriptHost {
    let mut host = ScriptHost::new();
    if let Err(error) = host.register_language(Box::new(SleepLanguage::default())) {
        panic!("{error}");
    }
    host.set_phase_budget(Duration::MAX);
    let mut sources = MemorySources::new();
    if script {
        sources.insert("Bench.sleep", "sleep");
        let Some(class) = dm.registry().class_id("Script") else {
            panic!("no Script class");
        };
        let id = ok(dm.create(class));
        ok(dm.set_by_name(id, "Source", Value::String("Bench.sleep".into())));
        ok(dm.set_parent(id, Some(workspace)));
    }
    ok(host.start_game(dm, RunContext::Server, sources));
    assert_eq!(host.thread_count(), usize::from(script));
    host
}

fn ok<T, E: std::fmt::Display>(result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{error}"),
    }
}

/// Creates `PARTS` parts under `parent` and destroys them, reusing `parts`.
fn churn(dm: &mut DataModel, parent: Id, class: ClassId, parts: &mut Vec<Id>) {
    for _ in 0..PARTS {
        let id = ok(dm.create(class));
        ok(dm.set_parent(id, Some(parent)));
        parts.push(id);
    }
    for id in parts.drain(..) {
        ok(dm.destroy(id));
    }
}

fn bench_lifecycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("create_destroy_1000");
    let mut parts = Vec::with_capacity(PARTS);

    let (mut dm, workspace, part) = game();
    let mut deliveries: Vec<Delivery> = Vec::new();
    group.bench_function("no_host", |b| {
        b.iter(|| {
            churn(&mut dm, workspace, part, &mut parts);
            dm.drain_deliveries(&mut deliveries);
            black_box(deliveries.len());
            deliveries.clear();
        });
    });

    let (mut dm, workspace, part) = game();
    let Some(folder) = dm.registry().class_id("Folder") else {
        panic!("no Folder class");
    };
    let quiet = ok(dm.create(folder));
    let Some(name) = dm.registry().prop_id(folder, "Name") else {
        panic!("no Name property");
    };
    ok(dm.connect_property_changed(quiet, name, ListenerKey(1)));
    group.bench_function("no_host_connected", |b| {
        b.iter(|| {
            churn(&mut dm, workspace, part, &mut parts);
            dm.drain_deliveries(&mut deliveries);
            black_box(deliveries.len());
            deliveries.clear();
        });
    });

    for (label, script) in [("host_empty_game", false), ("host_one_script", true)] {
        let (mut dm, workspace, part) = game();
        let mut host = host_game(&mut dm, workspace, script);
        let mut now = 0.0;
        group.bench_function(label, |b| {
            b.iter(|| {
                churn(&mut dm, workspace, part, &mut parts);
                now += FRAME;
                let mut resumed = 0;
                for phase in Phase::FRAME {
                    resumed += host.step(&mut dm, phase, now).resumed;
                }
                black_box(resumed)
            });
        });
        assert_eq!(host.thread_count(), usize::from(script));
        assert!(host.output().lines.is_empty(), "{:?}", host.output().lines);
    }
    group.finish();
}

criterion_group!(benches, bench_lifecycle);
criterion_main!(benches);
