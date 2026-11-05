//! The HDR post chain on a real adapter (#85): what a frame plans and reports, MLAA and MSAA smoothing edges, bloom
//! glowing around a lamp at night at every effect quality, the grade, switching anti-aliasing and effects between
//! frames, and both instance paths finishing the same image. Skips when the machine has no adapter (see
//! `support::gpu`). `METALLIC_SAVE_IMAGES=1` saves the images to the test's temporary folder.

mod support;

use std::path::PathBuf;

use metallic_core::glam::{Quat, Vec3};
use metallic_core::{CFrame, Value};
use metallic_render::post::{BloomSettings, PostSettings};
use metallic_render::resolution::ResolutionSettings;
use metallic_render::scene::{InstanceBinding, RenderComponents};
use metallic_render::{
    AntiAliasing, CameraReader, ContextOptions, GpuScene, LightingReader, OffscreenTarget, PassId, PassSet, PostStats,
    RenderContext, Renderer, RendererSettings, wgpu,
};
use support::scenes::{self, Game, Look};
use support::{gpu, gpu_with, read_back_rgba8};

const SIZE: u32 = 256;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

fn save(name: &str, pixels: &[[u8; 4]]) {
    if std::env::var("METALLIC_SAVE_IMAGES").is_ok_and(|v| v == "1") {
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("post_{name}.png"));
        scenes::write_png(&path, SIZE, SIZE, pixels);
        eprintln!("wrote {}", path.display());
    }
}

/// Draws `game` for a few frames with `settings` changed by `change`, and reads the image back.
fn render(
    context: &RenderContext,
    game: &mut Game,
    change: impl FnOnce(&mut RendererSettings),
) -> (Vec<[u8; 4]>, PostStats) {
    render_with(context, game, change, |_| {})
}

/// [`render`], with `prepare` changing the renderer after the first frame.
fn render_with(
    context: &RenderContext,
    game: &mut Game,
    change: impl FnOnce(&mut RendererSettings),
    prepare: impl FnOnce(&mut Renderer),
) -> (Vec<[u8; 4]>, PostStats) {
    let mut settings = RendererSettings {
        gpu_timing: false,
        ..RendererSettings::new(FORMAT, SIZE, SIZE)
    };
    change(&mut settings);
    let target = OffscreenTarget::new(context, SIZE, SIZE, FORMAT).expect("a target");
    let mut renderer = Renderer::new(context, settings).expect("a renderer");
    renderer.wait_for_pipelines();
    let mut prepare = Some(prepare);
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
    let mut camera = CameraReader::new();
    for _ in 0..3 {
        let camera = camera.read(&game.dm, game.workspace);
        if let Some(camera) = camera {
            scene.set_camera_position(&game.dm, camera.position);
        }
        scene.sync(&mut game.dm);
        renderer.set_lighting(LightingReader::new().read(&game.dm, lighting));
        renderer.render_scene(target.view(), &mut scene, camera.as_ref());
        // After the first frame, which fills the material table from the game.
        if let Some(prepare) = prepare.take() {
            prepare(&mut renderer);
        }
    }
    (read_back_rgba8(context, target.texture()), renderer.post_stats())
}

fn demo() -> Game {
    let mut game = Game::new();
    scenes::demo(&mut game, Vec3::ZERO);
    game
}

/// A white neon block turned 20° about the view axis, on black: long slanted edges and nothing else.
fn slanted_block() -> Game {
    let mut game = Game::new();
    game.part(
        Look::new("Block", "Neon", [255, 255, 255]),
        Vec3::new(6.0, 6.0, 1.0),
        CFrame::from_parts(Vec3::ZERO, Quat::from_rotation_z(20f32.to_radians())),
    );
    game.set_camera(CFrame::look_at(Vec3::new(0.0, 0.0, 12.0), Vec3::ZERO, Vec3::Y), 50.0);
    game
}

/// The block's edges alone: no sky, no bloom (which would blur them), black around.
fn edges_only(settings: &mut RendererSettings) {
    settings.passes = PassSet::default().without(PassId::Sky).without(PassId::Bloom);
}

/// How many pixels are neither the background nor the block: the steps of the edges, smoothed.
fn in_between(pixels: &[[u8; 4]]) -> usize {
    pixels
        .iter()
        .filter(|p| (24..=216).contains(&((u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])) / 3)))
        .count()
}

