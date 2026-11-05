//! The Auto preset's calibration on a fixed workload (#139, ADR 0008 §8).
//!
//! Auto chooses its tier by timing a built-in scene, the same on every machine and whatever game is played: the
//! `calibration` stress scene ([`StressKind::Calibration`]: 3,000 parts of mixed shapes, 300 see-through, 256 local
//! lights of which 16 cast shadows, under the sun's cascades), drawn by a renderer of its own into an offscreen
//! [`CALIBRATION_SIZE`] target with the post chain, at each tier [`Calibrator`] measures (Ultra, then High, then
//! Medium). The game's own frames never enter the measurement, so the tier does not depend on which game ran first.
//!
//! - **When.** It starts once the game's first frame is presented and the game's renderer has compiled its lit
//!   pipelines (at most 2 s later): the workload draws with the same shaders, which a device such as DX12's compiles
//!   once and keeps, so its own compiles then cost milliseconds instead of competing with the game's first frame.
//! - **Nothing waits on the frame path.** The workload is built on a worker thread, in three steps each started right
//!   after a game frame is presented (its game and GPU scene; its renderer, at Low; its first frame, which waits for
//!   its pipelines there), so the device memory they make is cleared while the game's thread waits for its next frame
//!   (#152). Ultra's textures then arrive off the frame, one a frame, like any settings change's. After each game frame
//!   presented, [`Calibrator::batch`] workload frames are drawn back to back; their timestamps are read back
//!   asynchronously like the game's, one to three frames late. A probe's switch to a cheaper tier makes its textures
//!   off the frame (#152) and compiles nothing on the frame (#86).
//! - **Clocks.** The batches fill the GPU time the game's frames leave (the game's own frame interval, measured before
//!   the workload runs, less the game's GPU time and a margin), submitted back to back with no present between them,
//!   so the GPU is busy most of the time and runs at the clocks a game that needs them gets: a light game at 60 fps
//!   leaves it idle most of the time, and an idle GPU's clocks made the workload measure four times as long. The
//!   calibrator takes each tier's lower quartile, so the frames still timed while the clocks rise do not decide.
//! - **The game** draws at Medium meanwhile, with Auto's dynamic resolution, and switches once, to the tier chosen,
//!   when the calibration is done; through the same off-the-frame path as any settings change.
//! - **Memory.** The workload's renderer holds its own textures (at Ultra: 3 × 3072² cascades and their static map,
//!   the lights' 4096² atlas and 8x MSAA attachments at 1080p, about 380 MiB) until the calibration ends, when it is
//!   dropped.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use metallic_core::DataModel;
use metallic_render::resolution::ResolutionSettings;
use metallic_render::settings::{CalibrationStep, Calibrator, Quality, Tier};
use metallic_render::{
    Camera, CameraReader, GpuScene, LightingReader, OffscreenTarget, PassId, PassSet, RenderContext, Renderer,
    RendererSettings, TIMING_RING_SIZE, wgpu,
};

use crate::stress::{StressKind, StressScene};

/// The version of the calibration workload: the scene, its size and seed, and how it is measured (the calibrator's
/// thresholds and sample counts). Saved with a calibration ([`metallic_render::settings::Calibration::workload`]); a
/// calibration of another version is measured again, so change it whenever any of those change.
pub const CALIBRATION_WORKLOAD: u32 = 1;

/// The size the workload is drawn at: 1080p, the resolution the tiers are sized for (ADR 0008's budgets). A larger
/// or smaller window is the game's dynamic resolution's to absorb.
pub const CALIBRATION_SIZE: (u32, u32) = (1920, 1080);

/// The workload's seed: the same scene on every machine.
const CALIBRATION_SEED: u64 = 139;

/// How long a calibration may take before it gives up: a device whose timestamps never arrive gets the tier of its
/// kind of adapter ([`Calibrator::untimed_tier`]) rather than calibrating forever.
const MAX_DURATION: Duration = Duration::from_secs(10);

/// The steps the workload is built in, one after each game frame presented ([`Workload::new`]).
const BUILD_STEPS: usize = 3;

