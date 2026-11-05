//! [`SceneRenderer`]: `metallic_render` as the engine's [`RenderHook`]. A thin adapter: the renderer crate cannot
//! depend on the engine (ADR 0001), so the hook lives here and only forwards.

use std::sync::Arc;
use std::time::{Duration, Instant};

use metallic_classes::services::user_game_settings::{UserGameSettings, set_user_game_settings};
use metallic_core::{DataModel, Id};
use metallic_platform::{Size, Window};
use metallic_render::post::PostSettings;
use metallic_render::resolution::ResolutionSettings;
use metallic_render::scene::{RenderComponents, SyncStats, UploadStats};
use metallic_render::settings::{Calibration, Calibrator, GraphicsSettings, Quality, Tier};
use metallic_render::{
    AntiAliasing, CameraReader, Capabilities, ContextOptions, CullSettings, DrawStats, GpuScene, LightSettings,
    LightingReader, LodSettings, PassId, PassSet, RenderContext, RenderError, RenderSurface, Renderer,
    RendererSettings, SceneLighting, ShadowSettings, wgpu,
};

use crate::calibration::{AutoCalibration, CALIBRATION_SIZE, CALIBRATION_WORKLOAD, CalibrationOutcome, GameFrame};
use crate::hooks::{RenderFrame, RenderHook, RenderHookError};
use crate::perf::{FrameSample, GraphicsStatus, PerfStats, PerfSummary, draw_overlay};

/// Frames between two refreshes of the overlay's percentiles: about four times a second at 60 fps.
const SUMMARY_EVERY: u32 = 15;

/// The device and the frame loop, kept while the window comes and goes.
struct Device {
    context: RenderContext,
    renderer: Renderer,
}

/// Draws the game into the engine's window: install it with [`Engine::set_render_hook`](crate::Engine::set_render_hook).
///
/// Each frame it syncs the GPU-resident scene ([`GpuScene`]) with the data model's changes and draws the parts in
/// `Workspace` from `Workspace.CurrentCamera` ([`Renderer::render_scene`]), lit by the `Lighting` service and the
/// point and spot lights in `Workspace`, with the sky behind them; without a current camera the frame is the clear colour. `Lighting` is read only in frames after
/// it changed: the scene watches it on the change consumer it drains anyway ([`GpuScene::watch`]). The device and
/// the scene survive [`RenderHook::detach`], so a mobile app coming back to the foreground only makes a new surface.
///
/// It also keeps the frame statistics ([`SceneRenderer::perf`]): every drawn frame records its frame time, the
/// script and physics time the engine measured ([`Engine::set_cpu_timing`](crate::Engine::set_cpu_timing)), its own
/// render CPU time, the GPU timings, the render scale and the draws. [`SceneRenderer::set_overlay`] shows them on
/// screen.
///
/// # Graphics settings
///
/// [`SceneRenderer::set_graphics`] applies a player's [`GraphicsSettings`] (ADR 0008 §8): the preset's settings of
/// every system, with the player's overrides, at once. Without it the renderer keeps the per-system settings set
/// one by one ([`SceneRenderer::set_cull`] and the others; the renderer's defaults until then). With the Auto preset
/// and no calibration for the adapter, driver and workload it attaches to, it calibrates on a fixed workload
/// ([`crate::calibration`], #139): built on a worker thread, then drawn offscreen after each game frame at each tier
/// [`Calibrator`] measures, while the game draws at Medium with dynamic resolution; the game then switches once, to
/// the tier chosen, and [`SceneRenderer::take_calibration`] hands the result to the app to save.
/// [`SceneRenderer::write_user_settings`] writes the settings in effect into the game's `UserGameSettings` service,
/// which scripts read.
///
/// Switching settings at run time waits for nothing: the shadow maps, the light atlas and the scaled scene textures
/// are made again without waiting for the GPU (the old ones are freed once it is done with them), and every pipeline
/// is compiled off the frame, in the background from the renderer's creation: a switch whose pipelines are still
/// compiling takes effect in the first frame after they are ([`metallic_render::pipelines`]).
pub struct SceneRenderer {
    options: ContextOptions,
    clear_color: wgpu::Color,
    cull: CullSettings,
    shadows: ShadowSettings,
    lights: LightSettings,
    lod: LodSettings,
    resolution: ResolutionSettings,
    post: PostSettings,
    anti_aliasing: AntiAliasing,
    merge_scene_passes: bool,
    post_chain: bool,
    bloom_compute: bool,
    lod_tint: bool,
    present_mode: Option<wgpu::PresentMode>,
    /// The player's graphics settings, once set ([`SceneRenderer::set_graphics`]).
    graphics: Option<GraphicsSettings>,
    /// The tier drawn at: the graphics settings', or Medium while calibrating.
    tier: Option<Tier>,
    /// Auto's calibration, while it runs.
    calibration: Option<AutoCalibration>,
    /// A calibration finished and not yet taken by the app.
    new_calibration: Option<Calibration>,
    /// What the last calibration run here did.
    last_calibration: Option<CalibrationOutcome>,
    /// The frame cap the app applies, reported to scripts.
    frame_cap: Option<u32>,
    /// `UserGameSettings` must be written again.
    settings_changed: bool,
    device: Option<Device>,
    surface: Option<RenderSurface>,
    scene: Option<GpuScene>,
    /// `Workspace`, the root of what is drawn and the owner of `CurrentCamera`.
    world: Option<Id>,
    camera: CameraReader,
    /// The `Lighting` service, watched by the scene so it is read again only when it changes.
    lighting_service: Option<Id>,
    lighting_reader: LightingReader,
    /// What `Lighting` said when last read, kept for a renderer made later (a new surface format).
    lighting: SceneLighting,
    last_sync: SyncStats,
    last_upload: UploadStats,
    perf: PerfStats,
    /// The overlay's percentiles, refreshed every [`SUMMARY_EVERY`] frames.
    summary: PerfSummary,
    since_summary: u32,
    overlay: bool,
    /// When the previous frame's render started: frame time is the time between two.
    frame_start: Option<Instant>,
    /// When the device was being made (the first attach), until the first frame is presented.
    attached_at: Option<Instant>,
    /// Milliseconds from the first attach until the first frame was presented.
    first_frame_ms: Option<f32>,
}

