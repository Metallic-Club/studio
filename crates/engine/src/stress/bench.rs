//! Running a stress scene in a window for a number of frames and reporting what the frames cost ([`run_bench`]).
//!
//! The run: a new standalone game ([`RunContext::Local`], no scripts) with the scene built into it, drawn by
//! [`SceneRenderer`] in a window of the scene's size, uncapped (no vsync) unless asked otherwise. Each frame first
//! moves the scene's moving parts to where they are at that frame (frame `n` is `n / 60` seconds, so every run
//! moves them the same way whatever its frame rate), then runs the engine's frame. The first `warm_up` frames are
//! dropped (pipelines, first uploads, promotion of the moving parts to the dynamic region, the GPU clocks rising);
//! the next `frames` are summarised ([`PerfSummary`]) and the window closes.
//!
//! The platform's event loop can only be made once per process, so a process runs one benchmark; the `stress`
//! example runs every scene by starting itself once per scene.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::rc::Rc;
use std::time::Instant;

use metallic_platform::{
    AppHandler, DeviceEvent, EventLoopContext, GamepadEvent, Platform, WindowConfig, WindowEvent, WindowId,
};
use metallic_render::{ContextOptions, wgpu};

use super::{StressKind, StressScene};
use crate::config::{EngineConfig, EngineMode};
use crate::engine::Engine;
use crate::error::EngineError;
use crate::perf::{PerfSummary, json_string};
use crate::scene_renderer::SceneRenderer;
use crate::windowed::{WindowedApp, WindowedOptions};
use crate::{RunContext, StdOutput};

/// Simulated seconds per frame for the scene's motion: the motion is a function of the frame number.
const MOTION_FPS: f64 = 60.0;

/// How a benchmark runs.
#[derive(Clone, Debug)]
pub struct BenchOptions {
    /// The scene.
    pub scene: StressKind,
    /// Frames measured, after the warm-up.
    pub frames: u32,
    /// Frames run and dropped first.
    pub warm_up: u32,
    /// The scene's seed.
    pub seed: u64,
    /// The window's size in logical pixels; the scene's own ([`StressKind::window_size`]) when `None`.
    pub window: Option<(u32, u32)>,
    /// Present with vsync (`Fifo`), which caps the frame rate at the display's; off by default, so frame times
    /// show the work.
    pub vsync: bool,
    /// Show the performance overlay while running (it costs a little CPU and GPU time).
    pub overlay: bool,
    /// How the adapter is chosen (`WGPU_BACKEND` picks the backend).
    pub context: ContextOptions,
}

impl BenchOptions {
    /// 1,000 measured frames after 120 of warm-up, seed 1, the scene's window size, no vsync, no overlay.
    pub fn new(scene: StressKind) -> Self {
        Self {
            scene,
            frames: 1_000,
            warm_up: 120,
            seed: 1,
            window: None,
            vsync: false,
            overlay: false,
            context: ContextOptions::default(),
        }
    }
}

/// What a benchmark measured, and on what. [`BenchReport::to_json`] prints it.
#[derive(Clone, Debug)]
pub struct BenchReport {
    /// The scene.
    pub scene: StressKind,
    /// Its seed.
    pub seed: u64,
    /// Frames measured.
    pub frames: u32,
    /// Frames dropped first.
    pub warm_up: u32,
    /// The adapter's name.
    pub adapter: String,
    /// The backend (`Vulkan`, `Dx12`, `Metal`, `Gl`).
    pub backend: String,
    /// Discrete, integrated, ...
    pub device_type: String,
    /// The driver name and version.
    pub driver: String,
    /// The drawn size in physical pixels.
    pub width: u32,
    /// The drawn height in physical pixels.
    pub height: u32,
    /// The present mode the surface used.
    pub present_mode: String,
    /// Whether the overlay was on.
    pub overlay: bool,
    /// Parts in the scene.
    pub parts: usize,
    /// See-through parts.
    pub see_through: usize,
    /// Light instances.
    pub lights: usize,
    /// Moving parts.
    pub moving: usize,
    /// Parts whose centre is in view ([`StressScene::in_view`]).
    pub in_view: usize,
    /// How long building the scene took, in milliseconds.
    pub build_ms: f32,
    /// The measured frames.
    pub summary: PerfSummary,
}

