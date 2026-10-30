//! Renders the default avatar's clips in the engine, sampled at evenly spaced times: one strip per
//! clip, a side view above a front three-quarter view (no window; needs a GPU adapter).
//!
//! ```text
//! cargo run --release -p metallic_render --example avatar_clips -- <output folder> [clip ...]
//! ```
//! With no clip names it renders the locomotion set and the emotes. Each clip is sampled 8 times
//! (a loop over one cycle, a one-shot from its first to its last frame) with the engine's own
//! skinning and shading, so it checks what the Blender review strips cannot: the glb as the game
//! loads it. The floor ticks are 1 stud apart and move with a gait's authored speed, so a planted
//! foot keeps its place against them.
use metallic_core::{Color3, Value, Vec3 as V3, World};
use metallic_render::glam::{Mat4, Vec3};
use metallic_render::metallic_asset::{Pose, SkinnedModel};
use metallic_render::{Camera, Quality, RenderContext, RenderSettings, Renderer, SkinnedInstance};
use std::error::Error;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const PANEL: (u32, u32) = (240, 360);
const SAMPLES: usize = 8;
/// Metres per stud.
const STUD: f32 = 0.28;
const DEFAULT_CLIPS: &[&str] = &[
    "idle",
    "idle_b",
    "walk_slow",
    "walk",
    "run",
    "jump",
    "fall",
    "land",
    "land_hard",
    "swim",
    "swim_idle",
    "climb",
    "cheer",
    "wave",
    "point",
    "dance",
    "sit",
];
/// Clips that play once (the rest loop).
const ONE_SHOTS: &[&str] = &["jump", "land", "land_hard", "wave", "point", "cheer"];
/// Authored speeds of the gaits, studs/s (the floor moves under them).
const GAITS: &[(&str, f32)] = &[("walk_slow", 6.0), ("walk", 16.0), ("run", 24.0)];

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(
        args.next()
            .ok_or("usage: avatar_clips <folder> [clip ...]")?,
    );
    let mut names: Vec<String> = args.collect();
    if names.is_empty() {
        names = DEFAULT_CLIPS.iter().map(|s| s.to_string()).collect();
    }
    std::fs::create_dir_all(&out)?;
    let glb = include_bytes!("../../../art/avatar/out/default_avatar.glb");
    let model = std::sync::Arc::new(SkinnedModel::from_glb(glb)?);
    let ctx = RenderContext::new_headless()?;
    let mut r = Renderer::with_settings(
        &ctx,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings {
            quality: Quality::High,
        },
    );
    let scale = 1.0 / STUD;
    let height = model.height() * scale;
    let mut pose = Pose::rest(&model.skeleton);
    let mut joints = Vec::new();
    for name in &names {
        let Some(clip) = model.clips.iter().find(|c| &c.name == name) else {
            eprintln!("no clip `{name}`, skipped");
            continue;
        };
        let speed = GAITS
            .iter()
            .find(|(n, _)| n == name)
            .map_or(0.0, |&(_, v)| v);
        // A loop's last key repeats its first: sample one cycle without it.
        let looped = !ONE_SHOTS.contains(&name.as_str());
        let times: Vec<f32> = (0..SAMPLES)
            .map(|i| {
                if looped {
                    clip.duration * i as f32 / SAMPLES as f32
                } else {
                    clip.duration * i as f32 / (SAMPLES - 1) as f32
                }
            })
            .collect();
        let mut rows: Vec<Vec<u8>> = Vec::new();
        // Side view from the avatar's left (it faces +Z, the lit side), then a front three-quarter view.
        for view in [Vec3::new(1.0, 0.05, 0.0), Vec3::new(0.42, 0.14, 0.9)] {
            let mut panels = Vec::new();
            for &t in &times {
                pose.locals.clone_from(&model.skeleton.rest);
                clip.sample(t, &mut pose);
                pose.skin_matrices(
                    &model.skeleton,
                    Mat4::from_scale(Vec3::splat(scale)),
                    &mut joints,
                );
                r.sync(&floor(speed * t)?);
                r.set_skinned(&[SkinnedInstance {
                    model: &model,
                    joints: &joints,
                    slot_colors: &[],
                    center: Vec3::new(0.0, height * 0.5, 0.0),
                    height,
                    detail_scale: scale,
                }]);
                let look = Vec3::new(0.0, height * 0.5, 0.0);
                let cam = Camera {
                    position: look + view.normalize() * 22.0,
                    look_at: look,
                    fov_deg: 22.0,
                };
                panels.push(r.capture(&cam, PANEL)?);
            }
            rows.push(join_horizontally(&panels, PANEL));
        }
        let (w, h) = (PANEL.0 * SAMPLES as u32, PANEL.1 * rows.len() as u32);
        let path = out.join(format!("{name}.png"));
        write_png(&path, (w, h), &rows.concat())?;
        println!("saved {} (t = {times:.3?} s)", path.display());
    }
    r.set_skinned(&[]);
    Ok(())
}

/// A light floor with dark ticks every stud along Z, shifted back by `travelled` studs (the
/// character walks toward +Z).
fn floor(travelled: f32) -> Result<World> {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").ok_or("no Workspace")?;
    let mut part = |size: V3, pos: V3, color: Color3| -> Result<()> {
        let p = w.create("Part", Some(ws))?;
        w.set(p, "Size", Value::Vector3(size))?;
        w.set(p, "Position", Value::Vector3(pos))?;
        w.set(p, "Color", Value::Color3(color))?;
        w.set(p, "Anchored", Value::Bool(true))?;
        Ok(())
    };
    part(
        V3::new(40.0, 1.0, 60.0),
        V3::new(0.0, -0.5, 0.0),
        Color3::new(205.0, 205.0, 200.0),
    )?;
    let shift = f64::from(travelled.rem_euclid(1.0));
    for i in -20..20 {
        part(
            V3::new(4.0, 0.04, 0.08),
            V3::new(0.0, 0.02, f64::from(i) - shift),
            Color3::new(60.0, 60.0, 60.0),
        )?;
    }
    Ok(w)
}

fn join_horizontally(panels: &[Vec<u8>], (w, h): (u32, u32)) -> Vec<u8> {
    let n = panels.len();
    let row = w as usize * 4;
    let mut out = vec![0u8; row * n * h as usize];
    for (i, panel) in panels.iter().enumerate() {
        for y in 0..h as usize {
            let at = y * row * n + i * row;
            out[at..at + row].copy_from_slice(&panel[y * row..(y + 1) * row]);
        }
    }
    out
}

fn write_png(path: &Path, (w, h): (u32, u32), rgba: &[u8]) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(())
}