/// How long a calibration waits for the game's lit pipelines before it starts anyway.
const MAX_WAIT: Duration = Duration::from_secs(2);

/// What the engine says about a game frame it presented, for the calibration.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GameFrame {
    /// The time since the game's frame before it; `None` for the first.
    pub frame_time: Option<Duration>,
    /// The GPU time of the game's newest timed frame.
    pub gpu_ms: Option<f32>,
    /// The game's renderer has compiled its lit pipelines ([`Renderer::lit_pipelines_ready`]).
    pub lit_ready: bool,
}

/// The game's frame intervals kept while the workload is built.
const INTERVALS: usize = 8;
/// The game's frame interval assumed when none was seen before the workload ran: a 60 fps frame.
const DEFAULT_INTERVAL_MS: f32 = 1000.0 / 60.0;
/// The GPU time left free in each game frame interval besides the game's own frame, in milliseconds: the batches
/// end this long before the game's next frame is due, for the frames that take a little longer than the last.
const ROOM_MARGIN_MS: f32 = 1.5;

/// What a finished calibration chose, and what it cost ([`crate::SceneRenderer::last_calibration`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CalibrationOutcome {
    /// The tier chosen.
    pub tier: Tier,
    /// The GPU time of the workload at that tier, in milliseconds: the lower quartile of its timed frames (0 when not
    /// measured).
    pub gpu_ms: f32,
    /// From the game's first frame presented until done: how long the game drew at Medium.
    pub duration: Duration,
    /// Game frames drawn meanwhile, the first included.
    pub game_frames: u32,
    /// Of those, the frames it waited for the game's lit pipelines before building the workload.
    pub waited_frames: u32,
    /// Workload frames drawn.
    pub workload_frames: u64,
    /// The longest game frame meanwhile (the time between two game frames; the game's first frame, which waits for
    /// its own pipelines, left out).
    pub worst_game_frame: Duration,
    /// How long building the workload took on its worker thread (its pipelines and first frame included).
    pub build: Duration,
}

/// Auto's calibration while it runs: waiting for the game's first frame, the workload being built on its worker
/// thread, then drawn after each game frame.
pub(crate) struct AutoCalibration {
    state: State,
    format: wgpu::TextureFormat,
    size: (u32, u32),
    /// When the game's first frame was presented (the first [`AutoCalibration::after_frame`]).
    started: Option<Instant>,
    /// Game frames presented while waiting to start.
    waited: u32,
    game_frames: u32,
    /// The next frame time is the game's first frame's (its pipeline wait): not the calibration's.
    skip_frame_time: bool,
    worst_game_frame: Duration,
    /// The game's frame intervals while the workload is built, before it loads the GPU, in milliseconds.
    intervals: [f32; INTERVALS],
    interval_count: usize,
    /// The game's own frame interval, which the workload's batches fill: the median of those, once it runs.
    interval_ms: f32,
    /// For tests: `Some(cost)` feeds the calibrator each workload frame as it is drawn, at `cost(tier measured)`,
    /// instead of its GPU timings.
    timing_override: Option<fn(Tier) -> f32>,
    /// How long it may take before it gives up ([`MAX_DURATION`]; tests on software rasterisers allow more).
    max_duration: Duration,
}

enum State {
    /// Not started: it starts after a game frame is presented.
    Waiting,
    /// The workload being built on a worker thread, a step after each game frame `go` says was presented.
    Building {
        built: Receiver<Result<Workload, String>>,
        go: Sender<()>,
    },
    /// The workload drawn after each game frame.
    Running(Box<Workload>),
}

