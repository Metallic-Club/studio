//! Renders the default avatar in the engine for review (no window; needs a GPU adapter).
//!
//! ```text
//! cargo run --release -p metallic_runtime --example avatar_shots -- <output folder>
//! ```
//! Writes, with deterministic 1/60 s ticks and scripted input:
//! - `idle_start_pad.png`, `run_game_camera.png`, `jump_side.png`: the Parkour template from the
//!   start pad (game camera, then a side view in the air);
//! - `closeup_shading.png`: a close 3/4 view of the idle avatar (shading, shadows, outlines);
//! - `strip_run_side.png`: eight side views of the run on a flat floor, 1/15 s apart (frames 0,
//!   2, ... 14 at 30 fps), laid out like `art/avatar/renders/phaseB/strip_run_side_clay_v2.png`.
//!   The dark floor ticks are 1 stud apart and fixed in the world.
use metallic_core::{Color3, Value, Vec3, World};
use metallic_render::glam::Vec3 as GVec3;
use metallic_render::{Camera, Quality, RenderContext, RenderSettings, Renderer};
use metallic_runtime::{FrameInput, Game, StdoutSink, TICK};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::rc::Rc;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const SHOT: (u32, u32) = (1280, 720);
const PANEL: (u32, u32) = (340, 520);

fn main() -> Result<()> {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: avatar_shots <folder>")?,
    );
    std::fs::create_dir_all(&out)?;
    let ctx = RenderContext::new_headless()?;
    let mut r = Renderer::with_settings(
        &ctx,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings {
            quality: Quality::High,
        },
    );
    let parkour = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/parkour");

    // Parkour: idle on the start pad, then run forward and jump.
    let mut g = Game::open(&parkour, None, Rc::new(StdoutSink))?;
    g.start()?;
    run(&mut g, FrameInput::default(), 1.5);
    save(
        &mut r,
        &g,
        &g.camera(),
        SHOT,
        &out.join("idle_start_pad.png"),
    )?;
    let forward = FrameInput {
        movement: [0.0, 1.0],
        run: true,
        ..FrameInput::default()
    };
    run(&mut g, forward, 0.5);
    save(
        &mut r,
        &g,
        &g.camera(),
        SHOT,
        &out.join("run_game_camera.png"),
    )?;
    // A jump from the island, before the arch (a fresh game so it starts from the pad).
    let mut g = Game::open(&parkour, None, Rc::new(StdoutSink))?;
    g.start()?;
    run(&mut g, FrameInput::default(), 1.0);
    run(&mut g, forward, 0.25);
    let leap = FrameInput {
        jump: true,
        ..forward
    };
    run(&mut g, leap, TICK);
    run(&mut g, forward, 0.2);
    let root = root_of(&g)?;
    let side = Camera {
        position: root + GVec3::new(10.0, 0.0, -3.0),
        look_at: root + GVec3::new(0.0, -0.8, -1.0),
        fov_deg: 45.0,
    };
    save(&mut r, &g, &side, SHOT, &out.join("jump_side.png"))?;

    // Close-up of the idle avatar from the front three-quarter side (it faces -Z).
    let mut g = Game::open(&parkour, None, Rc::new(StdoutSink))?;
    g.start()?;
    run(&mut g, FrameInput::default(), 1.5);
    let root = root_of(&g)?;
    let close = Camera {
        position: root + GVec3::new(4.0, 0.6, -6.0),
        look_at: root + GVec3::new(0.0, -0.6, 0.0),
        fov_deg: 50.0,
    };
    save(&mut r, &g, &close, SHOT, &out.join("closeup_shading.png"))?;

    // Run strip on a flat floor, camera following from the +X side (the avatar runs toward -Z,
    // which is screen right).
    let mut g = Game::from_world("strip".into(), floor_with_ticks()?, Rc::new(StdoutSink));
    g.start()?;
    run(&mut g, FrameInput::default(), 0.5);
    run(&mut g, forward, 1.0);
    let mut panels = Vec::new();
    for _ in 0..8 {
        let root = root_of(&g)?;
        let cam = Camera {
            position: root + GVec3::new(12.0, -0.25, 0.0),
            look_at: root + GVec3::new(0.0, -0.25, 0.0),
            fov_deg: 35.0,
        };
        g.sync_renderer(&mut r);
        panels.push(r.capture(&cam, PANEL)?);
        run(&mut g, forward, 1.0 / 15.0);
    }
    let (w, h) = PANEL;
    let n = panels.len() as u32;
    let mut strip = vec![0u8; (w * n * h * 4) as usize];
    for (i, panel) in panels.iter().enumerate() {
        for y in 0..h as usize {
            let src = &panel[y * w as usize * 4..(y + 1) * w as usize * 4];
            let at = (y * (w * n) as usize + i * w as usize) * 4;
            strip[at..at + src.len()].copy_from_slice(src);
        }
    }
    write_png(&out.join("strip_run_side.png"), (w * n, h), &strip)?;
    g.stop();
    Ok(())
}

/// Advances `seconds` of game time in fixed ticks with `input` held.
fn run(g: &mut Game, input: FrameInput, seconds: f64) {
    for _ in 0..(seconds / TICK).round().max(1.0) as usize {
        g.frame(&input, TICK);
    }
}

fn root_of(g: &Game) -> Result<GVec3> {
    let p = g.root_position().ok_or("no character")?;
    Ok(GVec3::new(p.x as f32, p.y as f32, p.z as f32))
}

fn save(r: &mut Renderer, g: &Game, cam: &Camera, size: (u32, u32), path: &Path) -> Result<()> {
    g.sync_renderer(r);
    let px = r.capture(cam, size)?;
    write_png(path, size, &px)?;
    println!("saved {}", path.display());
    Ok(())
}

fn write_png(path: &Path, (w, h): (u32, u32), rgba: &[u8]) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(())
}

/// A light grey floor (top at y = 0) with dark ticks every stud along the run (the Z axis).
fn floor_with_ticks() -> Result<World> {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").ok_or("no Workspace")?;
    let mut part = |size: Vec3, pos: Vec3, color: Color3| -> Result<()> {
        let p = w.create("Part", Some(ws))?;
        w.set(p, "Size", Value::Vector3(size))?;
        w.set(p, "Position", Value::Vector3(pos))?;
        w.set(p, "Color", Value::Color3(color))?;
        w.set(p, "Anchored", Value::Bool(true))?;
        Ok(())
    };
    part(
        Vec3::new(60.0, 1.0, 400.0),
        Vec3::new(0.0, -0.5, -150.0),
        Color3::new(205.0, 205.0, 200.0),
    )?;
    for i in 0..120 {
        part(
            Vec3::new(1.2, 0.04, 0.08),
            Vec3::new(1.6, 0.02, -(i as f64)),
            Color3::new(60.0, 60.0, 60.0),
        )?;
    }
    Ok(w)
}
