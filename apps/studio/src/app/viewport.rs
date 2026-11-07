//! The engine's side of the viewport (ADR 0009, Viewport): a [`SceneRenderer`] drawing the open game into a texture of
//! the Viewport panel's size on the shell's device, registered with egui-wgpu for the panel to show.
//!
//! The shell calls [`ViewportRenderer::frame`] each frame between running the interface (which sizes the panel and
//! moves the camera) and painting it, so the frame painted shows the game as the camera sees it now.

use metallic_engine::metallic_render::settings::GraphicsSettings;
use metallic_engine::metallic_render::{RenderContext, wgpu};
use metallic_engine::{FrameStats, SceneRenderer};

use super::Studio;

/// How the viewport's texture is sampled. It is drawn one texel to a pixel, also mid-resize (#311), so this rarely shows.
const FILTER: wgpu::FilterMode = wgpu::FilterMode::Linear;

/// The engine's renderer for the viewport, and the texture egui-wgpu shows it with. One for Studio's lifetime: a
/// project opened later is drawn by the same renderer, its pipelines compiled once.
#[derive(Debug, Default)]
pub struct ViewportRenderer {
    scene: SceneRenderer,
    /// Whether the renderer draws into a viewport target yet.
    attached: bool,
    /// Why it could not attach; it does not try again.
    error: Option<String>,
    /// The texture registered with egui-wgpu, and the target's generation it shows.
    texture: Option<(egui::TextureId, u64)>,
    /// The game drawn last ([`metallic_engine::metallic_core::DataModel::uid`]): another is a project opened.
    game: Option<u64>,
    /// The Rendering settings last applied.
    graphics: Option<GraphicsSettings>,
}

impl ViewportRenderer {
    /// A renderer not drawing yet: it attaches when a viewport is first shown.
    pub fn new() -> Self {
        Self::default()
    }

    /// The engine's renderer.
    pub fn scene(&self) -> &SceneRenderer {
        &self.scene
    }

    /// Draws the open game into the viewport's texture if the panel was shown this frame: attaches on first use
    /// (adopting the renderer the splash made, if there is one), applies the Rendering settings when they changed,
    /// sizes the target to the panel, draws from the editor camera, and (re)registers the texture with `painter`.
    /// Returns whether another frame is wanted soon even without input: a resize or assets still on their way.
    pub fn frame(&mut self, context: &RenderContext, painter: &mut egui_wgpu::Renderer, studio: &mut Studio) -> bool {
        let shown = studio.editor().is_some_and(|editor| editor.viewport.shown);
        if !shown {
            return false;
        }
        // Before attaching, so a renderer attached takes them at once.
        let graphics = &studio.settings().rendering;
        if self.graphics.as_ref() != Some(graphics) {
            self.scene.set_graphics(graphics.clone());
            self.scene.set_frame_cap(graphics.frame_cap.filter(|cap| *cap > 0));
            self.graphics = Some(graphics.clone());
        }
        if !self.attached && self.error.is_none() {
            self.attach(context, studio);
        }
        let Some(editor) = studio.editor_mut() else {
            return false;
        };
        let viewport = &mut editor.viewport;
        viewport.shown = false;
        if let Some(error) = &self.error {
            viewport.error = Some(error.clone());
            return false;
        }
        if self.game != Some(editor.dm.uid()) {
            self.game = Some(editor.dm.uid());
            if let Some(assets) = editor.mesh_assets.take() {
                self.scene.set_mesh_assets(assets);
            }
        }
        let wanted = viewport.pixels;
        if let Some((width, height)) = wanted {
            self.scene.resize_viewport(width, height);
        }
        self.scene.set_camera(Some(viewport.camera.camera()));
        self.scene.render_viewport(&mut editor.dm, FrameStats::default());

        let Some(target) = self.scene.viewport() else {
            return false;
        };
        let generation = target.generation();
        let device = context.device();
        match self.texture {
            Some((id, shown)) if shown == generation => viewport.texture = Some(id),
            Some((id, _)) => {
                let _turn = context.descriptor_heap_turn();
                painter.update_egui_texture_from_wgpu_texture(device, target.view(), FILTER, id);
                self.texture = Some((id, generation));
                viewport.texture = Some(id);
            }
            None => {
                let _turn = context.descriptor_heap_turn();
                let id = painter.register_native_texture(device, target.view(), FILTER);
                self.texture = Some((id, generation));
                viewport.texture = Some(id);
            }
        }
        viewport.texture_pixels = Some(target.size());
        let resizing = wanted.is_some_and(|size| size != target.size());
        let loading = self.scene.assets().is_some_and(|assets| assets.pending() > 0);
        let compiling = self
            .scene
            .renderer()
            .is_some_and(|renderer| renderer.pipeline_stats().pending > 0);
        resizing || loading || compiling
    }

    /// Starts drawing into a viewport target on `context`'s device: with the splash's renderer when there is one
    /// (its first frame's pipelines already compiled), else a new one at the panel's size.
    fn attach(&mut self, context: &RenderContext, studio: &mut Studio) {
        let ahead = studio.take_viewport_renderer();
        let size = studio
            .editor()
            .and_then(|editor| editor.viewport.pixels)
            .unwrap_or((1, 1));
        let adopted = ahead.map(|renderer| self.scene.attach_viewport_with(context, renderer));
        let attached = match adopted {
            Some(Ok(())) => Ok(()),
            Some(Err(error)) => {
                log::warn!("render: the splash's renderer could not draw the viewport ({error}); making another");
                self.scene.attach_viewport(context, size.0, size.1)
            }
            None => self.scene.attach_viewport(context, size.0, size.1),
        };
        match attached {
            Ok(()) => self.attached = true,
            Err(error) => {
                log::error!("render: the viewport could not start: {error}");
                self.error = Some(format!("The viewport could not start: {error}"));
            }
        }
    }
}
