//! The Assets panel's rendered thumbnails (#681): a renderer of its own draws the thumbnails the tiles asked for
//! ([`crate::editor::pictures`]), one a frame: a material on a ball, lit as a new game is; a scene from its own
//! camera. Each is drawn into the renderer's target and copied into a texture of its own, which egui shows. A
//! thumbnail drawn while its assets are still loading is drawn again once they are.

use metallic_engine::metallic_core::glam::{Quat, Vec3};
use metallic_engine::metallic_core::{Color3, DataModel, EnumItem, Id, Value};
use metallic_engine::metallic_project::{AssetId, load_scene_bytes};
use metallic_engine::metallic_render::{Camera, RenderContext, ViewportTarget, wgpu};
use metallic_engine::{FrameStats, SceneRenderer};

use crate::editor::EditorState;
use crate::editor::pictures::{Picture, Wanted};

/// A thumbnail's side, in pixels.
pub const SIDE: u32 = 128;

/// The ball a material is drawn on: its radius, in units.
const BALL_RADIUS: f32 = 1.0;
/// How far the camera is from the ball's centre.
const BALL_DISTANCE: f32 = 4.0;
/// The camera's vertical field of view, in degrees.
const BALL_FOV: f32 = 35.0;

/// How far a material's ball reaches from its picture's centre, as a share of the picture's side (#697). The ball is
/// in the middle of the view, so its outline is a circle, which the Assets browser cuts the picture to.
pub fn ball_radius() -> f32 {
    let angular = (BALL_RADIUS / BALL_DISTANCE).asin();
    angular.tan() / (BALL_FOV.to_radians() / 2.0).tan() / 2.0
}

/// The camera a material's ball is drawn from: from above and to the side, its outline [`ball_radius`] of the picture.
/// The inspector's preview (#742) is drawn from it too, so it is cut to the ball the same way.
pub fn ball_camera() -> Camera {
    let rotation = Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.35);
    Camera {
        position: rotation * Vec3::new(0.0, 0.0, BALL_DISTANCE),
        rotation,
        fov_y: BALL_FOV.to_radians(),
        near: Camera::DEFAULT_NEAR,
    }
}

/// How far a drag of one point turns the preview's ball, in radians.
const SPIN_PER_POINT: f32 = 0.012;

/// `turn` (the ball's rotation) turned further by a drag of `drag` points across the preview (#805): as if the ball
/// were spun by the pointer, its front following the drag, about the axis across the drag as the camera sees it.
pub fn spin(turn: Quat, drag: egui::Vec2) -> Quat {
    let camera = ball_camera().rotation;
    let up = camera * Vec3::Y;
    let right = camera * Vec3::X;
    // A drag right turns the front right, about the camera's up; a drag down (screen y grows) turns it down, about
    // its right.
    let across =
        Quat::from_axis_angle(up, drag.x * SPIN_PER_POINT) * Quat::from_axis_angle(right, drag.y * SPIN_PER_POINT);
    (across * turn).normalize()
}

/// A game holding a MeshPart of the mesh `id`, whose bounds are `size`: at its own proportions, as large as the ball
/// ([`BALL_RADIUS`] round its centre), at the origin.
fn mesh_game(id: AssetId, size: [f32; 3]) -> Option<DataModel> {
    let mut dm = metallic_engine::metallic_classes::new_game().ok()?;
    let workspace = dm.service(dm.registry().class_id("Workspace")?)?;
    let part = dm.create(dm.registry().class_id("MeshPart")?).ok()?;
    dm.set_by_name(part, "MeshId", Value::String(id.to_string().into()))
        .ok()?;
    // Both sides (#850): a flat model facing away from the camera still shows.
    dm.set_by_name(part, "DoubleSided", Value::Bool(true)).ok()?;
    dm.set_by_name(part, "Size", Value::Vector3(fitted(size))).ok()?;
    dm.set_parent(part, Some(workspace)).ok()?;
    Some(dm)
}

/// A mesh's `size` scaled so its bounds' corners are [`BALL_RADIUS`] from its centre, as the ball's outline is.
fn fitted(size: [f32; 3]) -> Vec3 {
    let size = Vec3::from(size).max(Vec3::splat(1e-3));
    size * (2.0 * BALL_RADIUS / size.length())
}

/// The thumbnails' renderer and the textures it made.
#[derive(Default)]
pub struct Thumbnails {
    /// Boxed, as the viewport's are: it lives in Studio's window, which must fit the main thread's stack.
    scene: Box<SceneRenderer>,
    attached: bool,
    failed: bool,
    /// The project whose assets the renderer reads.
    project: Option<std::path::PathBuf>,
    /// The ball a material is drawn on.
    ball: Option<(DataModel, Id)>,
    /// The texture of each thumbnail, kept for its asset.
    textures: Vec<(AssetId, wgpu::Texture)>,
    /// A thumbnail drawn while its assets still loaded, to draw again.
    again: Option<Wanted>,
}

impl std::fmt::Debug for Thumbnails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Thumbnails")
            .field("textures", &self.textures.len())
            .finish_non_exhaustive()
    }
}

/// A game holding one ball, [`BALL_RADIUS`] in radius, at the origin.
fn ball_game() -> Option<(DataModel, Id)> {
    let mut dm = metallic_engine::metallic_classes::new_game().ok()?;
    let workspace = dm.service(dm.registry().class_id("Workspace")?)?;
    let ball = dm.create(dm.registry().class_id("Part")?).ok()?;
    dm.set_by_name(ball, "Shape", Value::Enum(EnumItem::new("PartShape", "Ball")))
        .ok()?;
    dm.set_by_name(ball, "Size", Value::Vector3(Vec3::splat(2.0 * BALL_RADIUS)))
        .ok()?;
    dm.set_by_name(ball, "Color", Value::Color3(Color3::WHITE)).ok()?;
    dm.set_parent(ball, Some(workspace)).ok()?;
    Some((dm, ball))
}

