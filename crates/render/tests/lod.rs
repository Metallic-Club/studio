//! Levels of detail on a real adapter (#80): rows of balls and cylinders receding to 360 m, drawn with levels and at
//! full detail, must look the same while the levels cut the triangles; both instance paths draw the same levels; the
//! sun's shadows of coarser casters put no acne on the lit side of the parts receiving them, at every preset; and the
//! debug view colours parts by level. Skips when the machine has no adapter (see `support::gpu`).
//!
//! The acne check is not blind: with the cascades' casters drawn at full detail instead (a shadow finer than the
//! coarse surface receiving it), it counted 454 of 31,336 part pixels darker at Medium, against none.
//!
//! `METALLIC_SAVE_IMAGES=1` saves the images to the test's temporary folder.

mod support;

use std::path::PathBuf;

use metallic_core::glam::Vec3;
use metallic_core::{CFrame, Id, Value};
use metallic_render::scene::{InstanceBinding, RenderComponents};
use metallic_render::{
    CameraReader, ContextOptions, DrawStats, GpuScene, LightingReader, LodSettings, OffscreenTarget, PassId, PassSet,
    RenderContext, Renderer, RendererSettings, ShadowSettings, wgpu,
};
use support::scenes::{self, Game, Look};
use support::{gpu, gpu_with, read_back_rgba8};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 270;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

fn lighting_service(game: &Game) -> Id {
    let class = game.dm.registry().class_id("Lighting").expect("Lighting");
    game.dm.service(class).expect("a Lighting")
}

/// A floor and (with `parts`), every 6 m from 6 m to 360 m ahead of the camera, a row of five parts across: balls
/// 3 m across and cylinders 6 m long and 2 m across, lying across the view, in turn. The camera stands 3 m up
/// looking along the rows; the sun is at `clock` hours.
fn rows(clock: f64, parts: bool) -> Game {
    let mut game = Game::new();
    game.part(
        Look::new("Block", "Concrete", [160, 158, 150]),
        Vec3::new(200.0, 1.0, 900.0),
        CFrame::new(Vec3::new(0.0, -0.5, -400.0)),
    );
    for row in (1..=60).filter(|_| parts) {
        let z = -6.0 * row as f32;
        for column in -2..=2 {
            let x = column as f32 * 9.0;
            if (row + column) % 2 == 0 {
                game.part(
                    Look::new("Ball", "SmoothPlastic", [13, 105, 172]),
                    Vec3::splat(3.0),
                    CFrame::new(Vec3::new(x, 1.5, z)),
                );
            } else {
                game.part(
                    Look::new("Cylinder", "Plastic", [196, 120, 40]),
                    Vec3::new(6.0, 2.0, 2.0),
                    CFrame::new(Vec3::new(x, 1.0, z)),
                );
            }
        }
    }
    game.set_camera(
        CFrame::look_at(Vec3::new(0.0, 3.0, 0.0), Vec3::new(0.0, 0.0, -40.0), Vec3::Y),
        70.0,
    );
    game.dm
        .set_by_name(lighting_service(&game), "ClockTime", Value::Number(clock))
        .expect("ClockTime");
    game
}

/// How a frame of [`rows`] is drawn.
#[derive(Clone, Copy)]
struct Look2 {
    lod: LodSettings,
    shadows: ShadowSettings,
    tint: bool,
}

impl Look2 {
    fn levels(lod: LodSettings) -> Self {
        Self {
            lod,
            shadows: ShadowSettings::default(),
            tint: false,
        }
    }
}