impl SceneRenderer {
    /// A renderer that picks its adapter with `options` when it first attaches to a window.
    pub fn new(options: ContextOptions) -> Self {
        Self {
            options,
            clear_color: wgpu::Color {
                r: 0.36,
                g: 0.55,
                b: 0.82,
                a: 1.0,
            },
            cull: CullSettings::default(),
            shadows: ShadowSettings::default(),
            lights: LightSettings::default(),
            lod: LodSettings::default(),
            resolution: ResolutionSettings::NATIVE,
            post: PostSettings::default(),
            anti_aliasing: AntiAliasing::Off,
            merge_scene_passes: true,
            post_chain: true,
            bloom_compute: true,
            lod_tint: false,
            present_mode: None,
            graphics: None,
            tier: None,
            calibration: None,
            new_calibration: None,
            last_calibration: None,
            frame_cap: None,
            settings_changed: false,
            device: None,
            surface: None,
            scene: None,
            world: None,
            camera: CameraReader::new(),
            lighting_service: None,
            lighting_reader: LightingReader::new(),
            lighting: SceneLighting::default(),
            last_sync: SyncStats::default(),
            last_upload: UploadStats::default(),
            perf: PerfStats::default(),
            summary: PerfSummary::default(),
            since_summary: 0,
            overlay: false,
            frame_start: None,
            attached_at: None,
            first_frame_ms: None,
        }
    }

    /// Milliseconds from the first attach to a window (the device and the renderer made, the first frame's pipelines
    /// compiled) until the first frame was presented: what a player waits for the first image. `None` before it.
    pub fn first_frame_ms(&self) -> Option<f32> {
        self.first_frame_ms
    }

    /// Changes the colour frames start from (linear).
    pub fn set_clear_color(&mut self, color: wgpu::Color) {
        self.clear_color = color;
        if let Some(device) = self.device.as_mut() {
            device.renderer.set_clear_color(color);
        }
    }

    /// Changes what the camera culls (frustum, view distance, screen size; see [`metallic_render::cull`]). On by
    /// default; graphics settings ([`SceneRenderer::set_graphics`]) set the thresholds. Applies now and to every later renderer.
    pub fn set_cull(&mut self, cull: CullSettings) {
        self.cull = cull;
        if let Some(device) = self.device.as_mut() {
            device.renderer.set_cull(cull);
        }
    }

    /// What the camera culls.
    pub fn cull(&self) -> CullSettings {
        self.cull
    }

    /// Changes how the sun's shadows are drawn (cascades, map size, distance; see [`metallic_render::shadow`]). On
    /// by default; graphics settings ([`SceneRenderer::set_graphics`]) set them, and `Lighting.GlobalShadows` turns a game's shadows off. Applies
    /// now and to every later renderer.
    pub fn set_shadows(&mut self, shadows: ShadowSettings) {
        self.shadows = shadows;
        if let Some(device) = self.device.as_mut() {
            device.renderer.set_shadows(shadows);
        }
    }

    /// How the sun's shadows are drawn.
    pub fn shadows(&self) -> ShadowSettings {
        self.shadows
    }

    /// Changes how point and spot lights are drawn (the light budget, shadowed lights, the shadow atlas; see
    /// [`metallic_render::lights`]). The defaults are the Medium preset's; graphics settings ([`SceneRenderer::set_graphics`]) set them, and each
    /// light's `Enabled` and `Shadows` are the game's. Applies now and to every later renderer.
    pub fn set_lights(&mut self, lights: LightSettings) {
        self.lights = lights;
        if let Some(device) = self.device.as_mut() {
            device.renderer.set_lights(lights);
        }
    }

