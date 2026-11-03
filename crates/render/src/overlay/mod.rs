//! The debug overlay: solid rectangles and text in screen space, drawn in the UI pass over everything else. It
//! carries the performance overlay (frame-time graph and numbers, ADR 0008 §10) and anything else a developer wants
//! on screen without a UI system.
//!
//! - **Cheap and toggled:** nothing exists on the GPU until [`crate::Renderer::set_overlay_enabled`] turns it on
//!   (a settings change: the pipeline and buffer are made then, never in a frame). While on, a frame costs one
//!   instanced draw, plus one buffer write when the contents changed.
//! - **No font crate:** text uses a built-in 5 × 7 bitmap font (printable ASCII, lower case drawn as upper case)
//!   whose pixels travel in each glyph's instance data, so there is no texture and no bind group.
//! - **No allocation per frame:** [`Overlay`] holds at most [`OVERLAY_MAX_QUADS`] quads in storage reserved when
//!   it is enabled; what does not fit is dropped and counted ([`Overlay::dropped`]). [`Overlay::text_fmt`] lays
//!   out `format_args!` output glyph by glyph as it is written, with no intermediate string.
//!
//! Coordinates are physical pixels from the target's top-left corner. Colours are sRGB with straight alpha.
//! The contents stay until changed: [`Overlay::clear`] and redraw whatever changes.

mod font;

use std::fmt;

use bytemuck::{Pod, Zeroable};

pub use font::{GLYPH_COLUMNS, GLYPH_ROWS};

/// The most quads (rectangles and glyphs) the overlay holds.
pub const OVERLAY_MAX_QUADS: usize = 8192;

/// Horizontal distance from one glyph to the next at scale 1, in pixels (5 columns and a gap).
pub const GLYPH_ADVANCE: f32 = (GLYPH_COLUMNS + 1) as f32;

/// Distance from one line of text to the next at scale 1, in pixels (7 rows and a gap).
pub const LINE_HEIGHT: f32 = (GLYPH_ROWS + 2) as f32;

/// One quad in pixels, as the caller drew it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Quad {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: [u8; 4],
    /// `None` for a solid rectangle.
    glyph: Option<[u32; 2]>,
}

/// What the overlay draws: rectangles and text, in physical pixels from the top-left corner. Get it from
/// [`crate::Renderer::overlay_mut`]. See the [module documentation](self).
///
/// [`Overlay::default`] has no storage and drops everything until reserved; [`Overlay::new`] reserves it.
#[derive(Debug, Default)]
pub struct Overlay {
    quads: Vec<Quad>,
    dropped: usize,
    /// Bumped by every change, so the renderer uploads only changed contents.
    generation: u64,
}

impl Overlay {
    /// An empty overlay with its storage reserved, so no later call allocates. (The renderer's own overlay
    /// reserves its storage when it is first enabled.)
    pub fn new() -> Self {
        let mut overlay = Self::default();
        overlay.reserve();
        overlay
    }

    /// Reserves the storage for [`OVERLAY_MAX_QUADS`] quads, so no later call allocates.
    pub(crate) fn reserve(&mut self) {
        self.quads
            .reserve_exact(OVERLAY_MAX_QUADS.saturating_sub(self.quads.len()));
    }

    /// Removes everything.
    pub fn clear(&mut self) {
        if !self.quads.is_empty() || self.dropped != 0 {
            self.quads.clear();
            self.dropped = 0;
            self.generation += 1;
        }
    }

    /// Quads held (rectangles and visible glyphs).
    pub fn len(&self) -> usize {
        self.quads.len()
    }

    /// Whether nothing is drawn.
    pub fn is_empty(&self) -> bool {
        self.quads.is_empty()
    }