impl BenchReport {
    /// The report as one JSON object (times in milliseconds).
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(2048);
        // Writing to a `String` cannot fail.
        let _ = self.write_json(&mut out);
        out
    }

    fn write_json(&self, out: &mut String) -> std::fmt::Result {
        write!(
            out,
            "{{\"scene\":{},\"seed\":{},\"warm_up\":{},",
            json_string(self.scene.name()),
            self.seed,
            self.warm_up
        )?;
        write!(
            out,
            "\"adapter\":{},\"backend\":{},\"device_type\":{},\"driver\":{},",
            json_string(&self.adapter),
            json_string(&self.backend),
            json_string(&self.device_type),
            json_string(&self.driver)
        )?;
        write!(
            out,
            "\"width\":{},\"height\":{},\"present_mode\":{},\"overlay\":{},",
            self.width,
            self.height,
            json_string(&self.present_mode),
            self.overlay
        )?;
        write!(
            out,
            "\"parts\":{},\"see_through\":{},\"lights\":{},\"moving\":{},\"in_view\":{},\"build_ms\":{:.1},",
            self.parts, self.see_through, self.lights, self.moving, self.in_view, self.build_ms
        )?;
        self.summary.write_json_members(out);
        out.push('}');
        Ok(())
    }
}

/// Builds `options.scene`, runs it in a window for the warm-up and the measured frames, and reports. Call it on the
/// main thread, once per process (the event loop cannot be made twice).
pub fn run_bench(options: &BenchOptions) -> Result<BenchReport, EngineError> {
    let config = EngineConfig::new(RunContext::Local).with_mode(EngineMode::Windowed);
    let mut engine = Engine::new(config)?;
    let build_start = Instant::now();
    let scene = StressScene::build(engine.data_model_mut(), options.scene, options.seed)?;
    let build_ms = (build_start.elapsed().as_secs_f64() * 1000.0) as f32;

    let mut renderer = SceneRenderer::new(options.context.clone());
    renderer.set_perf_window(options.frames.max(1) as usize);
    renderer.set_overlay(options.overlay);
    renderer.set_present_mode(if options.vsync {
        wgpu::PresentMode::Fifo
    } else {
        wgpu::PresentMode::AutoNoVsync
    });
    let renderer = Rc::new(RefCell::new(renderer));
    engine.set_render_hook(Box::new(Rc::clone(&renderer)))?;
    engine.start()?;

    let (width, height) = options.window.unwrap_or(options.scene.window_size());
    let title = format!("Metallic stress: {}", options.scene.name());
    let window = WindowConfig::new(title).with_size(width, height);
    let mut app = BenchApp {
        inner: WindowedApp::new(engine, WindowedOptions::new(window))?,
        renderer: Rc::clone(&renderer),
        scene,
        warm_up: u64::from(options.warm_up),
        end: u64::from(options.warm_up) + u64::from(options.frames.max(1)),
        summary: None,
        present_mode: None,
        error: None,
    };
    Platform::new()?.run(&mut app)?;
    if let Some(error) = app.error.take().or_else(|| app.inner.take_error()) {
        return Err(error);
    }
    let summary = app.summary.ok_or(EngineError::Mode(
        "the benchmark's window closed before every frame ran",
    ))?;

    let renderer = renderer.borrow();
    let caps = renderer.capabilities();
    let settings = renderer.renderer().map(|r| *r.settings());
    Ok(BenchReport {
        scene: options.scene,
        seed: options.seed,
        frames: options.frames.max(1),
        warm_up: options.warm_up,
        adapter: caps.map(|c| c.adapter_name.clone()).unwrap_or_default(),
        backend: caps.map(|c| format!("{:?}", c.backend)).unwrap_or_default(),
        device_type: caps.map(|c| format!("{:?}", c.device_type)).unwrap_or_default(),
        driver: caps.map(|c| c.driver.clone()).unwrap_or_default(),
        width: settings.map_or(0, |s| s.width),
        height: settings.map_or(0, |s| s.height),
        present_mode: app.present_mode.map(|mode| format!("{mode:?}")).unwrap_or_default(),
        overlay: options.overlay,
        parts: app.scene.parts(),
        see_through: app.scene.see_through(),
        lights: app.scene.lights(),
        moving: app.scene.moving(),
        in_view: app.scene.in_view(),
        build_ms,
        summary,
    })
}