#[test]
fn a_frame_reports_its_passes() {
    let Some(context) = gpu() else { return };
    let caps = context.capabilities();
    let hdr = metallic_render::post::hdr_format(&context);
    eprintln!(
        "adapter: {} ({:?}): HDR scene colour {hdr:?}",
        caps.adapter_name, caps.backend
    );
    let (_, stats) = render(&context, &mut demo(), |_| {});
    eprintln!("default: {stats:?}");
    assert!(stats.finish && !stats.mlaa);
    assert_eq!(stats.scene_format, hdr);
    assert_eq!(stats.bloom_levels as usize, BloomSettings::HIGH.levels(SIZE));
    // Opaque, sky, transparent and the finish pass, which draws UI (#97); bloom's chain at half the size and below,
    // one compute pass where the device has compute shaders (#145), else a render pass per step.
    assert_eq!(stats.full_screen_passes, 4);
    let render_chain = (3 + 2 * stats.bloom_levels - 1 + 1, 0);
    if metallic_render::post::bloom_compute_format(&context).is_some() {
        assert_eq!((stats.render_passes, stats.compute_passes), (4, 1));
    } else {
        assert_eq!((stats.render_passes, stats.compute_passes), render_chain);
    }
    let (_, passes) = render(&context, &mut demo(), |s| s.post.bloom.compute = false);
    assert_eq!((passes.render_passes, passes.compute_passes), render_chain);
    let (_, mlaa) = render(&context, &mut demo(), |s| s.passes = s.passes.with(PassId::Mlaa));
    assert!(mlaa.mlaa);
    assert_eq!(mlaa.full_screen_passes, 5);
    let (_, merged) = render(&context, &mut demo(), |s| s.passes = s.passes.with(PassId::Scene));
    assert_eq!(merged.full_screen_passes, 2, "the scene, and the finish pass with UI");
    assert_eq!(merged.render_passes, stats.render_passes - 2);
    let (_, low) = render(&context, &mut demo(), |s| s.post = PostSettings::LOW);
    assert_eq!(low.bloom_levels as usize, BloomSettings::LOW.levels(SIZE));
    assert!(low.bloom_levels < stats.bloom_levels, "Low skips the finest level");
    // Before the post chain: the scene straight into the output.
    let (_, direct) = render(&context, &mut demo(), |s| {
        s.passes = PassSet::default().without(PassId::Post);
    });
    assert!(!direct.finish && direct.bloom_levels == 0);
    assert_eq!(direct.scene_format, FORMAT);
    assert_eq!((direct.full_screen_passes, direct.render_passes), (4, 4));
}

/// Merging the scene passes into one render pass (#97) changes where depth and colour live between them, never what
/// is drawn: each preset's frame layout (MLAA, MSAA, a render scale, a depth prepass, with the post chain or without
/// it) draws the same image merged as apart, to the bit, with the overlay drawn in the last pass.
#[test]
fn merged_scene_passes_draw_the_same_image() {
    let Some(context) = gpu() else { return };
    let caps = context.capabilities();
    eprintln!("adapter: {} ({:?})", caps.adapter_name, caps.backend);
    let counts = Renderer::new(&context, RendererSettings::new(FORMAT, SIZE, SIZE))
        .expect("a renderer")
        .msaa_sample_counts();
    let samples = match AntiAliasing::Msaa(4).supported(counts) {
        AntiAliasing::Msaa(samples) => samples,
        _ => 1,
    };
    let chain = PassSet::default();
    let direct = PassSet::default().without(PassId::Post).without(PassId::Bloom);
    let layouts = [
        ("default", chain, 1, 1.0),
        ("mlaa (medium)", chain.with(PassId::Mlaa), 1, 1.0),
        ("mlaa at 75% (low)", chain.with(PassId::Mlaa), 1, 0.75),
        ("msaa (high)", chain, samples, 1.0),
        ("msaa at 75%", chain, samples, 0.75),
        ("depth prepass", chain.with(PassId::DepthPrepass), 1, 1.0),
        (
            "depth prepass with msaa",
            chain.with(PassId::DepthPrepass),
            samples,
            1.0,
        ),
        ("no post chain", direct, 1, 1.0),
        ("no post chain, msaa", direct, samples, 1.0),
        ("no post chain at 75%", direct, 1, 0.75),
    ];
    for (name, passes, msaa, scale) in layouts {
        let draw = |merged: bool| {
            render_with(
                &context,
                &mut demo(),
                |s| {
                    s.passes = passes.with_if(PassId::Scene, merged);
                    s.msaa_samples = msaa;
                    s.resolution = ResolutionSettings::fixed(scale);
                },
                |r| {
                    r.set_overlay_enabled(true);
                    r.overlay_mut().text(4.0, 4.0, 2.0, [255, 255, 0, 255], "MERGED?");
                    r.wait_for_pipelines();
                },
            )
        };
        let (apart, apart_stats) = draw(false);
        let (merged, merged_stats) = draw(true);
        let diff = scenes::compare(&apart, &merged, 0);
        eprintln!(
            "{name}: render passes {} apart, {} merged; {diff:?}",
            apart_stats.render_passes, merged_stats.render_passes
        );
        // Opaque, sky and transparent in one (a depth prepass stays apart).
        assert_eq!(merged_stats.render_passes, apart_stats.render_passes - 2);
        if diff.max != 0 {
            let file = name.replace([' ', '(', ')', '%', ','], "_");
            save(&format!("merged_{file}_apart"), &apart);
            save(&format!("merged_{file}_merged"), &merged);
        }
        assert_eq!(
            diff.max, 0,
            "{name}: merging the scene passes changed the image: {diff:?}"
        );
    }
}

