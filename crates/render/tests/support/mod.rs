//! Test-only helpers shared by the GPU tests and the `clear` example: getting a device (or skipping when the machine
//! has none), reading a rendered target back, and reading what the scene's shaders wrote before the post chain. The
//! readback blocks on the GPU, which the renderer itself never does; it lives here, outside the crate, so it cannot
//! end up in a frame.

// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

pub(crate) mod scenes;

use std::sync::{Mutex, Once, PoisonError};

use metallic_render::scene::RenderComponents;
use metallic_render::{
    CameraReader, ContextOptions, GpuScene, LightingReader, OffscreenTarget, PassId, RenderContext, RenderError,
    Renderer, RendererSettings, wgpu,
};

/// Set to `1` (CI does, on its software rasterisers) to fail instead of skip when there is no adapter.
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
    let bgra = matches!(
        texture.format(),
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    );
    read_back_texels(context, texture, 4)
        .as_chunks::<4>()
        .0
        .iter()
        .map(|px| if bgra { [px[2], px[1], px[0], px[3]] } else { *px })
        .collect()
}

/// Copies `texture` (an `Rgba16Float` 2D texture with `COPY_SRC`) back to the CPU, one `[r, g, b, a]` per pixel in
/// row order, waiting for the GPU. A value that is not a finite number comes back as one.
pub(crate) fn read_back_rgba16f(context: &RenderContext, texture: &wgpu::Texture) -> Vec<[f32; 4]> {
    assert_eq!(texture.format(), wgpu::TextureFormat::Rgba16Float);
    read_back_texels(context, texture, 8)
        .as_chunks::<8>()
        .0
        .iter()
        .map(|px| std::array::from_fn(|c| f16_to_f32(u16::from_le_bytes([px[2 * c], px[2 * c + 1]]))))
        .collect()
}

/// A half float's bits as an `f32` (exactly: every half float is one).
fn f16_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    match exponent {
        0 => sign * mantissa * 2f32.powi(-24),
        31 if mantissa == 0.0 => sign * f32::INFINITY,
        31 => f32::NAN,
        _ => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    }
}

/// The format [`render_raw`] draws into: half floats, which keep a value that is not a finite number as one.
pub(crate) const RAW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// `game` drawn (two frames, so everything is uploaded) at `size` × `size` with `change`'s settings, lit by its
/// `Lighting`, straight into a [`RAW_FORMAT`] target without the post chain, and read back: what the scene's shaders
/// wrote, linear and unclamped. (The finish pass reads a value that is not a finite number as 0, which hides it.)
pub(crate) fn render_raw(
    context: &RenderContext,
    game: &mut scenes::Game,
    size: u32,
    change: impl FnOnce(&mut RendererSettings),
) -> Vec<[f32; 4]> {
    let mut settings = RendererSettings {
        gpu_timing: false,
        ..RendererSettings::new(RAW_FORMAT, size, size)
    };
    change(&mut settings);
    settings.passes = settings.passes.without(PassId::Post);
    let target = OffscreenTarget::new(context, size, size, RAW_FORMAT).expect("a target");
    let mut renderer = Renderer::new(context, settings).expect("a renderer");
    renderer.wait_for_pipelines();
    let mut scene = GpuScene::new(
        context,
        &mut game.dm,
        Some(game.workspace),
        &RenderComponents::builtin(),
    )
    .expect("a scene");
    let lighting = game
        .dm
        .service(game.dm.registry().class_id("Lighting").expect("Lighting"))
        .expect("a Lighting");
    let mut reader = CameraReader::new();
    for _ in 0..2 {
        let camera = reader.read(&game.dm, game.workspace);
        if let Some(camera) = camera {
            scene.set_camera_position(&game.dm, camera.position);
        }
        scene.sync(&mut game.dm);
        renderer.set_lighting(LightingReader::new().read(&game.dm, lighting));
        renderer.render_scene(target.view(), &mut scene, camera.as_ref());
    }
    assert_eq!(
        renderer.post_stats().scene_format,
        RAW_FORMAT,
        "drawn without the post chain"
    );
    read_back_rgba16f(context, target.texture())
}

