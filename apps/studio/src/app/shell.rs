//! The shell: Studio's own winit loop, its one window, and egui drawn by `egui-wgpu` on the engine's
//! [`RenderContext`] (ADR 0009, Toolkit).
//!
//! The window opens as the splash (720 × 420, borderless, centred, hidden until its first frame is drawn), and on
//! [`WindowRequest::BecomeMainWindow`] the same window becomes the main one.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use egui::{ColorImage, ViewportId};
use egui_wgpu::{RendererOptions, ScreenDescriptor};
use metallic_engine::graphics::pipeline_cache_dir;
use metallic_engine::metallic_classes::input::feed::MouseBehavior;
use metallic_engine::metallic_platform::AppId;
use metallic_engine::metallic_project::PROJECT_EXTENSION;
use metallic_engine::metallic_render::{ContextOptions, RenderContext, RenderSurface, wgpu};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::monitor::MonitorHandle;
use winit::window::{CursorGrabMode, Icon, Window, WindowAttributes, WindowId};

use super::game::GameRenderer;
use super::startup::{Startup, studio_steps};
use super::viewport::ViewportRenderer;
use super::{Studio, WindowRequest, native, splash};
use crate::projects::FileDialog;
use crate::theme::tokens::radius;
use crate::theme::{fonts, rounded_outline};
use crate::{STUDIO_APP_ID, StudioError, emblem};

/// The main window's size, in points.
const MAIN_SIZE: LogicalSize<f64> = LogicalSize::new(1280.0, 800.0);
/// The smallest the main window may be made, in points.
const MAIN_MIN_SIZE: LogicalSize<f64> = LogicalSize::new(960.0, 600.0);
/// How soon the viewport is drawn again while something it shows is still coming in, without input.
const VIEWPORT_POLL: std::time::Duration = std::time::Duration::from_millis(16);