impl AutoCalibration {
    /// A calibration drawing its workload into targets of `format` (the window's, so its pipelines are the ones the
    /// game's renderer compiles too) at `size` ([`CALIBRATION_SIZE`]). It starts after the next game frame is
    /// presented ([`AutoCalibration::after_frame`]): a game's first frame waits for its pipelines, and the workload's
    /// compiles, of the same shaders, would only compete with them; once they are done the device has compiled most of
    /// what the workload draws with.
    pub(crate) fn new(format: wgpu::TextureFormat, size: (u32, u32)) -> Self {
        Self {
            state: State::Waiting,
            format,
            size,
            started: None,
            waited: 0,
            game_frames: 0,
            skip_frame_time: false,
            worst_game_frame: Duration::ZERO,
            intervals: [0.0; INTERVALS],
            interval_count: 0,
            interval_ms: DEFAULT_INTERVAL_MS,
            timing_override: None,
            max_duration: MAX_DURATION,
        }
    }

    /// Starts building the workload on a worker thread, its first step now (a game frame was just presented).
    fn start(&mut self, context: &RenderContext) {
        let (sender, built) = mpsc::channel();
        let (go, steps) = mpsc::channel();
        let worker_context = context.clone();
        let (format, size) = (self.format, self.size);
        let spawned = std::thread::Builder::new()
            .name("metallic calibration".into())
            .spawn(move || {
                // `None`, or a send to a receiver that is gone: the calibration was dropped meanwhile.
                if let Some(workload) = Workload::new(&worker_context, format, size, &steps) {
                    let _ = sender.send(workload);
                }
            });
        let built = match spawned {
            Ok(_) => built,
            // No thread: build it here, every step at once (a platform without threads compiles inline anyway).
            Err(_) => {
                let (sender, built) = mpsc::channel();
                let (go, steps) = mpsc::channel();
                for _ in 0..BUILD_STEPS {
                    let _ = go.send(());
                }
                if let Some(workload) = Workload::new(context, format, size, &steps) {
                    let _ = sender.send(workload);
                }
                built
            }
        };
        let _ = go.send(());
        self.state = State::Building { built, go };
    }

    /// The tier being measured.
    pub(crate) fn probe(&self) -> Tier {
        match &self.state {
            State::Waiting | State::Building { .. } => Tier::Ultra,
            State::Running(workload) => workload.calibrator.probe(),
        }
    }

    /// After a game `frame` was presented on `context`: starts the calibration once the game's lit pipelines are
    /// compiled, moves the workload's build on a step, takes the workload once it is built, draws this frame's
    /// workload frames, and says the result once there is one. Never waits for the GPU or for the worker thread.
    pub(crate) fn after_frame(&mut self, context: &RenderContext, frame: GameFrame) -> Option<CalibrationOutcome> {
        let GameFrame {
            frame_time,
            gpu_ms: game_gpu_ms,
            lit_ready,
        } = frame;
        let started = *self.started.get_or_insert_with(Instant::now);
        if let State::Waiting = self.state {
            self.waited += 1;
            // Without a frame time this was the game's first frame: the next frame time is its pipeline wait.
            self.skip_frame_time = frame_time.is_none();
            if lit_ready || started.elapsed() > MAX_WAIT {
                self.start(context);
            }
            return None;
        }
        self.game_frames += 1;
        if let Some(frame_time) = frame_time
            && !std::mem::take(&mut self.skip_frame_time)
        {
            self.worst_game_frame = self.worst_game_frame.max(frame_time);
            if let (State::Building { .. }, Some(slot)) = (&self.state, self.intervals.get_mut(self.interval_count)) {
                *slot = frame_time.as_secs_f32() * 1000.0;
                self.interval_count += 1;
            }
        }
        let device_type = context.capabilities().device_type;
        let step = match &mut self.state {
            State::Waiting => None,
            State::Building { built, go } => match built.try_recv() {
                Ok(Ok(workload)) => {
                    // The game's own pace, before the workload loads the GPU: vsync's, a frame cap's, or as fast as
                    // it goes.
                    let intervals = &mut self.intervals[..self.interval_count];
                    if !intervals.is_empty() {
                        intervals.sort_unstable_by(f32::total_cmp);
                        self.interval_ms = intervals[intervals.len() / 2];
                    }
                    self.state = State::Running(Box::new(workload));
                    None
                }
                Ok(Err(error)) => {
                    log::warn!("graphics: could not build the calibration workload: {error}");
                    Some((Calibrator::untimed_tier(device_type), 0.0))
                }
                Err(TryRecvError::Empty) => {
                    // A game frame was just presented: the worker's next step, if it has one.
                    let _ = go.send(());
                    None
                }
                Err(TryRecvError::Disconnected) => {
                    log::warn!("graphics: the calibration workload's thread ended without it");
                    Some((Calibrator::untimed_tier(device_type), 0.0))
                }
            },
            State::Running(workload) => {
                let room_ms = (self.interval_ms - game_gpu_ms.unwrap_or(0.0) - ROOM_MARGIN_MS).max(0.0);
                workload.draw(room_ms, self.timing_override)
            }
        };
        let step = step.or_else(|| {
            (started.elapsed() > self.max_duration).then(|| {
                log::warn!("graphics: calibration took over {:?}; giving up", self.max_duration);
                (Calibrator::untimed_tier(device_type), 0.0)
            })
        });
        step.map(|(tier, gpu_ms)| {
            let (workload_frames, build) = match &self.state {
                State::Running(workload) => (workload.frames, workload.build),
                State::Waiting | State::Building { .. } => (0, Duration::ZERO),
            };
            CalibrationOutcome {
                tier,
                gpu_ms,
                duration: self.started.map_or(Duration::ZERO, |started| started.elapsed()),
                game_frames: self.waited + self.game_frames,
                waited_frames: self.waited,
                workload_frames,
                worst_game_frame: self.worst_game_frame,
                build,
            }
        })
    }
}

