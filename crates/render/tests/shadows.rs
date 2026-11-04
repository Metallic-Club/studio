//! Sun shadows on a real adapter (#82): a pillar's shadow on the floor against a committed reference image,
//! `CastShadow` and `Lighting.GlobalShadows` turning shadows off, the same image from the vertex-buffer fallback,
//! and the caching of the far cascade: a frame in which nothing static changed and the camera and sun stayed draws
//! only the near cascades, which hold the moving parts; and a cube resting on the floor under a low sun, shadowed
//! right up to its foot with no acne on the lit surfaces (#126). Skips when the machine has no adapter (see
//! `support::gpu`).
//!
//! The reference is `tests/reference/shadows.png`, compared with the tolerance of `tests/parts.rs`. To make a new
//! one (after a deliberate change to how shadows look), run with `METALLIC_UPDATE_REFERENCES=1` and look at it.
//! `METALLIC_SAVE_IMAGES=1` saves the contact test's image to the test's temporary folder.

mod support;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use metallic_core::glam::{Quat, Vec3, Vec4};
use metallic_core::{CFrame, Id, Value};
use metallic_render::scene::{InstanceBinding, RenderComponents};
use metallic_render::{
    CameraReader, ContextOptions, GpuScene, LightingReader, OffscreenTarget, PassSet, RenderContext, Renderer,
    RendererSettings, ShadowSettings, wgpu,
};
use support::scenes::{self, Game, Look};
use support::{gpu, gpu_with, read_back_rgba8};

const SIZE: u32 = 256;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const THRESHOLD: u8 = 12;
const MAX_OVER: f64 = 0.01;
const MAX_MEAN: f64 = 1.0;

/// The pillar's height; it stands on the floor at the origin.
const PILLAR_HEIGHT: f32 = 8.0;

/// What [`shadow_game`] built, to change later.
struct ShadowGame {
    game: Game,
    pillar: Id,
    /// A block 150 m from the camera, in the far (cached) cascade.
    far_block: Id,
    /// A ball near the camera that tests move.
    mover: Id,
}

/// A floor, a pillar casting a long afternoon shadow, a ball resting beside it, a far block and a loose ball, seen
/// from above at an angle. 16:00, so the sun is in the west-south-west and the pillar's shadow falls towards the
/// north-east.
fn shadow_game() -> ShadowGame {
    let mut game = Game::new();
    game.part(
        Look::new("Block", "Concrete", [160, 158, 150]),
        Vec3::new(600.0, 1.0, 600.0),
        CFrame::new(Vec3::new(0.0, -0.5, 0.0)),
    );
    let pillar = game.part(
        Look::new("Block", "Plastic", [196, 40, 28]),
        Vec3::new(2.0, PILLAR_HEIGHT, 2.0),
        CFrame::new(Vec3::new(0.0, PILLAR_HEIGHT / 2.0, 0.0)),
    );
    game.part(
        Look::new("Ball", "SmoothPlastic", [13, 105, 172]),
        Vec3::splat(3.0),
        CFrame::new(Vec3::new(-8.0, 1.5, -5.0)),
    );
    let far_block = game.part(
        Look::new("Block", "Plastic", [75, 151, 75]),
        Vec3::new(10.0, 10.0, 10.0),
        CFrame::new(Vec3::new(0.0, 5.0, -130.0)),
    );
    let mover = game.part(
        Look::new("Ball", "Neon", [255, 200, 40]),
        Vec3::splat(1.0),
        CFrame::new(Vec3::new(6.0, 3.0, 6.0)),
    );
    game.set_camera(
        CFrame::look_at(Vec3::new(4.0, 16.0, 24.0), Vec3::new(2.0, 0.0, -2.0), Vec3::Y),
        60.0,
    );
    let lighting = lighting_service(&game);
    game.dm
        .set_by_name(lighting, "ClockTime", Value::Number(16.0))
        .expect("ClockTime");
    ShadowGame {
        game,
        pillar,
        far_block,
        mover,
    }
}

fn lighting_service(game: &Game) -> Id {
    let class = game.dm.registry().class_id("Lighting").expect("Lighting");
    game.dm.service(class).expect("a Lighting")
}

/// The direction towards the sun.
fn sun(game: &Game) -> Vec3 {
    let Ok(Value::Vector3(sun)) = game.dm.get_by_name(lighting_service(game), "SunDirection") else {
        panic!("no SunDirection");
    };
    sun
}

