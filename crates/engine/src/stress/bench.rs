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
use metallic_render::post::PostSettings;
use metallic_render::resolution::ResolutionSettings;
use metallic_render::settings::{GraphicsSettings, Preset, Quality, Tier};
use metallic_render::{AntiAliasing, ContextOptions, CullSettings, LightSettings, LodSettings, ShadowSettings, wgpu};

use super::{StressKind, StressScene};
use crate::config::{EngineConfig, EngineMode};
use crate::engine::Engine;
use crate::error::EngineError;
use crate::perf::{GraphicsStatus, PerfSummary, json_string};
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
    /// What the camera culls (the renderer's defaults unless changed; [`CullSettings::OFF`] draws everything).
    pub cull: CullSettings,
    /// The camera's vertical field of view in degrees; the scene's own when `None`.
    pub field_of_view: Option<f32>,
    /// How the sun's shadows are drawn (the renderer's defaults unless changed; [`ShadowSettings::OFF`] draws none).
    pub shadows: ShadowSettings,
    /// How point and spot lights are drawn (the renderer's defaults, the Medium preset's, unless changed;
    /// [`LightSettings::OFF`] draws none).
    pub lights: LightSettings,
    /// How the round parts' levels of detail are chosen (the renderer's defaults, the High preset's, unless
    /// changed; [`LodSettings::OFF`] draws every part at full detail).
    pub lod: LodSettings,
    /// Colour the parts by level of detail ([`metallic_render::Renderer::set_lod_tint`]).
    pub lod_tint: bool,
    /// Walk the camera through the scene ([`StressScene::move_camera`]) instead of keeping it still.
    pub moving_camera: bool,
    /// A player's graphics settings ([`SceneRenderer::set_graphics`]), which then set the culling, shadows, lights,
    /// levels of detail and resolution instead of the fields above; `None` keeps those.
    pub graphics: Option<GraphicsSettings>,
    /// The scene's resolution when there are no graphics settings (the window's own by default).
    pub resolution: ResolutionSettings,
    /// Switch between the Low and Ultra presets every this many frames, warm-up included (the first switch goes to
    /// the one the run did not start at), to measure what a switch costs.
    pub switch_presets: Option<u32>,
    /// Draw with the post chain (on by default); off draws the frame as before it (#85), to measure what it costs
    /// ([`SceneRenderer::set_post_chain`]).
    pub post_chain: bool,
    /// The anti-aliasing when there are no graphics settings (none, the renderer's default, when `None`).
    pub anti_aliasing: Option<AntiAliasing>,
    /// The post effects' quality when there are no graphics settings (the renderer's defaults, High's, when `None`).
    pub effects: Option<PostSettings>,
    /// Record the scene passes as one render pass when there are no graphics settings.
    pub merge_scene_passes: bool,
    /// Toggle the next effect of [`TOGGLES`] every this many frames, warm-up included (from the first frame, while
    /// the renderer still compiles its pipelines in the background), to show that no switch hitches (#86).
    pub toggle_effects: Option<u32>,
    /// The frame time frames should take, which the spike counter measures against (a frame over twice it, after
    /// the first 2 s, is a spike); a 60 fps frame by default.
    pub frame_target_ms: Option<f32>,
}

/// One step of the effect-toggle cycle ([`BenchOptions::toggle_effects`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Toggle {
    /// A graphics preset, with its own anti-aliasing and effect quality.
    Preset(Preset),
    /// Anti-aliasing (MSAA falls back to what the device has).
    AntiAliasing(AntiAliasing),
    /// The post effects' quality.
    Effects(Tier),
    /// Point and spot lights on (the preset's settings) or off.
    Lights(bool),
    /// The sun's shadows on (the preset's settings) or off.
    Shadows(bool),
    /// The performance overlay on or off.
    Overlay(bool),
}