#[test]
fn mlaa_and_msaa_smooth_edges() {
    let Some(context) = gpu() else { return };
    let (off, _) = render(&context, &mut slanted_block(), edges_only);
    let (mlaa, _) = render(&context, &mut slanted_block(), |s| {
        edges_only(s);
        s.passes = s.passes.with(PassId::Mlaa);
    });
    save("edges_off", &off);
    save("edges_mlaa", &mlaa);
    let (steps, smoothed) = (in_between(&off), in_between(&mlaa));
    eprintln!("pixels between block and background: off {steps}, MLAA {smoothed}");
    assert!(
        smoothed > 3 * steps.max(1),
        "MLAA blends the edges: {steps} -> {smoothed}"
    );
    // Away from the edges nothing changes.
    let diff = scenes::compare(&off, &mlaa, 8);
    assert!(diff.over < 0.03, "{diff:?}");
    let counts = Renderer::new(&context, RendererSettings::new(FORMAT, SIZE, SIZE))
        .expect("a renderer")
        .msaa_sample_counts();
    if let AntiAliasing::Msaa(samples) = AntiAliasing::Msaa(4).supported(counts) {
        let (msaa, stats) = render(&context, &mut slanted_block(), |s| {
            edges_only(s);
            s.msaa_samples = samples;
        });
        save("edges_msaa", &msaa);
        let multisampled = in_between(&msaa);
        eprintln!("MSAA {samples}x: {multisampled}");
        assert_eq!(stats.msaa_samples, samples);
        assert!(
            multisampled > 3 * steps.max(1),
            "MSAA resolves the edges: {steps} -> {multisampled}"
        );
    }
}

/// A neon lamp over a dark floor at night.
fn night_lamp() -> Game {
    let mut game = Game::new();
    game.part(
        Look::new("Block", "Concrete", [90, 90, 90]),
        Vec3::new(60.0, 1.0, 60.0),
        CFrame::new(Vec3::new(0.0, -0.5, 0.0)),
    );
    game.part(
        Look::new("Ball", "Neon", [255, 220, 160]),
        Vec3::splat(1.0),
        CFrame::new(Vec3::new(0.0, 2.0, 0.0)),
    );
    game.set_camera(
        CFrame::look_at(Vec3::new(0.0, 3.0, 10.0), Vec3::new(0.0, 2.0, 0.0), Vec3::Y),
        50.0,
    );
    let lighting = game
        .dm
        .service(game.dm.registry().class_id("Lighting").expect("Lighting"))
        .expect("a Lighting");
    game.dm
        .set_by_name(lighting, "ClockTime", Value::Number(0.0))
        .expect("ClockTime");
    game
}

/// The mean level of the pixels in a ring `from`..`to` pixels around the image's centre (where the lamp is).
fn ring(pixels: &[[u8; 4]], from: f32, to: f32) -> f32 {
    let (mut sum, mut count) = (0.0, 0.0);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let d = ((x as f32 - SIZE as f32 / 2.0).powi(2) + (y as f32 - SIZE as f32 / 2.0).powi(2)).sqrt();
            if (from..to).contains(&d) {
                let p = pixels[(y * SIZE + x) as usize];
                sum += f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2]);
                count += 3.0;
            }
        }
    }
    sum / count
}