/// Renders `game` (three frames, so everything is uploaded and every level chosen) as `look` says, reads the image
/// back and returns it with the last frame's draw statistics, those of its shadows from the first (the shadow maps
/// are drawn once and kept while nothing changes).
fn render(context: &RenderContext, game: &mut Game, look: Look2) -> (Vec<[u8; 4]>, DrawStats) {
    let target = OffscreenTarget::new(context, WIDTH, HEIGHT, FORMAT).expect("a target");
    let mut renderer = Renderer::new(
        context,
        RendererSettings {
            shadows: look.shadows,
            lod: look.lod,
            gpu_timing: false,
            // The debug view's colours are told apart by hue, which bloom would mix with their surroundings.
            passes: if look.tint {
                PassSet::default().without(PassId::Bloom)
            } else {
                PassSet::default()
            },
            ..RendererSettings::new(FORMAT, WIDTH, HEIGHT)
        },
    )
    .expect("a renderer");
    renderer.set_lod_tint(look.tint);
    let mut scene = GpuScene::new(
        context,
        &mut game.dm,
        Some(game.workspace),
        &RenderComponents::builtin(),
    )
    .expect("a scene");
    let lighting = LightingReader::new().read(&game.dm, lighting_service(game));
    renderer.set_lighting(lighting);
    let camera = CameraReader::new().read(&game.dm, game.workspace).expect("a camera");
    let mut first = None;
    for _ in 0..3 {
        scene.set_camera_position(&game.dm, camera.position);
        scene.sync(&mut game.dm);
        let submission = renderer.render_scene(target.view(), &mut scene, Some(&camera));
        let _ = context.device().poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        });
        first.get_or_insert(renderer.draw_stats());
    }
    let first = first.unwrap_or_default();
    let stats = DrawStats {
        shadow_draws: first.shadow_draws,
        shadow_instances: first.shadow_instances,
        shadow_triangles: first.shadow_triangles,
        ..renderer.draw_stats()
    };
    (read_back_rgba8(context, target.texture()), stats)
}

/// Saves `pixels` to the test's temporary folder as `name`.png when `METALLIC_SAVE_IMAGES=1`, for looking at.
fn save_image(name: &str, pixels: &[[u8; 4]]) {
    if std::env::var("METALLIC_SAVE_IMAGES").is_ok_and(|v| v == "1") {
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
        scenes::write_png(&out, WIDTH, HEIGHT, pixels);
        eprintln!("wrote {}", out.display());
    }
}

/// The brightness of a pixel: the mean of its three channels.
fn level(p: [u8; 4]) -> f32 {
    (f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2])) / 3.0
}

#[test]
fn far_parts_lose_triangles_not_looks() {
    let Some(context) = gpu() else { return };
    let (full, full_stats) = render(&context, &mut rows(14.0, true), Look2::levels(LodSettings::OFF));
    save_image("lod_off", &full);
    assert_eq!(full_stats.level_instances[1..], [0, 0, 0]);
    // Per preset, the share of pixels that may differ by more than 12 levels: the outlines stray by under a quarter
    // of a pixel at High (the default), half at Medium and one at Low, so a few edge pixels flip, more the coarser.
    for (name, lod, max_over) in [
        ("lod_high", LodSettings::HIGH, 0.004),
        ("lod_medium", LodSettings::MEDIUM, 0.012),
        ("lod_low", LodSettings::LOW, 0.03),
    ] {
        let (pixels, stats) = render(&context, &mut rows(14.0, true), Look2::levels(lod));
        save_image(name, &pixels);
        let diff = scenes::compare(&pixels, &full, 12);
        eprintln!(
            "{name}: {} → {} triangles, {} → {} shadow triangles, levels {:?}, {} opaque draws; against full detail \
             {diff:?}",
            full_stats.opaque_triangles,
            stats.opaque_triangles,
            full_stats.shadow_triangles,
            stats.shadow_triangles,
            stats.level_instances,
            stats.opaque_draws
        );
        assert_eq!(stats.opaque_instances, full_stats.opaque_instances, "the same parts");
        assert!(
            stats.opaque_triangles * 2 < full_stats.opaque_triangles,
            "{name}: far parts are coarser"
        );
        assert!(stats.shadow_triangles < full_stats.shadow_triangles, "{name}");
        assert!(
            stats.level_instances[1..].iter().all(|&n| n > 0),
            "{name}: every level is used"
        );
        assert!(diff.over < max_over, "{name}: {diff:?}");
        assert!(diff.mean < 50.0 * max_over, "{name}: {diff:?}");
        // One draw per kind and level: blocks, balls at four levels, cylinders at three.
        assert!(stats.opaque_draws <= 8, "{name}: {}", stats.opaque_draws);
    }
}