    /// Quads dropped since the last [`Overlay::clear`] because the overlay was full.
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// Changes whenever the contents change.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    fn push(&mut self, quad: Quad) {
        // Never grow past the reserved storage: a full overlay drops, it does not allocate.
        if self.quads.len() < self.quads.capacity().min(OVERLAY_MAX_QUADS) {
            self.quads.push(quad);
        } else {
            self.dropped += 1;
        }
        self.generation += 1;
    }

    /// A solid rectangle `w` × `h` pixels with its top-left corner at `x`, `y`.
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [u8; 4]) {
        if w > 0.0 && h > 0.0 && color[3] > 0 {
            self.push(Quad {
                x,
                y,
                w,
                h,
                color,
                glyph: None,
            });
        }
    }

    /// Draws `text` with its top-left corner at `x`, `y`, each font pixel `scale` × `scale` screen pixels. A
    /// newline starts a new line at `x`. Returns where the next character would go.
    pub fn text(&mut self, x: f32, y: f32, scale: f32, color: [u8; 4], text: &str) -> (f32, f32) {
        let mut pen = Pen {
            overlay: self,
            left: x,
            x,
            y,
            scale,
            color,
        };
        pen.write(text);
        (pen.x, pen.y)
    }

    /// Like [`Overlay::text`], for `format_args!(...)`: the formatted text is laid out as it is produced, with no
    /// string in between.
    pub fn text_fmt(&mut self, x: f32, y: f32, scale: f32, color: [u8; 4], args: fmt::Arguments<'_>) -> (f32, f32) {
        let mut pen = Pen {
            overlay: self,
            left: x,
            x,
            y,
            scale,
            color,
        };
        // A `Pen` never fails to write.
        let _ = fmt::Write::write_fmt(&mut pen, args);
        (pen.x, pen.y)
    }

    /// Writes the quads as the shader reads them into `out` (cleared first), for a `width` × `height` target.
    fn encode(&self, width: u32, height: u32, out: &mut Vec<GpuQuad>) {
        out.clear();
        let (sx, sy) = (2.0 / width.max(1) as f32, 2.0 / height.max(1) as f32);
        for quad in &self.quads {
            let (left, top) = (quad.x * sx - 1.0, 1.0 - quad.y * sy);
            let (right, bottom) = ((quad.x + quad.w) * sx - 1.0, 1.0 - (quad.y + quad.h) * sy);
            out.push(GpuQuad {
                rect: [left, top, right, bottom],
                color: u32::from_le_bytes(quad.color),
                bits: quad.glyph.unwrap_or([0; 2]),
                kind: u32::from(quad.glyph.is_some()),
            });
        }
    }
}

/// Lays text out glyph by glyph; also the [`fmt::Write`] behind [`Overlay::text_fmt`].
struct Pen<'a> {
    overlay: &'a mut Overlay,
    left: f32,
    x: f32,
    y: f32,
    scale: f32,
    color: [u8; 4],
}

impl Pen<'_> {
    fn write(&mut self, text: &str) {
        for c in text.chars() {
            if c == '\n' {
                self.x = self.left;
                self.y += LINE_HEIGHT * self.scale;
                continue;
            }
            if c != ' ' {
                self.overlay.push(Quad {
                    x: self.x,
                    y: self.y,
                    w: GLYPH_COLUMNS as f32 * self.scale,
                    h: GLYPH_ROWS as f32 * self.scale,
                    color: self.color,
                    glyph: Some(font::glyph(c)),
                });
            }
            self.x += GLYPH_ADVANCE * self.scale;
        }
    }
}

impl fmt::Write for Pen<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write(s);
        Ok(())
    }
}

/// One quad as the shader reads it (`QuadIn` in `overlay.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
struct GpuQuad {
    rect: [f32; 4],
    color: u32,
    bits: [u32; 2],
    kind: u32,
}

const QUAD_SIZE: u64 = size_of::<GpuQuad>() as u64;

/// The overlay's GPU side: its pipeline, instance buffer and the copy last uploaded. Made when the overlay is
/// turned on.
#[derive(Debug)]
pub(crate) struct OverlayPass {
    pipeline: wgpu::RenderPipeline,
    format: wgpu::TextureFormat,
    buffer: wgpu::Buffer,
    staging: Vec<GpuQuad>,
    /// The overlay generation and target size the buffer holds.
    uploaded: Option<(u64, u32, u32)>,
    count: u32,
}

