//! The shell: Studio's own winit loop, its window, and egui drawn by `egui-wgpu` on the engine's [`RenderContext`]
//! (ADR 0009, Toolkit).
//!
//! The window opens as the splash (720 × 420, borderless, centred, hidden until its first frame is drawn), and on
//! [`WindowRequest::BecomeMainWindow`] the same window becomes the main one.
//!
//! **Windows of their own** (#331): Studio code can show a native window beside the main one with egui's immediate
//! viewports (`Context::show_viewport_immediate`), as Settings does. The shell opens a window for each viewport shown,
//! draws it with the same device and UI renderer (so the icons, fonts and rendered views serve every window) while
//! the main window draws, and closes it once it is no longer shown. Events in such a window redraw the main one,
//! which draws it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{ColorImage, ImmediateViewport, ViewportBuilder, ViewportId, ViewportInfo};
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

/// What the main window shares with the windows of their own (#331): the UI renderer, whose textures serve every
/// window, and those windows. Drawing a window of its own happens inside the main window's interface pass, so this is
/// borrowed only between passes, never across one.
struct Shared {
    context: RenderContext,
    painter: egui_wgpu::Renderer,
    windows: Windows,
}

/// Studio's windows of their own: egui's immediate viewports, each a native window.
#[derive(Default)]
struct Windows {
    open: HashMap<ViewportId, OwnWindow>,
    by_window: HashMap<WindowId, ViewportId>,
    /// Viewports shown that have no window yet, and how to make one: opened once the frame is drawn.
    wanted: Vec<(ViewportId, ViewportBuilder)>,
    /// The soonest one of them asked to be drawn again.
    repaint: Option<Duration>,
}

/// One window of its own.
struct OwnWindow {
    window: Arc<Window>,
    surface: RenderSurface,
    egui: egui_winit::State,
    info: ViewportInfo,
    /// Whether it was shown in the frame being drawn: one that was not is closed after it.
    shown: bool,
    /// Whether it is on the screen yet: it opens hidden and shows once its first frame is drawn (#409), so it never
    /// flashes empty.
    revealed: bool,
}

/// Studio's window, the device it is drawn with, and the interface in it.
struct StudioWindow {
    window: Arc<Window>,
    context: RenderContext,
    surface: RenderSurface,
    egui: egui_winit::State,
    /// The UI renderer and the windows of their own (#331).
    shared: Rc<RefCell<Shared>>,
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
    /// The window's place as it was last not maximised nor minimised (#505): what Studio keeps when it closes.
    normal_place: Option<crate::settings::WindowPlace>,
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

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.as_mut() else {
            return;
        };
        if id != window.window.id() {
            window.own_window_event(id, &event);
            return;
        }
        if window.egui.on_window_event(&window.window, &event).repaint {
            window.window.request_redraw();
        }
        match event {
            WindowEvent::CloseRequested => {
                window.keep_place();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                window.surface.resize(&window.context, size.width, size.height);
                window.note_place();
                window.window.request_redraw();
            }
            WindowEvent::Moved(_) => window.note_place(),
            WindowEvent::RedrawRequested => {
                if let Err(error) = window.frame(event_loop) {
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
        let shared = Rc::new(RefCell::new(Shared {
            context: context.clone(),
            painter,
            windows: Windows::default(),
        }));
        // Windows of their own (#331): egui draws each immediate viewport through this, inside the main window's pass.
        let weak = Rc::downgrade(&shared);
        egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| draw_own_window(&weak, ctx, viewport));
        egui.egui_ctx().set_embed_viewports(false);

        let reduce_motion = native::prefers_reduced_motion();
        let startup = Startup::spawn(studio_steps(context.clone()))?;
        // The mark at its size on the splash, in the window's pixels, so it is drawn one texel to a pixel.
        let mark = emblem::render((splash::EMBLEM * window.scale_factor() as f32).round() as u32);
        let image = ColorImage::from_rgba_unmultiplied([mark.width as usize, mark.height as usize], &mark.rgba);
        let mut studio = Studio::new(startup, Some(image), reduce_motion);
        // A small file, read again properly by the startup steps: only so the splash shows the user's accent (#319).
        if let Some(path) = super::startup::studio_settings_path() {
            studio.set_splash_accent(crate::settings::SettingsStore::peek(&path).appearance.accent);
        }
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
            shared,
            studio,
            viewport: ViewportRenderer::new(),
            game: GameRenderer::new(),
            vsync: None,
            repaint_at: None,
            cursor: GameCursor::default(),
            title: super::APP_TITLE.to_owned(),
            normal_place: None,
        };
        // Shown once there is a frame to show, so it never flashes empty.
        this.frame(event_loop)?;
        this.window.set_visible(true);
        Ok(this)
    }

