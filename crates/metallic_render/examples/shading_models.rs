//! Renders the reference scene under each shading model and times them at 1080p (#63).
//!
//! `cargo run --release -p metallic_render --example shading_models -- OUT_DIR [WIDTH HEIGHT]`
//! writes `OUT_DIR/<name>.png` (default 960 x 540) for the shadings below and prints, per
//! shading, the median, 10th and 90th percentile frame time at 1920 x 1080 over 200 frames
//! (submit to idle, High quality), and the luma mean / p1 / p99 and mean chroma of each image. `docs/rendering/shading-models/` holds its output.
#[path = "../tests/support/reference_scene.rs"]
mod reference_scene;

use metallic_render::{ModelParams, Quality, RenderContext, RenderSettings, Renderer, Shading};
use reference_scene::{chroma, lumas, reference_camera, reference_world, stats};

fn shadings() -> Vec<(&'static str, Shading)> {
    let over = |m: &str, p: ModelParams| (m.to_string(), p);
    let soft = ModelParams {
        wrap: 0.3,
        roughness_floor: 0.5,
        ..ModelParams::LIT
    };
    let glass_lit = ModelParams {
        wrap: 0.3,
        ..ModelParams::LIT
    };
    vec![
        ("lit", Shading::uniform(ModelParams::LIT)),
        (
            "lit-soft",
            Shading {
                default: soft,
                materials: vec![
                    over(
                        "Metal",
                        ModelParams {
                            roughness_floor: 0.35,
                            ..soft
                        },
                    ),
                    over("Glass", ModelParams::LIT),
                    over(
                        "Ice",
                        ModelParams {
                            roughness_floor: 0.2,
                            ..soft
                        },
                    ),
                ],
                character: None,
            },
        ),
        (
            "toon",
            Shading {
                default: ModelParams::TOON,
                materials: vec![
                    over(
                        "Metal",
                        ModelParams {
                            bands: 3.0,
                            spec_size: 0.2,
                            spec_strength: 1.0,
                            ..ModelParams::TOON
                        },
                    ),
                    over("Glass", glass_lit),
                ],
                character: None,
            },
        ),
        (
            "toon-stepped",
            Shading::uniform(ModelParams {
                bands: 3.0,
                softness: 0.0,
                shadow_tint: [215.0, 205.0, 255.0],
                spec_size: 0.06,
                spec_strength: 0.6,
                light_step_ev: 1.0,
                ..ModelParams::TOON
            }),
        ),
        ("flat", Shading::uniform(ModelParams::FLAT)),
        ("unlit", Shading::uniform(ModelParams::UNLIT)),
        (
            "mixed",
            Shading {
                default: ModelParams::LIT,
                materials: vec![
                    over("Plastic", ModelParams::TOON),
                    over("Wood", ModelParams::FLAT),
                    over("Brick", ModelParams::FLAT),
                    over("Grass", ModelParams::UNLIT),
                ],
                character: None,
            },
        ),
    ]
}

fn write_png(path: &std::path::Path, px: &[u8], (w, h): (u32, u32)) {
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = std::path::PathBuf::from(args.get(1).map_or("shading-models", String::as_str));
    let width: u32 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(960);
    let height: u32 = args.get(3).and_then(|a| a.parse().ok()).unwrap_or(540);
    std::fs::create_dir_all(&out).expect("output folder");
    let ctx = RenderContext::new_headless().expect("headless device");
    println!("adapter: {}", ctx.adapter_name);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = Renderer::with_settings(
        &ctx,
        format,
        RenderSettings {
            quality: Quality::High,
        },
    );
    renderer.sync(&reference_world());
    let cam = reference_camera();

    let size = (1920, 1080);
    let tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());

    for (name, shading) in shadings() {
        renderer.set_shading(&shading);
        let px = renderer.capture(&cam, (width, height)).expect("capture");
        write_png(&out.join(format!("{name}.png")), &px, (width, height));
        let (mean, p1, p99) = stats(&lumas(&px));
        println!(
            "{name:<13} luma mean {mean:.3} p1 {p1:.3} p99 {p99:.3} chroma {:.3}",
            chroma(&px)
        );
        let mut ms = Vec::with_capacity(200);
        for i in 0..260 {
            let t = std::time::Instant::now();
            renderer.render(&view, &cam, size);
            let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
            if i >= 60 {
                ms.push(t.elapsed().as_secs_f64() * 1000.0);
            }
        }
        ms.sort_by(f64::total_cmp);
        println!(
            "{name:<13} 1080p: median {:.3} ms, p10 {:.3}, p90 {:.3}",
            ms[ms.len() / 2],
            ms[ms.len() / 10],
            ms[ms.len() * 9 / 10]
        );
    }
}