/// The pixels of a `width`-wide image with a channel that is not a finite number: where (x, y from the top left)
/// and what.
pub(crate) fn non_finite(pixels: &[[f32; 4]], width: u32) -> Vec<((u32, u32), [f32; 4])> {
    pixels
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.iter().all(|c| c.is_finite()))
        .map(|(i, &p)| ((i as u32 % width, i as u32 / width), p))
        .collect()
}

/// Copies `texture` (a 2D texture of `texel_size`-byte texels with `COPY_SRC`) back to the CPU, its rows packed
/// without padding, waiting for the GPU.
fn read_back_texels(context: &RenderContext, texture: &wgpu::Texture, texel_size: u32) -> Vec<u8> {
    let (width, height) = (texture.width(), texture.height());
    let unpadded = width * texel_size;
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
    let data = buffer.get_mapped_range(..).expect("readback is mapped");
    let mut texels = Vec::with_capacity((unpadded * height) as usize);
    for row in data.chunks_exact(padded as usize) {
        texels.extend_from_slice(&row[..unpadded as usize]);
    }
    texels
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

/// The mean difference, in 8-bit levels, an image may have from its committed reference when drawn on `context`
/// (`scenes::compare`'s `mean`). The references are drawn through the post chain's `Rg11b10Ufloat` scene target,
/// whose 5- and 6-bit mantissas round every pixel a little darker; a device without that format
/// (`metallic_render::post::hdr_format`; llvmpipe's OpenGL is one) draws into `Rgba16Float`, which keeps those bits,
/// so its whole image comes out up to a level brighter than the reference (a mean of 0.7 to 1.2 measured, #155),
/// with nothing changed in what is drawn. Real differences still show in the share of pixels over the threshold.
pub(crate) fn reference_max_mean(context: &RenderContext) -> f64 {
    if metallic_render::post::hdr_format(context) == wgpu::TextureFormat::Rg11b10Ufloat {
        1.0
    } else {
        1.5
    }
}

/// wgpu's warnings and errors, which include the graphics API's validation messages in debug builds (Vulkan
/// validation layers, the D3D12 debug layer) where the machine has them installed, as CI's images do, and the GL
/// backend's own errors (such as a texture bound with a view dimension its GL target cannot have, #155).
struct WgpuMessages(Mutex<Vec<(log::Level, String)>>);

impl log::Log for WgpuMessages {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn && metadata.target().starts_with("wgpu")
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            let message = format!("{}: {}", record.target(), record.args());
            // The Vulkan loader's own messages while instances are made (on a machine without a Vulkan driver,
            // "Found no drivers" is an error) say nothing about the frames drawn; the loader names the instance.
            if message.contains("[Loader Message") || message.contains("type: INSTANCE") {
                return;
            }
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((record.level(), message));
        }
    }

    fn flush(&self) {}
}

static MESSAGES: WgpuMessages = WgpuMessages(Mutex::new(Vec::new()));

/// Starts collecting wgpu's messages (once per test binary).
pub(crate) fn collect_wgpu_messages() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        if log::set_logger(&MESSAGES).is_ok() {
            log::set_max_level(log::LevelFilter::Warn);
        }
    });
}

/// Fails on any validation error wgpu reported since the last check (by any test of the binary); prints the
/// warnings.
pub(crate) fn assert_no_wgpu_errors() {
    let messages = std::mem::take(&mut *MESSAGES.0.lock().unwrap_or_else(PoisonError::into_inner));
    for (level, message) in &messages {
        eprintln!("wgpu {level}: {message}");
    }
    let errors = messages.iter().filter(|(level, _)| *level == log::Level::Error).count();
    assert_eq!(errors, 0, "wgpu reported {errors} error(s), printed above");
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