/// Opens Studio's window and runs it until it is closed. Call it on the main thread. `open` is a project to open
/// straight into the editor once the splash is done (the command line's).
pub fn run(open: Option<PathBuf>) -> Result<(), StudioError> {
    let event_loop = EventLoop::new()?;
    let mut shell = Shell {
        open,
        ..Shell::default()
    };
    event_loop.run_app(&mut shell)?;
    match shell.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// The event loop's handler: the window and its GPU side once the loop has resumed.
#[derive(Default)]
struct Shell {
    window: Option<StudioWindow>,
    error: Option<StudioError>,
    /// The project to open after the splash, until the window takes it.
    open: Option<PathBuf>,
}

/// Studio's window, the device it is drawn with, and the interface in it.
struct StudioWindow {
    window: Arc<Window>,
    context: RenderContext,
    surface: RenderSurface,
    egui: egui_winit::State,
    painter: egui_wgpu::Renderer,
    studio: Studio,
    /// The engine drawing the game into the Viewport panel's texture.
    viewport: ViewportRenderer,
    /// The running game drawn into the Game tab's texture (Play in Studio).
    game: GameRenderer,
    /// The Rendering settings' vsync as last applied to the window.
    vsync: Option<bool>,
    /// When egui asked to be drawn again, if not at once.
    repaint_at: Option<Instant>,
    /// What the game asked of the cursor, as last applied to the window (ADR 0010 §4).
    cursor: GameCursor,
    /// The title last given to the window.
    title: String,
}

impl ApplicationHandler for Shell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        match StudioWindow::open(event_loop, self.open.take()) {
            Ok(window) => self.window = Some(window),
            Err(error) => {
                self.error = Some(error);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.as_mut() else {
            return;
        };
        if window.egui.on_window_event(&window.window, &event).repaint {
            window.window.request_redraw();
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                window.surface.resize(&window.context, size.width, size.height);
                window.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = window.frame() {
                    self.error = Some(error);
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device: DeviceId, event: DeviceEvent) {
        // Raw mouse motion, which keeps coming while the cursor is held: what a game's camera reads (ADR 0010 §4).
        if let (Some(window), DeviceEvent::MouseMotion { delta }) = (self.window.as_mut(), event)
            && window.cursor.held.is_some()
            && window.egui.on_mouse_motion(delta)
        {
            window.window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.as_mut() else {
            return;
        };
        match window.repaint_at {
            Some(at) if at <= Instant::now() => {
                window.repaint_at = None;
                window.window.request_redraw();
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Some(at) => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}

impl StudioWindow {
    /// Opens the window as the splash, makes the render device on it, starts the startup steps, and shows the
    /// splash's first frame. `project` opens once the splash is done.
    fn open(event_loop: &ActiveEventLoop, project: Option<PathBuf>) -> Result<Self, StudioError> {
        let monitor = event_loop
            .primary_monitor()
            .or_else(|| event_loop.available_monitors().next());
        let window = Arc::new(event_loop.create_window(splash_attributes(monitor.as_ref()))?);

        let size = window.inner_size();
        let options = ContextOptions {
            pipeline_cache_dir: AppId::parse(STUDIO_APP_ID)
                .and_then(|app| pipeline_cache_dir(&app))
                .ok(),
            ..ContextOptions::default()
        };
        let (context, surface) =
            RenderContext::new_with_window(Arc::clone(&window), size.width, size.height, &options)?;
        log::info!(
            "render: {} ({:?})",
            context.adapter().get_info().name,
            context.adapter().get_info().backend
        );
        shape_as_splash(&window);

        let ctx = egui::Context::default();
        fonts::install(&ctx);
        let max_texture_side = context.capabilities().max_texture_size as usize;
        let egui = egui_winit::State::new(
            ctx,
            ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(max_texture_side),
        );
        let painter = egui_wgpu::Renderer::new(context.device(), surface.format(), RendererOptions::default());

        let reduce_motion = native::prefers_reduced_motion();
        let startup = Startup::spawn(studio_steps(context.clone()))?;
        // The mark at its size on the splash, in the window's pixels, so it is drawn one texel to a pixel.
        let mark = emblem::render((splash::EMBLEM * window.scale_factor() as f32).round() as u32);
        let image = ColorImage::from_rgba_unmultiplied([mark.width as usize, mark.height as usize], &mark.rgba);
        let mut studio = Studio::new(startup, Some(image), reduce_motion);
        if let Some(project) = project {
            studio.open_after_splash(project);
        }
        studio.set_file_dialog(Box::new(NativeDialog {
            window: Arc::clone(&window),
        }));

        let mut this = Self {
            window,
            context,
            surface,
            egui,
            painter,
            studio,
            viewport: ViewportRenderer::new(),
            game: GameRenderer::new(),
            vsync: None,
            repaint_at: None,
            cursor: GameCursor::default(),
            title: super::APP_TITLE.to_owned(),
        };
        // Shown once there is a frame to show, so it never flashes empty.
        this.frame()?;
        this.window.set_visible(true);
        Ok(this)
    }

    /// Runs the interface for one frame and draws it.
    fn frame(&mut self) -> Result<(), StudioError> {
        let input = self.egui.take_egui_input(&self.window);
        let ctx = self.egui.egui_ctx().clone();
        let studio = &mut self.studio;
        let now = Instant::now();
        let mut output = ctx.run_ui(input, |ui| studio.ui(ui, now));
        self.egui
            .handle_platform_output(&self.window, std::mem::take(&mut output.platform_output));
        // Every delta is applied below; taken out, they are drained as they are.
        let mut textures = std::mem::take(&mut output.textures_delta);
        // The game into the viewport's texture, from the camera this frame's input moved, before the UI shows it.
        let viewport_busy = self.viewport.frame(&self.context, &mut self.painter, &mut self.studio)
            | self.game.frame(&self.context, &mut self.painter, &mut self.studio);
        self.apply_vsync();

        let jobs = ctx.tessellate(output.shapes, output.pixels_per_point);
        let (width, height) = self.surface.size();
        let screen = ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point: output.pixels_per_point,
        };
        let device = self.context.device();
        let queue = self.context.queue();
        {
            // The viewport's renderer may be making textures on the startup thread at the same time.
            let _turn = self.context.descriptor_heap_turn();
            for (id, deltas) in textures.set.drain() {
                for delta in &deltas {
                    self.painter.update_texture(device, queue, id, delta);
                }
            }
        }
        if let Some(frame) = self.surface.acquire(&self.context)? {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("studio ui"),
            });
            let mut buffers = {
                let _turn = self.context.descriptor_heap_turn();
                self.painter.update_buffers(device, queue, &mut encoder, &jobs, &screen)
            };
            let clear = egui::Rgba::from(self.studio.clear_color());
            {
                let mut pass = encoder
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("studio ui"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: frame.view(),
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: f64::from(clear.r()),
                                    g: f64::from(clear.g()),
                                    b: f64::from(clear.b()),
                                    a: f64::from(clear.a()),
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    })
                    .forget_lifetime();
                self.painter.render(&mut pass, &jobs, &screen);
            }
            buffers.push(encoder.finish());
            self.context.submit(buffers);
            self.window.pre_present_notify();
            frame.present(&self.context);
        }
        for id in textures.free.drain() {
            self.painter.free_texture(&id);
        }

        self.apply_game_cursor();
        let title = self.studio.window_title();
        if title != self.title {
            self.window.set_title(&title);
            self.title = title;
        }
        for request in self.studio.take_requests() {
            match request {
                WindowRequest::BecomeMainWindow => self.become_main_window(),
            }
        }
        let mut delay = output
            .viewport_output
            .get(&ViewportId::ROOT)
            .map_or(std::time::Duration::MAX, |viewport| viewport.repaint_delay);
        if viewport_busy {
            // A resize, assets or pipelines still coming in: drawn as they land, even without input.
            delay = delay.min(VIEWPORT_POLL);
        }
        if delay.is_zero() {
            self.window.request_redraw();
            self.repaint_at = None;
        } else {
            self.repaint_at = now.checked_add(delay);
        }
        Ok(())
    }

    /// Presents with vsync or without, as the Rendering settings say (with it when they leave it to Studio).
    fn apply_vsync(&mut self) {
        let vsync = self.studio.settings().rendering.vsync;
        if vsync == self.vsync {
            return;
        }
        self.vsync = vsync;
        let mode = if vsync.unwrap_or(true) {
            wgpu::PresentMode::Fifo
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        self.surface.set_present_mode(&self.context, mode);
    }

    /// Turns the splash into Studio's main window: decorated, resizable, [`MAIN_SIZE`], centred where the splash was
    /// and kept on its monitor.
    fn become_main_window(&self) {
        let window = &self.window;
        let centre = window.outer_position().ok().map(|at| {
            let size = window.outer_size();
            (
                f64::from(at.x) + f64::from(size.width) / 2.0,
                f64::from(at.y) + f64::from(size.height) / 2.0,
            )
        });
        native::set_window_shape(window, None);
        window.set_transparent(false);
        window.set_decorations(true);
        window.set_resizable(true);
        window.set_min_inner_size(Some(MAIN_MIN_SIZE));
        let _ = window.request_inner_size(MAIN_SIZE);
        if let Some(centre) = centre {
            let size: PhysicalSize<f64> = MAIN_SIZE.to_physical(window.scale_factor());
            let mut at = PhysicalPosition::new(centre.0 - size.width / 2.0, centre.1 - size.height / 2.0);
            // A window larger than the monitor keeps its title bar reachable.
            if let Some(monitor) = window.current_monitor() {
                at.x = at.x.max(f64::from(monitor.position().x));
                at.y = at.y.max(f64::from(monitor.position().y));
            }
            window.set_outer_position(PhysicalPosition::new(at.x.round() as i32, at.y.round() as i32));
        }
        window.focus_window();
        window.request_redraw();
    }
}

/// The cursor as the running game asked for it (`MouseBehavior`, `MouseIconEnabled`), applied to Studio's window: the
/// Player has no window of its own (ADR 0010 §4).
#[derive(Debug, Default)]
struct GameCursor {
    /// Where the cursor is held, in the window's physical pixels, while the game holds it.
    held: Option<PhysicalPosition<f64>>,
    /// Whether the cursor is hidden.
    hidden: bool,
}

impl StudioWindow {
    /// Holds, frees, shows or hides the cursor as the game asks while it has the input; frees and shows it otherwise.
    /// `LockCenter` holds it at the Game tab's centre, `LockCurrentPosition` where it was. The window is asked to lock
    /// the cursor, or else to confine it, and the cursor is put back on the point every frame, so a click never lands
    /// outside the tab.
    fn apply_game_cursor(&mut self) {
        let wanted = self.studio.game_cursor();
        let scale = self.window.scale_factor();
        let physical = |at: egui::Pos2| PhysicalPosition::new(f64::from(at.x) * scale, f64::from(at.y) * scale);
        let held = wanted.and_then(|(state, tab)| match state.behavior {
            MouseBehavior::Default => None,
            MouseBehavior::LockCenter => Some(physical(tab.center())),
            MouseBehavior::LockCurrentPosition => Some(self.cursor.held.unwrap_or_else(|| {
                let at = self.egui.egui_ctx().input(|input| input.pointer.latest_pos());
                physical(at.filter(|at| tab.contains(*at)).unwrap_or(tab.center()))
            })),
        });
        if held.is_some() != self.cursor.held.is_some() {
            let grabbed = if held.is_some() {
                self.window
                    .set_cursor_grab(CursorGrabMode::Locked)
                    .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined))
            } else {
                self.window.set_cursor_grab(CursorGrabMode::None)
            };
            if let Err(error) = grabbed {
                log::warn!("play: the cursor could not be held for MouseBehavior: {error}");
            }
        }
        if let Some(at) = held
            && let Err(error) = self.window.set_cursor_position(at)
        {
            log::debug!("play: the cursor could not be put back: {error}");
        }
        self.cursor.held = held;
        let hidden = held.is_some() || wanted.is_some_and(|(state, _)| !state.icon_visible);
        if hidden != self.cursor.hidden {
            self.window.set_cursor_visible(!hidden);
            self.cursor.hidden = hidden;
        }
    }
}

/// The system's file dialogs (`rfd`), in front of Studio's window. Each blocks until the user answers, as a modal
/// dialog does; the window is redrawn when it closes.
struct NativeDialog {
    window: Arc<Window>,
}

impl FileDialog for NativeDialog {
    fn pick_project(&mut self) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title("Add a project")
            .add_filter("Metallic project", &[PROJECT_EXTENSION])
            .set_parent(&*self.window)
            .pick_file()
    }

    fn pick_folder(&mut self, start: Option<&Path>) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Choose a folder for the project")
            .set_parent(&*self.window);
        if let Some(start) = start {
            dialog = dialog.set_directory(start);
        }
        dialog.pick_folder()
    }
}