/// A renderer, a scene of `game` and a camera reader, ready to draw frames.
struct Frames {
    renderer: Renderer,
    scene: GpuScene,
    target: OffscreenTarget,
    camera: CameraReader,
    lighting: LightingReader,
}

impl Frames {
    fn new(context: &RenderContext, game: &mut Game, shadows: ShadowSettings) -> Self {
        let target = OffscreenTarget::new(context, SIZE, SIZE, FORMAT).expect("a target");
        let renderer = Renderer::new(
            context,
            RendererSettings {
                passes: PassSet::default(),
                gpu_timing: true,
                shadows,
                ..RendererSettings::new(FORMAT, SIZE, SIZE)
            },
        )
        .expect("a renderer");
        let scene = GpuScene::new(
            context,
            &mut game.dm,
            Some(game.workspace),
            &RenderComponents::builtin(),
        )
        .expect("a scene");
        Self {
            renderer,
            scene,
            target,
            camera: CameraReader::new(),
            lighting: LightingReader::new(),
        }
    }

    /// One frame of `game` as it is now, lit by its `Lighting`.
    fn frame(&mut self, context: &RenderContext, game: &mut Game) {
        let camera = self.camera.read(&game.dm, game.workspace);
        if let Some(camera) = camera {
            self.scene.set_camera_position(&game.dm, camera.position);
        }
        self.scene.sync(&mut game.dm);
        let lighting = self.lighting.read(&game.dm, lighting_service(game));
        self.renderer.set_lighting(lighting);
        let submission = self
            .renderer
            .render_scene(self.target.view(), &mut self.scene, camera.as_ref());
        // Keep at most one frame in flight, outside the frame (offscreen nothing else paces the CPU).
        let _ = context.device().poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        });
    }

    fn pixels(&self, context: &RenderContext) -> Vec<[u8; 4]> {
        read_back_rgba8(context, self.target.texture())
    }
}

/// Renders `game` (two frames, so everything is uploaded) with `shadows` and reads the image back.
fn render(context: &RenderContext, game: &mut Game, shadows: ShadowSettings) -> Vec<[u8; 4]> {
    let mut frames = Frames::new(context, game, shadows);
    for _ in 0..2 {
        frames.frame(context, game);
    }
    frames.pixels(context)
}

/// Where world point `point` lands on the target, in pixels from the top left.
fn project(game: &Game, point: Vec3) -> (u32, u32) {
    let camera = CameraReader::new().read(&game.dm, game.workspace).expect("a camera");
    let clip = camera.view_projection(1.0) * Vec4::from((point - camera.position, 1.0));
    let (x, y) = (clip.x / clip.w, clip.y / clip.w);
    (
        ((x * 0.5 + 0.5) * SIZE as f32) as u32,
        ((0.5 - y * 0.5) * SIZE as f32) as u32,
    )
}

/// The mean of the three channels around pixel `(x, y)` (a 3 × 3 block, so an edge pixel does not decide).
fn level(pixels: &[[u8; 4]], (x, y): (u32, u32)) -> f32 {
    let mut sum = 0u32;
    for dy in 0..3 {
        for dx in 0..3 {
            let p = pixels[((y + dy - 1) * SIZE + x + dx - 1) as usize];
            sum += u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
        }
    }
    sum as f32 / 27.0
}

/// Two floor points 5 m from the pillar's foot: one in its shadow (down-sun) and one lit (up-sun), on the image.
fn floor_points(game: &Game) -> ((u32, u32), (u32, u32)) {
    let sun = sun(game);
    let down_sun = Vec3::new(-sun.x, 0.0, -sun.z).normalize();
    // The pillar's shadow is PILLAR_HEIGHT / tan(elevation) long; 5 m is well inside it.
    assert!(
        PILLAR_HEIGHT * (1.0 - sun.y * sun.y).sqrt() / sun.y > 8.0,
        "the shadow is long"
    );
    (project(game, down_sun * 5.0), project(game, -down_sun * 5.0))
}