/// The effect-toggle cycle: every preset, every anti-aliasing, every effect quality, and lights, shadows and the
/// overlay off and on again.
pub const TOGGLES: [Toggle; 20] = [
    Toggle::Preset(Preset::Low),
    Toggle::AntiAliasing(AntiAliasing::Off),
    Toggle::AntiAliasing(AntiAliasing::Mlaa),
    Toggle::AntiAliasing(AntiAliasing::Msaa(2)),
    Toggle::AntiAliasing(AntiAliasing::Msaa(4)),
    Toggle::AntiAliasing(AntiAliasing::Msaa(8)),
    Toggle::Effects(Tier::Low),
    Toggle::Effects(Tier::Medium),
    Toggle::Effects(Tier::High),
    Toggle::Effects(Tier::Ultra),
    Toggle::Lights(false),
    Toggle::Lights(true),
    Toggle::Shadows(false),
    Toggle::Shadows(true),
    Toggle::Overlay(true),
    Toggle::Overlay(false),
    Toggle::Preset(Preset::Medium),
    Toggle::Preset(Preset::High),
    Toggle::Preset(Preset::Ultra),
    Toggle::AntiAliasing(AntiAliasing::Mlaa),
];

impl Toggle {
    /// Applies the toggle to `renderer`; `graphics` is the preset in effect, whose lights and shadows a toggle
    /// turns back on.
    fn apply(self, renderer: &mut SceneRenderer, graphics: &mut GraphicsSettings) {
        match self {
            Toggle::Preset(preset) => {
                *graphics = GraphicsSettings {
                    vsync: graphics.vsync,
                    ..GraphicsSettings::new(preset)
                };
                renderer.set_graphics(graphics.clone());
            }
            Toggle::AntiAliasing(aa) => renderer.set_anti_aliasing(aa),
            Toggle::Effects(tier) => renderer.set_post(Quality::for_tier(tier).post),
            Toggle::Lights(on) => renderer.set_lights(if on {
                graphics.quality().lights
            } else {
                LightSettings::OFF
            }),
            Toggle::Shadows(on) => renderer.set_shadows(if on {
                graphics.quality().shadows
            } else {
                ShadowSettings::OFF
            }),
            Toggle::Overlay(on) => renderer.set_overlay(on),
        }
    }
}

