//! The material editor's preview (#680): a renderer of its own drawing a ball with the material, lit as a new game
//! is, into a texture the Assets browser's inspector shows (#742). One for Studio's lifetime. It draws from the
//! thumbnails' camera ([`super::thumbnails::ball_camera`]), so the inspector cuts it to the ball alone, and only when
//! something changed: another material or size, an edit of the material, or assets that were still loading.

use metallic_engine::metallic_core::glam::{Quat, Vec3};
use metallic_engine::metallic_core::{DataModel, EnumItem, Id, Value};
use metallic_engine::metallic_project::AssetId;
use metallic_engine::metallic_render::{RenderContext, wgpu};
use metallic_engine::{FrameStats, SceneRenderer};

use crate::editor::EditorState;

/// The preview's renderer, its little game, and the texture egui shows.
#[derive(Default)]
pub struct MaterialPreview {
    /// Boxed, as the viewport's are: it lives in Studio's window, which must fit the main thread's stack.
    scene: Box<SceneRenderer>,
    /// The ball's game and the ball.
    game: Option<(DataModel, Id)>,
    attached: bool,
    /// The texture registered with egui-wgpu, and the target's generation it shows.
    texture: Option<(egui::TextureId, u64)>,
    /// The part of the texture the last drawing filled (#757).
    uv: Option<egui::Rect>,
    /// The project whose assets the renderer reads.
    project: Option<std::path::PathBuf>,
    /// What it drew last: the material and the size. Drawn again only when either changes, the material is edited,
    /// or `again` says its assets were still loading.
    drawn: Option<(AssetId, (u32, u32))>,
    /// How the ball was turned when it was last drawn (#805).
    drawn_turn: Quat,
    /// The last drawing was made while assets still loaded.
    again: bool,
    /// Why it could not start; it does not try again.
    failed: bool,
}

impl std::fmt::Debug for MaterialPreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaterialPreview")
            .field("attached", &self.attached)
            .field("drawn", &self.drawn)
            .finish_non_exhaustive()
    }
}

/// A game holding one ball, 2 m across, at the origin.
fn ball_game() -> Option<(DataModel, Id)> {
    let mut dm = metallic_engine::metallic_classes::new_game().ok()?;
    let workspace = dm.service(dm.registry().class_id("Workspace")?)?;
    let ball = dm.create(dm.registry().class_id("Part")?).ok()?;
    dm.set_by_name(ball, "Shape", Value::Enum(EnumItem::new("PartShape", "Ball")))
        .ok()?;
    dm.set_by_name(ball, "Size", Value::Vector3(Vec3::splat(2.0))).ok()?;
    dm.set_by_name(
        ball,
        "Color",
        Value::Color3(metallic_engine::metallic_core::Color3::WHITE),
    )
    .ok()?;
    dm.set_parent(ball, Some(workspace)).ok()?;
    Some((dm, ball))
}

impl MaterialPreview {
    /// Draws the material the inspector shows this frame, if it asked (`editor.materials.preview`), at the size it
    /// asked, when something changed since the last drawing, and tells it the texture. `changed` are the materials
    /// edited since the last frame.
    pub fn frame(
        &mut self,
        context: &RenderContext,
        painter: &mut egui_wgpu::Renderer,
        editor: &mut EditorState,
        changed: &[AssetId],
    ) {
        let Some((id, (width, height))) = editor.materials.preview.take() else {
            return;
        };
        if self.failed {
            return;
        }
        let new_project = self.project.as_deref() != Some(editor.project.path());
        if new_project {
            self.project = Some(editor.project.path().to_path_buf());
            self.scene.set_assets(editor.project.asset_reader());
        } else if !changed.is_empty()
            && let Some(assets) = self.scene.assets_mut()
        {
            assets.read_from(editor.project.asset_reader());
            for material in changed {
                assets.reload_material(&material.to_string());
            }
        }
        let wanted = (id, (width.max(1), height.max(1)));
        let turn = editor.materials.preview_turn;
        let stale =
            new_project || self.again || self.drawn != Some(wanted) || changed.contains(&id) || turn != self.drawn_turn;
        if !stale && let Some((texture, _)) = self.texture {
            editor.materials.preview_texture = Some((id, texture, self.uv.unwrap_or(crate::panels::assets::WHOLE)));
            return;
        }
        if self.game.is_none() {
            self.game = ball_game();
        }
        let Some((dm, ball)) = self.game.as_mut() else {
            self.failed = true;
            return;
        };
        let key = id.to_string();
        if !matches!(dm.get_by_name(*ball, "Material"), Ok(Value::String(ref now)) if **now == *key) {
            let _ = dm.set_by_name(*ball, "Material", Value::String(key.into()));
        }
        // Turned as it was dragged (#805).
        let _ = dm.set_by_name(
            *ball,
            "CFrame",
            Value::CFrame(metallic_engine::metallic_core::CFrame::from_parts(Vec3::ZERO, turn)),
        );
        self.drawn_turn = turn;
        if !self.attached {
            match self.scene.attach_viewport(context, width.max(1), height.max(1)) {
                Ok(()) => self.attached = true,
                Err(error) => {
                    log::warn!("render: the material preview could not start: {error}");
                    self.failed = true;
                    return;
                }
            }
        }
        self.scene.set_camera(Some(super::thumbnails::ball_camera()));
        self.scene.resize_viewport(width.max(1), height.max(1));
        self.scene.render_viewport(dm, FrameStats::default());
        self.drawn = Some(wanted);
        self.again = self.scene.assets().is_some_and(|assets| assets.pending() > 0);
        editor.materials.preview_loading = self.again;
        let Some(target) = self.scene.viewport() else {
            return;
        };
        // The target keeps room beyond the size drawn, which fills its top-left part (#757).
        self.uv = Some(drawn_part((width.max(1), height.max(1)), target.size()));
        let generation = target.generation();
        let device = context.device();
        let texture = match self.texture {
            Some((texture, shown)) if shown == generation => texture,
            Some((texture, _)) => {
                let _turn = context.descriptor_heap_turn();
                painter.update_egui_texture_from_wgpu_texture(device, target.view(), wgpu::FilterMode::Linear, texture);
                self.texture = Some((texture, generation));
                texture
            }
            None => {
                let _turn = context.descriptor_heap_turn();
                let texture = painter.register_native_texture(device, target.view(), wgpu::FilterMode::Linear);
                self.texture = Some((texture, generation));
                texture
            }
        };
        editor.materials.preview_texture = Some((id, texture, self.uv.unwrap_or(crate::panels::assets::WHOLE)));
    }
}

/// The part of a target `room` pixels across that a frame `drawn` pixels across fills, from its top-left corner, in
/// texture coordinates.
fn drawn_part(drawn: (u32, u32), room: (u32, u32)) -> egui::Rect {
    let share = |drawn: u32, room: u32| (drawn as f32 / room.max(1) as f32).min(1.0);
    egui::Rect::from_min_max(
        egui::pos2(0.0, 0.0),
        egui::pos2(share(drawn.0, room.0), share(drawn.1, room.1)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_drawn_part_is_the_target_s_top_left() {
        assert_eq!(
            drawn_part((180, 180), (240, 360)),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(0.75, 0.5))
        );
        assert_eq!(
            drawn_part((64, 64), (64, 64)),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
        );
    }
}
