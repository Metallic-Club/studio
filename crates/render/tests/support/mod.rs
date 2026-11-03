//! Test-only helpers shared by the GPU tests and the `clear` example: getting a device (or skipping when the machine
//! has none) and reading a rendered target back. The readback blocks on the GPU, which the renderer itself never
//! does; it lives here, outside the crate, so it cannot end up in a frame.

// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

pub(crate) mod scenes;

use metallic_render::{ContextOptions, RenderContext, RenderError, wgpu};

/// Set to `1` (CI does, on its Vulkan software rasteriser) to fail instead of skip when there is no adapter.
pub(crate) const REQUIRE_GPU_ENV: &str = "METALLIC_REQUIRE_GPU";

/// A device on any adapter, or `None` (after saying why) when the machine has none and `METALLIC_REQUIRE_GPU` is
/// not set.
pub(crate) fn gpu() -> Option<RenderContext> {
    gpu_with(&ContextOptions::default())
}

/// Like [`gpu`], with explicit options.
pub(crate) fn gpu_with(options: &ContextOptions) -> Option<RenderContext> {
    match RenderContext::new_offscreen(options) {
        Ok(context) => Some(context),
        Err(error @ RenderError::NoAdapter { .. }) => {
            if std::env::var(REQUIRE_GPU_ENV).is_ok_and(|value| value == "1") {
                panic!("{error}, and {REQUIRE_GPU_ENV}=1");
            }
            eprintln!("skipped: {error}");
            None
        }
        Err(error) => panic!("could not create a device: {error}"),
    }
}

/// Copies `texture` (an 8-bit RGBA or BGRA 2D texture with `COPY_SRC`) back to the CPU, one `[r, g, b, a]` per
/// pixel in row order, waiting for the GPU.
pub(crate) fn read_back_rgba8(context: &RenderContext, texture: &wgpu::Texture) -> Vec<[u8; 4]> {
    let (width, height) = (texture.width(), texture.height());
    let unpadded = width * 4;
    let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("test readback"),
        size: u64::from(padded * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
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
    context.queue().submit(Some(encoder.finish()));
    buffer.map_async(wgpu::MapMode::Read, .., |result| {
        result.expect("map the readback buffer")
    });
    context
        .device()
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("wait for the readback");
    let bgra = matches!(
        texture.format(),
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    );
    let data = buffer.get_mapped_range(..).expect("readback is mapped");
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for row in data.chunks_exact(padded as usize) {
        for px in row[..unpadded as usize].as_chunks::<4>().0 {
            pixels.push(if bgra {
                [px[2], px[1], px[0], px[3]]
            } else {
                [px[0], px[1], px[2], px[3]]
            });
        }
    }
    pixels
}

/// Copies the first `size` bytes of `buffer` (which needs `COPY_SRC`) back to the CPU, waiting for the GPU.
pub(crate) fn read_back_buffer(context: &RenderContext, buffer: &wgpu::Buffer, size: u64) -> Vec<u8> {
    if size == 0 {
        return Vec::new();
    }
    let readback = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("test buffer readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &readback, 0, size);
    context.queue().submit(Some(encoder.finish()));
    readback.map_async(wgpu::MapMode::Read, .., |result| {
        result.expect("map the readback buffer")
    });
    context
        .device()
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("wait for the readback");
    let data = readback.get_mapped_range(..).expect("readback is mapped");
    data.to_vec()
}

/// Asserts every pixel is within `tolerance` of `expected` in each channel.
pub(crate) fn assert_all_pixels(pixels: &[[u8; 4]], expected: [u8; 4], tolerance: u8) {
    for (i, pixel) in pixels.iter().enumerate() {
        let close = pixel
            .iter()
            .zip(expected)
            .all(|(&got, want)| got.abs_diff(want) <= tolerance);
        assert!(close, "pixel {i} is {pixel:?}, expected {expected:?} ± {tolerance}");
    }
}
