//! [`SceneRenderer`]: `metallic_render` as the engine's [`RenderHook`]. A thin adapter: the renderer crate cannot
//! depend on the engine (ADR 0001), so the hook lives here and only forwards.

use std::sync::Arc;
use std::time::{Duration, Instant};

use metallic_core::{DataModel, Id};
use metallic_platform::{Size, Window};
use metallic_render::scene::{RenderComponents, SyncStats, UploadStats};
use metallic_render::{
    CameraReader, Capabilities, ContextOptions, CullSettings, DrawStats, GpuScene, LightingReader, RenderContext,
    RenderError, RenderSurface, Renderer, RendererSettings, SceneLighting, ShadowSettings, wgpu,
};

use crate::hooks::{RenderFrame, RenderHook, RenderHookError};
use crate::perf::{FrameSample, PerfStats, PerfSummary, draw_overlay};

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
/// `Workspace` from `Workspace.CurrentCamera` ([`Renderer::render_scene`]), lit by the `Lighting` service with the
/// sky behind them; without a current camera the frame is the clear colour. `Lighting` is read only in frames after
/// it changed: the scene watches it on the change consumer it drains anyway ([`GpuScene::watch`]). The device and
/// the scene survive [`RenderHook::detach`], so a mobile app coming back to the foreground only makes a new surface.
///
/// It also keeps the frame statistics ([`SceneRenderer::perf`]): every drawn frame records its frame time, the
/// script and physics time the engine measured ([`Engine::set_cpu_timing`](crate::Engine::set_cpu_timing)), its own
/// render CPU time, the GPU timings and the draws. [`SceneRenderer::set_overlay`] shows them on screen.
pub struct SceneRenderer {
    options: ContextOptions,
    clear_color: wgpu::Color,
    cull: CullSettings,
    shadows: ShadowSettings,
    present_mode: Option<wgpu::PresentMode>,
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
            present_mode: None,
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
        }
    }

    /// Changes the colour frames start from (linear).
    pub fn set_clear_color(&mut self, color: wgpu::Color) {
        self.clear_color = color;
        if let Some(device) = self.device.as_mut() {
            device.renderer.set_clear_color(color);
        }
    }

    /// Changes what the camera culls (frustum, view distance, screen size; see [`metallic_render::cull`]). On by
    /// default; graphics presets (#84) set the thresholds. Applies now and to every later renderer.
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
    /// by default; graphics presets (#84) set them, and `Lighting.GlobalShadows` turns a game's shadows off. Applies
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
        self.perf = PerfStats::new(frames);
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
            if let Some(mode) = self.present_mode {
                surface.set_present_mode(&device.context, mode);
            }
        }
        self.surface = Some(surface);
        // Time spent away (suspended, opening the window) is not a frame.
        self.frame_start = None;
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
        if self.overlay {
            if self.since_summary == 0 {
                self.summary = self.perf.summary();
            }
            self.since_summary = (self.since_summary + 1) % SUMMARY_EVERY;
            draw_overlay(device.renderer.overlay_mut(), &self.perf, &self.summary);
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
        match image {
            Some(image) => {
                device.renderer.render_scene(image.view(), scene, camera.as_ref());
                render_cpu = start.elapsed().saturating_sub(acquire);
                image.present(&device.context);
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
                device.context.queue().submit(std::iter::once(encoder.finish()));
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
            .with_cull(cull);
            self.perf.record(sample, device.renderer.gpu_timings(), draws);
        }
    }
}

impl SceneRenderer {
    fn settings(&self, format: wgpu::TextureFormat, width: u32, height: u32) -> RendererSettings {
        RendererSettings {
            clear_color: self.clear_color,
            cull: self.cull,
            shadows: self.shadows,
            ..RendererSettings::new(format, width, height)
        }
    }
}
