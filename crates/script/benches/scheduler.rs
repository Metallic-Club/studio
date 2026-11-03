//! Task scheduler benchmarks, through a minimal backend that does no work of its own, so the numbers are the
//! host's.
//!
//! Run with `cargo bench -p metallic_script --bench scheduler`. Issue #45 sets the target: a frame (all four phases)
//! with 5,000 sleepers and nothing due costs under 50 µs and allocates nothing.
//!
//! - `frame/5000_sleepers_none_due`: a whole frame, four `step`s, with 5,000 threads in `task.wait(1e9)`.
//! - `heartbeat/5000_due`: one `Heartbeat` that resumes 5,000 due sleepers, each going back to sleep (a
//!   `while true do task.wait(1) end` loop in 5,000 threads).
//! - `heartbeat/1000_deferred_chain`: one `Heartbeat` that wakes a thread which starts 20 chains of 50 deferred
//!   threads, each deferring the next: 1,000 deferred resumes. (One chain of 1,000 would pass Roblox's re-entrancy
//!   limit of 80, which the host enforces.)

use std::collections::HashMap;
use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use metallic_core::{DataModel, ListenerKey, ScriptValue};
use metallic_script::{
    ChunkId, CompileError, HostContext, ModuleKey, Phase, Resume, ScriptFailure, ScriptHost, ScriptLanguage, ThreadId,
    YieldReason,
};

/// What a bench thread does each time it is resumed.
#[derive(Clone, Copy, Debug)]
enum Behavior {
    /// The script's main thread: starts `count` sleepers of `seconds`, or becomes a kicker.
    Main(Plan),
    /// Sleeps `seconds` again (`while true do task.wait(seconds) end`).
    Sleep(f64),
    /// Defers `chains` chains of `links` threads, then sleeps until the next frame.
    Kicker { chains: u32, links: u32 },
    /// Defers the next link while `left` is not zero, then finishes.
    Chain(u32),
}

/// A compiled bench "script".
#[derive(Clone, Copy, Debug)]
enum Plan {
    Sleepers { count: u32, seconds: f64 },
    Kicker { chains: u32, links: u32 },
}

#[derive(Default)]
struct BenchLanguage {
    chunks: Vec<Plan>,
    threads: HashMap<ThreadId, Behavior>,
    next: u64,
}

impl BenchLanguage {
    fn thread(&mut self, behavior: Behavior) -> ThreadId {
        self.next += 1;
        let id = ThreadId(self.next);
        self.threads.insert(id, behavior);
        id
    }

    fn defer_chain(&mut self, left: u32, ctx: &mut HostContext<'_>) -> Option<Resume> {
        let next = self.thread(Behavior::Chain(left));
        match ctx.defer(next, Vec::new()) {
            Ok(()) => None,
            Err(error) => Some(Resume::Errored(ScriptFailure::from(error))),
        }
    }
}

impl ScriptLanguage for BenchLanguage {
    fn name(&self) -> &str {
        "bench"
    }

    fn extensions(&self) -> &[&str] {
        &["bench"]
    }

    fn compile(&mut self, source: &str, chunk_name: &str) -> Result<ChunkId, CompileError> {
        let words: Vec<&str> = source.split_whitespace().collect();
        let number = |i: usize| words.get(i).and_then(|w| w.parse::<f64>().ok());
        let plan = match (words.first().copied(), number(1), number(2)) {
            (Some("sleepers"), Some(count), Some(seconds)) => Plan::Sleepers {
                count: count as u32,
                seconds,
            },
            (Some("kicker"), Some(chains), Some(links)) => Plan::Kicker {
                chains: chains as u32,
                links: links as u32,
            },
            _ => {
                return Err(CompileError {
                    chunk: chunk_name.to_owned(),
                    line: 1,
                    column: None,
                    message: "expected `sleepers <n> <seconds>` or `kicker <chains> <links>`".to_owned(),
                });
            }
        };
        self.chunks.push(plan);
        Ok(ChunkId(self.chunks.len() as u64 - 1))
    }