    /// How point and spot lights are drawn.
    pub fn lights(&self) -> LightSettings {
        self.lights
    }

    /// Changes how the round parts' levels of detail are chosen (the LOD bias; see [`metallic_render::lod`]). The
    /// High preset's by default; graphics settings ([`SceneRenderer::set_graphics`]) set it. Applies now and to every later renderer.
    pub fn set_lod(&mut self, lod: LodSettings) {
        self.lod = lod;
        if let Some(device) = self.device.as_mut() {
            device.renderer.set_lod(lod);
        }
    }

    /// How the round parts' levels of detail are chosen.
    pub fn lod(&self) -> LodSettings {
        self.lod
    }

    /// Colours every part by the level of detail it is drawn at, a debug view
    /// ([`metallic_render::Renderer::set_lod_tint`]). Off by default. Applies now and to every later renderer.
    pub fn set_lod_tint(&mut self, on: bool) {
        self.lod_tint = on;
        if let Some(device) = self.device.as_mut() {
            device.renderer.set_lod_tint(on);
        }
    }

    /// Changes the scene's resolution: a render scale, or dynamic resolution holding a GPU frame time (see
    /// [`metallic_render::resolution`]). The target's own resolution by default; graphics settings set it. Applies
    /// now and to every later renderer.
    pub fn set_resolution(&mut self, resolution: ResolutionSettings) {
        self.resolution = resolution;
        if let Some(device) = self.device.as_mut() {
            device.renderer.set_resolution(resolution);
        }
    }

    /// The scene's resolution settings.
    pub fn resolution(&self) -> ResolutionSettings {
        self.resolution
    }

    /// Changes how the post chain finishes the image: exposure, grade, bloom's resolution and filter, MLAA's search
    /// (see [`metallic_render::post`]). The renderer's defaults until graphics settings set the effect quality.
    /// Applies now and to every later renderer.
    pub fn set_post(&mut self, post: PostSettings) {
        self.post = post;
        let drawn = self.drawn_post();
        if let Some(device) = self.device.as_mut() {
            device.renderer.set_post(drawn);
        }
    }

    /// How the post chain finishes the image.
    pub fn post(&self) -> PostSettings {
        self.post
    }

    /// Draws bloom's chain as a render pass per step when `on` is false, as devices without compute shaders do
    /// (WebGL 2, GLES 3.0), instead of one compute pass (#145): for measuring what the compute pass saves, and for
    /// testing the render passes on any device. On by default. Applies now and to every later renderer.
    pub fn set_bloom_compute(&mut self, on: bool) {
        self.bloom_compute = on;
        self.set_post(self.post);
    }

    /// The post settings the renderer draws with: [`SceneRenderer::post`], with bloom's compute pass off when
    /// [`SceneRenderer::set_bloom_compute`] turned it off.
    fn drawn_post(&self) -> PostSettings {
        let mut post = self.post;
        post.bloom.compute &= self.bloom_compute;
        post
    }

    /// Changes the anti-aliasing: off (the default), MLAA, or MSAA, which falls back to the largest sample count the
    /// device has, or to MLAA without one ([`AntiAliasing::supported`]). Applies now and to every later renderer.
    pub fn set_anti_aliasing(&mut self, aa: AntiAliasing) {
        self.anti_aliasing = aa;
        if let Some(device) = self.device.as_mut() {
            apply_anti_aliasing(&mut device.renderer, aa);
        }
        self.settings_changed = true;
    }

    /// The anti-aliasing asked for (the renderer's [`Renderer::anti_aliasing`] says what it draws).
    pub fn anti_aliasing(&self) -> AntiAliasing {
        self.anti_aliasing
    }

    /// Records the scene passes as one render pass ([`PassId::Scene`]), so colour and depth stay in tile memory on
    /// tile-based GPUs, and two render passes' CPU is saved everywhere (#97). On by default and at every preset (and
    /// graphics settings keep it on on tile-based GPUs); off is for measuring. Applies now and to every later renderer.
    pub fn set_merge_scene_passes(&mut self, merge: bool) {
        self.merge_scene_passes = merge;
        self.apply_passes();
    }

    /// Draws the frame without the post chain when `on` is false, as before it existed (#85): the scene straight into
    /// the window, its passes apart, with no bloom, tone curve or MLAA. For measuring what the chain costs; presets
    /// never turn it off (it is part of a game's look). On by default. Applies now and to every later renderer.
    pub fn set_post_chain(&mut self, on: bool) {
        self.post_chain = on;
        self.apply_passes();
    }

    /// The passes a renderer runs, with MLAA when `mlaa`.
    fn passes(&self, mlaa: bool) -> PassSet {
        PassSet::default()
            .with_if(PassId::Post, self.post_chain)
            .with_if(PassId::Bloom, self.post_chain)
            .with_if(PassId::Scene, self.merge_scene_passes && self.post_chain)
            .with_if(PassId::Mlaa, mlaa)
    }