/// The workload: its game, scene, renderer and target, and the calibrator timing it.
struct Workload {
    /// The workload's game, which the scene is attached to, and what moves in it.
    game: DataModel,
    stress: StressScene,
    scene: GpuScene,
    renderer: Renderer,
    target: OffscreenTarget,
    camera: Option<Camera>,
    calibrator: Calibrator,
    /// Workload frames drawn.
    frames: u64,
    /// How long it took to build.
    build: Duration,
}

impl Workload {
    /// Builds the workload and draws its first frame, at Low, then asks for the first probe's settings (made off the
    /// frame by the frames after). In [`BUILD_STEPS`] steps, each started when `go` says a game frame was just
    /// presented: each makes buffers or textures, and a new device memory block made then holds wgpu's allocator
    /// while the OS clears it (#152), which a game frame writing its own buffers at that moment would wait for. Right
    /// after a present the game's thread is waiting for the next frame instead. `None` when `go`'s sender is gone (the
    /// calibration was dropped).
    fn new(
        context: &RenderContext,
        format: wgpu::TextureFormat,
        (width, height): (u32, u32),
        go: &Receiver<()>,
    ) -> Option<Result<Self, String>> {
        go.recv().ok()?;
        let start = Instant::now();
        Some(Self::build(context, format, (width, height), go, start))
    }

