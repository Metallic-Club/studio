//! Saving a frame as a PNG (`--capture FRAME:PNG`).
use std::path::Path;

/// An offscreen colour target that can be read back.
pub struct CaptureTarget {
    texture: wgpu::Texture,
    /// View to draw into.
    pub view: wgpu::TextureView,
    size: (u32, u32),
    format: wgpu::TextureFormat,
}

impl CaptureTarget {
    /// Creates a `size` target of `format` (the window's surface format).
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: (u32, u32)) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("metallic player capture"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        CaptureTarget {
            texture,
            view,
            size,
            format,
        }
    }

    /// Reads the target back and writes it to `path` as an RGBA PNG.
    pub fn save_png(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: &Path,
    ) -> Result<(), String> {
        let (w, h) = self.size;
        let row = w * 4;
        let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic player readback"),
            size: u64::from(padded) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("metallic player readback"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| e.to_string())?;
        rx.recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let bgra = matches!(
            self.format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        let mut pixels = Vec::with_capacity((row * h) as usize);
        {
            let data = slice.get_mapped_range().map_err(|e| e.to_string())?;
            for y in 0..h as usize {
                let start = y * padded as usize;
                for px in data[start..start + row as usize].as_chunks::<4>().0 {
                    if bgra {
                        pixels.extend_from_slice(&[px[2], px[1], px[0], 255]);
                    } else {
                        pixels.extend_from_slice(&[px[0], px[1], px[2], 255]);
                    }
                }
            }
        }
        buffer.unmap();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()
            .and_then(|mut wr| wr.write_image_data(&pixels))
            .map_err(|e| e.to_string())
    }
}