impl BenchOptions {
    /// 1,000 measured frames after 120 of warm-up, seed 1, the scene's window size and camera (still), no vsync, no
    /// overlay, the default culling, shadows, lights and levels of detail.
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
            cull: CullSettings::default(),
            field_of_view: None,
            shadows: ShadowSettings::default(),
            lights: LightSettings::default(),
            lod: LodSettings::default(),
            lod_tint: false,
            moving_camera: false,
            graphics: None,
            resolution: ResolutionSettings::NATIVE,
            switch_presets: None,
            post_chain: true,
            anti_aliasing: None,
            effects: None,
            merge_scene_passes: false,
            toggle_effects: None,
            frame_target_ms: None,
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
    /// The camera's vertical field of view in degrees.
    pub field_of_view: f32,
    /// What the camera culled.
    pub cull: CullSettings,
    /// How the sun's shadows were drawn.
    pub shadows: ShadowSettings,
    /// How point and spot lights were drawn.
    pub light_settings: LightSettings,
    /// How the levels of detail were chosen.
    pub lod: LodSettings,
    /// Whether the camera walked ([`BenchOptions::moving_camera`]).
    pub moving_camera: bool,
    /// How long building the scene took, in milliseconds.
    pub build_ms: f32,
    /// Milliseconds from making the device until the first frame was presented
    /// ([`SceneRenderer::first_frame_ms`]).
    pub first_frame_ms: Option<f32>,
    /// The graphics settings in effect at the end of the run.
    pub graphics: GraphicsStatus,
    /// Auto's calibration, when it ran: the tier chosen and the median GPU frame time it measured.
    pub calibration: Option<(&'static str, f32)>,
    /// The frames between preset switches ([`BenchOptions::switch_presets`]), and how many switches the measured
    /// frames saw.
    pub switches: Option<(u32, u32)>,
    /// The frames between effect toggles ([`BenchOptions::toggle_effects`]), how many toggles the run made (warm-up
    /// included), and the most pipelines pending in any frame.
    pub toggles: Option<(u32, u32, u32)>,
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
            "\"parts\":{},\"see_through\":{},\"lights\":{},\"moving\":{},\"in_view\":{},\"field_of_view\":{:.1},\
             \"build_ms\":{:.1},",
            self.parts, self.see_through, self.lights, self.moving, self.in_view, self.field_of_view, self.build_ms
        )?;
        match self.first_frame_ms {
            Some(ms) => write!(out, "\"first_frame_ms\":{ms:.1},")?,
            None => out.push_str("\"first_frame_ms\":null,"),
        }
        let cull = &self.cull;
        write!(
            out,
            "\"cull\":{{\"enabled\":{},\"max_distance\":{},\"min_screen_size\":{:.2}}},",
            cull.enabled,
            if cull.max_distance.is_finite() {
                format!("{:.1}", cull.max_distance)
            } else {
                "null".to_owned()
            },
            cull.min_screen_size
        )?;
        let shadows = &self.shadows;
        write!(
            out,
            "\"shadows\":{{\"enabled\":{},\"cascades\":{},\"resolution\":{},\"distance\":{:.1},\"dynamic\":{}}},\
             \"moving_camera\":{},",
            shadows.enabled,
            shadows.cascade_count(),
            shadows.resolution,
            shadows.distance,
            shadows.dynamic_count(),
            self.moving_camera
        )?;
        let lights = &self.light_settings;
        write!(
            out,
            "\"light_settings\":{{\"enabled\":{},\"budget\":{},\"per_cluster\":{},\"shadows\":{},\
             \"shadowed_lights\":{},\"atlas\":{},\"tile\":{}}},",
            lights.enabled,
            lights.light_budget(),
            lights.cluster_cap(),
            lights.shadows,
            lights.shadowed_lights,
            lights.atlas_resolution,
            lights.tile_resolution
        )?;
        write!(
            out,
            "\"lod\":{{\"enabled\":{},\"bias\":{:.2}}},",
            self.lod.enabled, self.lod.bias
        )?;
        out.push_str("\"graphics\":{");
        self.graphics.write_json_members(out);
        match self.calibration {
            Some((tier, gpu_ms)) => write!(
                out,
                ",\"calibration\":{{\"tier\":{},\"gpu_ms\":{gpu_ms:.3}}}",
                json_string(tier)
            )?,
            None => out.push_str(",\"calibration\":null"),
        }
        match self.switches {
            Some((every, count)) => write!(out, ",\"switch_every\":{every},\"switches\":{count}")?,
            None => out.push_str(",\"switch_every\":null,\"switches\":0"),
        }
        match self.toggles {
            Some((every, count, pending)) => write!(
                out,
                ",\"toggle_every\":{every},\"toggles\":{count},\"max_pending_pipelines\":{pending}}},"
            )?,
            None => out.push_str(",\"toggle_every\":null,\"toggles\":0,\"max_pending_pipelines\":0},"),
        }
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
    let mut scene = StressScene::build(engine.data_model_mut(), options.scene, options.seed)?;
    if let Some(field_of_view) = options.field_of_view {
        scene
            .set_field_of_view(engine.data_model_mut(), field_of_view)
            .map_err(crate::stress::StressError::from)?;
    }
    let build_ms = (build_start.elapsed().as_secs_f64() * 1000.0) as f32;

    let mut renderer = SceneRenderer::new(options.context.clone());
    renderer.set_perf_window(options.frames.max(1) as usize);
    if let Some(target) = options.frame_target_ms {
        renderer.perf_mut().set_frame_target_ms(target);
    }
    renderer.set_overlay(options.overlay);
    renderer.set_cull(options.cull);
    renderer.set_shadows(options.shadows);
    renderer.set_lights(options.lights);
    renderer.set_lod(options.lod);
    renderer.set_resolution(options.resolution);
    if let Some(aa) = options.anti_aliasing {
        renderer.set_anti_aliasing(aa);
    }
    if let Some(effects) = options.effects {
        renderer.set_post(effects);
    }
    renderer.set_merge_scene_passes(options.merge_scene_passes);
    if let Some(graphics) = options.graphics.clone() {
        renderer.set_graphics(graphics);
    }
    renderer.set_post_chain(options.post_chain);
    renderer.set_lod_tint(options.lod_tint);
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
        moving_camera: options.moving_camera,
        warm_up: u64::from(options.warm_up),
        end: u64::from(options.warm_up) + u64::from(options.frames.max(1)),
        switch: options.switch_presets.filter(|&every| every > 0).map(|every| {
            let graphics = options.graphics.clone().unwrap_or_default();
            (u64::from(every), graphics)
        }),
        switches: 0,
        toggle: options.toggle_effects.filter(|&every| every > 0).map(|every| {
            let graphics = options.graphics.clone().unwrap_or_default();
            (u64::from(every), graphics)
        }),
        toggles: 0,
        max_pending: 0,
        summary: None,
        present_mode: None,
        error: None,
        trace: std::env::var_os("METALLIC_TRACE_SWITCHES").map(|_| SwitchTrace::default()),
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
        field_of_view: app.scene.field_of_view(),
        cull: renderer.cull(),
        shadows: renderer.shadows(),
        light_settings: renderer.lights(),
        lod: renderer.lod(),
        moving_camera: options.moving_camera,
        build_ms,
        first_frame_ms: renderer.first_frame_ms(),
        graphics: renderer.graphics_status(),
        calibration: renderer
            .graphics()
            .and_then(|graphics| graphics.calibration.as_ref())
            .map(|calibration| (calibration.tier.name(), calibration.gpu_ms)),
        switches: app.switch.as_ref().map(|(every, _)| (*every as u32, app.switches)),
        toggles: app
            .toggle
            .as_ref()
            .map(|(every, _)| (*every as u32, app.toggles, app.max_pending)),
        summary,
    })
}