    fn spawn(&mut self, chunk: ChunkId, _ctx: &mut HostContext<'_>) -> Result<ThreadId, ScriptFailure> {
        let plan = self
            .chunks
            .get(chunk.0 as usize)
            .copied()
            .ok_or_else(|| ScriptFailure::runtime("unknown chunk", None))?;
        Ok(self.thread(Behavior::Main(plan)))
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

    fn resume_error(&mut self, thread: ThreadId, message: &str, _ctx: &mut HostContext<'_>) -> Resume {
        self.threads.remove(&thread);
        Resume::Errored(ScriptFailure::runtime(message, None))
    }

    fn resume(&mut self, thread: ThreadId, _args: &[ScriptValue], ctx: &mut HostContext<'_>) -> Resume {
        let Some(&behavior) = self.threads.get(&thread) else {
            return Resume::Errored(ScriptFailure::runtime("dead thread", None));
        };
        match behavior {
            Behavior::Main(Plan::Sleepers { count, seconds }) => {
                for _ in 0..count {
                    // `task.spawn` of a function whose first statement is `task.wait(seconds)`.
                    let child = self.thread(Behavior::Sleep(seconds));
                    ctx.spawned(child, Resume::Yielded(YieldReason::Wait { seconds }));
                }
                Resume::Finished(Vec::new())
            }
            Behavior::Main(Plan::Kicker { chains, links }) | Behavior::Kicker { chains, links } => {
                self.threads.insert(thread, Behavior::Kicker { chains, links });
                for _ in 0..chains {
                    if let Some(failed) = self.defer_chain(links - 1, ctx) {
                        return failed;
                    }
                }
                Resume::Yielded(YieldReason::Wait { seconds: 0.0 })
            }
            Behavior::Sleep(seconds) => Resume::Yielded(YieldReason::Wait { seconds }),
            Behavior::Chain(left) => {
                if left > 0
                    && let Some(failed) = self.defer_chain(left - 1, ctx)
                {
                    return failed;
                }
                Resume::Finished(Vec::new())
            }
        }
    }

    fn release_thread(&mut self, thread: ThreadId) {
        self.threads.remove(&thread);
    }

    fn release_listener(&mut self, _listener: ListenerKey) {}

    fn memory_used(&self) -> usize {
        0
    }

    fn set_memory_limit(&mut self, _bytes: Option<usize>) {}
}

/// A game and a host running one bench script with `source`, with no phase budget.
fn host_running(source: &str) -> (DataModel, ScriptHost) {
    let mut dm = match metallic_classes::new_game() {
        Ok(dm) => dm,
        Err(error) => panic!("{error}"),
    };
    let mut host = ScriptHost::new();
    if let Err(error) = host.register_language(Box::new(BenchLanguage::default())) {
        panic!("{error}");
    }
    host.set_phase_budget(Duration::MAX);
    let Some(folder) = dm.registry().class_id("Folder") else {
        panic!("no Folder class");
    };
    let script = match dm.create(folder) {
        Ok(id) => id,
        Err(error) => panic!("{error}"),
    };
    if let Err(error) = host.run_script(&mut dm, script, "Bench.bench", source) {
        panic!("{error}");
    }
    (dm, host)
}

fn bench_scheduler(c: &mut Criterion) {
    const FRAME: f64 = 1.0 / 60.0;

    let (mut dm, mut host) = host_running("sleepers 5000 1e9");
    assert_eq!(host.sleeper_count(), 5000);
    let mut now = 0.0;
    c.bench_function("frame/5000_sleepers_none_due", |b| {
        b.iter(|| {
            now += FRAME;
            let mut resumed = 0;
            for phase in Phase::FRAME {
                resumed += host.step(&mut dm, phase, now).resumed;
            }
            black_box(resumed)
        });
    });
    assert_eq!(host.sleeper_count(), 5000);

    let (mut dm, mut host) = host_running("sleepers 5000 1");
    let mut now = 0.0;
    now += 1.0;
    assert_eq!(host.step(&mut dm, Phase::Heartbeat, now).resumed, 5000);
    c.bench_function("heartbeat/5000_due", |b| {
        b.iter(|| {
            now += 1.0;
            black_box(host.step(&mut dm, Phase::Heartbeat, now).resumed)
        });
    });

    let (mut dm, mut host) = host_running("kicker 20 50");
    // The main thread already queued the first 20 chains; run them, so each frame starts clean.
    let mut now = 0.0;
    assert_eq!(host.step(&mut dm, Phase::PreSimulation, now).resumed, 1000);
    now += FRAME;
    assert_eq!(host.step(&mut dm, Phase::Heartbeat, now).resumed, 1001);
    assert!(host.output().lines.is_empty(), "{:?}", host.output().lines);
    c.bench_function("heartbeat/1000_deferred_chain", |b| {
        b.iter(|| {
            now += FRAME;
            black_box(host.step(&mut dm, Phase::Heartbeat, now).resumed)
        });
    });
}

criterion_group!(benches, bench_scheduler);
criterion_main!(benches);
