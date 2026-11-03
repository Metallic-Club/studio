//! [`SceneRenderer`]: `metallic_render` as the engine's [`RenderHook`]. A thin adapter: the renderer crate cannot
//! depend on the engine (ADR 0001), so the hook lives here and only forwards.

use std::sync::Arc;

use metallic_core::glam::Vec3;
use metallic_core::{DataModel, Id, PropId};
use metallic_platform::{Size, Window};
use metallic_render::scene::{RenderComponents, SyncStats, UploadStats};
use metallic_render::{
    ContextOptions, GpuScene, RenderContext, RenderError, RenderSurface, Renderer, RendererSettings, wgpu,
};

use crate::hooks::{RenderFrame, RenderHook, RenderHookError};

/// The device and the frame loop, kept while the window comes and goes.
struct Device {
    context: RenderContext,
    renderer: Renderer,
}

/// What the renderer reads from `Workspace`, looked up once per data model.
#[derive(Clone, Copy, Debug)]
struct WorldIds {
    workspace: Id,
    current_camera: Option<PropId>,
    /// The camera last seen and its `CFrame` property.
    camera: Option<(Id, PropId)>,
}

/// Draws the game into the engine's window: install it with [`Engine::set_render_hook`](crate::Engine::set_render_hook).
///
/// Each frame it syncs the GPU-resident scene ([`GpuScene`]) with the data model's changes and uploads them in the
/// frame's submission. Parts are not drawn yet (#77): the frame is the clear colour, with the scene kept current
/// for the passes that will draw it. The device and the scene survive [`RenderHook::detach`], so a mobile app
/// coming back to the foreground only makes a new surface.
pub struct SceneRenderer {
    options: ContextOptions,
    clear_color: wgpu::Color,
    device: Option<Device>,
    surface: Option<RenderSurface>,
    scene: Option<GpuScene>,
    world: Option<WorldIds>,
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

    /// What the last frame's sync and upload did.
    pub fn last_frame(&self) -> (SyncStats, UploadStats) {
        (self.last_sync, self.last_upload)
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
        let workspace = registry.class_id("Workspace").and_then(|class| dm.service(class));
        self.world = workspace.map(|workspace| WorldIds {
            workspace,
            current_camera: dm.prop_id(workspace, "CurrentCamera").ok(),
            camera: None,
        });
        // A scene of another data model is dropped as it is: that game, and its consumer, are gone.
        self.scene = Some(GpuScene::new(
            &device.context,
            dm,
            workspace,
            &RenderComponents::builtin(),
        )?);
        Ok(())
    }

    /// The current camera's position, re-resolving its `CFrame` property when the camera changed.
    fn camera_position(&mut self, dm: &DataModel) -> Option<Vec3> {
        let world = self.world.as_mut()?;
        let camera = dm.reference(world.workspace, world.current_camera?).ok()??;
        let prop = match world.camera {
            Some((id, prop)) if id == camera => prop,
            _ => {
                let prop = dm.prop_id(camera, "CFrame").ok()?;
                world.camera = Some((camera, prop));
                prop
            }
        };
        dm.cframe(camera, prop).ok().map(|cframe| cframe.position())
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
        let camera = self.camera_position(frame.dm);
        let (Some(device), Some(scene)) = (self.device.as_mut(), self.scene.as_mut()) else {
            return;
        };
        if let Some(camera) = camera {
            scene.set_camera_position(frame.dm, camera);
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
        let mut upload = UploadStats::default();
        match image {
            Some(image) => {
                device
                    .renderer
                    .render_with(image.view(), |encoder| upload = scene.upload(encoder));
                image.present(&device.context);
            }
            None => {
                // No image this frame (minimised, occluded): the upload still goes out, or it would be lost.
                let mut encoder = device
                    .context
                    .device()
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("metallic scene upload"),
                    });
                upload = scene.upload(&mut encoder);
                device.context.queue().submit(std::iter::once(encoder.finish()));
            }
        }
        self.last_upload = upload;
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