fn check_reference(name: &str, pixels: &[[u8; 4]]) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/reference/{name}.png"));
    if std::env::var("METALLIC_UPDATE_REFERENCES").is_ok_and(|v| v == "1") {
        scenes::write_png(&path, SIZE, SIZE, pixels);
        eprintln!("wrote {}", path.display());
        return;
    }
    let Some((width, height, reference)) = scenes::read_png(&path) else {
        panic!(
            "{} is missing; run with METALLIC_UPDATE_REFERENCES=1 to make it",
            path.display()
        );
    };
    assert_eq!((width, height), (SIZE, SIZE));
    let diff = scenes::compare(pixels, &reference, THRESHOLD);
    eprintln!(
        "{name}: max difference {}, mean {:.3}, {:.3}% of pixels over {THRESHOLD}",
        diff.max,
        diff.mean,
        diff.over * 100.0
    );
    if diff.over > MAX_OVER || diff.mean > MAX_MEAN {
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
        scenes::write_png(&out, SIZE, SIZE, pixels);
        panic!(
            "{name} differs from the reference beyond the tolerance ({diff:?}); the image is in {}",
            out.display()
        );
    }
}

#[test]
fn a_pillar_casts_its_shadow_on_the_floor() {
    let Some(context) = gpu() else { return };
    let mut built = shadow_game();
    let pixels = render(&context, &mut built.game, ShadowSettings::default());
    check_reference("shadows", &pixels);
    let (shadowed, lit) = floor_points(&built.game);
    let (dark, bright) = (level(&pixels, shadowed), level(&pixels, lit));
    eprintln!("floor in the shadow at {shadowed:?}: {dark:.1}; lit at {lit:?}: {bright:.1}");
    // The low sun lights the floor at a grazing angle, so the sky's light is most of it: about 3/4 in sRGB.
    assert!(dark < bright * 0.85, "the floor is not darker in the shadow");
    // Without shadows the same floor is lit on both sides.
    let unshadowed = render(&context, &mut built.game, ShadowSettings::OFF);
    let (dark, bright) = (level(&unshadowed, shadowed), level(&unshadowed, lit));
    assert!((dark - bright).abs() < bright * 0.08, "{dark} against {bright}");
}

#[test]
fn cast_shadow_and_global_shadows_turn_shadows_off() {
    let Some(context) = gpu() else { return };
    let mut built = shadow_game();
    let off = render(&context, &mut built.game, ShadowSettings::OFF);
    let (shadowed, _) = floor_points(&built.game);

    // The pillar alone stops casting: its shadow is gone, the ball's stays.
    built
        .game
        .dm
        .set_by_name(built.pillar, "CastShadow", Value::Bool(false))
        .expect("CastShadow");
    let no_pillar = render(&context, &mut built.game, ShadowSettings::default());
    let diff = (level(&no_pillar, shadowed) - level(&off, shadowed)).abs();
    assert!(diff < 3.0, "the pillar still casts ({diff})");
    assert!(
        scenes::compare(&no_pillar, &off, THRESHOLD).over > 0.001,
        "the ball's shadow went too"
    );

    // `Lighting.GlobalShadows` off: exactly the image without shadows.
    built
        .game
        .dm
        .set_by_name(built.pillar, "CastShadow", Value::Bool(true))
        .expect("CastShadow");
    let lighting = lighting_service(&built.game);
    built
        .game
        .dm
        .set_by_name(lighting, "GlobalShadows", Value::Bool(false))
        .expect("GlobalShadows");
    let global_off = render(&context, &mut built.game, ShadowSettings::default());
    assert_eq!(scenes::compare(&global_off, &off, 0).max, 0);
}

#[test]
fn the_vertex_buffer_fallback_draws_the_same_shadows() {
    let Some(primary) = gpu() else { return };
    let Some(fallback) = gpu_with(&ContextOptions {
        force_vertex_instances: true,
        ..ContextOptions::default()
    }) else {
        return;
    };
    assert_eq!(InstanceBinding::for_context(&fallback), InstanceBinding::Vertex);
    let storage = render(&primary, &mut shadow_game().game, ShadowSettings::default());
    let vertex = render(&fallback, &mut shadow_game().game, ShadowSettings::default());
    let diff = scenes::compare(&vertex, &storage, 1);
    eprintln!("fallback against storage: {diff:?}");
    assert!(diff.max <= 2, "{diff:?}");
}

/// The contact scene's layout on the floor: the horizontal direction the sun's shadows fall in and the one across
/// it.
struct ContactLayout {
    down_sun: Vec3,
    across: Vec3,
}