#[test]
fn bloom_glows_around_a_lamp_at_night_at_every_effect_quality() {
    let Some(context) = gpu() else { return };
    let (plain, _) = render(&context, &mut night_lamp(), |s| {
        s.passes = s.passes.without(PassId::Bloom)
    });
    save("lamp_plain", &plain);
    let around = ring(&plain, 14.0, 30.0);
    for (name, post) in [
        ("low", PostSettings::LOW),
        ("medium", PostSettings::MEDIUM),
        ("high", PostSettings::HIGH),
    ] {
        let (bloomed, stats) = render(&context, &mut night_lamp(), |s| s.post = post);
        save(&format!("lamp_{name}"), &bloomed);
        let glow = ring(&bloomed, 14.0, 30.0);
        eprintln!(
            "{name}: around the lamp {around:.2} -> {glow:.2} ({} levels)",
            stats.bloom_levels
        );
        assert!(glow > around + 1.0, "{name}: no glow ({around} -> {glow})");
        // The glow is around the lamp, not a veil over the image: the corners barely change.
        let (dark, veiled) = (ring(&plain, 150.0, 200.0), ring(&bloomed, 150.0, 200.0));
        assert!((veiled - dark).abs() < 2.0, "{name}: {dark} -> {veiled}");
    }
}

/// Bloom's chain as one compute pass (#145) runs the render passes' filters at the same coordinates, so the image
/// is the one the render passes draw (as devices without compute shaders do). Where its levels are the scene colour's
/// `Rg11b10Ufloat` (DX12, Vulkan) a pixel or two differ by a level; where they are `Rgba16Float` (OpenGL, which writes
/// no `Rg11b10Ufloat` storage), which keeps a few more bits of the glow, up to two levels and 0.5 on average. The lamp
/// at night (where the glow is the image) and the demo by day, at every effect quality.
#[test]
fn bloom_draws_the_same_image_in_a_compute_pass_as_in_render_passes() {
    let Some(context) = gpu() else { return };
    if metallic_render::post::bloom_compute_format(&context).is_none() {
        eprintln!("no compute shaders: bloom draws render passes only");
        return;
    }
    for (scene, make) in [("lamp", night_lamp as fn() -> Game), ("demo", demo)] {
        for (name, post) in [
            ("low", PostSettings::LOW),
            ("medium", PostSettings::MEDIUM),
            ("high", PostSettings::HIGH),
        ] {
            let (compute, stats) = render(&context, &mut make(), |s| s.post = post);
            assert_eq!(stats.compute_passes, 1, "{scene} {name}");
            let (passes, stats) = render(&context, &mut make(), |s| {
                s.post = post;
                s.post.bloom.compute = false;
            });
            assert_eq!(stats.compute_passes, 0, "{scene} {name}");
            save(&format!("bloom_{scene}_{name}_compute"), &compute);
            save(&format!("bloom_{scene}_{name}_passes"), &passes);
            let diff = scenes::compare(&compute, &passes, 1);
            eprintln!("{scene} {name}: compute pass against render passes: {diff:?}");
            assert!(diff.max <= 2 && diff.mean <= 1.0, "{scene} {name}: {diff:?}");
        }
    }
}

/// A pixel the scene's shaders write as not a number (here a neon material whose metalness is NaN) stays one bad
/// pixel: bloom's blur must not carry it over the image (which lavapipe, whose maths yields NaN where GPUs do not,
/// showed as whole black frames), and MLAA must not either.
#[test]
fn a_pixel_that_is_not_a_number_stays_where_it_is() {
    let Some(context) = gpu() else { return };
    let poison = |renderer: &mut Renderer| {
        let neon = metallic_render::MaterialParams::builtin("Neon").expect("Neon");
        let materials = renderer.parts_mut().materials_mut();
        let rows: Vec<usize> = (0..256)
            .filter(|&row| {
                materials
                    .get(row)
                    .is_some_and(|params| params.emissive == neon.emissive)
            })
            .collect();
        assert!(!rows.is_empty(), "the neon row");
        for row in rows {
            materials.set(
                row,
                metallic_render::MaterialParams {
                    metalness: f32::NAN,
                    ..neon
                },
            );
        }
    };
    for (name, passes) in [
        ("bloom", PassSet::default()),
        ("bloom and MLAA", PassSet::default().with(PassId::Mlaa)),
    ] {
        let (clean, _) = render(&context, &mut demo(), |s| s.passes = passes);
        let (poisoned, _) = render_with(&context, &mut demo(), |s| s.passes = passes, poison);
        save(&format!("nan_{name}"), &poisoned);
        let diff = scenes::compare(&clean, &poisoned, 12);
        eprintln!("{name}: a NaN neon cube against a lit one: {diff:?}");
        // The cube (about 1% of the image) and its own glow differ; the rest of the image does not.
        assert!(diff.over < 0.03 && diff.mean < 3.0, "{name}: {diff:?}");
    }
}

