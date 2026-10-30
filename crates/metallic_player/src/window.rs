//! The windowed player: winit window, wgpu surface, input, frame loop and statistics.
use crate::args::Args;
use crate::capture::CaptureTarget;
use crate::overlay::Overlay;
use metallic_render::{RenderContext, RenderSettings, Renderer};
use metallic_runtime::{Action, Game, InputState};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{
    DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

/// How long `--fps` measures.
const FPS_SECONDS: f64 = 60.0;

/// Plays `game` in a window until it is closed (or `--frames` / `--fps` ends it).
pub fn run(game: Game, args: &Args) -> ExitCode {
    let event_loop = match EventLoop::new() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: cannot open a window: {e}");
            return ExitCode::from(1);
        }
    };
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        game,
        args: args.clone(),
        gfx: None,
        input: InputState::new(),
        gamepads: gilrs::Gilrs::new().ok(),
        dragging: false,
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
    input: InputState,
    gamepads: Option<gilrs::Gilrs>,
    dragging: bool,
    last: Instant,
    frame: u64,
    frame_times: Vec<f64>,
    started: Option<Instant>,
    failed: bool,
}

fn action_for(key: KeyCode) -> Option<Action> {
    Some(match key {
        KeyCode::KeyW | KeyCode::ArrowUp => Action::Forward,
        KeyCode::KeyS | KeyCode::ArrowDown => Action::Back,
        KeyCode::KeyA | KeyCode::ArrowLeft => Action::Left,
        KeyCode::KeyD | KeyCode::ArrowRight => Action::Right,
        KeyCode::Space => Action::Jump,
        KeyCode::ShiftLeft | KeyCode::ShiftRight => Action::Run,
        _ => return None,
    })
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

    fn poll_gamepad(&mut self) {
        let Some(gilrs) = self.gamepads.as_mut() else {
            return;
        };
        while gilrs.next_event().is_some() {}
        let pad = gilrs
            .gamepads()
            .find(|(_, g)| g.is_connected())
            .map(|(_, g)| {
                let axis = |a| g.value(a);
                (
                    [axis(gilrs::Axis::LeftStickX), axis(gilrs::Axis::LeftStickY)],
                    [
                        axis(gilrs::Axis::RightStickX),
                        axis(gilrs::Axis::RightStickY),
                    ],
                    g.is_pressed(gilrs::Button::South),
                )
            });
        let (left, right, jump) = pad.unwrap_or(([0.0; 2], [0.0; 2], false));
        self.input.gamepad(left, right, jump);
    }

    fn set_dragging(&mut self, on: bool) {
        self.dragging = on;
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
        self.poll_gamepad();
        let frame_input = self.input.take_frame(dt as f32);
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
            self.game.render(&mut gfx.renderer, &target.view, size);
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
                self.input.release_all();
                self.set_dragging(false);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if code == KeyCode::Escape && event.state == ElementState::Pressed {
                        event_loop.exit();
                    } else if let Some(action) = action_for(code) {
                        self.input.set(action, event.state == ElementState::Pressed);
                    }
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Right,
                ..
            } => self.set_dragging(state == ElementState::Pressed),
            WindowEvent::MouseWheel { delta, .. } => {
                let notches = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 60.0) as f32,
                };
                self.input.wheel(notches);
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event
            && self.dragging
        {
            self.input.mouse_orbit(delta.0 as f32, delta.1 as f32);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(gfx) = self.gfx.as_ref() {
            gfx.window.request_redraw();
        }
    }
}
