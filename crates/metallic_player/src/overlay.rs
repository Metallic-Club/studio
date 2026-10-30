//! The in-game GUI pass: egui paints the game's PlayerGui over the 3D view.
use egui::{Id, LayerId, Order, ViewportInfo};
use metallic_runtime::Game;

/// egui state for drawing the GUI over a target of one texture format.
pub struct Overlay {
    ctx: egui::Context,
    renderer: egui_wgpu::Renderer,
}

impl Overlay {
    /// Creates the overlay for targets of `format` and installs a system font.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Overlay {
        let ctx = egui::Context::default();
        if metallic_runtime::gui::install_system_font(&ctx).is_none() {
            eprintln!("warning: no system font found; GUI text shows as boxes");
        }
        let renderer =
            egui_wgpu::Renderer::new(device, format, egui_wgpu::RendererOptions::default());
        Overlay { ctx, renderer }
    }

    /// Paints the game's GUI into `target` (already holding the 3D frame) of `size` pixels at
    /// `scale` pixels per point.
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        size: (u32, u32),
        scale: f32,
        game: &Game,
    ) {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size.0 as f32 / scale, size.1 as f32 / scale),
            )),
            ..Default::default()
        };
        raw.viewports.insert(
            raw.viewport_id,
            ViewportInfo {
                native_pixels_per_point: Some(scale),
                ..Default::default()
            },
        );
        self.ctx.begin_pass(raw);
        let painter = self
            .ctx
            .layer_painter(LayerId::new(Order::Foreground, Id::unique("metallic_gui")));
        game.paint_gui(&painter);
        let full = self.ctx.end_pass();
        let primitives = self.ctx.tessellate(full.shapes, full.pixels_per_point);
        for (id, deltas) in &full.textures_delta.set {
            for delta in deltas {
                self.renderer.update_texture(device, queue, *id, delta);
            }
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [size.0, size.1],
            pixels_per_point: full.pixels_per_point,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("metallic gui"),
        });
        let extra = self
            .renderer
            .update_buffers(device, queue, &mut encoder, &primitives, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("metallic gui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &primitives, &screen);
        }
        queue.submit(extra.into_iter().chain([encoder.finish()]));
        for id in &full.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}
