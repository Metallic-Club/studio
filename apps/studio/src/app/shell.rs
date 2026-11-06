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
use metallic_engine::metallic_platform::AppId;
use metallic_engine::metallic_project::PROJECT_EXTENSION;
use metallic_engine::metallic_render::{ContextOptions, RenderContext, RenderSurface, wgpu};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::monitor::MonitorHandle;
use winit::window::{Icon, Window, WindowAttributes, WindowId};

use super::startup::{Startup, studio_steps};
use super::{Studio, WindowRequest, native, splash};
use crate::projects::FileDialog;
use crate::theme::tokens::radius;
use crate::theme::{fonts, rounded_outline};
use crate::{STUDIO_APP_ID, StudioError, emblem};

/// The main window's size, in points.
const MAIN_SIZE: LogicalSize<f64> = LogicalSize::new(1280.0, 800.0);
/// The smallest the main window may be made, in points.
const MAIN_MIN_SIZE: LogicalSize<f64> = LogicalSize::new(960.0, 600.0);

/// Opens Studio's window and runs it until it is closed. Call it on the main thread.
pub fn run() -> Result<(), StudioError> {
    let event_loop = EventLoop::new()?;
    let mut shell = Shell::default();
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
}

/// Studio's window, the device it is drawn with, and the interface in it.
struct StudioWindow {
    window: Arc<Window>,
    context: RenderContext,
    surface: RenderSurface,
    egui: egui_winit::State,
    painter: egui_wgpu::Renderer,
    studio: Studio,
    /// When egui asked to be drawn again, if not at once.
    repaint_at: Option<Instant>,
}

impl ApplicationHandler for Shell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        match StudioWindow::open(event_loop) {
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
    /// splash's first frame.
    fn open(event_loop: &ActiveEventLoop) -> Result<Self, StudioError> {
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
            repaint_at: None,
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

        for request in self.studio.take_requests() {
            match request {
                WindowRequest::BecomeMainWindow => self.become_main_window(),
            }
        }
        let delay = output
            .viewport_output
            .get(&ViewportId::ROOT)
            .map_or(std::time::Duration::MAX, |viewport| viewport.repaint_delay);
        if delay.is_zero() {
            self.window.request_redraw();
            self.repaint_at = None;
        } else {
            self.repaint_at = now.checked_add(delay);
        }
        Ok(())
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
        .with_title("Metallic Studio")
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
