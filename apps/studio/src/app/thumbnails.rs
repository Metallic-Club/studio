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

/// The thumbnails' renderer and the textures it made.
#[derive(Default)]
pub struct Thumbnails {
    scene: SceneRenderer,
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

/// A game holding one ball, 2 m across, at the origin.
fn ball_game() -> Option<(DataModel, Id)> {
    let mut dm = metallic_engine::metallic_classes::new_game().ok()?;
    let workspace = dm.service(dm.registry().class_id("Workspace")?)?;
    let ball = dm.create(dm.registry().class_id("Part")?).ok()?;
    dm.set_by_name(ball, "Shape", Value::Enum(EnumItem::new("PartShape", "Ball")))
        .ok()?;
    dm.set_by_name(ball, "Size", Value::Vector3(Vec3::splat(2.0))).ok()?;
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
        let Some(wanted) = self.again.take().or_else(|| editor.pictures.next()) else {
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
        let rotation = Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.35);
        self.scene.set_camera(Some(Camera {
            position: rotation * Vec3::new(0.0, 0.0, 4.0),
            rotation,
            fov_y: 35f32.to_radians(),
            near: Camera::DEFAULT_NEAR,
        }));
        self.scene.render_viewport(dm, FrameStats::default());
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