    fn build(
        context: &RenderContext,
        format: wgpu::TextureFormat,
        (width, height): (u32, u32),
        go: &Receiver<()>,
        start: Instant,
    ) -> Result<Self, String> {
        let gone = || "the calibration was dropped".to_owned();
        let mut game = metallic_classes::new_game().map_err(|error| error.to_string())?;
        let stress = StressScene::build(&mut game, StressKind::Calibration, CALIBRATION_SEED)
            .map_err(|error| error.to_string())?;
        let registry = game.registry().clone();
        let workspace = registry.class_id("Workspace").and_then(|class| game.service(class));
        let lighting = registry.class_id("Lighting").and_then(|class| game.service(class));
        let components = metallic_render::scene::RenderComponents::builtin();
        let mut scene = GpuScene::new(context, &mut game, workspace, &components).map_err(|error| error.to_string())?;
        let camera = workspace.and_then(|world| CameraReader::new().read(&game, world));
        if let Some(camera) = camera {
            scene.set_camera_position(&game, camera.position);
        }
        // Made at Low, whose textures are small: a scene's first frame makes its textures at once. Ultra's (3 × 3072²
        // cascades, the 4096² atlas, 8x MSAA at 1080p) are then made off the frame like any settings change's, one a
        // frame (#152).
        go.recv().map_err(|_| gone())?;
        let first = probe_quality(Tier::Low);
        let settings = RendererSettings {
            cull: first.cull,
            shadows: first.shadows,
            lights: first.lights,
            lod: first.lod,
            resolution: first.resolution,
            post: first.post,
            passes: passes(first.anti_aliasing.mlaa()),
            ..RendererSettings::new(format, width, height)
        };
        let mut renderer = Renderer::new(context, settings).map_err(|error| error.to_string())?;
        if let Some(lighting) = lighting {
            renderer.set_lighting(LightingReader::new().read(&game, lighting));
        }
        let target = OffscreenTarget::new(context, width, height, format).map_err(|error| error.to_string())?;
        let mut workload = Self {
            game,
            stress,
            scene,
            renderer,
            target,
            camera,
            calibrator: Calibrator::new(),
            frames: 0,
            build: Duration::ZERO,
        };
        // The scene's first frame: it waits for its pipelines (the lit ones included) here, on this thread, so no game
        // frame does. Then the first probe's settings, whose pipelines and textures follow off the frame.
        go.recv().map_err(|_| gone())?;
        workload.render();
        apply(&mut workload.renderer, &probe_quality(workload.calibrator.probe()));
        workload.build = start.elapsed();
        Ok(workload)
    }

    /// Draws the next workload frame: its moving parts where they are in that frame (the motion is a function of the
    /// frame number, at 60 fps, so every machine draws the same frames), synced, then drawn.
    fn render(&mut self) {
        // The workload's own game, built by this module: a part it cannot move is a bug, not something to stop for.
        if let Err(error) = self.stress.animate(&mut self.game, self.frames as f64 / 60.0) {
            log::debug!("graphics: the calibration workload could not move its parts: {error}");
        }
        self.scene.sync(&mut self.game);
        self.renderer
            .render_scene(self.target.view(), &mut self.scene, self.camera.as_ref());
        self.frames += 1;
    }

    /// Draws this game frame's workload frames, as many as fill `room_ms` of GPU time ([`Calibrator::batch`]), and
    /// feeds their timings to the calibrator: the tier chosen and its time, once done. With `timing_override`, each
    /// frame's GPU time is `cost(tier drawn)` instead (tests).
    fn draw(&mut self, room_ms: f32, timing_override: Option<fn(Tier) -> f32>) -> Option<(Tier, f32)> {
        for _ in 0..self.calibrator.batch(room_ms) {
            if !self.calibrator.settled() && self.drawn_at_probe() {
                self.calibrator.probe_drawn_from(self.renderer.frame_index());
            }
            self.render();
            match self.feed(timing_override) {
                CalibrationStep::Measuring => {}
                CalibrationStep::Probe(tier) => {
                    log::info!("graphics: calibration measures {tier:?} next");
                    apply(&mut self.renderer, &probe_quality(tier));
                    return None;
                }
                CalibrationStep::Done(tier, gpu_ms) => return Some((tier, gpu_ms)),
            }
        }
        None
    }

    /// Whether the renderer draws the probe's settings: no switch waiting for pipelines, no texture being made.
    fn drawn_at_probe(&self) -> bool {
        let quality = probe_quality(self.calibrator.probe());
        let settings = self.renderer.settings();
        let samples = quality
            .anti_aliasing
            .supported(self.renderer.msaa_sample_counts())
            .msaa_samples();
        settings.msaa_samples == samples
            && settings.shadows == quality.shadows
            && settings.lights == quality.lights
            && settings.post == quality.post
            && !self.renderer.pipeline_stats().switch_waiting
            && self.renderer.texture_stats().pending == 0
    }

