//! The windowed player: winit window, wgpu surface, input, frame loop and statistics.
use crate::args::Args;
use crate::capture::CaptureTarget;
use crate::devices::{GamepadSource, key_control, mouse_control};
use crate::overlay::Overlay;
use metallic_render::glam::Vec3;
use metallic_render::{Camera, RenderContext, RenderSettings, Renderer};
use metallic_runtime::input::{Control, Device, RawEvent, ScriptedSource};
use metallic_runtime::{FrameInput, Game, InputRuntime, TICK};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

/// How long `--fps` measures.
const FPS_SECONDS: f64 = 60.0;
/// `--autopilot` turns the camera (and so the run direction) this fast, in radians per second:
/// at the run speed of 24 studs/s that is a circle of 5 studs radius, which stays on the
/// Parkour spawn island.
const AUTOPILOT_TURN: f32 = 4.8;
/// `--autopilot` jumps this often, in seconds.
const AUTOPILOT_JUMP_EVERY: f64 = 1.5;

/// Plays `game` in a window until it is closed (or `--frames` / `--fps` ends it).
pub fn run(game: Game, args: &Args, script: Option<ScriptedSource>) -> ExitCode {
    let event_loop = match EventLoop::new() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: cannot open a window: {e}");
            return ExitCode::from(1);
        }
    };
    event_loop.set_control_flow(ControlFlow::Poll);
    let epoch = Instant::now();
    let input = InputRuntime::from_world(&game.world().borrow());
    for warning in input.warnings() {
        eprintln!("warning: input: {warning}");
    }
    let mut app = App {
        game,
        args: args.clone(),
        gfx: None,
        input,
        gamepads: GamepadSource::new(epoch),
        script,
        epoch,
        events: Vec::new(),
        captured: false,
        last: Instant::now(),
        frame: 0,
        frame_times: Vec::new(),
        started: None,
        failed: false,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("error: {e}");
        return ExitCode::from(1);
    }
    app.game.stop();
    if app.failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    ctx: RenderContext,
    renderer: Renderer,
    overlay: Overlay,
    minimised: bool,
}

struct App {
    game: Game,
    args: Args,
    gfx: Option<Gfx>,
    input: InputRuntime,
    gamepads: GamepadSource,
    /// `--input-script`: when set, the input clock counts frames, see `App::input_clock`.
    script: Option<ScriptedSource>,
    /// When the player started: the zero of the input clock.
    epoch: Instant,
    /// Scratch for the script's events.
    events: Vec<RawEvent>,
    /// The cursor is locked and mouse movement is sent as `MouseDelta`.
    captured: bool,
    last: Instant,
    frame: u64,
    frame_times: Vec<f64>,
    started: Option<Instant>,
    failed: bool,
}

impl App {
    fn create_gfx(&mut self, event_loop: &ActiveEventLoop) -> Result<Gfx, String> {
        // `--fps` measures at exactly 1920x1080, so that window cannot be resized.
        let attrs = Window::default_attributes()
            .with_title(format!("{} - Metallic player", self.game.name()))
            .with_inner_size(PhysicalSize::new(1920, 1080))
            .with_resizable(!self.args.fps);
        let window = Arc::new(event_loop.create_window(attrs).map_err(|e| e.to_string())?);
        let (ctx, surface) =
            RenderContext::new_for_window(window.clone()).map_err(|e| e.to_string())?;
        let adapter = ctx.adapter.as_ref().ok_or("no graphics adapter")?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(adapter, size.width.max(1), size.height.max(1))
            .ok_or("the window surface is not supported by the graphics adapter")?;
        let caps = surface.get_capabilities(adapter);
        if let Some(f) = caps.formats.iter().find(|f| f.is_srgb()) {
            config.format = *f;
        }
        config.present_mode = if self.args.fps {
            wgpu::PresentMode::AutoNoVsync
        } else {
            wgpu::PresentMode::AutoVsync
        };
        surface.configure(&ctx.device, &config);
        let settings = RenderSettings {
            quality: self.args.quality,
        };
        let mut renderer = Renderer::with_settings(&ctx, config.format, settings);
        if let Ok(world) = self.game.world().try_borrow() {
            renderer.sync(&world);
        }
        renderer.prewarm((config.width, config.height));
        let overlay = Overlay::new(&ctx.device, config.format);
        println!(
            "metallic-player: {} on {}, {}x{}, {:?} quality",
            self.game.name(),
            ctx.adapter_name,
            config.width,
            config.height,
            self.args.quality
        );
        Ok(Gfx {
            window,
            surface,
            config,
            ctx,
            renderer,
            overlay,
            minimised: false,
        })
    }

