//! An egui pass of its own for the Player's menu and HUD, drawn over the game and its GUI.
use egui::ViewportInfo;

/// egui state for drawing the menu over a target of one texture format.
pub struct UiLayer {
    ctx: egui::Context,
    renderer: egui_wgpu::Renderer,
}

impl UiLayer {
    /// Creates the layer for targets of `format` (text uses egui's bundled fonts).
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> UiLayer {
        let ctx = egui::Context::default();
        let renderer =
            egui_wgpu::Renderer::new(device, format, egui_wgpu::RendererOptions::default());
        UiLayer { ctx, renderer }
    }

    /// Runs `paint` in one egui pass and draws the result into `target` (which already holds
    /// the frame) of `size` pixels at `scale` pixels per point.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        size: (u32, u32),
        scale: f32,
        time: f64,
        paint: impl FnOnce(&egui::Context),
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
            time: Some(time),
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
        paint(&self.ctx);
        let mut full = self.ctx.end_pass();
        if full.shapes.is_empty() && full.textures_delta.is_empty() {
            return;
        }
        let shapes = std::mem::take(&mut full.shapes);
        let primitives = self.ctx.tessellate(shapes, full.pixels_per_point);
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
            label: Some("metallic menu"),
        });
        let extra = self
            .renderer
            .update_buffers(device, queue, &mut encoder, &primitives, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("metallic menu"),
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
        // Applied above; egui checks that no delta is dropped unhandled.
        full.textures_delta.clear();
    }
}
