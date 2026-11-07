//! Studio drawn offscreen, for the tests and the screenshots: egui-wgpu on an engine device without a window, drawn as
//! the shell draws it, the viewport by the same [`ViewportRenderer`]. No window opens and no real input is read.

use std::time::{Duration, Instant};

use metallic_engine::metallic_render::{ContextOptions, RenderContext, RenderError, wgpu};

use super::viewport::ViewportRenderer;
use super::{Studio, splash};
use crate::theme::fonts;

/// An image read back: width, height and RGBA rows.
pub(super) type Pixels = (u32, u32, Vec<u8>);

/// The frame's format: what the shell's surface usually is, and what the screenshots are written as.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Studio's frames drawn into a texture.
pub(super) struct Offscreen {
    pub(super) context: RenderContext,
    pub(super) ctx: egui::Context,
    painter: egui_wgpu::Renderer,
    pub(super) viewport: ViewportRenderer,
    target: wgpu::Texture,
    size: egui::Vec2,
    pixels_per_point: f32,
    frame: usize,
    start: Instant,
}

/// A device on any adapter, or `None` (after saying why) when the machine has none and `METALLIC_REQUIRE_GPU` is not
/// set, as the engine's GPU tests do.
pub(super) fn gpu() -> Option<RenderContext> {
    match RenderContext::new_offscreen(&ContextOptions::default()) {
        Ok(context) => Some(context),
        Err(error @ RenderError::NoAdapter { .. }) => {
            if std::env::var("METALLIC_REQUIRE_GPU").is_ok_and(|value| value == "1") {
                panic!("{error}, and METALLIC_REQUIRE_GPU=1");
            }
            eprintln!("skipped: {error}");
            None
        }
        Err(error) => panic!("could not create a device: {error}"),
    }
}

impl Offscreen {
    /// Frames of `size` points at `pixels_per_point`, or `None` without an adapter (see [`gpu`]).
    pub(super) fn new(size: egui::Vec2, pixels_per_point: f32) -> Option<Self> {
        let context = gpu()?;
        let ctx = egui::Context::default();
        fonts::install(&ctx);
        let painter = egui_wgpu::Renderer::new(context.device(), FORMAT, egui_wgpu::RendererOptions::default());
        let target = Self::make_target(&context, size, pixels_per_point);
        Some(Self {
            context,
            ctx,
            painter,
            viewport: ViewportRenderer::new(),
            target,
            size,
            pixels_per_point,
            frame: 0,
            start: Instant::now(),
        })
    }

    fn make_target(context: &RenderContext, size: egui::Vec2, pixels_per_point: f32) -> wgpu::Texture {
        let (width, height) = (
            (size.x * pixels_per_point).round() as u32,
            (size.y * pixels_per_point).round() as u32,
        );
        context.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("studio screenshot"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// Makes the window `size` points from the next frame, as a user's resize does.
    pub(super) fn set_size(&mut self, size: egui::Vec2) {
        self.size = size;
        self.target = Self::make_target(&self.context, size, self.pixels_per_point);
    }

    /// Runs and draws one frame of `studio`, with `events` as its input. The splash shows in the first frame, and its
    /// minimum time is out by the second.
    pub(super) fn frame_with(&mut self, studio: &mut Studio, events: Vec<egui::Event>) {
        let (device, queue) = (self.context.device(), self.context.queue());
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            time: Some(self.frame as f64 / 60.0),
            events,
            ..Default::default()
        };
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(self.pixels_per_point);
        let now = match self.frame {
            0 => self.start,
            frame => self.start + splash::MIN_TIME + Duration::from_millis(16 * frame as u64),
        };
        self.frame += 1;
        let mut output = self.ctx.run_ui(input, |ui| studio.ui(ui, now));
        let mut textures = std::mem::take(&mut output.textures_delta);
        self.viewport.frame(&self.context, &mut self.painter, studio);
        for (id, deltas) in textures.set.drain() {
            for delta in &deltas {
                self.painter.update_texture(device, queue, id, delta);
            }
        }
        let jobs = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.target.width(), self.target.height()],
            pixels_per_point: output.pixels_per_point,
        };
        let view = self.target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let mut buffers = self.painter.update_buffers(device, queue, &mut encoder, &jobs, &screen);
        let clear = egui::Rgba::from(studio.clear_color());
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("studio screenshot"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: f64::from(clear.r()),
                                g: f64::from(clear.g()),
                                b: f64::from(clear.b()),
                                a: f64::from(clear.a()),
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            self.painter.render(&mut pass, &jobs, &screen);
        }
        buffers.push(encoder.finish());
        self.context.submit(buffers);
        for id in textures.free.drain() {
            self.painter.free_texture(&id);
        }
    }

    /// Runs and draws one frame of `studio` without input.
    pub(super) fn frame(&mut self, studio: &mut Studio) {
        self.frame_with(studio, Vec::new());
    }

    /// The last frame drawn.
    pub(super) fn read(&self) -> Pixels {
        read_texture(&self.context, &self.target)
    }
}

/// `texture`'s pixels (8-bit RGBA), read back waiting for the GPU.
pub(super) fn read_texture(context: &RenderContext, texture: &wgpu::Texture) -> Pixels {
    let device = context.device();
    let (width, height) = (texture.width(), texture.height());
    let padded = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("studio screenshot readback"),
        size: u64::from(padded * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        texture.size(),
    );
    context.submit(Some(encoder.finish()));
    buffer.map_async(wgpu::MapMode::Read, .., |result| result.expect("map the readback"));
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("wait for the readback");
    let data = buffer.get_mapped_range(..).expect("mapped");
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for row in data.chunks_exact(padded as usize) {
        pixels.extend_from_slice(&row[..(width * 4) as usize]);
    }
    (width, height, pixels)
}
