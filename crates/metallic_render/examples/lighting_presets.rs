//! Renders every lighting preset on the reference scene and writes PNGs.
//!
//! `cargo run -p metallic_render --example lighting_presets -- OUT_DIR [WIDTH HEIGHT]`
//! writes `OUT_DIR/<preset id>.png` (default 320 x 180, the Studio thumbnail size) and prints
//! luma mean / p1 / p99 and mean chroma per preset. `docs/lighting/presets/` holds its output.
#[path = "../tests/support/reference_scene.rs"]
mod reference_scene;

use metallic_core::lighting::{PRESETS, apply_preset};
use metallic_render::{Quality, RenderContext, RenderSettings, Renderer};
use reference_scene::{chroma, lumas, reference_camera, reference_world, stats};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = std::path::PathBuf::from(args.get(1).map_or("lighting-previews", String::as_str));
    let width: u32 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(320);
    let height: u32 = args.get(3).and_then(|a| a.parse().ok()).unwrap_or(180);
    std::fs::create_dir_all(&out).expect("output folder");
    let ctx = RenderContext::new_headless().expect("headless device");
    println!("adapter: {}", ctx.adapter_name);
    let mut renderer = Renderer::with_settings(
        &ctx,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings {
            quality: Quality::High,
        },
    );
    for preset in &PRESETS {
        let mut world = reference_world();
        apply_preset(&mut world, preset).expect("apply preset");
        renderer.sync(&world);
        let px = renderer
            .capture(&reference_camera(), (width, height))
            .expect("capture");
        let (mean, p1, p99) = stats(&lumas(&px));
        println!(
            "{:<16} luma mean {mean:.3} p1 {p1:.3} p99 {p99:.3} chroma {:.3}",
            preset.id,
            chroma(&px)
        );
        let path = out.join(format!("{}.png", preset.id));
        let file = std::fs::File::create(&path).expect("create png");
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::High);
        let rgb: Vec<u8> = px
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        enc.write_header()
            .expect("png header")
            .write_image_data(&rgb)
            .expect("png data");
    }
}