impl ContactLayout {
    /// A floor point `out` metres from the cube's down-sun foot (the cube is 2 m wide, centred on the origin) and
    /// `side` metres across.
    fn floor(&self, out: f32, side: f32) -> Vec3 {
        self.down_sun * (1.0 + out) + self.across * side
    }

    /// The ramp's centre and turn: beside the cube on the sun's side, tilted 30° across the sun (towards the
    /// camera), so the low sun meets it at about 15°.
    fn ramp(&self) -> CFrame {
        let tilt = Quat::from_axis_angle(self.down_sun, -30f32.to_radians());
        CFrame::from_parts(
            self.across * 3.5 - self.down_sun * 0.5 + Vec3::Y * 0.6,
            tilt * self.cube_turn(),
        )
    }

    /// Turns the cube so one face looks down-sun.
    fn cube_turn(&self) -> Quat {
        Quat::from_rotation_y(f32::atan2(-self.down_sun.z, self.down_sun.x))
    }
}

/// A 2 m cube resting exactly on the floor at the origin (its bottom on the floor's top) under a low sun (16:30,
/// about 17° up), turned so a face looks down-sun, a ramp tilted 30° beside it, and the camera down-sun of the
/// cube looking back at its foot.
fn contact_game() -> (Game, ContactLayout) {
    let mut game = Game::new();
    let lighting = lighting_service(&game);
    game.dm
        .set_by_name(lighting, "ClockTime", Value::Number(16.5))
        .expect("ClockTime");
    let towards = sun(&game);
    let down_sun = Vec3::new(-towards.x, 0.0, -towards.z).normalize();
    let layout = ContactLayout {
        down_sun,
        across: Vec3::Y.cross(down_sun),
    };
    game.part(
        Look::new("Block", "Concrete", [160, 158, 150]),
        Vec3::new(200.0, 1.0, 200.0),
        CFrame::new(Vec3::new(0.0, -0.5, 0.0)),
    );
    game.part(
        Look::new("Block", "Plastic", [163, 162, 165]),
        Vec3::splat(2.0),
        CFrame::from_parts(Vec3::new(0.0, 1.0, 0.0), layout.cube_turn()),
    );
    game.part(
        Look::new("Block", "Plastic", [163, 162, 165]),
        Vec3::new(2.4, 0.4, 2.4),
        layout.ramp(),
    );
    let eye = layout.down_sun * 6.0 + layout.across * 3.0 + Vec3::Y * 4.5;
    let target = layout.down_sun * 1.0 + layout.across * 1.0 + Vec3::Y * 0.4;
    game.set_camera(CFrame::look_at(eye, target, Vec3::Y), 60.0);
    (game, layout)
}

/// The mean of the three channels of the one pixel world point `point` lands on (one pixel, so a speckle is not
/// averaged away).
fn pixel_level(game: &Game, pixels: &[[u8; 4]], point: Vec3) -> f32 {
    let (x, y) = project(game, point);
    let p = pixels[(y * SIZE + x) as usize];
    (f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2])) / 3.0
}

fn mean_level(game: &Game, pixels: &[[u8; 4]], points: &[Vec3]) -> f32 {
    points.iter().map(|&p| pixel_level(game, pixels, p)).sum::<f32>() / points.len() as f32
}

/// `point(a, b)` for every `a` in `xs` and `b` in `zs`.
fn grid_points(xs: &[f32], zs: &[f32], point: impl Fn(f32, f32) -> Vec3) -> Vec<Vec3> {
    xs.iter()
        .flat_map(|&x| zs.iter().map(move |&z| (x, z)))
        .map(|(x, z)| point(x, z))
        .collect()
}