/// The splash's window: [`splash::SIZE`], borderless, fixed, see-through around its shape, centred on `monitor`,
/// hidden until drawn, with the Metallic mark as its icon.
fn splash_attributes(monitor: Option<&MonitorHandle>) -> WindowAttributes {
    let size = LogicalSize::new(f64::from(splash::SIZE.x), f64::from(splash::SIZE.y));
    let mark = emblem::render(256);
    let icon = Icon::from_rgba(mark.rgba, mark.width, mark.height)
        .map_err(|error| log::warn!("window: the emblem is not a usable icon: {error}"))
        .ok();
    let mut attributes = Window::default_attributes()
        .with_title(super::APP_TITLE)
        .with_inner_size(size)
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_visible(false)
        .with_window_icon(icon);
    if let Some(monitor) = monitor {
        let pixels: PhysicalSize<f64> = size.to_physical(monitor.scale_factor());
        let area = monitor.size();
        let origin = monitor.position();
        attributes = attributes.with_position(PhysicalPosition::new(
            f64::from(origin.x) + (f64::from(area.width) - pixels.width) / 2.0,
            f64::from(origin.y) + (f64::from(area.height) - pixels.height) / 2.0,
        ));
    }
    attributes
}

/// Clips the window to the splash's outline (its rounded corners), in its physical pixels.
fn shape_as_splash(window: &Window) {
    let size = window.inner_size();
    let scale = window.scale_factor() as f32;
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.width as f32, size.height as f32));
    let outline = rounded_outline(rect, f32::from(radius::PANEL) * scale, 8);
    native::set_window_shape(window, Some(&outline));
}