    /// Gives the renderer its passes again, keeping its anti-aliasing: the one asked for, as the device draws it (the
    /// renderer may still be drawing the one before while the new one's pipelines compile).
    fn apply_passes(&mut self) {
        let Some(mlaa) = self
            .renderer()
            .map(|r| self.anti_aliasing.supported(r.msaa_sample_counts()).mlaa())
        else {
            return;
        };
        let passes = self.passes(mlaa);
        if let Some(device) = self.device.as_mut() {
            device.renderer.set_passes(passes);
        }
    }

    /// Applies a player's graphics settings: the settings of every system at the preset's tier with the player's
    /// overrides ([`GraphicsSettings::quality`]), and vsync when the settings say. With Auto and no calibration for
    /// this adapter, driver and workload ([`CALIBRATION_WORKLOAD`]), calibrates (starting at once when attached,
    /// otherwise when it attaches), drawing at Medium until it is done. Applies now and to every later renderer; waits
    /// for nothing.
    pub fn set_graphics(&mut self, graphics: GraphicsSettings) {
        if let Some(vsync) = graphics.vsync {
            self.set_present_mode(if vsync {
                wgpu::PresentMode::Fifo
            } else {
                wgpu::PresentMode::AutoNoVsync
            });
        }
        self.calibration = None;
        self.graphics = Some(graphics);
        self.start_graphics();
    }

    /// The player's graphics settings, once set; with Auto, its calibration once made.
    pub fn graphics(&self) -> Option<&GraphicsSettings> {
        self.graphics.as_ref()
    }

    /// A calibration Auto finished since the last call, for the app to save with the settings (it is already in
    /// [`SceneRenderer::graphics`]).
    pub fn take_calibration(&mut self) -> Option<Calibration> {
        self.new_calibration.take()
    }

    /// The tier Auto's calibration is measuring on its workload, while it runs (the game draws at Medium meanwhile).
    pub fn calibrating(&self) -> Option<Tier> {
        self.calibration.as_ref().map(AutoCalibration::probe)
    }

    /// What the last calibration run by this renderer did: the tier chosen, how long it took, the workload frames
    /// drawn and the game's worst frame meanwhile. `None` until one finishes (and when the settings' calibration was
    /// loaded rather than measured).
    pub fn last_calibration(&self) -> Option<&CalibrationOutcome> {
        self.last_calibration.as_ref()
    }

    /// Tells the renderer the frame cap the app applies (`None`: uncapped), which scripts read in
    /// `UserGameSettings.FrameRateCap`, and which the spike counter measures against (twice a capped frame is a
    /// spike; a 60 fps frame's twice when uncapped). The renderer does not pace frames itself.
    pub fn set_frame_cap(&mut self, fps: Option<u32>) {
        self.frame_cap = fps;
        self.perf.set_frame_target_ms(match fps {
            Some(fps) if fps > 0 => 1000.0 / fps as f32,
            _ => crate::perf::DEFAULT_FRAME_TARGET_MS,
        });
        self.settings_changed = true;
    }

    /// The graphics settings in effect, for the performance overlay and reports: the preset, the tier, Auto's
    /// calibration, and this frame's render scale and dynamic resolution.
    pub fn graphics_status(&self) -> GraphicsStatus {
        GraphicsStatus {
            preset: self.graphics.as_ref().map(|g| g.preset),
            tier: self.tier,
            calibrating: self.calibrating(),
            resolution: self.renderer().map(Renderer::resolution_stats).unwrap_or_default(),
            post: self.renderer().map(Renderer::post_stats).unwrap_or_default(),
            pipelines: self.renderer().map(Renderer::pipeline_stats).unwrap_or_default(),
        }
    }

    /// Applies the graphics settings now: when Auto needs a calibration on the attached adapter, starts it and draws
    /// at Medium (with Auto's dynamic resolution) until it is done; otherwise draws at their tier.
    fn start_graphics(&mut self) {
        let Some(graphics) = self.graphics.as_ref() else {
            return;
        };
        let device = self.device.as_ref().map(|device| {
            let caps = device.context.capabilities();
            let calibrate = graphics.needs_calibration(&caps.adapter_name, &caps.driver, CALIBRATION_WORKLOAD);
            (calibrate, device.renderer.gpu_timing_enabled())
        });
        match (device, self.device.as_ref()) {
            (Some((true, true)), Some(device)) => {
                // A safe tier for any machine while the workload is measured; the game switches once, when it is done.
                let quality = graphics.quality_at(Tier::Medium);
                log::info!("graphics: calibrating the Auto preset on its workload; drawing at Medium meanwhile");
                let format = device.renderer.settings().color_format;
                self.calibration = Some(AutoCalibration::new(format, CALIBRATION_SIZE));
                self.apply_quality(&quality);
            }
            (Some((true, false)), _) => {
                // No timestamps to measure with: the tier from the kind of adapter.
                let tier = self
                    .capabilities()
                    .map_or(Tier::Medium, |caps| Calibrator::untimed_tier(caps.device_type));
                self.finish_calibration(tier, 0.0);
            }
            _ => {
                let quality = graphics.quality();
                self.apply_quality(&quality);
            }
        }
    }