/// A cube resting on a plate under a low sun: its shadow reaches its foot, with no lit band between them (#126),
/// and the lit surfaces (the cube's top, a ramp the sun meets at about 15° and the floor it meets at 17°) have no
/// acne. With the default cascades, with ADR 0008's Low (one 1024² cascade, about 30 cm a texel, where the depth
/// bias reaches its cap) and with the 5 × 5 filter (whose taps reach furthest).
#[test]
fn the_sun_shadow_meets_the_foot_of_a_resting_cube() {
    let Some(context) = gpu() else { return };
    let (mut game, layout) = contact_game();
    let coarse = ShadowSettings {
        cascades: 1,
        resolution: 1024,
        ..ShadowSettings::default()
    };
    let wide = ShadowSettings {
        filter_radius: 2,
        ..ShadowSettings::default()
    };
    let unshadowed = render(&context, &mut game, ShadowSettings::OFF);
    let settings = [
        ("default", ShadowSettings::default()),
        ("one 1024² cascade", coarse),
        ("5 × 5 filter", wide),
    ];
    for (name, settings) in settings {
        let shadowed = render(&context, &mut game, settings);
        if settings == ShadowSettings::default() && std::env::var("METALLIC_SAVE_IMAGES").is_ok_and(|v| v == "1") {
            let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("contact_sun.png");
            scenes::write_png(&out, SIZE, SIZE, &shadowed);
            eprintln!("wrote {}", out.display());
        }
        let sides = [-0.7, -0.35, 0.0, 0.35, 0.7];
        let band = grid_points(&[0.03, 0.06, 0.09, 0.12], &sides, |out, side| layout.floor(out, side));
        let interior = grid_points(&[0.5, 0.6, 0.7, 0.8], &sides, |out, side| layout.floor(out, side));
        let lit = grid_points(&[0.06, 0.3, 0.6], &[-2.3, -2.5, -2.7], |out, side| {
            layout.floor(out, side)
        });
        let (band, interior, lit_level) = (
            mean_level(&game, &shadowed, &band),
            mean_level(&game, &shadowed, &interior),
            mean_level(&game, &shadowed, &lit),
        );
        let leak = (band - interior) / (lit_level - interior).max(1.0);
        eprintln!(
            "contact_sun, {name}: band {band:.1}, interior {interior:.1}, lit {lit_level:.1}, leak {:.1}%",
            leak * 100.0
        );
        // Acne: no lit point is darker than without shadows.
        let ramp = layout.ramp();
        let turn = layout.cube_turn();
        let mut surfaces = grid_points(&sides, &sides, |x, z| turn * Vec3::new(x, 2.0, z));
        surfaces.extend(grid_points(
            &[-0.9, -0.45, 0.0, 0.45, 0.9],
            &[-0.9, -0.45, 0.0, 0.45, 0.9],
            |x, z| ramp * Vec3::new(x, 0.2, z),
        ));
        surfaces.extend(lit);
        let (mut worst, mut at) = (0.0f32, Vec3::ZERO);
        for point in surfaces {
            let drop = pixel_level(&game, &unshadowed, point) - pixel_level(&game, &shadowed, point);
            if drop > worst {
                (worst, at) = (drop, point);
            }
        }
        eprintln!("contact_sun, {name}: the lit surfaces' largest drop with shadows on: {worst:.1} levels at {at}");
        // The low sun meets the floor at 17°, so the sky's light is most of the lit floor's.
        assert!(lit_level > interior + 15.0, "{name}: the cube casts no clear shadow");
        assert!(
            leak < 0.05,
            "{name}: light leaks under the cube's foot: the band 3 to 12 cm out is {:.1}% of the way from the shadow \
             to the light",
            leak * 100.0
        );
        assert!(
            worst <= 3.0,
            "{name}: shadow acne on a lit surface: {worst:.1} levels darker at {at}"
        );
    }
}

/// The cascades a frame drew, as a bit mask.
fn redrawn(frames: &Frames) -> u8 {
    frames.renderer.draw_stats().shadow_redrawn
}

fn move_to(game: &mut Game, id: Id, at: Vec3) {
    game.dm
        .set_by_name(id, "CFrame", Value::CFrame(CFrame::new(at)))
        .expect("moved");
}