    /// Runs the interface for one frame and draws it, with the windows of their own shown in it; then closes those
    /// no longer shown and opens those newly shown (#331).
    fn frame(&mut self, event_loop: &ActiveEventLoop) -> Result<(), StudioError> {
        let ctx = self.egui.egui_ctx().clone();
        let mut input = self.egui.take_egui_input(&self.window);
        // Where the main window is on the screen, so a window of its own opens over it (#331).
        let info = input.viewports.entry(ViewportId::ROOT).or_default();
        egui_winit::update_viewport_info(info, &ctx, &self.window, false);
        let studio = &mut self.studio;
        let now = Instant::now();
        let mut output = ctx.run_ui(input, |ui| studio.ui(ui, now));
        self.egui
            .handle_platform_output(&self.window, std::mem::take(&mut output.platform_output));
        let mut shared = self.shared.borrow_mut();
        let Shared {
            context,
            painter,
            windows,
        } = &mut *shared;
        // The game into the viewport's texture, from the camera this frame's input moved, before the UI shows it.
        let viewport_busy = self.viewport.frame(context, painter, &mut self.studio)
            | self.game.frame(context, painter, &mut self.studio);
        let clear = self.studio.clear_color();
        paint(
            context,
            painter,
            &mut self.surface,
            &self.window,
            &ctx,
            Painted {
                textures: std::mem::take(&mut output.textures_delta),
                shapes: std::mem::take(&mut output.shapes),
                pixels_per_point: output.pixels_per_point,
                clear,
            },
        )?;
        let own_repaint = windows.repaint.take();
        windows.close_unshown();
        let opened = windows.open_wanted(context, &ctx, event_loop);
        drop(shared);
        self.apply_vsync();

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
            .map_or(Duration::MAX, |viewport| viewport.repaint_delay);
        if viewport_busy {
            // A resize, assets or pipelines still coming in: drawn as they land, even without input.
            delay = delay.min(VIEWPORT_POLL);
        }
        // A window of its own is drawn when this one is: as soon as it asks, and at once when one just opened.
        if let Some(own) = own_repaint {
            delay = delay.min(own);
        }
        if opened {
            delay = Duration::ZERO;
        }
        if delay.is_zero() {
            self.window.request_redraw();
            self.repaint_at = None;
        } else {
            self.repaint_at = now.checked_add(delay);
        }
        Ok(())
    }

