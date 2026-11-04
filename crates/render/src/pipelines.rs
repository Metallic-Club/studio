//! Pipeline compiles off the frame (ADR 0008 §9, #86): every pipeline is compiled on worker threads, the ones the
//! first frame needs before [`crate::Renderer::new`] returns and every other permutation in the background, and a
//! settings change that needs a pipeline still compiling waits for it while the frame keeps drawing as before.
//!
//! - **What a frame needs first** (the scene's pipelines for the format and MSAA count it draws with, without local
//!   lights, and the post chain's passes it runs) is compiled on every worker at once, and the renderer returns when
//!   they are done: the time to the first frame.
//! - **Everything else** follows in the background: the lit variants of the scene's pipelines and every post
//!   effect's passes (which a frame may switch on at any time), then the scene's pipelines for every other MSAA
//!   count.
//! - **A compile may wait for another** whose shader stages it shares (the depth prepass and the opaque pass after
//!   it for the opaque pass; another MSAA count for the first one's), so a backend that caches compiled stages
//!   inside the device (DX12, whose FXC takes up to half a second for the lit parts' shader) compiles each stage
//!   once, not once per worker that happens to need it at the same time.
//! - **A frame never waits.** It takes what has finished (one lock, nothing allocated) and draws with it: lights
//!   appear once their pipelines exist, and a settings change whose pipelines are still compiling takes effect in the
//!   first frame after they are ([`crate::Renderer`]'s settings say what is drawn until then).
//! - **OpenGL compiles on the thread that owns its context**, so there (and on WebAssembly, or a machine with one
//!   core, or where no thread can be started) every compile runs inline, all of them before
//!   [`crate::Renderer::new`] returns, as before #86.
//!
//! A compile that fails (a wgpu validation error, which panics in wgpu's default error handler) is caught on its
//! worker and counted ([`PipelineStats::failed`]); its pipeline stays missing and nothing that needs it is drawn.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::pipeline_cache::PipelineCacheStore;

/// Makes one pipeline on `device`, with the pipeline cache where there is one. Owns (clones of) its shader module
/// and layouts, so it can run on any thread.
pub(crate) type Recipe = Box<dyn FnOnce(&wgpu::Device, Option<&wgpu::PipelineCache>) -> wgpu::RenderPipeline + Send>;

/// One of the scene's pipelines for a colour format and MSAA count (see [`crate::parts`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PartPipeline {
    /// The depth prepass.
    Prepass,
    /// The sky's full-screen triangle.
    Sky,
    /// Opaque parts with `lights` (0: none, 1: local lights, 2: local lights and their shadows).
    Opaque(u8),
    /// Opaque parts after a depth prepass, likewise.
    Tested(u8),
    /// See-through parts, likewise.
    Transparent(u8),
}

impl PartPipeline {
    /// How many there are for one format and MSAA count.
    pub(crate) const COUNT: usize = 11;

    /// What a frame without local lights draws with, those with shader stages of their own first.
    pub(crate) const BASE: [PartPipeline; 5] = [
        PartPipeline::Opaque(0),
        PartPipeline::Transparent(0),
        PartPipeline::Sky,
        PartPipeline::Prepass,
        PartPipeline::Tested(0),
    ];

    /// What frames with local lights add (without and with their shadows), likewise.
    pub(crate) const LIT: [PartPipeline; 6] = [
        PartPipeline::Opaque(2),
        PartPipeline::Opaque(1),
        PartPipeline::Transparent(2),
        PartPipeline::Transparent(1),
        PartPipeline::Tested(2),
        PartPipeline::Tested(1),
    ];

    /// The pipeline of the same target whose shader stages this one reuses entirely: its compile waits for that
    /// one's (see the [module documentation](self)).
    pub(crate) fn shares_stages_of(self) -> Option<PartPipeline> {
        match self {
            PartPipeline::Prepass => Some(PartPipeline::Opaque(0)),
            PartPipeline::Tested(lights) => Some(PartPipeline::Opaque(lights)),
            PartPipeline::Sky | PartPipeline::Opaque(_) | PartPipeline::Transparent(_) => None,
        }
    }

    /// The pipeline's slot in a target's array.
    pub(crate) fn index(self) -> usize {
        match self {
            PartPipeline::Prepass => 0,
            PartPipeline::Sky => 1,
            PartPipeline::Opaque(lights) => 2 + 3 * usize::from(lights.min(2)),
            PartPipeline::Tested(lights) => 3 + 3 * usize::from(lights.min(2)),
            PartPipeline::Transparent(lights) => 4 + 3 * usize::from(lights.min(2)),
        }
    }
}

