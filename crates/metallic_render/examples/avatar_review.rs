//! Renders the default avatar in the engine for a model review: real lighting and shadows in the
//! avatar's toon style (or the standard style with `--standard`), nothing but a ground plate. No window; needs a GPU adapter (or a software one).
//!
//! ```text
//! cargo run --release -p metallic_render --example avatar_review -- <avatar.glb> <output folder> [--standard]
//! ```
//! Writes, all 480 x 800:
//! - `engine_<view>_lod<n>.png` for the views front, front34, side, back and back34 (the concept
//!   sheet's five body views) at LOD0, LOD1 and LOD2, in the rest pose with curled fingers;
//! - `engine_<pose>.png` for the concept's three poses (`pose_hand_on_hip`, `pose_run`,
//!   `pose_sit_back`), LOD0, seen from about the drawing's angle.
//!
//! The figure turns on a turntable and the camera and sun stay put, so every view is lit from
//! the same screen direction, as on the concept sheet. The camera is far away with a narrow
//! field of view (close to the sheet's flat projection). LOD1 and LOD2 are forced by passing a
//! smaller figure height to the level-of-detail selection; the figure is drawn at full size.
use metallic_core::{Color3, Value, Vec3 as V3, World};
use metallic_render::glam::{Mat4, Quat, Vec3};
use metallic_render::metallic_asset::{Pose, SkinnedModel};
use metallic_render::{
    Camera, Quality, RenderContext, RenderSettings, Renderer, SkinStyle, SkinnedInstance,
};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const SIZE: (u32, u32) = (480, 800);
/// Studs per metre (1 stud = 0.28 m).
const STUDS_PER_M: f32 = 1.0 / 0.28;
/// Avatar height in studs.
const HEIGHT: f32 = 5.5;
const FOV_DEG: f32 = 18.0;

/// View name and turntable angle in degrees (the sheet's azimuths: positive turns the figure's
/// right side toward the camera, as render_phaseB.py's azimuth does).
const VIEWS: [(&str, f32); 5] = [
    ("front", 0.0),
    ("front34", 30.0),
    ("side", 90.0),
    ("back", 180.0),
    ("back34", 225.0),
];
/// Concept poses: clip, turntable angle, height of the framed middle and framed height (studs).
const POSES: [(&str, f32, f32, f32); 3] = [
    ("pose_hand_on_hip", -18.0, HEIGHT * 0.5, HEIGHT * 1.36),
    ("pose_run", 55.0, HEIGHT * 0.5, HEIGHT * 1.36),
    ("pose_sit_back", 35.0, HEIGHT * 0.36, HEIGHT * 1.05),
];

fn main() -> Result<()> {
    let standard = std::env::args().any(|a| a == "--standard");
    let style = if standard {
        SkinStyle::Standard
    } else {
        SkinStyle::Toon
    };
    let mut args = std::env::args().skip(1).filter(|a| !a.starts_with("--"));
    let glb = PathBuf::from(args.next().ok_or("usage: avatar_review <glb> <folder>")?);
    let out = PathBuf::from(args.next().ok_or("usage: avatar_review <glb> <folder>")?);
    std::fs::create_dir_all(&out)?;
    let model = Arc::new(SkinnedModel::from_glb(&std::fs::read(&glb)?)?);

    let ctx = RenderContext::new_headless()?;
    let mut r = Renderer::with_settings(
        &ctx,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings {
            quality: Quality::High,
        },
    );
    // Key light from the upper left of the screen and a little in front, like the sheet.
    r.sky.to_sun = Vec3::new(-0.55, 0.75, 0.6).normalize();
    r.sync(&ground());

    let camera = framing(HEIGHT * 0.5, HEIGHT * 1.24);

    let fists = model.clip("pose_run").ok_or("no pose_run clip")?;
    for (name, angle) in VIEWS {
        let pose = rest_with_fists(&model, fists);
        for (lod, lod_height) in [(0, HEIGHT), (1, 1.0), (2, 0.4)] {
            draw(style, &mut r, &model, &pose, angle, lod_height);
            let px = r.capture(&camera, SIZE)?;
            assert_eq!(r.skinned_lods(), [lod], "LOD forcing");
            write_png(&out.join(format!("engine_{name}_lod{lod}.png")), SIZE, &px)?;
        }
    }
    for (clip, angle, mid, framed) in POSES {
        let Some(c) = model.clip(clip) else {
            eprintln!("no clip {clip}, skipped");
            continue;
        };
        let mut pose = Pose::rest(&model.skeleton);
        c.sample(0.0, &mut pose);
        draw(style, &mut r, &model, &pose, angle, HEIGHT);
        let px = r.capture(&framing(mid, framed), SIZE)?;
        write_png(&out.join(format!("engine_{clip}.png")), SIZE, &px)?;
    }
    println!("wrote {}", out.display());
    Ok(())
}

/// The rest pose (the model's own stance) with the fingers and thumbs closed into fists as in
/// `fists` (a clip whose first frame has closed hands), the way the concept sheet draws the
/// turnaround.
fn rest_with_fists(model: &SkinnedModel, fists: &metallic_render::metallic_asset::Clip) -> Pose {
    let mut curled = Pose::rest(&model.skeleton);
    fists.sample(0.0, &mut curled);
    let mut pose = Pose::rest(&model.skeleton);
    for (j, name) in model.skeleton.names.iter().enumerate() {
        if ["Thumb", "Index", "Middle", "Ring", "Little"]
            .iter()
            .any(|f| name.contains(f))
        {
            pose.locals[j] = curled.locals[j];
        }
    }
    pose
}

/// A camera slightly above the middle that fits `framed` studs into the frame height.
fn framing(mid_y: f32, framed: f32) -> Camera {
    let distance = (framed * 0.5) / (FOV_DEG.to_radians() * 0.5).tan();
    let mid = Vec3::new(0.0, mid_y, 0.0);
    Camera {
        position: mid + Vec3::new(0.0, distance * 0.04, distance),
        look_at: mid,
        fov_deg: FOV_DEG,
    }
}

/// A pale ground plate so the figure has a floor and a cast shadow.
fn ground() -> World {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").expect("workspace");
    let plate = w.create("Part", Some(ws)).expect("part");
    w.set(plate, "Size", Value::Vector3(V3::new(60.0, 1.0, 60.0)))
        .expect("size");
    w.set(plate, "Position", Value::Vector3(V3::new(0.0, -0.5, 0.0)))
        .expect("position");
    w.set(
        plate,
        "Color",
        Value::Color3(Color3::new(200.0, 196.0, 188.0)),
    )
    .expect("colour");
    w
}

fn draw(
    style: SkinStyle,
    r: &mut Renderer,
    model: &Arc<SkinnedModel>,
    pose: &Pose,
    angle_deg: f32,
    lod_height: f32,
) {
    let m =
        Mat4::from_rotation_translation(Quat::from_rotation_y(angle_deg.to_radians()), Vec3::ZERO)
            * Mat4::from_scale(Vec3::splat(STUDS_PER_M));
    let mut joints = Vec::new();
    pose.skin_matrices(&model.skeleton, m, &mut joints);
    r.set_skinned(&[SkinnedInstance {
        model,
        joints: &joints,
        slot_colors: &[],
        center: Vec3::new(0.0, HEIGHT * 0.5, 0.0),
        height: lod_height,
        detail_scale: STUDS_PER_M,
        style,
    }]);
}

fn write_png(path: &Path, (w, h): (u32, u32), rgba: &[u8]) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(())
}
