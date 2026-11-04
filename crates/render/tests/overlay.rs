//! The debug overlay (#78) on a real device, offscreen: rectangles and glyphs land on the pixels they should, in the
//! same colour on sRGB and UNORM targets, blended over the frame; nothing is drawn while it is off or without a UI
//! pass. Writes the drawn image to `CARGO_TARGET_TMPDIR/overlay_<format>.png` for a look.

mod support;

use std::path::PathBuf;

use metallic_render::overlay::GLYPH_ADVANCE;
use metallic_render::{OffscreenTarget, PassId, PassSet, RenderContext, Renderer, RendererSettings, wgpu};
use support::{gpu, read_back_rgba8, scenes};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 96;

/// Renders one frame with the overlay drawn by `draw` and reads it back.
fn render(
    context: &RenderContext,
    format: wgpu::TextureFormat,
    passes: PassSet,
    enabled: bool,
    draw: impl FnOnce(&mut metallic_render::Overlay),
) -> Vec<[u8; 4]> {
    let target = OffscreenTarget::new(context, WIDTH, HEIGHT, format).expect("a target");
    let mut renderer = Renderer::new(
        context,
        RendererSettings {
            passes,
            gpu_timing: false,
            ..RendererSettings::new(format, WIDTH, HEIGHT)
        },
    )
    .expect("a renderer");
    renderer.wait_for_pipelines();
    renderer.set_overlay_enabled(enabled);
    assert_eq!(renderer.overlay_enabled(), enabled);
    draw(renderer.overlay_mut());
    renderer.render(target.view());
    read_back_rgba8(context, target.texture())
}

fn at(pixels: &[[u8; 4]], x: u32, y: u32) -> [u8; 4] {
    pixels[(y * WIDTH + x) as usize]
}

fn close(a: [u8; 4], b: [u8; 4], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= tolerance)
}

/// A red square, a half-transparent white square and the glyph "8" at scale 4, then every printable character.
fn sample(overlay: &mut metallic_render::Overlay) {
    overlay.rect(10.0, 10.0, 20.0, 20.0, [255, 0, 0, 255]);
    overlay.rect(40.0, 10.0, 20.0, 20.0, [255, 255, 255, 128]);
    overlay.text(70.0, 10.0, 4.0, [255, 255, 255, 255], "8");
    let all: String = (' '..='~').collect();
    let (half, rest) = all.split_at(48);
    overlay.text(4.0, 50.0, 1.0, [96, 214, 120, 255], half);
    overlay.text(4.0, 62.0, 1.0, [240, 196, 70, 255], rest);
    overlay.text_fmt(
        4.0,
        76.0,
        1.0,
        [235, 238, 242, 255],
        format_args!("FRAME {:.2} MS  P99 {:.2}  1% LOW {:.2}", 2.3456, 4.1, 5.0),
    );
}

#[test]
fn rectangles_and_glyphs_land_where_they_should() {
    let Some(context) = gpu() else { return };
    for format in [wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureFormat::Rgba8UnormSrgb] {
        let pixels = render(&context, format, PassSet::default(), true, sample);
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("overlay_{format:?}.png"));
        scenes::write_png(&out, WIDTH, HEIGHT, &pixels);
        eprintln!("{format:?}: {}", out.display());

        assert!(close(at(&pixels, 15, 15), [255, 0, 0, 255], 1), "{format:?}");
        assert!(
            close(at(&pixels, 5, 5), [0, 0, 0, 255], 1),
            "{format:?}: outside is the clear colour"
        );
        // Half-transparent white over black: half the light, which an sRGB target encodes as about 188.
        let half = if format.is_srgb() { 188 } else { 128 };
        assert!(
            close(at(&pixels, 50, 20), [half, half, half, 255], 3),
            "{format:?}: {:?}",
            at(&pixels, 50, 20)
        );
        // "8" at scale 4: its top row is .###. (column 0 off, columns 1 to 3 on).
        let (x, y) = (70, 10);
        assert!(close(at(&pixels, x + 2, y + 2), [0, 0, 0, 255], 1), "{format:?}");
        assert!(
            close(at(&pixels, x + 4 + 2, y + 2), [255, 255, 255, 255], 1),
            "{format:?}"
        );
        // Its middle row is .###. too, and its second row #...#.
        assert!(
            close(at(&pixels, x + 2, y + 4 + 2), [255, 255, 255, 255], 1),
            "{format:?}"
        );
        assert!(
            close(at(&pixels, x + 8 + 2, y + 4 + 2), [0, 0, 0, 255], 1),
            "{format:?}"
        );
        // Nothing in the gap after the glyph's five columns (the advance is six).
        let gap = x + (GLYPH_ADVANCE * 4.0) as u32 - 2;
        assert!(close(at(&pixels, gap, y + 2), [0, 0, 0, 255], 1), "{format:?}");
    }
}

#[test]
fn nothing_is_drawn_while_off_or_without_a_ui_pass() {
    let Some(context) = gpu() else { return };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let black = |pixels: &[[u8; 4]]| pixels.iter().all(|&p| p == [0, 0, 0, 255]);
    assert!(black(&render(&context, format, PassSet::default(), false, sample)));
    let no_ui = PassSet::default().without(PassId::Ui);
    assert!(black(&render(&context, format, no_ui, true, sample)));
    // Only a UI pass: the overlay is drawn over the cleared output.
    let ui_only = PassSet::EMPTY.with(PassId::Ui);
    let pixels = render(&context, format, ui_only, true, sample);
    assert!(close(at(&pixels, 15, 15), [255, 0, 0, 255], 1));
}

#[test]
fn turning_the_overlay_off_and_on_keeps_its_contents() {
    let Some(context) = gpu() else { return };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let target = OffscreenTarget::new(&context, WIDTH, HEIGHT, format).expect("a target");
    let mut renderer = Renderer::new(
        &context,
        RendererSettings {
            gpu_timing: false,
            ..RendererSettings::new(format, WIDTH, HEIGHT)
        },
    )
    .expect("a renderer");
    renderer.wait_for_pipelines();
    renderer.set_overlay_enabled(true);
    sample(renderer.overlay_mut());
    renderer.render(target.view());
    renderer.set_overlay_enabled(false);
    renderer.render(target.view());
    assert!(!renderer.overlay().is_empty());
    renderer.set_overlay_enabled(true);
    renderer.render(target.view());
    let pixels = read_back_rgba8(&context, target.texture());
    assert!(close(at(&pixels, 15, 15), [255, 0, 0, 255], 1));
}