    /// Feeds the calibrator every workload frame timed since the last call, oldest first; with `timing_override`, the
    /// frame just drawn, at once, at `cost(probe)` (tests: deterministic, and on any device).
    fn feed(&mut self, timing_override: Option<fn(Tier) -> f32>) -> CalibrationStep {
        if let Some(cost) = timing_override {
            let frame = self.renderer.frame_index().saturating_sub(1);
            return self.calibrator.observe(frame, cost(self.calibrator.probe()));
        }
        let mut timed = [(0u64, 0.0f32); TIMING_RING_SIZE];
        let mut count = 0;
        for timings in self.renderer.recent_gpu_timings() {
            if let Some(slot) = timed.get_mut(count) {
                *slot = (timings.frame(), timings.total_ms());
                count += 1;
            }
        }
        let timed = &mut timed[..count];
        timed.sort_unstable_by_key(|&(frame, _)| frame);
        for &(frame, gpu_ms) in timed.iter() {
            match self.calibrator.observe(frame, gpu_ms) {
                CalibrationStep::Measuring => {}
                step => return step,
            }
        }
        CalibrationStep::Measuring
    }
}

/// The settings the workload is drawn with at `tier`: the tier's, at the target's own resolution (no dynamic
/// resolution: the time measured is the tier's at full size).
fn probe_quality(tier: Tier) -> Quality {
    Quality {
        resolution: ResolutionSettings::NATIVE,
        ..Quality::for_tier(tier)
    }
}

/// The passes the workload's renderer runs: the post chain, bloom and the merged scene passes every tier draws, and
/// MLAA when `mlaa`.
fn passes(mlaa: bool) -> PassSet {
    PassSet::default()
        .with(PassId::Post)
        .with(PassId::Bloom)
        .with(PassId::Scene)
        .with_if(PassId::Mlaa, mlaa)
}

/// Gives the workload's renderer `quality`'s settings: a settings change, made off the frame (#86, #152).
fn apply(renderer: &mut Renderer, quality: &Quality) {
    renderer.set_cull(quality.cull);
    renderer.set_shadows(quality.shadows);
    renderer.set_lights(quality.lights);
    renderer.set_lod(quality.lod);
    renderer.set_resolution(quality.resolution);
    renderer.set_post(quality.post);
    let aa = quality.anti_aliasing.supported(renderer.msaa_sample_counts());
    renderer.set_passes(passes(aa.mlaa()));
    if let Err(error) = renderer.set_anti_aliasing(aa) {
        log::warn!("graphics: the calibration could not draw with {aa}: {error}");
    }
}

#[cfg(test)]
mod tests {
    use metallic_render::settings::CALIBRATION_SKIP;
    use metallic_render::{ContextOptions, FrameTimings, RenderError};

    use super::*;

    /// A device on any adapter, or `None` (after saying why) when the machine has none and `METALLIC_REQUIRE_GPU`
    /// is not set (CI sets it, on its software rasterisers).
    fn gpu() -> Option<RenderContext> {
        match RenderContext::new_offscreen(&ContextOptions::default()) {
            Ok(context) => Some(context),
            Err(error @ RenderError::NoAdapter { .. }) => {
                if std::env::var("METALLIC_REQUIRE_GPU").is_ok_and(|value| value == "1") {
                    panic!("{error}, and METALLIC_REQUIRE_GPU=1");
                }
                eprintln!("skipped: {error}");
                None
            }
            Err(error) => panic!("could not create a device: {error}"),
        }
    }

    /// What the workload costs here, by tier: Ultra over its share of a 60 fps frame (8.3 ms), High within its
    /// (10 ms). The same on every run, so the tier is High whatever runs beside it.
    fn workload_cost(tier: Tier) -> f32 {
        match tier {
            Tier::Ultra => 9.0,
            Tier::High => 6.0,
            Tier::Medium => 4.0,
            Tier::Low => 1.0,
        }
    }

    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
    /// Small, for software rasterisers: the size changes the time, which the test injects, not the calibration.
    const SIZE: (u32, u32) = (160, 90);

