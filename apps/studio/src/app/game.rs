//! The shell's side of the Game tab (ADR 0010 §1): the session's Player draws its game into a texture on the shell's
//! render device, registered with egui-wgpu for the Game panel to show, as the Viewport's is (`app::viewport`).
//!
//! The game's frames run in the interface's pass (`EditorState::tick`); this only draws the frame just run, and
//! only while the Game tab is shown.

use metallic_engine::metallic_render::{RenderContext, wgpu};

use super::Studio;

/// How the game's texture is sampled. Its corner is drawn one texel to a pixel (#330), so this rarely shows.
const FILTER: wgpu::FilterMode = wgpu::FilterMode::Linear;

/// The texture the Game tab shows: registered once per session with egui-wgpu.
#[derive(Debug, Default)]
pub struct GameRenderer {
    /// The texture registered with egui-wgpu, the session it shows (its number), and the target's generation.
    texture: Option<(egui::TextureId, u64, u64)>,
}

impl GameRenderer {
    /// A renderer with nothing registered.
    pub fn new() -> Self {
        Self::default()
    }

    /// Draws the session's game into the Game tab's texture if the tab was shown this frame: the Player attaches to
    /// `context`'s device at the tab's size on first use, resizes with the tab, and draws; the texture is
    /// (re)registered with `painter`. A texture of an ended session is freed. Returns whether another frame is wanted
    /// soon even without input (a resize under way).
    pub fn frame(&mut self, context: &RenderContext, painter: &mut egui_wgpu::Renderer, studio: &mut Studio) -> bool {
        let session = studio.editor_mut().and_then(|editor| editor.play.as_mut());
        let Some(session) = session else {
            if let Some((id, ..)) = self.texture.take() {
                painter.free_texture(&id);
            }
            return false;
        };
        if let Some((id, number, _)) = self.texture
            && number != session.number
        {
            painter.free_texture(&id);
            self.texture = None;
        }
        let view = &mut session.view;
        if !std::mem::take(&mut view.shown) {
            return false;
        }
        let Some((width, height)) = view.pixels else {
            return false;
        };
        let player = &mut session.player;
        if player.renderer().borrow().viewport().is_none() {
            if view.error.is_some() {
                return false;
            }
            if let Err(error) = player.attach_viewport(context, width, height) {
                log::error!("play: the game could not be drawn: {error}");
                view.error = Some(format!("The game could not be drawn: {error}"));
                return false;
            }
        }
        player.resize_viewport(width, height);
        player.render_viewport();

        let renderer = player.renderer().borrow();
        let Some(target) = renderer.viewport() else {
            return false;
        };
        let generation = target.generation();
        let device = context.device();
        match self.texture {
            Some((id, _, shown)) if shown == generation => view.texture = Some(id),
            Some((id, ..)) => {
                let _turn = context.descriptor_heap_turn();
                painter.update_egui_texture_from_wgpu_texture(device, target.view(), FILTER, id);
                self.texture = Some((id, session.number, generation));
                view.texture = Some(id);
            }
            None => {
                let _turn = context.descriptor_heap_turn();
                let id = painter.register_native_texture(device, target.view(), FILTER);
                self.texture = Some((id, session.number, generation));
                view.texture = Some(id);
            }
        }
        view.texture_pixels = Some(target.size());
        view.texture_limit = context.capabilities().max_texture_size;
        // The frame fills the tab's size in the target's corner (#330); it differs only past the device's limit.
        renderer.viewport_drawn_size() != Some((width, height))
    }
}