/// Which pipeline a compile makes: where the renderer puts it when it is done.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PipelineKey {
    /// One of the scene's pipelines for a colour format and MSAA count.
    Part {
        /// The scene's colour format.
        format: wgpu::TextureFormat,
        /// MSAA samples per pixel.
        samples: u32,
        /// Which one.
        pipeline: PartPipeline,
    },
    /// A variant of the finish pass (bit 0: bloom, bit 1: MLAA).
    Finish(u8),
    /// Bloom's first downsample: dual filter, 13 taps, or from a quarter.
    BloomFirst(u8),
    /// Bloom's later downsamples: dual filter or 13 taps.
    BloomDown(u8),
    /// Bloom's upsample.
    BloomUp,
    /// MLAA's edge pass.
    Mlaa,
    /// The upscale pass of frames without the finish pass.
    Upscale,
    /// The debug overlay.
    Overlay,
}

/// What the renderer's pipeline compiles have done: how many, how long, and how many are still compiling. For the
/// overlay and reports ([`crate::Renderer::pipeline_stats`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PipelineStats {
    /// Compiles queued or running.
    pub pending: u32,
    /// Pipelines compiled so far.
    pub compiled: u32,
    /// Compiles that failed (their pipelines stay missing; nothing that needs them is drawn).
    pub failed: u32,
    /// Whether compiles run on worker threads (not on OpenGL, WebAssembly or a single core).
    pub background: bool,
    /// Worker threads compiling (0 when compiles run inline).
    pub workers: u32,
    /// Milliseconds from the start of [`crate::Renderer::new`] until what the first frame draws was compiled: when
    /// it returned.
    pub first_frame_ms: f32,
    /// Milliseconds from the start of [`crate::Renderer::new`] until every compile queued so far was done, once they
    /// are (`None` while some are pending).
    pub all_ms: Option<f32>,
    /// The CPU time of every compile summed, in milliseconds (more than the wall time when workers ran in parallel).
    pub compile_ms: f32,
    /// A settings change is waiting for its pipelines; until they exist the frame draws as before it.
    pub switch_waiting: bool,
    /// Bytes of compiled pipelines loaded from disk at start: `None` without a pipeline cache (every backend but
    /// Vulkan, or no cache folder), 0 on a cold start (no usable file).
    pub cache_loaded: Option<u64>,
}

/// A queued compile.
struct Job {
    key: PipelineKey,
    recipe: Recipe,
    /// Starts only when this one is neither queued nor compiling.
    after: Option<PipelineKey>,
}

/// A finished compile: its pipeline, or `None` when it failed.
struct Done {
    key: PipelineKey,
    pipeline: Option<wgpu::RenderPipeline>,
    cpu: Duration,
}

/// What the workers and the renderer share.
#[derive(Default)]
struct PoolState {
    jobs: VecDeque<Job>,
    done: VecDeque<Done>,
    /// The compiles running.
    running: Vec<PipelineKey>,
    paused: bool,
    closed: bool,
    /// Compiles finished since the pipeline cache was last saved.
    unsaved: usize,
}

struct Pool {
    state: Mutex<PoolState>,
    /// Signalled when there is a job to take (or the pool closes, or resumes).
    work: Condvar,
    /// Signalled when a compile finishes.
    finished: Condvar,
}