    /// Runs the real calibration (its workload built on its worker thread, drawn on the device after each game frame)
    /// beside `game`, drawn every frame by a renderer of its own at Medium as the engine draws a game while it
    /// calibrates, with the workload's frames timed at [`workload_cost`]; returns what it chose.
    fn calibrate_beside(context: &RenderContext, game: StressKind) -> CalibrationOutcome {
        let mut dm = metallic_classes::new_game().expect("a game");
        let stress = StressScene::build(&mut dm, game, 1).expect("the game's scene");
        let workspace = dm
            .service(dm.registry().class_id("Workspace").expect("Workspace"))
            .expect("a Workspace");
        let components = metallic_render::scene::RenderComponents::builtin();
        let mut scene = GpuScene::new(context, &mut dm, Some(workspace), &components).expect("the game's GPU scene");
        let medium = Quality::MEDIUM;
        let settings = RendererSettings {
            cull: medium.cull,
            shadows: medium.shadows,
            lights: medium.lights,
            lod: medium.lod,
            post: medium.post,
            passes: passes(true),
            ..RendererSettings::new(FORMAT, SIZE.0, SIZE.1)
        };
        let mut renderer = Renderer::new(context, settings).expect("the game's renderer");
        let target = OffscreenTarget::new(context, SIZE.0, SIZE.1, FORMAT).expect("the game's target");
        let mut calibration = AutoCalibration::new(FORMAT, SIZE);
        calibration.timing_override = Some(workload_cost);
        // The frames are this loop's to count, however slow the adapter: lavapipe draws the workload at Ultra (3 × 3072²
        // cascades) at a few frames a second, past the 10 s a game gives a calibration.
        calibration.max_duration = Duration::from_secs(3600);
        let mut cameras = CameraReader::new();
        for frame in 0..3_000u64 {
            stress
                .animate(&mut dm, frame as f64 / 60.0)
                .expect("the game's parts move");
            let camera = cameras.read(&dm, workspace);
            scene.sync(&mut dm);
            renderer.render_scene(target.view(), &mut scene, camera.as_ref());
            // A test may wait for the GPU: here it paces the frames, as a window's present would.
            let _ = context.device().poll(wgpu::PollType::wait_indefinitely());
            let presented = GameFrame {
                frame_time: (frame > 0).then_some(Duration::from_micros(16_667)),
                gpu_ms: renderer.gpu_timings().map(FrameTimings::total_ms),
                lit_ready: renderer.lit_pipelines_ready(),
            };
            if let Some(outcome) = calibration.after_frame(context, presented) {
                assert_eq!(
                    renderer.settings().msaa_samples,
                    1,
                    "the game's frames are never the workload's"
                );
                return outcome;
            }
        }
        panic!("{game:?}: never calibrated");
    }

    #[test]
    fn the_tier_does_not_depend_on_the_game_played() {
        let Some(context) = gpu() else {
            return;
        };
        // A light game and a heavy, lit one: the calibration before #139 timed their own frames and chose by them.
        let light = calibrate_beside(&context, StressKind::Mobile);
        let heavy = calibrate_beside(&context, StressKind::ManyLights);
        for outcome in [&light, &heavy] {
            assert_eq!((outcome.tier, outcome.gpu_ms), (Tier::High, 6.0), "{outcome:?}");
            // Ultra measured (given up after 12 frames over its share), then High (16 frames), each after its switch.
            let least = 1 + 2 * CALIBRATION_SKIP + 12 + 16;
            assert!(outcome.workload_frames >= least, "{outcome:?}");
        }
    }

    #[test]
    fn the_workload_scene_is_the_same_on_every_machine() {
        let build = || {
            let mut dm = metallic_classes::new_game().expect("a game");
            let scene = StressScene::build(&mut dm, StressKind::Calibration, CALIBRATION_SEED).expect("the workload");
            (
                scene.parts(),
                scene.see_through(),
                scene.lights(),
                scene.moving(),
                scene.in_view(),
            )
        };
        let (parts, see_through, lights, moving, in_view) = build();
        assert_eq!((parts, see_through, lights, moving), (3_256, 299, 256, 62));
        // Nearly all of it in view, so every tier draws all of it.
        assert!(in_view > 3_000, "{in_view}");
        assert_eq!(build(), (parts, see_through, lights, moving, in_view));
    }
}