    fn apply_quality(&mut self, quality: &Quality) {
        self.tier = Some(quality.tier);
        self.set_cull(quality.cull);
        self.set_shadows(quality.shadows);
        self.set_lights(quality.lights);
        self.set_lod(quality.lod);
        self.set_resolution(quality.resolution);
        self.set_post(quality.post);
        self.set_anti_aliasing(quality.anti_aliasing);
        // Tile-based GPUs keep the scene in tile memory at every tier.
        let tiled = self.capabilities().is_some_and(|caps| caps.transient_attachments);
        self.set_merge_scene_passes(quality.merge_scene_passes || tiled);
        self.settings_changed = true;
    }

    /// Keeps Auto's choice of `tier` (measured at `gpu_ms`) for this adapter and draws at it.
    fn finish_calibration(&mut self, tier: Tier, gpu_ms: f32) {
        self.calibration = None;
        let (adapter, driver) = self
            .capabilities()
            .map(|caps| (caps.adapter_name.clone(), caps.driver.clone()))
            .unwrap_or_default();
        let frame = self.renderer().map_or(0, Renderer::frame_index);
        log::info!("graphics: Auto chose {tier:?} on {adapter} ({gpu_ms:.2} ms a frame), at frame {frame}");
        let calibration = Calibration {
            tier,
            gpu_ms,
            adapter,
            driver,
            workload: CALIBRATION_WORKLOAD,
        };
        self.new_calibration = Some(calibration.clone());
        if let Some(graphics) = self.graphics.as_mut() {
            graphics.calibration = Some(calibration);
            let quality = graphics.quality();
            self.apply_quality(&quality);
        }
    }

    /// Moves the calibration on after a game frame was presented (`frame_time` after the one before it): draws the
    /// workload frames due and, once it is done, switches the game to the tier chosen.
    fn advance_calibration(&mut self, frame_time: Option<Duration>) {
        let (Some(calibration), Some(device)) = (self.calibration.as_mut(), self.device.as_ref()) else {
            return;
        };
        let frame = GameFrame {
            frame_time,
            gpu_ms: device.renderer.gpu_timings().map(|timings| timings.total_ms()),
            lit_ready: device.renderer.lit_pipelines_ready(),
        };
        let Some(outcome) = calibration.after_frame(&device.context, frame) else {
            return;
        };
        log::info!(
            "graphics: calibrated {:.0} ms after the first frame, over {} game frames ({} waiting for the game's lit \
             pipelines; {} workload frames, built in {:.0} ms); the game's worst frame meanwhile {:.1} ms",
            outcome.duration.as_secs_f32() * 1000.0,
            outcome.game_frames,
            outcome.waited_frames,
            outcome.workload_frames,
            outcome.build.as_secs_f32() * 1000.0,
            outcome.worst_game_frame.as_secs_f32() * 1000.0,
        );
        self.last_calibration = Some(outcome);
        self.finish_calibration(outcome.tier, outcome.gpu_ms);
    }

    /// Writes the graphics settings in effect into `dm`'s `UserGameSettings`, which scripts read, when they changed
    /// since the last call (and the first time). The host calls it between frames (the render hook must not change
    /// the game); nothing happens without graphics settings ([`SceneRenderer::set_graphics`]).
    pub fn write_user_settings(&mut self, dm: &mut DataModel) {
        if !self.settings_changed {
            return;
        }
        self.settings_changed = false;
        let Some(graphics) = self.graphics.as_ref() else {
            return;
        };
        let tier = self.tier.unwrap_or_else(|| graphics.tier());
        let shadow_tier = graphics.shadow_quality.unwrap_or(tier);
        let (_, max_scale) = self.resolution.range();
        let settings = UserGameSettings {
            preset: match graphics.preset.tier() {
                Some(Tier::Low) => "Low",
                Some(Tier::Medium) => "Medium",
                Some(Tier::High) => "High",
                Some(Tier::Ultra) => "Ultra",
                None => "Auto",
            },
            quality_level: tier.level(),
            render_scale: f64::from(max_scale),
            dynamic_resolution: self.resolution.dynamic,
            target_frame_time: f64::from(self.resolution.target_ms),
            view_distance: if self.cull.max_distance.is_finite() {
                f64::from(self.cull.max_distance)
            } else {
                0.0
            },
            shadow_quality_level: shadow_tier.level(),
            shadow_distance: f64::from(self.shadows.distance),
            local_light_budget: self.lights.budget,
            lod_bias: f64::from(self.lod.bias),
            vsync: self.present_mode.is_none_or(|mode| {
                matches!(
                    mode,
                    wgpu::PresentMode::Fifo | wgpu::PresentMode::FifoRelaxed | wgpu::PresentMode::AutoVsync
                )
            }),
            frame_rate_cap: self.frame_cap.unwrap_or(0),
            // What is drawn: MSAA as the device allows it.
            anti_aliasing: match self.renderer().map_or(self.anti_aliasing, Renderer::anti_aliasing) {
                AntiAliasing::Off => "Off",
                AntiAliasing::Mlaa => "MLAA",
                AntiAliasing::Msaa(2) => "MSAA2",
                AntiAliasing::Msaa(4) => "MSAA4",
                AntiAliasing::Msaa(_) => "MSAA8",
            },
            effect_quality_level: graphics.effect_quality.unwrap_or(tier).level(),
        };
        if let Err(error) = set_user_game_settings(dm, &settings) {
            log::warn!("graphics: could not write UserGameSettings: {error}");
        }
    }