impl Thumbnails {
    /// Draws the next thumbnail asked for, if any. `changed` are the materials edited since the last frame, which
    /// are read again.
    pub fn frame(
        &mut self,
        context: &RenderContext,
        painter: &mut egui_wgpu::Renderer,
        editor: &mut EditorState,
        changed: &[AssetId],
    ) {
        if self.failed {
            return;
        }
        if self.project.as_deref() != Some(editor.project.path()) {
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
        let Some(wanted) = self.again.take().or_else(|| editor.pictures.take_next()) else {
            return;
        };
        if !self.attached {
            match self.scene.attach_viewport(context, SIDE, SIDE) {
                Ok(()) => self.attached = true,
                Err(error) => {
                    log::warn!("render: the thumbnails could not start: {error}");
                    self.failed = true;
                    return;
                }
            }
        }
        // A scene's own assets may be named in its file only: a newer view of the project reads them.
        if wanted.picture == Picture::Scene
            && let Some(assets) = self.scene.assets_mut()
        {
            assets.read_from(editor.project.asset_reader());
        }
        let drawn = match wanted.picture {
            Picture::Material => self.draw_material(wanted.id),
            Picture::Scene => self.draw_scene(editor, &wanted.path),
            Picture::Mesh => self.draw_mesh(editor, wanted.id),
        };
        if !drawn {
            return;
        }
        let pending = self.scene.assets().is_some_and(|assets| assets.pending() > 0);
        let Some(texture) = self.copy(context, painter, editor, wanted.id) else {
            return;
        };
        editor.pictures.done(wanted.id, wanted.fingerprint, texture);
        if pending {
            self.again = Some(wanted);
        }
    }

    fn draw_material(&mut self, id: AssetId) -> bool {
        if self.ball.is_none() {
            self.ball = ball_game();
        }
        let Some((dm, ball)) = self.ball.as_mut() else {
            self.failed = true;
            return false;
        };
        let _ = dm.set_by_name(*ball, "Material", Value::String(id.to_string().into()));
        self.scene.set_camera(Some(ball_camera()));
        self.scene.render_viewport(dm, FrameStats::default());
        true
    }

    /// A mesh's model (#806): a MeshPart of it at its own proportions, as large as the ball, seen from the ball's
    /// camera.
    fn draw_mesh(&mut self, editor: &mut EditorState, id: AssetId) -> bool {
        let Ok(mesh) = editor.project.asset_reader().load_mesh(&id.to_string()) else {
            return false;
        };
        let Some(mut dm) = mesh_game(id, mesh.size) else {
            return false;
        };
        self.scene.set_camera(Some(ball_camera()));
        self.scene.render_viewport(&mut dm, FrameStats::default());
        true
    }

    fn draw_scene(&mut self, editor: &mut EditorState, path: &str) -> bool {
        let Ok(bytes) = editor.project.read(path) else {
            return false;
        };
        let Ok(mut dm) = metallic_engine::metallic_classes::new_game() else {
            return false;
        };
        if load_scene_bytes(&mut dm, &bytes).is_err() {
            return false;
        }
        // From the scene's own camera.
        self.scene.set_camera(None);
        self.scene.render_viewport(&mut dm, FrameStats::default());
        true
    }

    /// Copies the renderer's target into `id`'s texture (made the first time) and returns egui's id for it.
    fn copy(
        &mut self,
        context: &RenderContext,
        painter: &mut egui_wgpu::Renderer,
        editor: &EditorState,
        id: AssetId,
    ) -> Option<egui::TextureId> {
        let target = self.scene.viewport()?;
        let (width, height) = target.size();
        let device = context.device();
        let made = self.textures.iter().position(|(asset, _)| *asset == id);
        let index = match made {
            Some(index) => index,
            None => {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("metallic asset thumbnail"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: ViewportTarget::FORMAT,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                self.textures.push((id, texture));
                self.textures.len() - 1
            }
        };
        let texture = &self.textures[index].1;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("metallic asset thumbnail"),
        });
        encoder.copy_texture_to_texture(
            target.texture().as_image_copy(),
            texture.as_image_copy(),
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        context.queue().submit([encoder.finish()]);
        if let (Some(_), Some(known)) = (made, editor.pictures.texture_of(id)) {
            return Some(known);
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let _turn = context.descriptor_heap_turn();
        Some(painter.register_native_texture(device, &view, wgpu::FilterMode::Linear))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_spins_the_ball_s_front_with_it() {
        let camera = ball_camera();
        // The ball's point facing the camera, in the camera's own axes after a turn.
        let front = |turn: Quat| {
            let towards_camera = camera.rotation * Vec3::Z;
            camera.rotation.inverse() * (turn * towards_camera)
        };
        // Down and left on the screen: the front goes left (x below 0) and down (y below 0).
        let turned = front(spin(Quat::IDENTITY, egui::vec2(-30.0, 30.0)));
        assert!(turned.x < -0.1 && turned.y < -0.1, "{turned:?}");
        // Up and right: the other way.
        let turned = front(spin(Quat::IDENTITY, egui::vec2(30.0, -30.0)));
        assert!(turned.x > 0.1 && turned.y > 0.1, "{turned:?}");
    }
}
