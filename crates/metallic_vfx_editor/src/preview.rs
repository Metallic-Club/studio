//! Live preview: paints the CPU simulation with the egui painter.
//!
//! The simulation's [`DrawList`] (world-space quads, premultiplied colour, sorted back to front) is
//! projected with the preview camera and drawn as textured egui meshes. Premultiplied colours with alpha 0
//! add light, so additive and alpha-blended particles share egui's single blend state, exactly like the
//! future wgpu renderer. Soft particles and lighting are GPU-only and not shown here.

use crate::theme::color;
use egui::{Color32, Mesh, Pos2, Rect, Sense, Shape, Stroke, TextureHandle, TextureOptions, Vec2};
use metallic_vfx::draw::{Camera, DrawList};
use metallic_vfx::textures;
use metallic_vfx::{ArtStyle, Texture, Vec3};

/// Orbit camera and texture cache for the preview.
pub struct Preview {
    /// Horizontal orbit angle in radians.
    pub yaw: f32,
    /// Vertical orbit angle in radians.
    pub pitch: f32,
    /// Distance from the target in studs.
    pub distance: f32,
    /// Orbit target.
    pub target: Vec3,
    /// Draw the ground grid.
    pub grid: bool,
    pub(crate) textures: Vec<(Texture, TextureHandle)>,
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.16,
            distance: 15.0,
            target: Vec3::new(0.0, 2.2, 0.0),
            grid: true,
            textures: Vec::new(),
        }
    }
}

impl Preview {
    /// Camera for the current orbit; 2D effects get an orthographic side view.
    #[must_use]
    pub fn camera(&self, style: ArtStyle) -> Camera {
        if style == ArtStyle::Flat2D {
            return Camera::orthographic(
                self.target + Vec3::new(0.0, 0.0, 40.0),
                self.target,
                self.distance * 0.62,
            );
        }
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let eye = self.target + Vec3::new(sy * cp, sp, cy * cp) * self.distance;
        Camera::perspective(eye, self.target, 45.0)
    }

    fn texture(&mut self, ctx: &egui::Context, t: &Texture) -> egui::TextureId {
        if let Some((_, h)) = self.textures.iter().find(|(k, _)| k == t) {
            return h.id();
        }
        let img = textures::generate(t);
        let color = egui::ColorImage::from_rgba_unmultiplied(
            [img.width as usize, img.height as usize],
            &img.rgba,
        );
        let options = if img.nearest {
            TextureOptions::NEAREST
        } else {
            TextureOptions::LINEAR
        };
        let handle = ctx.load_texture(format!("vfx-{}", t.label()), color, options);
        let id = handle.id();
        self.textures.push((t.clone(), handle));
        id
    }

    /// Texture id for a built-in texture (used by pickers).
    pub fn texture_id(&mut self, ctx: &egui::Context, t: &Texture) -> egui::TextureId {
        self.texture(ctx, t)
    }

    /// Allocates `rect`, handles orbit/zoom input and paints the draw list.
    pub fn show(&mut self, ui: &mut egui::Ui, rect: Rect, list: &DrawList, style: ArtStyle) {
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        if response.dragged() && style != ArtStyle::Flat2D {
            let d = response.drag_delta();
            self.yaw -= d.x * 0.008;
            self.pitch = (self.pitch + d.y * 0.006).clamp(-0.2, 1.35);
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.distance = (self.distance * (1.0 - scroll * 0.0015)).clamp(3.0, 60.0);
            }
        }
        if response.double_clicked() {
            let grid = self.grid;
            *self = Self {
                textures: std::mem::take(&mut self.textures),
                grid,
                ..Self::default()
            };
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, color::BG_BASE);
        let camera = self.camera(style);
        if self.grid {
            draw_ground(&painter, rect, &camera, style);
        }
        let ctx = ui.ctx().clone();
        let tex_ids: Vec<egui::TextureId> = list
            .textures
            .iter()
            .map(|t| self.texture(&ctx, t))
            .collect();
        let (w, h) = (rect.width(), rect.height());
        let mut mesh = Mesh::default();
        let mut current: Option<egui::TextureId> = None;
        for q in &list.quads {
            let Some(&tex) = tex_ids.get(q.texture as usize) else {
                continue;
            };
            let mut pts = [Pos2::ZERO; 4];
            let mut ok = true;
            for (i, c) in q.corners.iter().enumerate() {
                match camera.project(*c, w, h) {
                    Some(p) => {
                        let (mut x, mut y) = (p[0], p[1]);
                        if q.pixel_snap > 0.0 {
                            x = (x / q.pixel_snap).round() * q.pixel_snap;
                            y = (y / q.pixel_snap).round() * q.pixel_snap;
                        }
                        pts[i] = Pos2::new(rect.min.x + x, rect.min.y + y);
                    }
                    None => ok = false,
                }
            }
            if !ok {
                continue;
            }
            if current != Some(tex) {
                if !mesh.is_empty() {
                    painter.add(Shape::mesh(std::mem::take(&mut mesh)));
                }
                mesh = Mesh::with_texture(tex);
                current = Some(tex);
            }
            let base = mesh.vertices.len() as u32;
            for i in 0..4 {
                let c = q.colors[i];
                let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: pts[i],
                    uv: Pos2::new(q.uvs[i][0], q.uvs[i][1]),
                    color: Color32::from_rgba_premultiplied(to8(c[0]), to8(c[1]), to8(c[2]), to8(c[3])),
                });
            }
            mesh.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        if !mesh.is_empty() {
            painter.add(Shape::mesh(mesh));
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
}

fn draw_ground(painter: &egui::Painter, rect: Rect, camera: &Camera, style: ArtStyle) {
    let (w, h) = (rect.width(), rect.height());
    let off = rect.min.to_vec2();
    if style == ArtStyle::Flat2D {
        // A ground line for side views.
        if let (Some(a), Some(b)) = (
            camera.project(Vec3::new(-200.0, 0.0, 0.0), w, h),
            camera.project(Vec3::new(200.0, 0.0, 0.0), w, h),
        ) {
            painter.line_segment(
                [Pos2::new(a[0], a[1]) + off, Pos2::new(b[0], b[1]) + off],
                Stroke::new(1.0, color::LINE_STRONG),
            );
        }
        return;
    }
    let extent = 12;
    for i in -extent..=extent {
        let f = i as f32;
        let major = i % 4 == 0;
        for (a, b) in [
            (Vec3::new(f, 0.0, -extent as f32), Vec3::new(f, 0.0, extent as f32)),
            (Vec3::new(-extent as f32, 0.0, f), Vec3::new(extent as f32, 0.0, f)),
        ] {
            // Split lines so each piece can fade with distance from the centre.
            let pieces = 12;
            for k in 0..pieces {
                let p0 = a.lerp(b, k as f32 / pieces as f32);
                let p1 = a.lerp(b, (k + 1) as f32 / pieces as f32);
                let mid = (p0 + p1) * 0.5;
                let fade = (1.0 - mid.length() / extent as f32).clamp(0.0, 1.0);
                if fade <= 0.02 {
                    continue;
                }
                let (Some(s0), Some(s1)) = (camera.project(p0, w, h), camera.project(p1, w, h))
                else {
                    continue;
                };
                let base = if major { color::LINE_STRONG } else { color::LINE };
                let c = lerp_color(color::BG_BASE, base, fade);
                painter.line_segment(
                    [Pos2::new(s0[0], s0[1]) + off, Pos2::new(s1[0], s1[1]) + off],
                    Stroke::new(1.0, c),
                );
            }
        }
    }
    let _ = Vec2::ZERO;
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}