    /// Asks for a present mode (for example `AutoNoVsync` for an uncapped frame rate in benchmarks); the surface
    /// falls back to `Fifo` (vsync) when it cannot. Applies now and to every later surface.
    pub fn set_present_mode(&mut self, mode: wgpu::PresentMode) {
        self.present_mode = Some(mode);
        if let (Some(device), Some(surface)) = (self.device.as_ref(), self.surface.as_mut()) {
            surface.set_present_mode(&device.context, mode);
        }
    }

    /// The present mode the surface uses, once attached.
    pub fn present_mode(&self) -> Option<wgpu::PresentMode> {
        self.surface.as_ref().map(RenderSurface::present_mode)
    }

    /// Shows or hides the performance overlay (frame-time graph, percentiles, CPU and GPU time, draws; see
    /// [`crate::perf`]). Off by default. The statistics are kept either way.
    pub fn set_overlay(&mut self, on: bool) {
        self.overlay = on;
        if let Some(device) = self.device.as_mut() {
            device.renderer.set_overlay_enabled(on);
        }
    }

    /// Whether the performance overlay is shown.
    pub fn overlay(&self) -> bool {
        self.overlay
    }

    /// The frame statistics.
    pub fn perf(&self) -> &PerfStats {
        &self.perf
    }

    /// The frame statistics, to reset them (after a warm-up) or summarise them ([`PerfStats::summary`]).
    pub fn perf_mut(&mut self) -> &mut PerfStats {
        &mut self.perf
    }

    /// Keeps the statistics of the last `frames` frames ([`crate::perf::DEFAULT_WINDOW`] by default), dropping
    /// what was recorded. A benchmark sizes it to its whole run. Allocates; not for every frame.
    pub fn set_perf_window(&mut self, frames: usize) {
        let target = self.perf.frame_target_ms();
        self.perf = PerfStats::new(frames);
        self.perf.set_frame_target_ms(target);
    }

    /// What the adapter and device can do (its name, backend, driver), once attached.
    pub fn capabilities(&self) -> Option<&Capabilities> {
        self.device.as_ref().map(|device| device.context.capabilities())
    }

    /// The scene, once the first frame has attached it to the game.
    pub fn scene(&self) -> Option<&GpuScene> {
        self.scene.as_ref()
    }

    /// The renderer, once attached to a window: its draw statistics, timings, materials and light.
    pub fn renderer(&self) -> Option<&Renderer> {
        self.device.as_ref().map(|device| &device.renderer)
    }

    /// The renderer, to change its settings, materials or light.
    pub fn renderer_mut(&mut self) -> Option<&mut Renderer> {
        self.device.as_mut().map(|device| &mut device.renderer)
    }

    /// What the last frame's sync and upload did.
    pub fn last_frame(&self) -> (SyncStats, UploadStats) {
        (self.last_sync, self.last_upload)
    }

    /// What the last drawn frame drew.
    pub fn draw_stats(&self) -> DrawStats {
        self.renderer().map(Renderer::draw_stats).unwrap_or_default()
    }

    /// Attaches the scene to `dm` when it is not attached to it yet (the first frame, or a new game).
    fn ensure_scene(&mut self, dm: &mut DataModel) -> Result<(), RenderError> {
        if self
            .scene
            .as_ref()
            .is_some_and(|scene| scene.scene().is_attached_to(dm))
        {
            return Ok(());
        }
        let Some(device) = self.device.as_ref() else {
            return Ok(());
        };
        let registry = dm.registry().clone();
        self.world = registry.class_id("Workspace").and_then(|class| dm.service(class));
        self.lighting_service = registry.class_id("Lighting").and_then(|class| dm.service(class));
        // A scene of another data model is dropped as it is: that game, and its consumer, are gone.
        let mut scene = GpuScene::new(&device.context, dm, self.world, &RenderComponents::builtin())?;
        // The first sync reports it (every instance is new to the scene), so it is read then.
        if let Some(lighting) = self.lighting_service {
            scene.watch(lighting);
        }
        self.scene = Some(scene);
        // A newly loaded place's first frame waits for the pipelines it draws with (its lights, the settings' AA and
        // effects): a delayed first present rather than a frame that looks wrong.
        if let Some(device) = self.device.as_mut() {
            device.renderer.begin_scene();
        }
        Ok(())
    }

