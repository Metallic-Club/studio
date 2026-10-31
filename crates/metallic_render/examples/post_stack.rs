//! Renders the post stack v1 (#65) settings on the reference scene, with the default avatar in
//! it, and times them at 1080p.
//!
//! `cargo run --release -p metallic_render --example post_stack -- OUT_DIR [WIDTH HEIGHT]`
//! writes `OUT_DIR/<name>.png` (default 960 x 540) and prints, per setting, the median, 10th
//! and 90th percentile GPU time of a 1920 x 1080 frame from timestamp queries over 200 frames after 60 warm-up frames
//! (wall-clock time from submit to idle where the device has no timestamp queries).
//! `--software` uses the platform's software adapter (WARP on Windows) and 20 timed frames.
//! `docs/rendering/post-stack/` holds its output.
#[path = "../tests/support/reference_scene.rs"]
#[allow(dead_code)]
mod reference_scene;

use metallic_render::glam::{Mat4, Vec3};
use metallic_render::metallic_asset::{Pose, SkinnedModel};
use metallic_render::{
    EDGE_PASS, EdgePass, Lut3d, MLAA_PASS, MlaaPass, Quality, RenderContext, RenderSettings,
    Renderer, SkinStyle, SkinnedInstance,
};
use reference_scene::{reference_camera, reference_world};
use std::sync::Arc;

/// A warm, slightly lifted grade: shadows toward blue, highlights toward amber.
fn warm_lut() -> Lut3d {
    Lut3d::from_fn(33, |c| {
        let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let shadow = (1.0 - l).powi(2) * 0.04;
        let high = l * l * 0.05;
        [
            (c[0] * 0.97 + 0.02 + high).min(1.0),
            (c[1] * 0.97 + 0.015 + high * 0.6).min(1.0),
            (c[2] * 0.95 + 0.02 + shadow - high * 0.5).clamp(0.0, 1.0),
        ]
    })
}

struct Setting {
    name: &'static str,
    quality: Quality,
    edges: bool,
    lut: bool,
    aa: bool,
    style: SkinStyle,
}

const SETTINGS: [Setting; 7] = [
    Setting {
        name: "high-default",
        quality: Quality::High,
        edges: false,
        lut: false,
        aa: false,
        style: SkinStyle::Toon,
    },
    Setting {
        name: "high-edges",
        quality: Quality::High,
        edges: true,
        lut: false,
        aa: false,
        style: SkinStyle::Toon,
    },
    Setting {
        name: "high-edges-standard-avatar",
        quality: Quality::High,
        edges: true,
        lut: false,
        aa: false,
        style: SkinStyle::Standard,
    },
    Setting {
        name: "high-lut",
        quality: Quality::High,
        edges: false,
        lut: true,
        aa: false,
        style: SkinStyle::Toon,
    },
    Setting {
        name: "low-no-aa",
        quality: Quality::Low,
        edges: false,
        lut: false,
        aa: false,
        style: SkinStyle::Toon,
    },
    Setting {
        name: "low-mlaa",
        quality: Quality::Low,
        edges: false,
        lut: false,
        aa: true,
        style: SkinStyle::Toon,
    },
    Setting {
        name: "low-mlaa-edges",
        quality: Quality::Low,
        edges: true,
        lut: false,
        aa: true,
        style: SkinStyle::Toon,
    },
];

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
    let mut args: Vec<String> = std::env::args().collect();
    let software = args.iter().any(|a| a == "--software");
    args.retain(|a| a != "--software");
    let out = std::path::PathBuf::from(args.get(1).map_or("post-stack", String::as_str));
    let width: u32 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(960);
    let height: u32 = args.get(3).and_then(|a| a.parse().ok()).unwrap_or(540);
    std::fs::create_dir_all(&out).expect("output folder");
    let ctx = if software {
        RenderContext::new_headless_software()
    } else {
        RenderContext::new_headless()
    }
    .expect("headless device");
    let (warm, timed) = if software { (3, 20) } else { (60, 200) };
    println!("adapter: {}", ctx.adapter_name);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;

    let glb = include_bytes!("../../../art/avatar/out/default_avatar.glb");
    let model = Arc::new(SkinnedModel::from_glb(glb).expect("avatar"));
    let scale = 1.0 / 0.28;
    let mut joints = Vec::new();
    // Standing on the floor beside the parts dummy, turned a little toward the camera.
    let place = Mat4::from_translation(Vec3::new(3.0, 0.0, 6.0))
        * Mat4::from_rotation_y(0.4)
        * Mat4::from_scale(Vec3::splat(scale));
    Pose::rest(&model.skeleton).skin_matrices(&model.skeleton, place, &mut joints);
    let world = reference_world();
    let cam = reference_camera();
    let lut = warm_lut();

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

    for s in &SETTINGS {
        let mut r = Renderer::with_settings(&ctx, format, RenderSettings { quality: s.quality });
        r.sync(&world);
        r.set_skinned(&[SkinnedInstance {
            model: &model,
            joints: &joints,
            slot_colors: &[],
            center: Vec3::new(3.0, 2.75, 6.0),
            height: 5.5,
            detail_scale: scale,
            style: s.style,
        }]);
        r.post_mut()
            .get_mut::<EdgePass>(EDGE_PASS)
            .expect("edges")
            .enabled = s.edges;
        r.post_mut()
            .get_mut::<MlaaPass>(MLAA_PASS)
            .expect("aa")
            .enabled = s.aa;
        r.set_lut(s.lut.then_some(&lut), if s.lut { 1.0 } else { 0.0 });
        let px = r.capture(&cam, (width, height)).expect("capture");
        write_png(&out.join(format!("{}.png", s.name)), &px, (width, height));

        let timestamps = r.enable_gpu_timing();
        let clock = if timestamps { "GPU" } else { "wall" };
        let mut ms = Vec::with_capacity(timed);
        for i in 0..warm + timed {
            let t = std::time::Instant::now();
            r.render(&view, &cam, size);
            let gpu = r.read_gpu_time_ms();
            let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
            if i >= warm {
                ms.push(gpu.unwrap_or_else(|| t.elapsed().as_secs_f64() * 1000.0));
            }
        }
        ms.sort_by(f64::total_cmp);
        println!(
            "{:<27} 1080p {clock}: median {:.3} ms, p10 {:.3}, p90 {:.3}",
            s.name,
            ms[ms.len() / 2],
            ms[ms.len() / 10],
            ms[ms.len() * 9 / 10]
        );
    }
}