/// Black in, black out: a black clear colour is exactly 0 in the output, whatever the dither and the target.
#[test]
fn black_stays_black() {
    let Some(context) = gpu() else { return };
    for format in [wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureFormat::Rgba8Unorm] {
        let target = OffscreenTarget::new(&context, SIZE, SIZE, format).expect("a target");
        let mut renderer = Renderer::new(
            &context,
            RendererSettings {
                passes: PassSet::default().with(PassId::Mlaa),
                ..RendererSettings::new(format, SIZE, SIZE)
            },
        )
        .expect("a renderer");
        renderer.wait_for_pipelines();
        renderer.render(target.view());
        let pixels = read_back_rgba8(&context, target.texture());
        assert!(
            pixels.iter().all(|p| p[..3] == [0, 0, 0]),
            "{format:?}: {:?}",
            pixels.iter().find(|p| p[..3] != [0, 0, 0])
        );
    }
}

#[test]
fn exposure_and_grade_change_the_image() {
    let Some(context) = gpu() else { return };
    let (normal, _) = render(&context, &mut demo(), |_| {});
    let (bright, _) = render(&context, &mut demo(), |s| s.post.exposure = 2.0);
    let (grey, _) = render(&context, &mut demo(), |s| s.post.saturation = 0.0);
    let mean = |pixels: &[[u8; 4]]| {
        pixels
            .iter()
            .map(|p| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]))
            .sum::<u32>() as f32
            / pixels.len() as f32
    };
    assert!(mean(&bright) > mean(&normal) + 30.0);
    for p in &grey {
        let (lo, hi) = (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]));
        assert!(hi - lo <= 2, "{p:?} is not grey");
    }
}

#[test]
fn anti_aliasing_and_effects_switch_between_frames() {
    let Some(context) = gpu() else { return };
    let mut game = demo();
    let target = OffscreenTarget::new(&context, SIZE, SIZE, FORMAT).expect("a target");
    let mut renderer = Renderer::new(&context, RendererSettings::new(FORMAT, SIZE, SIZE)).expect("a renderer");
    renderer.wait_for_pipelines();
    let mut scene = GpuScene::new(
        &context,
        &mut game.dm,
        Some(game.workspace),
        &RenderComponents::builtin(),
    )
    .expect("a scene");
    let camera = CameraReader::new().read(&game.dm, game.workspace);
    scene.sync(&mut game.dm);
    let counts = renderer.msaa_sample_counts();
    let choices = AntiAliasing::ALL.map(|aa| aa.supported(counts));
    let effects = [PostSettings::LOW, PostSettings::MEDIUM, PostSettings::HIGH];
    for frame in 0..20 {
        let aa = choices[frame % choices.len()];
        renderer.set_anti_aliasing(aa).expect("a supported choice");
        renderer.set_post(effects[frame % effects.len()]);
        assert_eq!(renderer.anti_aliasing(), aa);
        renderer.render_scene(target.view(), &mut scene, camera.as_ref());
    }
    let pixels = read_back_rgba8(&context, target.texture());
    assert!(pixels.iter().any(|p| p[0] > 100), "the scene is drawn");
}

#[test]
fn both_instance_paths_finish_the_same_image() {
    let Some(primary) = gpu() else { return };
    let Some(fallback) = gpu_with(&ContextOptions {
        force_vertex_instances: true,
        ..ContextOptions::default()
    }) else {
        return;
    };
    assert_eq!(InstanceBinding::for_context(&fallback), InstanceBinding::Vertex);
    let counts = Renderer::new(&primary, RendererSettings::new(FORMAT, SIZE, SIZE))
        .expect("a renderer")
        .msaa_sample_counts();
    for aa in [AntiAliasing::Mlaa, AntiAliasing::Msaa(4).supported(counts)] {
        let setup = |s: &mut RendererSettings| {
            s.passes = s.passes.with_if(PassId::Mlaa, aa.mlaa());
            s.msaa_samples = aa.msaa_samples();
        };
        let (storage, _) = render(&primary, &mut demo(), setup);
        let (vertex, _) = render(&fallback, &mut demo(), setup);
        let diff = scenes::compare(&vertex, &storage, 1);
        eprintln!("{aa}: fallback against storage: {diff:?}");
        assert!(diff.max <= 2, "{aa}: {diff:?}");
    }
}