    /// What the scene is lit with: `Lighting` as last read.
    pub fn lighting(&self) -> SceneLighting {
        self.lighting
    }
}

impl Default for SceneRenderer {
    fn default() -> Self {
        Self::new(ContextOptions::default())
    }
}

impl std::fmt::Debug for SceneRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceneRenderer")
            .field("attached", &self.surface.is_some())
            .field("last_sync", &self.last_sync)
            .field("last_upload", &self.last_upload)
            .field("overlay", &self.overlay)
            .finish_non_exhaustive()
    }
}

impl RenderHook for SceneRenderer {
    fn attach(&mut self, window: &Window) -> Result<(), RenderHookError> {
        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));
        let window = Arc::new(window.clone());
        let created = self.device.is_none();
        if created {
            self.attached_at = Some(Instant::now());
        }
        let mut surface = match self.device.as_ref() {
            Some(device) => device.context.create_surface(window, width, height)?,
            None => {
                let (context, surface) = RenderContext::new_with_window(window, width, height, &self.options)?;
                let mut renderer = Renderer::new(&context, self.settings(surface.format(), width, height))?;
                renderer.set_lighting(self.lighting);
                self.device = Some(Device { context, renderer });
                surface
            }
        };
        let settings = self.settings(surface.format(), width, height);
        if let Some(device) = self.device.as_mut() {
            if device.renderer.settings().color_format != surface.format() {
                // A new surface on another monitor or after a resume may prefer another format.
                device.renderer = Renderer::new(&device.context, settings)?;
                device.renderer.set_lighting(self.lighting);
            }
            device.renderer.set_overlay_enabled(self.overlay);
            device.renderer.set_lod_tint(self.lod_tint);
            // A renderer is made without MSAA (its sample counts are known once it exists), then given its own.
            apply_anti_aliasing(&mut device.renderer, self.anti_aliasing);
            if let Some(mode) = self.present_mode {
                surface.set_present_mode(&device.context, mode);
            }
        }
        self.surface = Some(surface);
        // Time spent away (suspended, opening the window) is not a frame.
        self.frame_start = None;
        if created {
            // Auto calibrates on the adapter it now knows.
            self.start_graphics();
        }
        Ok(())
    }

    fn detach(&mut self) {
        self.surface = None;
    }

    fn resize(&mut self, size: Size, _scale_factor: f64) {
        let (Some(device), Some(surface)) = (self.device.as_mut(), self.surface.as_mut()) else {
            return;
        };
        surface.resize(&device.context, size.width, size.height);
        if size.width > 0
            && size.height > 0
            && let Err(error) = device.renderer.resize(size.width, size.height)
        {
            log::warn!("render: could not resize to {}x{}: {error}", size.width, size.height);
        }
    }

    fn render(&mut self, frame: &mut RenderFrame<'_>) {
        let start = Instant::now();
        let frame_time = self.frame_start.replace(start).map(|previous| start - previous);
        if let Err(error) = self.ensure_scene(frame.dm) {
            log::error!("render: {error}");
            return;
        }
        let status = self.overlay.then(|| self.graphics_status());
        let camera = self.world.and_then(|world| self.camera.read(frame.dm, world));
        let (Some(device), Some(scene)) = (self.device.as_mut(), self.scene.as_mut()) else {
            return;
        };
        if let Some(camera) = camera {
            scene.set_camera_position(frame.dm, camera.position);
        }
        self.last_sync = scene.sync(frame.dm);
        if self.last_sync.watched > 0
            && let Some(lighting) = self.lighting_service
        {
            self.lighting = self.lighting_reader.read(frame.dm, lighting);
            device.renderer.set_lighting(self.lighting);
        }
        if let Some(status) = status {
            if self.since_summary == 0 {
                self.summary = self.perf.summary();
            }
            self.since_summary = (self.since_summary + 1) % SUMMARY_EVERY;
            draw_overlay(device.renderer.overlay_mut(), &self.perf, &self.summary, &status);
        }
        let acquire_start = Instant::now();
        let image = match self.surface.as_mut().map(|surface| surface.acquire(&device.context)) {
            Some(Ok(image)) => image,
            Some(Err(error)) => {
                log::error!("render: {error}");
                None
            }
            None => None,
        };
        // Waiting for an image is mostly vsync, not work: it is left out of the render time.
        let acquire = acquire_start.elapsed();
        let render_cpu;
        let draws;
        let cull;
        let presented = image.is_some();
        match image {
            Some(image) => {
                device.renderer.render_scene(image.view(), scene, camera.as_ref());
                render_cpu = start.elapsed().saturating_sub(acquire);
                image.present(&device.context);
                if let Some(attached) = self.attached_at.take() {
                    self.first_frame_ms = Some(attached.elapsed().as_secs_f32() * 1000.0);
                }
                self.last_upload = device.renderer.upload_stats();
                draws = device.renderer.draw_stats();
                cull = device.renderer.cull_stats().cpu_time + device.renderer.shadow_stats().cull_time;
            }
            None => {
                // No image this frame (minimised, occluded): the upload still goes out, or it would be lost.
                let mut encoder = device
                    .context
                    .device()
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("metallic scene upload"),
                    });
                self.last_upload = scene.upload(&mut encoder);
                device.context.submit(std::iter::once(encoder.finish()));
                render_cpu = start.elapsed().saturating_sub(acquire);
                draws = DrawStats::default();
                cull = Duration::ZERO;
            }
        }
        if let Some(frame_time) = frame_time {
            let sample = FrameSample::new(
                frame_time,
                frame.stats.script_cpu,
                frame.stats.simulation_cpu,
                render_cpu,
            )
            .with_cull(cull)
            .with_render_scale(device.renderer.resolution_stats().scale);
            self.perf.record(sample, device.renderer.gpu_timings(), draws);
        }
        // The workload is drawn after a presented frame only: the swapchain paces it then.
        if presented {
            self.advance_calibration(frame_time);
        }
    }
}