    /// An event in a window of its own (#331): to its egui input, a close asked for to its viewport (the code showing
    /// it decides), a resize to its surface; and the main window redraws, which draws it.
    fn own_window_event(&mut self, id: WindowId, event: &WindowEvent) {
        let mut shared = self.shared.borrow_mut();
        let Shared { context, windows, .. } = &mut *shared;
        let Some(own) = windows
            .by_window
            .get(&id)
            .copied()
            .and_then(|viewport| windows.open.get_mut(&viewport))
        else {
            return;
        };
        let repaint = own.egui.on_window_event(&own.window, event).repaint;
        match event {
            WindowEvent::CloseRequested => own.info.events.push(egui::ViewportEvent::Close),
            WindowEvent::Resized(size) => own.surface.resize(context, size.width, size.height),
            _ => {}
        }
        let redraw = matches!(
            event,
            WindowEvent::CloseRequested
                | WindowEvent::Resized(_)
                | WindowEvent::RedrawRequested
                | WindowEvent::Focused(_)
        );
        if repaint || redraw {
            self.window.request_redraw();
        }
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

    /// Notes the window's place while it is neither maximised nor minimised, and is Studio's main window (not the
    /// splash): the place Studio keeps when it closes (#505).
    fn note_place(&mut self) {
        let window = &self.window;
        if !window.is_resizable() || window.is_maximized() || window.is_minimized() == Some(true) {
            return;
        }
        let (Ok(at), size) = (window.outer_position(), window.inner_size()) else {
            return;
        };
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.normal_place = Some(crate::settings::WindowPlace {
            x: at.x,
            y: at.y,
            width: size.width,
            height: size.height,
            maximized: false,
        });
    }

    /// Keeps the window's place in the settings as Studio closes (#505): where it last was, not maximised, and whether
    /// it is maximised now. Nothing before the splash became the main window.
    fn keep_place(&mut self) {
        self.note_place();
        if let Some(place) = self.normal_place {
            let maximized = self.window.is_maximized();
            self.studio
                .close_with_window_at(crate::settings::WindowPlace { maximized, ..place });
        }
    }

    /// Turns the splash into Studio's main window: decorated, resizable, where it was when Studio last closed (#505),
    /// or else [`MAIN_SIZE`] centred where the splash was and kept on its monitor.
    fn become_main_window(&mut self) {
        if let Some(place) = self.studio.settings().window
            && on_a_monitor(&place, &monitor_rects(&self.window))
        {
            let window = &self.window;
            native::set_window_shape(window, None);
            window.set_transparent(false);
            window.set_decorations(true);
            window.set_resizable(true);
            window.set_min_inner_size(Some(MAIN_MIN_SIZE));
            let _ = window.request_inner_size(PhysicalSize::new(place.width, place.height));
            window.set_outer_position(PhysicalPosition::new(place.x, place.y));
            self.normal_place = Some(crate::settings::WindowPlace {
                maximized: false,
                ..place
            });
            if place.maximized {
                window.set_maximized(true);
            }
            window.focus_window();
            window.request_redraw();
            return;
        }
        self.become_main_window_centred();
    }

    /// [`StudioWindow::become_main_window`] with no place kept, or one off every monitor: [`MAIN_SIZE`], centred
    /// where the splash was and kept on its monitor.
    fn become_main_window_centred(&self) {
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

impl Windows {
    /// Closes every window of its own not shown this frame, and marks the rest for the next.
    fn close_unshown(&mut self) {
        self.open.retain(|_, own| std::mem::take(&mut own.shown));
        let open = &self.open;
        self.by_window.retain(|_, viewport| open.contains_key(viewport));
    }

    /// Opens a window for each viewport shown without one. Returns whether any opened.
    fn open_wanted(&mut self, context: &RenderContext, ctx: &egui::Context, event_loop: &ActiveEventLoop) -> bool {
        let mut opened = false;
        for (viewport, builder) in std::mem::take(&mut self.wanted) {
            if self.open.contains_key(&viewport) {
                continue;
            }
            match OwnWindow::open(context, ctx, event_loop, viewport, &builder) {
                Ok(own) => {
                    self.by_window.insert(own.window.id(), viewport);
                    self.open.insert(viewport, own);
                    opened = true;
                }
                Err(error) => log::error!("window: could not open {:?}: {error}", builder.title),
            }
        }
        opened
    }
}

impl OwnWindow {
    /// A native window for `viewport` as `builder` describes it, with its surface on `context`'s device and its egui
    /// input.
    fn open(
        context: &RenderContext,
        ctx: &egui::Context,
        event_loop: &ActiveEventLoop,
        viewport: ViewportId,
        builder: &ViewportBuilder,
    ) -> Result<Self, StudioError> {
        // Hidden until its first frame is drawn (#409).
        let window = Arc::new(egui_winit::create_window(
            ctx,
            event_loop,
            &builder.clone().with_visible(false),
        )?);
        let size = window.inner_size();
        let surface = context.create_surface(Arc::clone(&window), size.width.max(1), size.height.max(1))?;
        let egui = egui_winit::State::new(
            ctx.clone(),
            viewport,
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(context.capabilities().max_texture_size as usize),
        );
        let mut info = ViewportInfo::default();
        egui_winit::update_viewport_info(&mut info, ctx, &window, true);
        Ok(Self {
            window,
            surface,
            egui,
            info,
            shown: true,
            revealed: false,
        })
    }
}

/// Draws an immediate viewport (#331) in its window: egui calls this from inside the main window's pass whenever Studio
/// shows one. A viewport without a window yet runs its contents once, unseen (egui needs them run), and its window
/// opens after the frame.
fn draw_own_window(shared: &std::rc::Weak<RefCell<Shared>>, ctx: &egui::Context, viewport: ImmediateViewport<'_>) {
    let ImmediateViewport {
        ids,
        builder,
        mut viewport_ui_cb,
    } = viewport;
    // The main window's scale and largest texture, for a pass without a window of its own: with egui's defaults it
    // would lay text out with other text options, rebuilding the font atlas every window shares, and Studio's text
    // garbled for a frame (#409, #440).
    let (scale, side) = ctx.input(|input| (input.viewport().native_pixels_per_point, input.max_texture_side));
    let Some(shared) = shared.upgrade() else {
        // Studio is closing: nothing to draw into, nor textures to keep.
        let mut output = ctx.run_ui(unseen_input(ids.this, scale, side), |ui| viewport_ui_cb(ui));
        output.textures_delta.clear();
        return;
    };
    let input = {
        let mut shared = shared.borrow_mut();
        let windows = &mut shared.windows;
        match windows.open.get_mut(&ids.this) {
            Some(own) => {
                own.shown = true;
                egui_winit::update_viewport_info(&mut own.info, ctx, &own.window, false);
                let mut input = own.egui.take_egui_input(&own.window);
                input.viewports.insert(ids.this, own.info.clone());
                Some(input)
            }
            None => {
                if !windows.wanted.iter().any(|(viewport, _)| *viewport == ids.this) {
                    windows.wanted.push((ids.this, builder));
                }
                None
            }
        }
    };
    let Some(input) = input else {
        let output = ctx.run_ui(unseen_input(ids.this, scale, side), |ui| viewport_ui_cb(ui));
        // The textures it made (a font's new glyphs) serve every window.
        let mut shared = shared.borrow_mut();
        let Shared { context, painter, .. } = &mut *shared;
        update_textures(context, painter, output.textures_delta);
        return;
    };
    // No borrow is held while the contents run: they may show another window, which comes back here.
    let mut output = ctx.run_ui(input, |ui| viewport_ui_cb(ui));
    let mut shared = shared.borrow_mut();
    let Shared {
        context,
        painter,
        windows,
    } = &mut *shared;
    let Some(own) = windows.open.get_mut(&ids.this) else {
        return;
    };
    own.info.events.clear();
    own.egui
        .handle_platform_output(&own.window, std::mem::take(&mut output.platform_output));
    if let Some(asked) = output.viewport_output.remove(&ids.this) {
        let mut actions = Vec::new();
        egui_winit::process_viewport_commands(ctx, &mut own.info, asked.commands, &own.window, &mut actions);
        windows.repaint = Some(
            windows
                .repaint
                .map_or(asked.repaint_delay, |delay| delay.min(asked.repaint_delay)),
        );
    }
    let Some(own) = windows.open.get_mut(&ids.this) else {
        return;
    };
    let clear = ctx.global_style().visuals.window_fill;
    let painted = Painted {
        textures: output.textures_delta,
        shapes: output.shapes,
        pixels_per_point: output.pixels_per_point,
        clear,
    };
    match paint(context, painter, &mut own.surface, &own.window, ctx, painted) {
        Ok(()) if !own.revealed => {
            own.window.set_visible(true);
            own.revealed = true;
        }
        Ok(()) => {}
        Err(error) => log::warn!("window: could not draw {:?}: {error}", own.info.title),
    }
}

/// The input of a viewport's unseen first run: no events, just its id and a description of it (egui needs one for the
/// viewport it runs) at `scale`, the main window's native pixels per point, with its largest texture side `side`. egui
/// keeps one font atlas for every window and makes it again when a pass's text options differ: these are the ones
/// every other pass has (#409, #440).
fn unseen_input(viewport: ViewportId, scale: Option<f32>, side: usize) -> egui::RawInput {
    let mut input = egui::RawInput {
        viewport_id: viewport,
        max_texture_side: Some(side),
        ..Default::default()
    };
    let info = ViewportInfo {
        native_pixels_per_point: scale,
        ..ViewportInfo::default()
    };
    input.viewports.insert(viewport, info);
    input
}

/// What a window's pass gave to draw.
struct Painted {
    textures: egui::TexturesDelta,
    shapes: Vec<egui::epaint::ClippedShape>,
    pixels_per_point: f32,
    clear: egui::Color32,
}

/// Applies the textures a pass made or changed to the UI renderer.
fn update_textures(context: &RenderContext, painter: &mut egui_wgpu::Renderer, mut textures: egui::TexturesDelta) {
    // The viewport's renderer may be making textures on the startup thread at the same time.
    let _turn = context.descriptor_heap_turn();
    for (id, deltas) in textures.set.drain() {
        for delta in &deltas {
            painter.update_texture(context.device(), context.queue(), id, delta);
        }
    }
    for id in textures.free.drain() {
        painter.free_texture(&id);
    }
}

/// Draws a pass's shapes into `surface` (`window`'s) over `clear` and presents it; textures it freed are freed after.
fn paint(
    context: &RenderContext,
    painter: &mut egui_wgpu::Renderer,
    surface: &mut RenderSurface,
    window: &Window,
    ctx: &egui::Context,
    painted: Painted,
) -> Result<(), StudioError> {
    let Painted {
        mut textures,
        shapes,
        pixels_per_point,
        clear,
    } = painted;
    let free = std::mem::take(&mut textures.free);
    update_textures(context, painter, textures);
    let jobs = ctx.tessellate(shapes, pixels_per_point);
    let (width, height) = surface.size();
    let screen = ScreenDescriptor {
        size_in_pixels: [width, height],
        pixels_per_point,
    };
    let device = context.device();
    let queue = context.queue();
    if let Some(frame) = surface.acquire(context)? {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("studio ui"),
        });
        let mut buffers = {
            let _turn = context.descriptor_heap_turn();
            painter.update_buffers(device, queue, &mut encoder, &jobs, &screen)
        };
        let clear = egui::Rgba::from(clear);
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
            painter.render(&mut pass, &jobs, &screen);
        }
        buffers.push(encoder.finish());
        context.submit(buffers);
        window.pre_present_notify();
        frame.present(context);
    }
    for id in free {
        painter.free_texture(&id);
    }
    Ok(())
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
    /// outside the tab. The editor camera's look and orbit hold it too, where the drag began, but shown (#385).
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
        let game_holds = held.is_some();
        let held = held.or_else(|| self.studio.held_pointer().map(physical));
        if held.is_some() != self.cursor.held.is_some() {
            let grabbed = if held.is_some() {
                self.window
                    .set_cursor_grab(CursorGrabMode::Locked)
                    .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined))
            } else {
                self.window.set_cursor_grab(CursorGrabMode::None)
            };
            if let Err(error) = grabbed {
                log::warn!("the cursor could not be held: {error}");
            }
        }
        if let Some(at) = held
            && let Err(error) = self.window.set_cursor_position(at)
        {
            log::debug!("the cursor could not be put back: {error}");
        }
        self.cursor.held = held;
        let hidden = game_holds || wanted.is_some_and(|(state, _)| !state.icon_visible);
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

    fn pick_font(&mut self) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title("Choose a font for scripts")
            .add_filter("Font", &["ttf", "otf"])
            .set_parent(&*self.window)
            .pick_file()
    }
}