impl OverlayPass {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic overlay quads"),
            size: QUAD_SIZE * OVERLAY_MAX_QUADS as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline: create_pipeline(device, format),
            format,
            buffer,
            staging: Vec::with_capacity(OVERLAY_MAX_QUADS),
            uploaded: None,
            count: 0,
        }
    }

    /// Recreates the pipeline for another colour format (a settings change).
    pub(crate) fn set_format(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        if format != self.format {
            self.pipeline = create_pipeline(device, format);
            self.format = format;
        }
    }

    /// Uploads `overlay` when it changed since the last upload (or the target size did).
    pub(crate) fn prepare(&mut self, queue: &wgpu::Queue, overlay: &Overlay, width: u32, height: u32) {
        let key = (overlay.generation(), width, height);
        if self.uploaded == Some(key) {
            return;
        }
        overlay.encode(width, height, &mut self.staging);
        self.count = self.staging.len() as u32;
        if !self.staging.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.staging));
        }
        self.uploaded = Some(key);
    }

    /// Records the overlay's one draw.
    pub(crate) fn draw(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        if self.count == 0 {
            return;
        }
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_vertex_buffer(0, self.buffer.slice(..u64::from(self.count) * QUAD_SIZE));
        render_pass.draw(0..6, 0..self.count);
    }
}

fn create_pipeline(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("metallic overlay"),
        source: wgpu::ShaderSource::Wgsl(include_str!("overlay.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("metallic overlay"),
        bind_group_layouts: &[],
        immediate_size: 0,
    });
    let attributes = wgpu::vertex_attr_array![
        0 => Float32x4,
        1 => Unorm8x4,
        2 => Uint32x2,
        3 => Uint32,
    ];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("metallic overlay"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: QUAD_SIZE,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &attributes,
            })],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some(if format.is_srgb() { "fs_linear" } else { "fs_encoded" }),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_lays_out_glyphs_and_skips_spaces() {
        let mut overlay = Overlay::default();
        overlay.reserve();
        let end = overlay.text(10.0, 20.0, 2.0, [255; 4], "A b\nC");
        // Three glyphs: the space is advanced over, not drawn.
        assert_eq!(overlay.len(), 3);
        assert_eq!(overlay.quads[0].x, 10.0);
        assert_eq!(overlay.quads[1].x, 10.0 + 2.0 * GLYPH_ADVANCE * 2.0);
        assert_eq!(overlay.quads[2].y, 20.0 + LINE_HEIGHT * 2.0);
        assert_eq!(end, (10.0 + GLYPH_ADVANCE * 2.0, 20.0 + LINE_HEIGHT * 2.0));
        assert_eq!(overlay.quads[0].w, 10.0);
        assert_eq!(overlay.quads[0].h, 14.0);
    }

    #[test]
    fn formatted_text_matches_plain_text() {
        let mut a = Overlay::default();
        let mut b = Overlay::default();
        a.reserve();
        b.reserve();
        a.text(0.0, 0.0, 1.0, [1, 2, 3, 4], "P99 4.25 MS");
        b.text_fmt(0.0, 0.0, 1.0, [1, 2, 3, 4], format_args!("P99 {:.2} {}", 4.25f32, "MS"));
        assert_eq!(a.quads, b.quads);
    }

    #[test]
    fn a_full_overlay_drops_instead_of_growing() {
        let mut overlay = Overlay::default();
        overlay.reserve();
        let capacity = overlay.quads.capacity();
        for i in 0..OVERLAY_MAX_QUADS + 10 {
            overlay.rect(i as f32, 0.0, 1.0, 1.0, [255; 4]);
        }
        assert_eq!(overlay.len(), OVERLAY_MAX_QUADS);
        assert_eq!(overlay.dropped(), 10);
        assert_eq!(overlay.quads.capacity(), capacity);
        overlay.clear();
        assert!(overlay.is_empty());
        assert_eq!(overlay.dropped(), 0);
    }

    #[test]
    fn encoding_maps_pixels_to_clip_space() {
        let mut overlay = Overlay::default();
        overlay.reserve();
        overlay.rect(0.0, 0.0, 50.0, 25.0, [10, 20, 30, 40]);
        let mut out = Vec::new();
        overlay.encode(100, 50, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].rect, [-1.0, 1.0, 0.0, 0.0]);
        assert_eq!(out[0].color.to_le_bytes(), [10, 20, 30, 40]);
        assert_eq!(out[0].kind, 0);
        // Invisible rectangles are not kept.
        overlay.rect(0.0, 0.0, 0.0, 5.0, [255; 4]);
        overlay.rect(0.0, 0.0, 5.0, 5.0, [255, 255, 255, 0]);
        assert_eq!(overlay.len(), 1);
    }
}