/// The windowed driver, plus the scene's motion and the frame count that ends the run.
struct BenchApp {
    inner: WindowedApp<StdOutput>,
    renderer: Rc<RefCell<SceneRenderer>>,
    scene: StressScene,
    moving_camera: bool,
    warm_up: u64,
    end: u64,
    /// Every how many frames the preset switches between Low and Ultra, and the settings it switches.
    switch: Option<(u64, GraphicsSettings)>,
    /// Switches made in the measured frames.
    switches: u32,
    /// Every how many frames the next effect of [`TOGGLES`] is toggled, and the graphics settings in effect.
    toggle: Option<(u64, GraphicsSettings)>,
    /// Toggles made (warm-up included).
    toggles: u32,
    /// The most pipelines pending after any frame.
    max_pending: u32,
    summary: Option<PerfSummary>,
    present_mode: Option<wgpu::PresentMode>,
    error: Option<EngineError>,
    /// `METALLIC_TRACE_SWITCHES`: the frames after each toggle, printed.
    trace: Option<SwitchTrace>,
}

/// Prints the frames after each toggle of the effect-toggle cycle on stderr (`METALLIC_TRACE_SWITCHES=1`): each
/// frame's time, what it drew of the shadows, the textures still being made off the frame, and the GPU time of its
/// passes once its timestamps arrive (one to three frames late), to see where a switch's cost goes and how many frames
/// it takes (#152).
#[derive(Debug, Default)]
struct SwitchTrace {
    /// The renderer's frame index at the last toggle, and the frames traced since.
    switch: Option<u64>,
    frames: u32,
    /// The newest frame whose GPU times were printed.
    printed: u64,
}

/// Frames traced after a toggle.
const TRACED_FRAMES: u32 = 10;

impl SwitchTrace {
    fn switched(&mut self, toggle: Toggle, renderer: Option<&metallic_render::Renderer>) {
        let Some(renderer) = renderer else { return };
        self.switch = Some(renderer.frame_index());
        self.frames = 0;
        match toggle {
            Toggle::Preset(preset) => eprintln!("switch to {preset:?} before frame {}", renderer.frame_index()),
            other => eprintln!("toggle {other:?} before frame {}", renderer.frame_index()),
        }
    }