/// Every monitor's area, in physical pixels: (left, top, width, height).
fn monitor_rects(window: &Window) -> Vec<(i32, i32, u32, u32)> {
    window
        .available_monitors()
        .map(|monitor| {
            let (at, size) = (monitor.position(), monitor.size());
            (at.x, at.y, size.width, size.height)
        })
        .collect()
}

/// Whether a window at `place` can be reached: its title bar's middle (the top 32 pixels, inset 100 from each side)
/// is on one of `monitors` (#505). A place kept for a monitor since unplugged is not used.
fn on_a_monitor(place: &crate::settings::WindowPlace, monitors: &[(i32, i32, u32, u32)]) -> bool {
    let (left, right) = (
        i64::from(place.x) + 100,
        i64::from(place.x) + i64::from(place.width) - 100,
    );
    let (top, bottom) = (i64::from(place.y), i64::from(place.y) + 32);
    monitors.iter().any(|&(x, y, width, height)| {
        let (x, y) = (i64::from(x), i64::from(y));
        left.max(x) < right.min(x + i64::from(width)) && top.max(y) < bottom.min(y + i64::from(height))
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// #505: a kept place is used only while its title bar is on a monitor.
    #[test]
    fn a_kept_window_place_is_used_only_on_a_monitor() {
        let place = |x, y| crate::settings::WindowPlace {
            x,
            y,
            width: 1280,
            height: 800,
            maximized: false,
        };
        let two = [(0, 0, 1920, 1080), (-2560, -200, 2560, 1440)];
        assert!(on_a_monitor(&place(100, 100), &two));
        assert!(on_a_monitor(&place(-2000, 0), &two), "on the left monitor");
        assert!(!on_a_monitor(&place(4000, 100), &two), "past every monitor");
        assert!(!on_a_monitor(&place(100, 1090), &two), "its title bar below the screen");
        assert!(!on_a_monitor(&place(-2000, 0), &two[..1]), "that monitor is gone");
    }

    #[test]
    fn an_unseen_pass_runs_at_the_main_window_s_scale() {
        // At egui's default scale it would lay text out again at another size and rebuild the shared font atlas (#409).
        let id = ViewportId::from_hash_of("metallic.settings");
        let input = unseen_input(id, Some(1.25), 8192);
        assert_eq!(input.viewport_id, id);
        assert_eq!(input.viewports[&id].native_pixels_per_point, Some(1.25));
        assert_eq!(input.max_texture_side, Some(8192));
        assert!(input.events.is_empty());
    }

    #[test]
    fn an_unseen_pass_keeps_the_font_atlas_every_window_shares() {
        let ctx = egui::Context::default();
        let draw = |ui: &mut egui::Ui| {
            ui.label("Settings");
        };
        // A whole new font image: the atlas made again, which garbled Studio's text for a frame (#440).
        let remade = |mut output: egui::FullOutput| {
            let font = egui::TextureId::default();
            let whole = output
                .textures_delta
                .set
                .iter()
                .any(|(id, deltas)| *id == font && deltas.iter().any(|delta| delta.pos.is_none()));
            output.textures_delta.clear();
            whole
        };
        let main = egui::RawInput {
            max_texture_side: Some(8192),
            ..Default::default()
        };
        assert!(remade(ctx.run_ui(main, draw)), "the first pass makes it");
        let id = ViewportId::from_hash_of("metallic.settings");
        assert!(!remade(ctx.run_ui(unseen_input(id, None, 8192), draw)));
        // A window egui had not seen starts at its default largest texture (2048): it was made again (before #440).
        let mut before = unseen_input(ViewportId::from_hash_of("metallic.other"), None, 8192);
        before.max_texture_side = None;
        assert!(remade(ctx.run_ui(before, draw)), "the test would catch it");
    }
}