impl Pool {
    fn lock(&self) -> MutexGuard<'_, PoolState> {
        // A panic while holding the lock leaves the queues whole (compiles run outside it).
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Takes the first queued job a worker may start (one whose `after` is neither queued nor compiling), waiting for
    /// one; `None` once the pool is closed.
    fn take(&self) -> Option<Job> {
        let mut state = self.lock();
        loop {
            if state.closed {
                return None;
            }
            let waits_on = |key: PipelineKey| state.running.contains(&key) || state.jobs.iter().any(|j| j.key == key);
            let startable = if state.paused {
                None
            } else {
                state
                    .jobs
                    .iter()
                    .position(|job| job.after.is_none_or(|after| !waits_on(after)))
            };
            if let Some(job) = startable.and_then(|at| state.jobs.remove(at)) {
                state.running.push(job.key);
                return Some(job);
            }
            state = self.work.wait(state).unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

/// A worker: takes jobs until the pool closes, and saves the pipeline cache whenever the queue runs dry.
fn work(pool: &Pool, device: &wgpu::Device, cache: Option<&PipelineCacheStore>) {
    while let Some(job) = pool.take() {
        let done = compile(job, device, cache);
        let save = {
            let mut state = pool.lock();
            if let Some(at) = state.running.iter().position(|&key| key == done.key) {
                state.running.swap_remove(at);
            }
            state.unsaved += 1;
            state.done.push_back(done);
            let idle = state.jobs.is_empty() && state.running.is_empty();
            let save = idle && std::mem::take(&mut state.unsaved) > 0;
            // Waiters, and workers whose next job waited for this one.
            pool.finished.notify_all();
            pool.work.notify_all();
            save
        };
        if save && let Some(cache) = cache {
            cache.save();
        }
    }
}

fn compile(job: Job, device: &wgpu::Device, cache: Option<&PipelineCacheStore>) -> Done {
    let start = Instant::now();
    let Job { key, recipe, .. } = job;
    let cache = cache.map(PipelineCacheStore::cache);
    let pipeline = catch_unwind(AssertUnwindSafe(|| recipe(device, cache))).ok();
    if std::env::var_os("METALLIC_TRACE_COMPILES").is_some() {
        eprintln!("compile {key:?}: {:?}", start.elapsed());
    }
    Done {
        key,
        pipeline,
        cpu: start.elapsed(),
    }
}

/// Runs the renderer's pipeline compiles: on worker threads, or inline where threads cannot compile.
pub(crate) struct Compiler {
    device: wgpu::Device,
    cache: Option<Arc<PipelineCacheStore>>,
    /// The workers' pool; `None` when compiles run inline.
    pool: Option<Arc<Pool>>,
    workers: u32,
    /// Compiles finished inline, not yet taken.
    inline_done: VecDeque<Done>,
    /// Compiles queued or running, not yet taken: never queued twice.
    queued: Vec<PipelineKey>,
    started: Instant,
    stats: PipelineStats,
}

impl Compiler {
    /// A compiler for `device`: with worker threads unless `inline` (OpenGL, which compiles on its context's
    /// thread) or the platform has none to spare.
    pub(crate) fn new(device: &wgpu::Device, cache: Option<Arc<PipelineCacheStore>>, inline: bool) -> Self {
        let started = Instant::now();
        let mut compiler = Self {
            device: device.clone(),
            cache,
            pool: None,
            workers: 0,
            inline_done: VecDeque::new(),
            queued: Vec::new(),
            started,
            stats: PipelineStats::default(),
        };
        if !inline && !cfg!(target_family = "wasm") {
            compiler.start_workers();
        }
        compiler.stats.background = compiler.pool.is_some();
        compiler.stats.workers = compiler.workers;
        compiler
    }

    /// Starts one worker per core but one (the calling thread's), at most eight. Where none can start, compiles run
    /// inline.
    fn start_workers(&mut self) {
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        let count = cores.saturating_sub(1).min(8);
        if count == 0 {
            return;
        }
        let pool = Arc::new(Pool {
            state: Mutex::new(PoolState::default()),
            work: Condvar::new(),
            finished: Condvar::new(),
        });
        let mut started = 0;
        for _ in 0..count {
            let (pool, device, cache) = (Arc::clone(&pool), self.device.clone(), self.cache.clone());
            let spawned = std::thread::Builder::new()
                .name("metallic pipelines".into())
                .spawn(move || work(&pool, &device, cache.as_deref()));
            if spawned.is_ok() {
                started += 1;
            }
        }
        if started > 0 {
            self.pool = Some(pool);
            self.workers = started;
        }
    }

    /// Queues the compile of `key`, to start once `after` is neither queued nor compiling (a pipeline whose shader
    /// stages it shares), unless it is queued already. Inline, it compiles now.
    pub(crate) fn submit(&mut self, key: PipelineKey, recipe: Recipe, after: Option<PipelineKey>) {
        if self.queued.contains(&key) {
            return;
        }
        self.queued.push(key);
        self.stats.pending = self.queued.len() as u32;
        self.stats.all_ms = None;
        let job = Job { key, recipe, after };
        match self.pool.as_ref() {
            Some(pool) => {
                pool.lock().jobs.push_back(job);
                pool.work.notify_one();
            }
            None => {
                let done = compile(job, &self.device, self.cache.as_deref());
                self.inline_done.push_back(done);
                self.save_inline();
            }
        }
    }

    /// Whether `key` is queued or compiling.
    pub(crate) fn is_queued(&self, key: PipelineKey) -> bool {
        self.queued.contains(&key)
    }

    /// Whether any compile is queued or compiling.
    pub(crate) fn busy(&self) -> bool {
        !self.queued.is_empty()
    }

    /// Hands every finished compile to `install`, without waiting. Allocates nothing; with nothing queued it touches
    /// nothing at all.
    pub(crate) fn drain(&mut self, mut install: impl FnMut(PipelineKey, wgpu::RenderPipeline)) {
        if self.queued.is_empty() {
            return;
        }
        while let Some(done) = self.inline_done.pop_front() {
            self.finish(done, &mut install);
        }
        if let Some(pool) = self.pool.clone() {
            loop {
                let done = pool.lock().done.pop_front();
                let Some(done) = done else {
                    break;
                };
                self.finish(done, &mut install);
            }
        }
    }

    /// Waits until nothing is queued or compiling, handing each compile to `install` as it finishes. Resumes paused
    /// compiles first. For [`crate::Renderer::new`] and loading screens, never a frame.
    pub(crate) fn wait(&mut self, mut install: impl FnMut(PipelineKey, wgpu::RenderPipeline)) {
        self.pause(false);
        loop {
            self.drain(&mut install);
            if self.queued.is_empty() {
                return;
            }
            let Some(pool) = self.pool.clone() else {
                return;
            };
            let state = pool.lock();
            if state.done.is_empty() {
                drop(
                    pool.finished
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner()),
                );
            }
        }
    }

    /// Stops starting compiles (those running finish) until called again with `false`.
    pub(crate) fn pause(&mut self, paused: bool) {
        if let Some(pool) = self.pool.as_ref() {
            pool.lock().paused = paused;
            pool.work.notify_all();
        }
    }

    fn finish(&mut self, done: Done, install: &mut impl FnMut(PipelineKey, wgpu::RenderPipeline)) {
        if let Some(at) = self.queued.iter().position(|&key| key == done.key) {
            self.queued.swap_remove(at);
        }
        self.stats.compile_ms += done.cpu.as_secs_f32() * 1000.0;
        match done.pipeline {
            Some(pipeline) => {
                self.stats.compiled += 1;
                install(done.key, pipeline);
            }
            None => self.stats.failed += 1,
        }
        self.stats.pending = self.queued.len() as u32;
        if self.queued.is_empty() {
            self.stats.all_ms = Some(self.since_start_ms());
        }
    }

    /// Saves the pipeline cache after inline compiles (workers save it themselves when their queue runs dry).
    fn save_inline(&mut self) {
        if let Some(cache) = self.cache.as_ref() {
            cache.mark_dirty();
        }
    }

    /// Writes the pipeline cache now if inline compiles changed it (at the end of a renderer's setup).
    pub(crate) fn flush_inline(&self) {
        if self.pool.is_none()
            && let Some(cache) = self.cache.as_ref()
        {
            cache.save_if_dirty();
        }
    }

    /// Milliseconds since the compiler was made.
    pub(crate) fn since_start_ms(&self) -> f32 {
        self.started.elapsed().as_secs_f32() * 1000.0
    }

    /// Marks the first frame's pipelines done, now.
    pub(crate) fn first_frame_ready(&mut self) {
        self.stats.first_frame_ms = self.since_start_ms();
    }

    /// What the compiles have done.
    pub(crate) fn stats(&self) -> PipelineStats {
        self.stats
    }
}

impl Drop for Compiler {
    /// Drops the compiles not started and lets the workers end; a compile running finishes on its own thread.
    fn drop(&mut self) {
        if let Some(pool) = self.pool.as_ref() {
            let mut state = pool.lock();
            state.closed = true;
            state.jobs.clear();
            drop(state);
            pool.work.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_part_pipeline_has_its_own_slot() {
        let mut slots: Vec<usize> = PartPipeline::BASE
            .iter()
            .chain(PartPipeline::LIT.iter())
            .map(|p| p.index())
            .collect();
        slots.sort_unstable();
        assert_eq!(slots, (0..PartPipeline::COUNT).collect::<Vec<_>>());
    }
}