    fn frame(&mut self, (frame_ms, render_ms): (f32, f32), renderer: Option<&metallic_render::Renderer>) {
        let (Some(switch), Some(renderer)) = (self.switch, renderer) else {
            return;
        };
        if self.frames < TRACED_FRAMES {
            self.frames += 1;
            let shadows = renderer.shadow_stats();
            let lights = renderer.light_stats();
            let textures = renderer.texture_stats();
            eprintln!(
                "  frame {}: {frame_ms:.2} ms (render {render_ms:.2}), cascades drawn {:03b} built {:03b}, tiles drawn \
                 {}, msaa {}, textures pending {} (made {}, {:.0} MiB, longest {:.1} ms)",
                renderer.frame_index().saturating_sub(1),
                shadows.redrawn_mask(),
                shadows.built_mask(),
                lights.tiles_redrawn,
                renderer.settings().msaa_samples,
                textures.pending,
                textures.made,
                textures.made_bytes as f64 / f64::from(1 << 20),
                textures.longest_ms
            );
        }
        if let Some(timings) = renderer.gpu_timings()
            && timings.frame() >= switch
            && timings.frame() < switch + u64::from(TRACED_FRAMES)
            && timings.frame() > self.printed
        {
            self.printed = timings.frame();
            let mut line = format!("    gpu {}: total {:.2}", timings.frame(), timings.total_ms());
            for cascade in 0..timings.cascade_count() {
                let _ = write!(
                    line,
                    ", c{cascade} {:.2}/{:.2}",
                    timings.cascade(cascade).unwrap_or(0.0),
                    timings.cascade_build(cascade).unwrap_or(0.0)
                );
            }
            let _ = write!(line, ", atlas {:.2}", timings.light_shadow_ms());
            for pass in timings.passes() {
                let _ = write!(line, ", {} {:.2}", pass.pass, pass.gpu_ms);
            }
            eprintln!("{line}");
        }
    }
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
        let moved = self.scene.animate(engine.data_model_mut(), time).and_then(|()| {
            if self.moving_camera {
                self.scene.move_camera(engine.data_model_mut(), time)
            } else {
                Ok(())
            }
        });
        if let Err(error) = moved {
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
        if let Some((every, graphics)) = self.switch.as_mut()
            && frames.is_multiple_of(*every)
            && frames < self.end
        {
            graphics.preset = if graphics.preset == Preset::Low {
                Preset::Ultra
            } else {
                Preset::Low
            };
            self.renderer.borrow_mut().set_graphics(graphics.clone());
            if frames >= self.warm_up {
                self.switches += 1;
            }
        }
        {
            let mut renderer = self.renderer.borrow_mut();
            let pending = renderer.graphics_status().pipelines.pending;
            self.max_pending = self.max_pending.max(pending);
            if std::env::var_os("METALLIC_TRACE_SPIKES").is_some()
                && let Some(last) = renderer.perf().last()
                && last.frame_ms > 2.0 * renderer.perf().frame_target_ms()
            {
                let every = self.toggle.as_ref().map_or(1, |(every, _)| *every);
                eprintln!(
                    "frame {frames}: {:.2} ms (render {:.2}), after {:?}",
                    last.frame_ms,
                    last.render_ms,
                    TOGGLES[((frames - 1) / every) as usize % TOGGLES.len()]
                );
            }
            if let Some(trace) = self.trace.as_mut()
                && let Some(last) = renderer.perf().last()
            {
                trace.frame((last.frame_ms, last.render_ms), renderer.renderer());
            }
            if let Some((every, graphics)) = self.toggle.as_mut()
                && frames.is_multiple_of(*every)
                && frames < self.end
            {
                let step = (frames / *every) as usize % TOGGLES.len();
                TOGGLES[step].apply(&mut renderer, graphics);
                self.toggles += 1;
                if let Some(trace) = self.trace.as_mut() {
                    trace.switched(TOGGLES[step], renderer.renderer());
                }
            }
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
            gpu_passes: 1 << PassId::Opaque as u16,
            ..PerfSummary::default()
        };
        summary.frame_ms.p50 = 2.0;
        summary.draws.opaque_draws = 4;
        summary.gpu_cascades = 3;
        summary.shadow_redraws = [10, 10, 1, 0];
        summary.shadow_builds = [0, 0, 9, 0];
        summary.draws.shadow_cascades = 3;
        summary.draws.shadow_casters = [5, 6, 7, 0];
        summary.draws.shadow_moving = [0, 0, 2, 0];
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
            field_of_view: 70.0,
            cull: CullSettings::default(),
            shadows: ShadowSettings::default(),
            light_settings: LightSettings::default(),
            lod: LodSettings::default(),
            moving_camera: false,
            build_ms: 12.5,
            first_frame_ms: Some(141.3),
            graphics: GraphicsStatus {
                preset: Some(Preset::Auto),
                tier: Some(metallic_render::settings::Tier::High),
                calibrating: None,
                resolution: metallic_render::resolution::ResolutionStats {
                    scale: 0.75,
                    width: 1440,
                    height: 810,
                    dynamic: true,
                    target_ms: 2.0,
                    min_scale: 0.5,
                    max_scale: 1.0,
                    gpu_ms: 2.1,
                },
                post: metallic_render::PostStats {
                    scene_format: metallic_render::wgpu::TextureFormat::Rg11b10Ufloat,
                    finish: true,
                    msaa_samples: 4,
                    mlaa: false,
                    bloom_levels: 5,
                    render_passes: 14,
                    full_screen_passes: 5,
                    planned: [
                        Some(PassId::Opaque),
                        Some(PassId::Sky),
                        Some(PassId::Transparent),
                        Some(PassId::Bloom),
                        Some(PassId::Post),
                        Some(PassId::Ui),
                        None,
                        None,
                        None,
                        None,
                    ],
                },
                pipelines: metallic_render::PipelineStats {
                    pending: 3,
                    compiled: 54,
                    workers: 8,
                    first_frame_ms: 141.0,
                    compile_ms: 2500.0,
                    cache_loaded: Some(1024),
                    ..metallic_render::PipelineStats::default()
                },
            },
            calibration: Some(("high", 3.25)),
            switches: Some((60, 16)),
            toggles: Some((30, 40, 48)),
            summary,
        };
        let json = report.to_json();
        assert!(json.starts_with("{\"scene\":\"parts_10k\""), "{json}");
        assert!(json.ends_with("}}"), "{json}");
        assert!(json.contains("\"adapter\":\"GPU \\\"X\\\"\""), "{json}");
        assert!(json.contains("\"fps_p50\":500.0"), "{json}");
        assert!(json.contains("\"build_ms\":12.5,\"first_frame_ms\":141.3,"), "{json}");
        assert!(json.contains("\"passes\":{\"opaque\":{"), "{json}");
        assert!(json.contains("\"opaque\":4"), "{json}");
        assert!(
            json.contains(
                "\"shadows\":{\"enabled\":true,\"cascades\":3,\"resolution\":2048,\"distance\":256.0,\"dynamic\":2}"
            ),
            "{json}"
        );
        assert!(json.contains("\"shadow\":[{\"mean\":"), "{json}");
        assert!(json.contains("\"shadow_build\":[{\"mean\":"), "{json}");
        assert!(
            json.contains(
                "\"shadow\":{\"cascades\":3,\"redraws\":[10,10,1],\"builds\":[0,0,9],\"casters\":[5,6,7],\
                 \"moving\":[0,0,2]"
            ),
            "{json}"
        );
        assert!(
            json.contains("\"light_settings\":{\"enabled\":true,\"budget\":128,\"per_cluster\":48"),
            "{json}"
        );
        assert!(json.contains("\"lights\":{\"in_scene\":0,"), "{json}");
        assert!(json.contains("\"light_shadows\":{\"mean\":"), "{json}");
        assert!(json.contains("\"lod\":{\"enabled\":true,\"bias\":2.00}"), "{json}");
        assert!(json.contains("\"culled\":0,\"levels\":[0,0,0,0]}"), "{json}");
        assert!(
            json.contains(
                "\"graphics\":{\"preset\":\"auto\",\"tier\":\"high\",\"calibrating\":null,\"resolution\":{\
                 \"scale\":0.7500,\"width\":1440,\"height\":810,\"dynamic\":true,\"target_ms\":2.000,\
                 \"min_scale\":0.500,\"max_scale\":1.000},\"post\":{\"scene_format\":\"rg11b10f\",\"finish\":true,\
                 \"anti_aliasing\":\"msaa4\",\"bloom_levels\":5,\"render_passes\":14,\"full_screen_passes\":5,\
                 \"passes\":[\"opaque\",\"sky\",\"transparent\",\"bloom\",\"post\",\"ui\"]},\
                 \"pipelines\":{\"compiled\":54,\"pending\":3,\"failed\":0,\"workers\":8,\"first_frame_ms\":141.0,\
                 \"all_ms\":null,\"compile_ms\":2500.0,\"switch_waiting\":false,\"cache_loaded\":1024},\
                 \"calibration\":{\"tier\":\"high\",\"gpu_ms\":3.250},\"switch_every\":60,\"switches\":16,\
                 \"toggle_every\":30,\"toggles\":40,\"max_pending_pipelines\":48},"
            ),
            "{json}"
        );
        assert!(
            json.contains("\"spikes\":{\"count\":0,\"over_ms\":0.000,\"worst_ms\":0.0000},"),
            "{json}"
        );
        assert!(json.contains("\"render_scale\":{\"mean\":"), "{json}");
        // Balanced braces.
        let open = json.matches('{').count();
        assert_eq!(open, json.matches('}').count(), "{json}");
    }
}