    /// Seconds on the input clock: real time, or the frame count in ticks when an input script
    /// drives the run, so the script plays the same way every time.
    fn input_clock(&self) -> f64 {
        if self.script.is_some() {
            self.frame as f64 * TICK
        } else {
            self.epoch.elapsed().as_secs_f64()
        }
    }

    /// Collects the gamepad and script events of this frame.
    fn poll_devices(&mut self, clock: f64) {
        self.input.poll_source(&mut self.gamepads);
        if let Some(script) = self.script.as_mut() {
            self.events.clear();
            script.poll_frame(self.frame, clock, &mut self.events);
            for e in self.events.drain(..) {
                self.input.push(e);
            }
        }
    }

    /// Locks the cursor while a `MouseDelta` binding is in use (by default: right mouse held).
    fn set_captured(&mut self, on: bool) {
        if on == self.captured {
            return;
        }
        self.captured = on;
        let Some(gfx) = self.gfx.as_ref() else {
            return;
        };
        gfx.window.set_cursor_visible(!on);
        // A platform without cursor grabbing still orbits; the cursor just stays visible there.
        let _ = if on {
            gfx.window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| gfx.window.set_cursor_grab(CursorGrabMode::Confined))
        } else {
            gfx.window.set_cursor_grab(CursorGrabMode::None)
        };
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        let clock = self.input_clock();
        self.poll_devices(clock);
        if let Ok(world) = self.game.world().try_borrow() {
            self.input.sync(&world);
        }
        let mut frame_input = self.input.frame(clock, dt as f32);
        let want_capture = self.input.wants_mouse_delta();
        self.set_captured(want_capture);
        if self.args.autopilot {
            autopilot(&mut frame_input, self.game.time(), dt);
        }
        self.game.frame(&frame_input, dt);
        self.frame += 1;

        let Some(gfx) = self.gfx.as_mut() else {
            return;
        };
        if gfx.minimised {
            return;
        }
        let size = (gfx.config.width, gfx.config.height);
        let scale = gfx.window.scale_factor() as f32;
        if let Some(path) = capture_path(&self.args, self.frame) {
            let target = CaptureTarget::new(&gfx.ctx.device, gfx.config.format, size);
            let camera = match self.args.capture_view {
                Some([x, y, z, lx, ly, lz]) => Camera {
                    position: Vec3::new(x, y, z),
                    look_at: Vec3::new(lx, ly, lz),
                    ..self.game.camera()
                },
                None => self.game.camera(),
            };
            self.game
                .render_from(&mut gfx.renderer, &target.view, size, &camera);
            gfx.overlay.draw(
                &gfx.ctx.device,
                &gfx.ctx.queue,
                &target.view,
                size,
                scale,
                &self.game,
            );
            match target.save_png(&gfx.ctx.device, &gfx.ctx.queue, &path) {
                Ok(()) => println!(
                    "metallic-player: saved frame {} to {}",
                    self.frame,
                    path.display()
                ),
                Err(e) => eprintln!("warning: could not save {}: {e}", path.display()),
            }
        }
        match gfx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                self.game.render(&mut gfx.renderer, &view, size);
                gfx.overlay.draw(
                    &gfx.ctx.device,
                    &gfx.ctx.queue,
                    &view,
                    size,
                    scale,
                    &self.game,
                );
                gfx.window.pre_present_notify();
                gfx.ctx.queue.present(frame);
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                gfx.surface.configure(&gfx.ctx.device, &gfx.config);
            }
            _ => {}
        }

        if self.args.fps {
            let started = *self.started.get_or_insert(now);
            self.frame_times.push(dt);
            if now.duration_since(started) >= Duration::from_secs_f64(FPS_SECONDS) {
                print_fps(&self.frame_times[1..], size);
                event_loop.exit();
            }
        }
        if self.args.frames.is_some_and(|n| self.frame >= n) {
            event_loop.exit();
        }
    }
}