/// The far cascade is cached: with the camera and the sun still, frames in which only a moving part moves draw the
/// two near cascades (which hold it) and keep the far one; a static part moving inside the far cascade draws it
/// once; the sun turning draws everything. The far cascade never holds the moving part.
#[test]
fn a_static_frame_does_not_redraw_the_far_cascade() {
    let Some(context) = gpu() else { return };
    let mut built = shadow_game();
    let game = &mut built.game;
    let mut frames = Frames::new(&context, game, ShadowSettings::default());
    frames.frame(&context, game);
    assert_eq!(redrawn(&frames), 0b111, "the first frame draws every cascade");
    let first = frames.renderer.draw_stats();
    assert_eq!(first.shadow_cascades, 3);
    assert!(first.shadow_draws >= 3 && first.shadow_instances > 0, "{first:?}");
    // The far cascade holds the far block (and everything else).
    assert!(first.shadow_casters[2] >= 4, "{first:?}");

    // Nothing changed at all: every map is kept.
    frames.frame(&context, game);
    assert_eq!(redrawn(&frames), 0, "an unchanged frame drew {:b}", redrawn(&frames));
    assert_eq!(frames.renderer.draw_stats().shadow_draws, 0);

    // The loose ball moves every frame. Its first moves change the static region (it is promoted to the dynamic
    // one, which takes it out of the far cascade), then only the near cascades are drawn again.
    let mut masks = Vec::new();
    for frame in 0..40 {
        move_to(game, built.mover, Vec3::new(6.0, 3.0 + frame as f32 * 0.05, 6.0));
        frames.frame(&context, game);
        masks.push(redrawn(&frames));
    }
    eprintln!("cascades drawn while the ball moves: {masks:?}");
    assert!(masks[10..].iter().all(|&mask| mask == 0b011), "{masks:?}");
    let steady = frames.renderer.draw_stats();
    assert_eq!(
        steady.shadow_casters[2],
        first.shadow_casters[2] - 1,
        "the far cascade holds the static casters only"
    );
    let stats = frames.renderer.shadow_stats();
    assert!(stats.cascade[2].cached && !stats.cascade[1].cached);
    // The far cascade's map is built from its static map and the moving ball, in every frame it moves (#121).
    assert!(stats.cascade[2].built && stats.cascade[2].moving == 1, "{stats:?}");

    // A static part moves inside the far cascade: it is drawn again, once.
    move_to(game, built.far_block, Vec3::new(3.0, 5.0, -130.0));
    move_to(game, built.mover, Vec3::new(6.0, 5.0, 6.0));
    frames.frame(&context, game);
    assert_eq!(redrawn(&frames), 0b111);
    move_to(game, built.mover, Vec3::new(6.0, 5.1, 6.0));
    frames.frame(&context, game);
    assert_eq!(redrawn(&frames), 0b011);

    // A recolour of a static part changes no shadow.
    game.dm
        .set_by_name(
            built.far_block,
            "Color",
            Value::Color3(metallic_core::Color3::new(0.1, 0.2, 0.3)),
        )
        .expect("Color");
    move_to(game, built.mover, Vec3::new(6.0, 5.2, 6.0));
    frames.frame(&context, game);
    assert_eq!(redrawn(&frames), 0b011);

    // The sun moves: every cascade.
    let lighting = lighting_service(game);
    game.dm
        .set_by_name(lighting, "ClockTime", Value::Number(16.1))
        .expect("ClockTime");
    frames.frame(&context, game);
    assert_eq!(redrawn(&frames), 0b111);

    // GPU time is reported per cascade: a kept cascade took none.
    if !frames.renderer.gpu_timing_enabled() {
        eprintln!("no timestamp queries: per-cascade GPU times not checked");
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut checked = false;
    while Instant::now() < deadline && !checked {
        move_to(game, built.mover, Vec3::new(6.0, 5.3, 6.0));
        frames.frame(&context, game);
        if let Some(timings) = frames.renderer.gpu_timings().copied()
            && timings.cascade_count() == 3
            && timings.cascade_drawn(0)
            && !timings.cascade_drawn(2)
        {
            eprintln!(
                "frame {}: cascades {:?} {:?} {:?} ms",
                timings.frame(),
                timings.cascade(0),
                timings.cascade(1),
                timings.cascade(2)
            );
            assert!(timings.cascade(0).is_some_and(|ms| ms > 0.0));
            assert_eq!(timings.cascade(2), Some(0.0));
            assert_eq!(timings.cascade(3), None);
            checked = true;
        }
    }
    assert!(checked, "no timings of a frame that kept the far cascade arrived");
}

/// What [`far_mover_game`] built.
struct FarMover {
    game: Game,
    /// A 6 m cube resting on the floor about 80 m from the camera, which tests move.
    cube: Id,
    /// Where it rests.
    at: Vec3,
    /// A static cube like it beside it, in the far cascade's static map, which tests recolour.
    block: Id,
    /// A floor point in the moving cube's shadow, one in the static cube's and a lit one between them, all on the
    /// image.
    shadowed: Vec3,
    static_shadowed: Vec3,
    lit: Vec3,
}

/// A floor and a 6 m cube on it about 80 m from the camera (well beyond the two dynamic cascades' 57 m, so in the
/// cached far cascade), seen from its shadow's side under the afternoon sun.
fn far_mover_game() -> FarMover {
    let mut game = Game::new();
    let lighting = lighting_service(&game);
    game.dm
        .set_by_name(lighting, "ClockTime", Value::Number(16.0))
        .expect("ClockTime");
    let towards = sun(&game);
    let down_sun = Vec3::new(-towards.x, 0.0, -towards.z).normalize();
    let across = Vec3::Y.cross(down_sun);
    game.part(
        Look::new("Block", "Concrete", [160, 158, 150]),
        Vec3::new(600.0, 1.0, 600.0),
        CFrame::new(Vec3::new(0.0, -0.5, 0.0)),
    );
    let at = Vec3::new(0.0, 3.0, -100.0);
    let cube = game.part(
        Look::new("Block", "Plastic", [196, 40, 28]),
        Vec3::splat(6.0),
        CFrame::new(at),
    );
    let block = game.part(
        Look::new("Block", "Plastic", [75, 151, 75]),
        Vec3::splat(6.0),
        CFrame::new(at - across * 18.0),
    );
    let foot = Vec3::new(at.x, 0.0, at.z);
    game.set_camera(
        CFrame::look_at(foot + down_sun * 70.0 + Vec3::Y * 45.0, foot, Vec3::Y),
        60.0,
    );
    FarMover {
        game,
        cube,
        at,
        block,
        // 4 m beyond each cube's down-sun face: inside its shadow, at least 6 m long under any afternoon sun.
        shadowed: foot + down_sun * 7.0,
        static_shadowed: foot - across * 18.0 + down_sun * 7.0,
        // 6 m beside the moving cube's side face, across the sun: lit.
        lit: foot - across * 9.0,
    }
}

/// Moves the far cube by a centimetre a frame for `frames` frames, so it is in the dynamic region (a moving part).
fn wiggle(frames: &mut Frames, context: &RenderContext, built: &mut FarMover, frames_count: usize) {
    for frame in 0..frames_count {
        let at = built.at + Vec3::new(0.01 * (frame % 2) as f32, 0.0, 0.0);
        move_to(&mut built.game, built.cube, at);
        frames.frame(context, &mut built.game);
    }
}

/// A moving part beyond the dynamic cascades casts a shadow with the default settings (#121): the far cascade's map
/// is its static map with the moving casters drawn on top. Its shadow is as dark as with every cascade dynamic, the
/// static map is still not drawn again while only moving parts move, and a frame that moves nothing, or changes the
/// scene without moving casters in the far cascade, builds nothing. Both instance paths draw the same.
#[test]
fn a_moving_part_beyond_the_dynamic_cascades_casts_a_shadow() {
    let Some(context) = gpu() else { return };
    let mut built = far_mover_game();
    let camera = CameraReader::new()
        .read(&built.game.dm, built.game.workspace)
        .expect("a camera");
    let forward = camera.rotation * Vec3::NEG_Z;
    for point in [built.shadowed, built.static_shadowed, built.lit] {
        let depth = (point - camera.position).dot(forward);
        assert!(depth > 70.0, "{point} is {depth} m deep, not in the far cascade");
    }
    let mut frames = Frames::new(&context, &mut built.game, ShadowSettings::default());
    wiggle(&mut frames, &context, &mut built, 16);
    let stats = frames.renderer.draw_stats();
    eprintln!("far mover: {stats:?}");
    assert_eq!(stats.shadow_cascades, 3);
    assert_eq!(stats.shadow_moving[2], 1, "the far cascade holds the moving cube");
    assert_eq!(
        stats.shadow_redrawn & 0b100,
        0,
        "the far cascade's static map was drawn again"
    );
    assert_eq!(stats.shadow_built, 0b100, "the far cascade's map was not built");
    let pixels = frames.pixels(&context);
    let (shadowed, static_shadowed, lit) = (
        project(&built.game, built.shadowed),
        project(&built.game, built.static_shadowed),
        project(&built.game, built.lit),
    );
    let (dark, dark_static, bright) = (
        level(&pixels, shadowed),
        level(&pixels, static_shadowed),
        level(&pixels, lit),
    );
    eprintln!(
        "floor in the moving cube's shadow at {shadowed:?}: {dark:.1}; in the static cube's at {static_shadowed:?}: \
         {dark_static:.1}; lit at {lit:?}: {bright:.1}"
    );
    assert!(dark < bright * 0.85, "the moving cube casts no shadow");
    // The static map was copied in under it.
    assert!(dark_static < bright * 0.85, "the static cube casts no shadow");

    // As dark as when every cascade holds every caster.
    let mut all_game = far_mover_game();
    let mut all = Frames::new(
        &context,
        &mut all_game.game,
        ShadowSettings {
            dynamic_cascades: 3,
            ..ShadowSettings::default()
        },
    );
    wiggle(&mut all, &context, &mut all_game, 16);
    let reference = all.pixels(&context);
    let levels_all = [shadowed, static_shadowed, lit].map(|point| level(&reference, point));
    eprintln!("with every cascade dynamic: {levels_all:.1?}");
    for (ours, all) in [dark, dark_static, bright].into_iter().zip(levels_all) {
        assert!(
            (ours - all).abs() < 2.0,
            "{ours} against {all} with every cascade dynamic"
        );
    }

    // Nothing changes: nothing is drawn or built.
    frames.frame(&context, &mut built.game);
    let still = frames.renderer.draw_stats();
    assert_eq!(
        (still.shadow_redrawn, still.shadow_built, still.shadow_draws),
        (0, 0, 0)
    );
    assert_eq!(still.shadow_moving[2], 1);

    // The cube stops casting: the far map is built once more, from the static map alone; then a change that moves
    // no caster (a recolour) builds nothing, though the scene changed.
    built
        .game
        .dm
        .set_by_name(built.cube, "CastShadow", Value::Bool(false))
        .expect("CastShadow");
    frames.frame(&context, &mut built.game);
    let gone = frames.renderer.draw_stats();
    assert_eq!((gone.shadow_built, gone.shadow_moving[2]), (0b100, 0), "{gone:?}");
    assert!(
        level(&frames.pixels(&context), shadowed) > bright * 0.95,
        "the shadow stayed"
    );
    for frame in 0..3 {
        let gray = 0.2 + 0.1 * frame as f32;
        built
            .game
            .dm
            .set_by_name(
                built.block,
                "Color",
                Value::Color3(metallic_core::Color3::new(gray, gray, gray)),
            )
            .expect("Color");
        frames.frame(&context, &mut built.game);
        let recoloured = frames.renderer.draw_stats();
        assert_eq!(recoloured.shadow_built & 0b100, 0, "frame {frame}: {recoloured:?}");
    }

    // The vertex-buffer fallback draws the same shadow.
    if let Some(fallback) = gpu_with(&ContextOptions {
        force_vertex_instances: true,
        ..ContextOptions::default()
    }) {
        let mut storage_game = far_mover_game();
        let mut storage = Frames::new(&context, &mut storage_game.game, ShadowSettings::default());
        wiggle(&mut storage, &context, &mut storage_game, 16);
        let mut vertex_game = far_mover_game();
        let mut vertex = Frames::new(&fallback, &mut vertex_game.game, ShadowSettings::default());
        wiggle(&mut vertex, &fallback, &mut vertex_game, 16);
        assert_eq!(vertex.renderer.draw_stats().shadow_moving[2], 1);
        let diff = scenes::compare(&vertex.pixels(&fallback), &storage.pixels(&context), 1);
        eprintln!("fallback against storage: {diff:?}");
        assert!(diff.max <= 2, "{diff:?}");
    }
}

/// With every cascade dynamic (the full-quality end of the setting) a moving part redraws them all, and the far
/// one holds it too.
#[test]
fn all_dynamic_cascades_redraw_with_every_change() {
    let Some(context) = gpu() else { return };
    let mut built = shadow_game();
    let game = &mut built.game;
    let settings = ShadowSettings {
        dynamic_cascades: 3,
        ..ShadowSettings::default()
    };
    let mut frames = Frames::new(&context, game, settings);
    for frame in 0..12 {
        move_to(game, built.mover, Vec3::new(6.0, 3.0 + frame as f32 * 0.05, 6.0));
        frames.frame(&context, game);
    }
    assert_eq!(redrawn(&frames), 0b111);
    assert!(!frames.renderer.shadow_stats().cascade[2].cached);
}