/// The windowed driver, plus the scene's motion and the frame count that ends the run.
struct BenchApp {
    inner: WindowedApp<StdOutput>,
    renderer: Rc<RefCell<SceneRenderer>>,
    scene: StressScene,
    warm_up: u64,
    end: u64,
    summary: Option<PerfSummary>,
    present_mode: Option<wgpu::PresentMode>,
    error: Option<EngineError>,
}

impl AppHandler for BenchApp {
    fn resumed(&mut self, cx: &mut dyn EventLoopContext) {
        self.inner.resumed(cx);
    }

    fn suspended(&mut self, cx: &mut dyn EventLoopContext) {
        self.inner.suspended(cx);
    }

    fn window_event(&mut self, cx: &mut dyn EventLoopContext, window: WindowId, event: WindowEvent) {
        self.inner.window_event(cx, window, event);
    }

    fn device_event(&mut self, cx: &mut dyn EventLoopContext, event: DeviceEvent) {
        self.inner.device_event(cx, event);
    }

    fn gamepad_event(&mut self, cx: &mut dyn EventLoopContext, event: GamepadEvent) {
        self.inner.gamepad_event(cx, event);
    }

    fn about_to_wait(&mut self, cx: &mut dyn EventLoopContext) {
        if self.summary.is_some() || self.error.is_some() {
            return;
        }
        let engine = self.inner.engine_mut();
        // Where the moving parts are in the frame about to run.
        let time = (engine.frame_count() + 1) as f64 / MOTION_FPS;
        if let Err(error) = self.scene.animate(engine.data_model_mut(), time) {
            self.error = Some(crate::stress::StressError::from(error).into());
            cx.exit();
            return;
        }
        let before = self.inner.engine().frame_count();
        self.inner.about_to_wait(cx);
        let frames = self.inner.engine().frame_count();
        if frames == before {
            return;
        }
        if frames == self.warm_up {
            self.renderer.borrow_mut().perf_mut().reset();
        }
        if frames >= self.end {
            let mut renderer = self.renderer.borrow_mut();
            self.summary = Some(renderer.perf_mut().summary());
            // Read while the surface exists: closing the window drops it.
            self.present_mode = renderer.present_mode();
            cx.exit();
        }
    }

    fn exiting(&mut self) {
        self.inner.exiting();
    }
}

#[cfg(test)]
mod tests {
    use metallic_render::PassId;

    use super::*;

    #[test]
    fn a_report_is_one_json_object() {
        let mut summary = PerfSummary {
            frames: 10,
            gpu_frames: 9,
            gpu_passes: 1 << PassId::Opaque as u8,
            ..PerfSummary::default()
        };
        summary.frame_ms.p50 = 2.0;
        summary.draws.opaque_draws = 4;
        let report = BenchReport {
            scene: StressKind::Parts10k,
            seed: 1,
            frames: 10,
            warm_up: 2,
            adapter: "GPU \"X\"".into(),
            backend: "Vulkan".into(),
            device_type: "DiscreteGpu".into(),
            driver: "1.0".into(),
            width: 1920,
            height: 1080,
            present_mode: "Immediate".into(),
            overlay: false,
            parts: 10_000,
            see_through: 0,
            lights: 0,
            moving: 100,
            in_view: 2_000,
            build_ms: 12.5,
            summary,
        };
        let json = report.to_json();
        assert!(json.starts_with("{\"scene\":\"parts_10k\""), "{json}");
        assert!(json.ends_with("}}"), "{json}");
        assert!(json.contains("\"adapter\":\"GPU \\\"X\\\"\""), "{json}");
        assert!(json.contains("\"fps_p50\":500.0"), "{json}");
        assert!(json.contains("\"passes\":{\"opaque\":{"), "{json}");
        assert!(json.contains("\"opaque\":4"), "{json}");
        // Balanced braces.
        let open = json.matches('{').count();
        assert_eq!(open, json.matches('}').count(), "{json}");
    }
}