/// Replaces the player's input: run forward while turning, and jump every
/// [`AUTOPILOT_JUMP_EVERY`] seconds of game time.
fn autopilot(input: &mut FrameInput, time: f64, dt: f64) {
    input.movement = [0.0, 1.0];
    input.run = true;
    input.orbit[0] += AUTOPILOT_TURN * dt as f32;
    input.jump = time % AUTOPILOT_JUMP_EVERY < 0.1;
}

fn capture_path(args: &Args, frame: u64) -> Option<PathBuf> {
    args.captures
        .iter()
        .find(|(f, _)| *f == frame)
        .map(|(_, p)| p.clone())
}

/// Prints average fps, frame-time percentiles and the slowest frame.
fn print_fps(times: &[f64], size: (u32, u32)) {
    if times.is_empty() {
        println!("metallic-player fps: no frames");
        return;
    }
    let total: f64 = times.iter().sum();
    let mut sorted = times.to_vec();
    sorted.sort_by(f64::total_cmp);
    let pct = |p: f64| sorted[((sorted.len() - 1) as f64 * p).round() as usize] * 1000.0;
    println!(
        "metallic-player fps: {}x{}, {} frames in {:.1} s, average {:.1} fps, frame ms p50 {:.2} p99 {:.2} max {:.2}",
        size.0,
        size.1,
        times.len(),
        total,
        times.len() as f64 / total,
        pct(0.5),
        pct(0.99),
        pct(1.0)
    );
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        match self.create_gfx(event_loop) {
            Ok(gfx) => self.gfx = Some(gfx),
            Err(e) => {
                eprintln!("error: cannot start graphics: {e}");
                self.failed = true;
                event_loop.exit();
            }
        }
        self.last = Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gfx) = self.gfx.as_mut() {
                    gfx.minimised = size.width == 0 || size.height == 0;
                    if !gfx.minimised {
                        gfx.config.width = size.width;
                        gfx.config.height = size.height;
                        gfx.surface.configure(&gfx.ctx.device, &gfx.config);
                    }
                }
            }
            WindowEvent::Focused(false) => {
                let now = self.input_clock();
                self.input.release_all(now);
                self.set_captured(false);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let down = event.state == ElementState::Pressed;
                    // Escape quits for now; it becomes the `Menu` action with the Player menu.
                    if code == KeyCode::Escape && down {
                        event_loop.exit();
                    } else if let Some(control) = key_control(code) {
                        let time = self.input_clock();
                        let value = f32::from(u8::from(down));
                        self.input.push(RawEvent::button(
                            Device::KeyboardMouse,
                            control,
                            value,
                            time,
                        ));
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(control) = mouse_control(button) {
                    let time = self.input_clock();
                    let value = f32::from(u8::from(state == ElementState::Pressed));
                    self.input.push(RawEvent::button(
                        Device::KeyboardMouse,
                        control,
                        value,
                        time,
                    ));
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let time = self.input_clock();
                let pos = [position.x as f32, position.y as f32];
                self.input.push(RawEvent::vector(
                    Device::KeyboardMouse,
                    Control::MousePosition,
                    pos,
                    time,
                ));
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let notches = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 60.0) as f32,
                };
                let time = self.input_clock();
                self.input
                    .push(RawEvent::wheel(Device::KeyboardMouse, notches, time));
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event
            && self.captured
        {
            let pos = [delta.0 as f32, delta.1 as f32];
            let time = self.input_clock();
            self.input.push(RawEvent::vector(
                Device::KeyboardMouse,
                Control::MouseDelta,
                pos,
                time,
            ));
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(gfx) = self.gfx.as_ref() {
            gfx.window.request_redraw();
        }
    }
}