#[test]
fn both_instance_paths_draw_the_same_levels() {
    let Some(primary) = gpu() else { return };
    let Some(fallback) = gpu_with(&ContextOptions {
        force_vertex_instances: true,
        ..ContextOptions::default()
    }) else {
        return;
    };
    assert_eq!(InstanceBinding::for_context(&fallback), InstanceBinding::Vertex);
    let look = Look2::levels(LodSettings::default());
    let (storage, storage_stats) = render(&primary, &mut rows(14.0, true), look);
    let (vertex, vertex_stats) = render(&fallback, &mut rows(14.0, true), look);
    assert_eq!(vertex_stats.level_instances, storage_stats.level_instances);
    assert_eq!(vertex_stats.opaque_draws, storage_stats.opaque_draws);
    assert_eq!(vertex_stats.shadow_triangles, storage_stats.shadow_triangles);
    let diff = scenes::compare(&vertex, &storage, 1);
    eprintln!("fallback against storage: {diff:?}");
    assert!(diff.max <= 2, "{diff:?}");
}

#[test]
fn coarser_shadows_put_no_acne_on_their_receivers() {
    let Some(context) = gpu() else { return };
    // Noon: each part's shadow falls on the floor beside it, never on another part, so on the parts' own pixels the
    // sun's shadow can only darken a lit surface by shadowing itself.
    let parts_only = |look: Look2| render(&context, &mut rows(12.0, true), look).0;
    let unshadowed = Look2 {
        shadows: ShadowSettings::OFF,
        ..Look2::levels(LodSettings::default())
    };
    let (bare, _) = render(&context, &mut rows(12.0, false), unshadowed);
    for (name, lod) in [
        ("medium", LodSettings::MEDIUM),
        ("low", LodSettings::LOW),
        ("high", LodSettings::HIGH),
    ] {
        let lit = parts_only(Look2 {
            shadows: ShadowSettings::OFF,
            ..Look2::levels(lod)
        });
        let shadowed = parts_only(Look2::levels(lod));
        // The parts' pixels: where the image without shadows differs from the bare floor.
        let mask: Vec<bool> = lit
            .iter()
            .zip(&bare)
            .map(|(a, b)| (level(*a) - level(*b)).abs() > 6.0)
            .collect();
        let on_parts = mask.iter().filter(|&&m| m).count();
        let darker = lit
            .iter()
            .zip(&shadowed)
            .zip(&mask)
            .filter(|((a, b), m)| **m && level(**a) - level(**b) > 3.0)
            .count();
        eprintln!("{name}: {darker} of {on_parts} part pixels darker with the sun's shadows");
        if lod == LodSettings::default() {
            save_image("lod_acne_shadowed", &shadowed);
        }
        assert!(on_parts > 1000, "{name}: the parts cover the view ({on_parts} pixels)");
        assert!(
            (darker as f64) < 0.002 * on_parts as f64,
            "{name}: {darker} of {on_parts} part pixels darker"
        );
    }
}

#[test]
fn the_debug_view_colours_parts_by_level() {
    let Some(context) = gpu() else { return };
    // Low, so that many far balls are at the coarsest level.
    let look = Look2 {
        tint: true,
        ..Look2::levels(LodSettings::LOW)
    };
    let (pixels, stats) = render(&context, &mut rows(14.0, true), look);
    save_image("lod_tint", &pixels);
    assert!(
        stats.level_instances.iter().all(|&n| n > 0),
        "{:?}",
        stats.level_instances
    );
    // Green (full detail) near, then yellow, orange and red (the coarsest balls) farther and farther; the floor
    // grey. The tints are linear colours, written sRGB-encoded.
    let hue = |pred: fn(f32, f32, f32) -> bool| {
        pixels
            .iter()
            .filter(|p| pred(f32::from(p[0]), f32::from(p[1]), f32::from(p[2])))
            .count()
    };
    let green = hue(|r, g, b| g > 2.0 * r && g > 2.0 * b);
    let yellow = hue(|r, g, b| (r - g).abs() < 0.15 * r && g > 2.0 * b);
    let orange = hue(|r, g, b| r > 1.4 * g && g > 2.0 * b && r < 3.0 * g);
    let red = hue(|r, g, b| r > 3.0 * g && r > 3.0 * b);
    eprintln!("green {green}, yellow {yellow}, orange {orange}, red {red}");
    assert!(
        green > 100 && yellow > 20 && orange > 20 && red > 5,
        "green {green}, yellow {yellow}, orange {orange}, red {red}"
    );
    // Off, nothing is tinted.
    let (plain, _) = render(&context, &mut rows(14.0, true), Look2::levels(LodSettings::LOW));
    assert!(scenes::compare(&plain, &pixels, 12).over > 0.01);
}
