//! [`SceneRenderer`]: `metallic_render` as the engine's [`RenderHook`]. A thin adapter: the renderer crate cannot
//! depend on the engine (ADR 0001), so the hook lives here and only forwards.

use std::sync::Arc;

use metallic_core::{DataModel, Id};
use metallic_platform::{Size, Window};
use metallic_render::scene::{RenderComponents, SyncStats, UploadStats};
use metallic_render::{
    CameraReader, ContextOptions, DrawStats, GpuScene, RenderContext, RenderError, RenderSurface, Renderer,
    RendererSettings, wgpu,
};

use crate::hooks::{RenderFrame, RenderHook, RenderHookError};

/// The device and the frame loop, kept while the window comes and goes.
struct Device {
    context: RenderContext,
    renderer: Renderer,
}

/// Draws the game into the engine's window: install it with [`Engine::set_render_hook`](crate::Engine::set_render_hook).
///
/// Each frame it syncs the GPU-resident scene ([`GpuScene`]) with the data model's changes and draws the parts in
/// `Workspace` from `Workspace.CurrentCamera` ([`Renderer::render_scene`]); without a current camera the frame is
/// the clear colour. The device and the scene survive [`RenderHook::detach`], so a mobile app coming back to the
/// foreground only makes a new surface.
pub struct SceneRenderer {
    options: ContextOptions,
    clear_color: wgpu::Color,
    device: Option<Device>,
    surface: Option<RenderSurface>,
    scene: Option<GpuScene>,
    /// `Workspace`, the root of what is drawn and the owner of `CurrentCamera`.
    world: Option<Id>,
    camera: CameraReader,
    last_sync: SyncStats,
    last_upload: UploadStats,
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
            device: None,
            surface: None,
            scene: None,
            world: None,
            camera: CameraReader::new(),
            last_sync: SyncStats::default(),
            last_upload: UploadStats::default(),
        }
    }

    /// Changes the colour frames start from (linear).
    pub fn set_clear_color(&mut self, color: wgpu::Color) {
        self.clear_color = color;
        if let Some(device) = self.device.as_mut() {
            device.renderer.set_clear_color(color);
        }
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
        // A scene of another data model is dropped as it is: that game, and its consumer, are gone.
        self.scene = Some(GpuScene::new(
            &device.context,
            dm,
            self.world,
            &RenderComponents::builtin(),
        )?);
        Ok(())
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
            .finish_non_exhaustive()
    }
}

impl RenderHook for SceneRenderer {
    fn attach(&mut self, window: &Window) -> Result<(), RenderHookError> {
        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));
        let window = Arc::new(window.clone());
        let surface = match self.device.as_ref() {
            Some(device) => device.context.create_surface(window, width, height)?,
            None => {
                let (context, surface) = RenderContext::new_with_window(window, width, height, &self.options)?;
                let renderer = Renderer::new(&context, self.settings(surface.format(), width, height))?;
                self.device = Some(Device { context, renderer });
                surface
            }
        };
        let settings = self.settings(surface.format(), width, height);
        if let Some(device) = self.device.as_mut()
            && device.renderer.settings().color_format != surface.format()
        {
            // A new surface on another monitor or after a resume may prefer another format.
            device.renderer = Renderer::new(&device.context, settings)?;
        }
        self.surface = Some(surface);
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
        let image = match self.surface.as_mut().map(|surface| surface.acquire(&device.context)) {
            Some(Ok(image)) => image,
            Some(Err(error)) => {
                log::error!("render: {error}");
                None
            }
            None => None,
        };
        match image {
            Some(image) => {
                device.renderer.render_scene(image.view(), scene, camera.as_ref());
                image.present(&device.context);
                self.last_upload = device.renderer.upload_stats();
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
            }
        }
    }
}

impl SceneRenderer {
    fn settings(&self, format: wgpu::TextureFormat, width: u32, height: u32) -> RendererSettings {
        RendererSettings {
            clear_color: self.clear_color,
            ..RendererSettings::new(format, width, height)
        }
    }
}