impl SceneRenderer {
    fn settings(&self, format: wgpu::TextureFormat, width: u32, height: u32) -> RendererSettings {
        RendererSettings {
            clear_color: self.clear_color,
            cull: self.cull,
            shadows: self.shadows,
            lights: self.lights,
            lod: self.lod,
            resolution: self.resolution,
            post: self.drawn_post(),
            // MLAA works everywhere; MSAA is set once the renderer knows its sample counts.
            passes: self.passes(self.anti_aliasing.mlaa()),
            ..RendererSettings::new(format, width, height)
        }
    }
}

/// Draws with `aa`, MSAA falling back to what `renderer`'s device has.
fn apply_anti_aliasing(renderer: &mut Renderer, aa: AntiAliasing) {
    let aa = aa.supported(renderer.msaa_sample_counts());
    if let Err(error) = renderer.set_anti_aliasing(aa) {
        log::warn!("graphics: could not draw with {aa}: {error}");
    }
}

#[cfg(test)]
mod tests {
    use metallic_core::{EnumItem, Value};
    use metallic_render::settings::Preset;

    use super::*;

    fn user_game_settings(dm: &DataModel, name: &str) -> Value {
        let class = dm.registry().class_id("UserGameSettings").expect("the class");
        let service = dm.service(class).expect("the service");
        dm.get_by_name(service, name).expect("the property")
    }

    #[test]
    fn graphics_settings_set_every_system_and_reach_scripts() {
        let mut dm = metallic_classes::new_game().expect("a game");
        let mut renderer = SceneRenderer::default();
        // Nothing to write before there are graphics settings.
        renderer.write_user_settings(&mut dm);
        assert_eq!(user_game_settings(&dm, "QualityLevel"), Value::Int(2));

        renderer.set_graphics(GraphicsSettings {
            render_scale: Some(0.8),
            vsync: Some(false),
            ..GraphicsSettings::new(Preset::High)
        });
        renderer.set_frame_cap(Some(144));
        let high = Quality::HIGH;
        assert_eq!(renderer.cull(), high.cull);
        assert_eq!(renderer.shadows(), high.shadows);
        assert_eq!(renderer.lights(), high.lights);
        assert_eq!(renderer.lod(), high.lod);
        assert_eq!(renderer.resolution().range(), (0.8, 0.8));
        let status = renderer.graphics_status();
        assert_eq!(
            (status.preset, status.tier, status.calibrating),
            (Some(Preset::High), Some(Tier::High), None)
        );

        renderer.write_user_settings(&mut dm);
        assert_eq!(
            user_game_settings(&dm, "GraphicsPreset"),
            Value::Enum(EnumItem::new("GraphicsPreset", "High"))
        );
        assert_eq!(user_game_settings(&dm, "QualityLevel"), Value::Int(3));
        assert_eq!(user_game_settings(&dm, "RenderScale"), Value::Number(f64::from(0.8f32)));
        assert_eq!(user_game_settings(&dm, "ViewDistance"), Value::Number(2048.0));
        assert_eq!(user_game_settings(&dm, "LocalLightBudget"), Value::Int(192));
        assert_eq!(user_game_settings(&dm, "VSync"), Value::Bool(false));
        assert_eq!(user_game_settings(&dm, "FrameRateCap"), Value::Int(144));

        // Auto before a device: drawn at Medium until it can calibrate, with dynamic resolution.
        renderer.set_graphics(GraphicsSettings::new(Preset::Auto));
        renderer.write_user_settings(&mut dm);
        assert_eq!(
            user_game_settings(&dm, "GraphicsPreset"),
            Value::Enum(EnumItem::new("GraphicsPreset", "Auto"))
        );
        assert_eq!(user_game_settings(&dm, "QualityLevel"), Value::Int(2));
        assert_eq!(user_game_settings(&dm, "DynamicResolution"), Value::Bool(true));
        assert_eq!(user_game_settings(&dm, "ViewDistance"), Value::Number(1024.0));
    }
}
