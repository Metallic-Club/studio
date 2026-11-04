//! Point and spot lights on a real adapter (#83): a point light's falloff, a spot light's cone and a shadowed local
//! light against committed reference images, on both instance paths; the light's range as a hard edge; a light in a
//! part following it; the shadow atlas kept while nothing changes and drawn again when a caster moves; and the
//! budget fading lights rather than popping them; and a cube resting on the floor under a spot, a near and a far
//! point light, shadowed right up to its foot with no acne on the lit surfaces, at every preset (#126). Skips when
//! the machine has no adapter (see `support::gpu`). `METALLIC_SAVE_IMAGES=1` saves the contact tests' images to the
//! test's temporary folder.
//!
//! The scenes are at night (`ClockTime` 0), so the local lights are most of the light. The references are
//! `tests/reference/lights_*.png`, compared with the tolerance of `tests/parts.rs`. To make new ones (after a
//! deliberate change to how local lights look), run with `METALLIC_UPDATE_REFERENCES=1` and look at them.

mod support;

use std::path::PathBuf;

use metallic_core::glam::{Quat, Vec3, Vec4};
use metallic_core::{CFrame, Color3, Id, Value};
use metallic_render::scene::{InstanceBinding, RenderComponents};
use metallic_render::{
    CameraReader, ContextOptions, GpuScene, LightSettings, LightingReader, OffscreenTarget, RenderContext, Renderer,
    RendererSettings, wgpu,
};
use support::scenes::{self, Game, Look};
use support::{gpu, gpu_with, read_back_rgba8};

const SIZE: u32 = 256;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const THRESHOLD: u8 = 12;
const MAX_OVER: f64 = 0.01;
const MAX_MEAN: f64 = 1.0;

/// A grey floor at night, seen from above at an angle.
fn night_floor() -> Game {
    let mut game = Game::new();
    game.part(
        Look::new("Block", "Concrete", [170, 168, 160]),
        Vec3::new(120.0, 1.0, 120.0),
        CFrame::new(Vec3::new(0.0, -0.5, 0.0)),
    );
    game.set_camera(
        CFrame::look_at(Vec3::new(0.0, 14.0, 16.0), Vec3::new(0.0, 0.0, -1.0), Vec3::Y),
        60.0,
    );
    let lighting = lighting_service(&game);
    game.dm
        .set_by_name(lighting, "ClockTime", Value::Number(0.0))
        .expect("ClockTime");
    game
}

fn lighting_service(game: &Game) -> Id {
    let class = game.dm.registry().class_id("Lighting").expect("Lighting");
    game.dm.service(class).expect("a Lighting")
}

/// Adds a `class` light (`PointLight` or `SpotLight`) under `parent` at `cframe` (relative to a part parent).
fn light(game: &mut Game, class: &str, parent: Id, cframe: CFrame, color: [u8; 3], brightness: f64, range: f64) -> Id {
    let dm = &mut game.dm;
    let class = dm.registry().class_id(class).expect("a light class");
    let id = dm.create(class).expect("a light");
    dm.set_by_name(id, "CFrame", Value::CFrame(cframe)).expect("CFrame");
    let [r, g, b] = color;
    dm.set_by_name(id, "Color", Value::Color3(Color3::from_rgb_u8(r, g, b)))
        .expect("Color");
    dm.set_by_name(id, "Brightness", Value::Number(brightness))
        .expect("Brightness");
    dm.set_by_name(id, "Range", Value::Number(range)).expect("Range");
    dm.set_parent(id, Some(parent)).expect("parented");
    id
}

/// A point light 3 m over the floor, range 12.
fn point_game() -> (Game, Id) {
    let mut game = night_floor();
    let ws = game.workspace;
    let id = light(
        &mut game,
        "PointLight",
        ws,
        CFrame::new(Vec3::new(0.0, 3.0, 0.0)),
        [255, 214, 170],
        2.0,
        12.0,
    );
    (game, id)
}

/// A spot light 6 m up pointing straight down, a 50° cone.
fn spot_game() -> (Game, Id) {
    let mut game = night_floor();
    let ws = game.workspace;
    let down = CFrame::from_parts(
        Vec3::new(0.0, 6.0, 0.0),
        Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
    );
    let id = light(&mut game, "SpotLight", ws, down, [170, 200, 255], 6.0, 20.0);
    game.dm.set_by_name(id, "Angle", Value::Number(50.0)).expect("Angle");
    (game, id)
}

/// A shadowed point light in a small neon lamp 2 m up, and a pillar beside it.
fn shadow_game() -> (Game, Id, Id) {
    let mut game = night_floor();
    let lamp = game.part(
        Look::new("Ball", "Neon", [255, 230, 190]),
        Vec3::splat(0.4),
        CFrame::new(Vec3::new(-2.0, 2.0, 0.0)),
    );
    let id = light(
        &mut game,
        "PointLight",
        lamp,
        CFrame::IDENTITY,
        [255, 230, 190],
        3.0,
        16.0,
    );
    game.dm.set_by_name(id, "Shadows", Value::Bool(true)).expect("Shadows");
    let pillar = game.part(
        Look::new("Block", "Plastic", [196, 40, 28]),
        Vec3::new(1.0, 4.0, 1.0),
        CFrame::new(Vec3::new(0.5, 2.0, 0.0)),
    );
    (game, id, pillar)
}

/// A renderer, a scene of a game and a camera reader, ready to draw frames.
struct Frames {
    renderer: Renderer,
    scene: GpuScene,
    target: OffscreenTarget,
    camera: CameraReader,
    lighting: LightingReader,
}

impl Frames {
    fn new(context: &RenderContext, game: &mut Game, lights: LightSettings) -> Self {
        let target = OffscreenTarget::new(context, SIZE, SIZE, FORMAT).expect("a target");
        let renderer = Renderer::new(
            context,
            RendererSettings {
                lights,
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
        let _ = context.device().poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        });
    }

    fn pixels(&self, context: &RenderContext) -> Vec<[u8; 4]> {
        read_back_rgba8(context, self.target.texture())
    }
}

/// Renders `game` (two frames) with `lights` and reads the image back.
fn render(context: &RenderContext, game: &mut Game, lights: LightSettings) -> Vec<[u8; 4]> {
    let mut frames = Frames::new(context, game, lights);
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

/// The mean of the three channels around pixel `(x, y)` (a 3 × 3 block).
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

fn floor_level(game: &Game, pixels: &[[u8; 4]], x: f32, z: f32) -> f32 {
    level(pixels, project(game, Vec3::new(x, 0.0, z)))
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

/// The fallback context (vertex-buffer instances), when the machine has a device.
fn fallback() -> Option<RenderContext> {
    let context = gpu_with(&ContextOptions {
        force_vertex_instances: true,
        ..ContextOptions::default()
    })?;
    assert_eq!(InstanceBinding::for_context(&context), InstanceBinding::Vertex);
    Some(context)
}

/// `make`'s game rendered on both instance paths: the storage path's image, checked against `name`'s reference,
/// and the fallback's, which must match it.
fn both_paths(name: &str, make: fn() -> Game) -> Option<Vec<[u8; 4]>> {
    let primary = gpu()?;
    let pixels = render(&primary, &mut make(), LightSettings::default());
    check_reference(name, &pixels);
    if let Some(fallback) = fallback() {
        let vertex = render(&fallback, &mut make(), LightSettings::default());
        let diff = scenes::compare(&vertex, &pixels, 1);
        eprintln!("{name}: fallback against storage: {diff:?}");
        assert!(diff.max <= 2, "{diff:?}");
    }
    Some(pixels)
}

#[test]
fn a_point_light_falls_off_to_nothing_at_its_range() {
    let Some(pixels) = both_paths("lights_point", || point_game().0) else {
        return;
    };
    let Some(context) = gpu() else { return };
    let (game, _) = point_game();
    let dark = render(&context, &mut point_game().0, LightSettings::OFF);
    // Brightest under the light, falling with distance, and nothing past the range (12 m from a light 3 m up: the
    // floor beyond about 11.6 m).
    let at = |pixels: &[[u8; 4]], x: f32| floor_level(&game, pixels, x, 0.0);
    let (under, near, far, outside) = (at(&pixels, 0.0), at(&pixels, 3.0), at(&pixels, 7.0), at(&pixels, 12.5));
    eprintln!("floor under the light {under:.1}, 3 m {near:.1}, 7 m {far:.1}, 12.5 m {outside:.1}");
    assert!(under > near + 10.0 && near > far + 10.0, "{under} {near} {far}");
    assert!(under > at(&dark, 0.0) + 60.0, "the light lights the floor");
    assert!(
        (outside - at(&dark, 12.5)).abs() < 1.5,
        "past the range nothing changes"
    );
}

#[test]
fn a_spot_light_lights_its_cone() {
    let Some(pixels) = both_paths("lights_spot", || spot_game().0) else {
        return;
    };
    let Some(context) = gpu() else { return };
    let (game, _) = spot_game();
    let dark = render(&context, &mut spot_game().0, LightSettings::OFF);
    // A 50° cone from 6 m up reaches 2.8 m from its centre on the floor.
    let at = |pixels: &[[u8; 4]], x: f32| floor_level(&game, pixels, x, 0.0);
    let (centre, inside, outside) = (at(&pixels, 0.0), at(&pixels, 2.0), at(&pixels, 4.0));
    eprintln!("cone centre {centre:.1}, 2 m {inside:.1}, 4 m {outside:.1}");
    assert!(centre > at(&dark, 0.0) + 60.0 && inside > at(&dark, 2.0) + 30.0);
    assert!(
        (outside - at(&dark, 4.0)).abs() < 1.5,
        "outside the cone nothing changes"
    );
}

#[test]
fn a_shadowed_light_casts_shadows_from_its_lamp() {
    let Some(pixels) = both_paths("lights_shadow", || shadow_game().0) else {
        return;
    };
    let Some(context) = gpu() else { return };
    let (mut game, id, _) = shadow_game();
    // Behind the pillar (seen from the lamp) the floor is in shadow; as far on the open side it is lit. The lamp's
    // own ball casts nothing.
    let behind = floor_level(&game, &pixels, 4.0, 0.0);
    let open = floor_level(&game, &pixels, -8.0, 0.0);
    let under = floor_level(&game, &pixels, -2.0, 0.0);
    eprintln!("floor behind the pillar {behind:.1}, on the open side {open:.1}, under the lamp {under:.1}");
    assert!(open > behind + 20.0, "no shadow behind the pillar");
    assert!(under > open, "the lamp's ball shadows its own light");
    // `Shadows` off: lit behind the pillar too.
    game.dm.set_by_name(id, "Shadows", Value::Bool(false)).expect("Shadows");
    let unshadowed = render(&context, &mut game, LightSettings::default());
    let behind_lit = floor_level(&game, &unshadowed, 4.0, 0.0);
    assert!(behind_lit > behind + 20.0, "{behind_lit} against {behind}");
}

/// The light in a lamp follows the lamp; a light switched off or out of the world lights nothing.
#[test]
fn a_light_in_a_part_follows_it() {
    let Some(context) = gpu() else { return };
    let mut game = night_floor();
    let lamp = game.part(
        Look::new("Ball", "Neon", [255, 230, 190]),
        Vec3::splat(0.4),
        CFrame::new(Vec3::new(-4.0, 2.0, 0.0)),
    );
    let id = light(
        &mut game,
        "PointLight",
        lamp,
        CFrame::IDENTITY,
        [255, 255, 255],
        2.0,
        6.0,
    );
    let mut frames = Frames::new(&context, &mut game, LightSettings::default());
    frames.frame(&context, &mut game);
    let before = frames.pixels(&context);
    assert_eq!(frames.renderer.light_stats().drawn, 1);
    let lights = frames.scene.scene().lights();
    assert!(lights[0].position.abs_diff_eq(Vec3::new(-4.0, 2.0, 0.0), 1e-5));
    // The lamp moves 8 m: so does its light.
    game.dm
        .set_by_name(lamp, "CFrame", Value::CFrame(CFrame::new(Vec3::new(4.0, 2.0, 0.0))))
        .expect("CFrame");
    frames.frame(&context, &mut game);
    let after = frames.pixels(&context);
    assert!(
        frames.scene.scene().lights()[0]
            .position
            .abs_diff_eq(Vec3::new(4.0, 2.0, 0.0), 1e-5)
    );
    let left = (
        level(&before, project(&game, Vec3::new(-4.0, 0.0, 0.0))),
        level(&after, project(&game, Vec3::new(-4.0, 0.0, 0.0))),
    );
    let right = (
        level(&before, project(&game, Vec3::new(4.0, 0.0, 0.0))),
        level(&after, project(&game, Vec3::new(4.0, 0.0, 0.0))),
    );
    assert!(left.0 > left.1 + 30.0 && right.1 > right.0 + 30.0, "{left:?} {right:?}");
    // Off: nothing drawn.
    game.dm.set_by_name(id, "Enabled", Value::Bool(false)).expect("Enabled");
    frames.frame(&context, &mut game);
    assert_eq!(frames.renderer.light_stats().drawn, 0);
    // On again, then the lamp leaves the world: the light goes with it.
    game.dm.set_by_name(id, "Enabled", Value::Bool(true)).expect("Enabled");
    game.dm.set_parent(lamp, None).expect("unparented");
    frames.frame(&context, &mut game);
    assert_eq!(frames.renderer.light_stats().in_scene, 0);
}

/// A still frame keeps every atlas tile; a part moving near the light draws its tiles again, and only those that see
/// it.
#[test]
fn the_atlas_is_kept_until_a_caster_moves() {
    let Some(context) = gpu() else { return };
    let (mut game, _, _) = shadow_game();
    let ball = game.part(
        Look::new("Ball", "Plastic", [40, 120, 220]),
        Vec3::splat(1.0),
        CFrame::new(Vec3::new(-2.0, 0.5, 3.0)),
    );
    let mut frames = Frames::new(&context, &mut game, LightSettings::default());
    frames.frame(&context, &mut game);
    let first = frames.renderer.light_stats();
    assert_eq!(
        (first.shadowed, first.shadow_tiles, first.tiles_redrawn),
        (1, 6, 6),
        "{first:?}"
    );
    assert!(frames.renderer.draw_stats().light_shadow_draws > 0);
    frames.frame(&context, &mut game);
    let still = frames.renderer.light_stats();
    assert_eq!(still.tiles_redrawn, 0, "{still:?}");
    assert_eq!(frames.renderer.draw_stats().light_shadow_draws, 0);
    // The ball (beside the lamp on +Z) moves every frame: the tiles that see it are drawn again, not all six.
    let mut redrawn = Vec::new();
    for frame in 0..12 {
        game.dm
            .set_by_name(
                ball,
                "CFrame",
                Value::CFrame(CFrame::new(Vec3::new(-2.0, 0.5 + frame as f32 * 0.01, 3.0))),
            )
            .expect("CFrame");
        frames.frame(&context, &mut game);
        redrawn.push(frames.renderer.light_stats().tiles_redrawn);
    }
    eprintln!("tiles drawn while the ball moves: {redrawn:?}");
    assert!(redrawn[4..].iter().all(|&n| (1..6).contains(&n)), "{redrawn:?}");
    // GPU time is reported for the atlas pass when it ran.
    if frames.renderer.gpu_timing_enabled() {
        let mut seen = false;
        for _ in 0..20 {
            frames.frame(&context, &mut game);
            if frames
                .renderer
                .gpu_timings()
                .is_some_and(|t| t.light_shadow_ms() >= 0.0)
            {
                seen = true;
            }
        }
        assert!(seen);
    }
}

/// The mean linear light of the three channels around pixel `(x, y)` (a 3 × 3 block): what fades add up in.
fn linear_level(pixels: &[[u8; 4]], (x, y): (u32, u32)) -> f32 {
    let decode = |c: u8| {
        let c = f32::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let mut sum = 0.0;
    for dy in 0..3 {
        for dx in 0..3 {
            let p = pixels[((y + dy - 1) * SIZE + x + dx - 1) as usize];
            sum += decode(p[0]) + decode(p[1]) + decode(p[2]);
        }
    }
    sum / 27.0
}

/// Over the budget, the most important lights are drawn; a light entering the budget fades in and the one it
/// pushes out fades out over the fade time, a little each frame. The fades are driven by a fixed step per frame
/// (`Renderer::set_light_fade_step`), so the test does not depend on how fast the machine draws.
#[test]
fn the_budget_fades_lights_in_and_out() {
    let Some(context) = gpu() else { return };
    let mut game = night_floor();
    let ws = game.workspace;
    let dim = light(
        &mut game,
        "PointLight",
        ws,
        CFrame::new(Vec3::new(-5.0, 2.0, 0.0)),
        [255, 255, 255],
        0.5,
        8.0,
    );
    light(
        &mut game,
        "PointLight",
        ws,
        CFrame::new(Vec3::new(5.0, 2.0, 0.0)),
        [255, 255, 255],
        1.0,
        8.0,
    );
    // Ten frames of fade.
    const FRAMES: usize = 10;
    let settings = LightSettings {
        budget: 1,
        fade_time: 1.0,
        ..LightSettings::default()
    };
    let mut frames = Frames::new(&context, &mut game, settings);
    frames
        .renderer
        .set_light_fade_step(Some(settings.fade_time / FRAMES as f32));
    frames.frame(&context, &mut game);
    let stats = frames.renderer.light_stats();
    assert_eq!(
        (stats.in_view, stats.in_budget, stats.drawn, stats.fading),
        (2, 1, 1, 0),
        "two lights in view, the brighter one in the budget of one: {stats:?}"
    );
    let left = project(&game, Vec3::new(-5.0, 0.0, 0.0));
    let right = project(&game, Vec3::new(5.0, 0.0, 0.0));
    let first = frames.pixels(&context);
    let mut levels = vec![(linear_level(&first, left), linear_level(&first, right))];

    // The dim one becomes the brighter (by more than the budget's hysteresis): it fades in, and the other fades out.
    game.dm
        .set_by_name(dim, "Brightness", Value::Number(1.5))
        .expect("Brightness");
    for frame in 1..=FRAMES + 2 {
        frames.frame(&context, &mut game);
        let stats = frames.renderer.light_stats();
        let expected = if frame < FRAMES { (2, 2) } else { (1, 0) };
        assert_eq!(
            (stats.drawn, stats.fading),
            expected,
            "frame {frame} of the fade: (lights drawn, lights fading) {stats:?}"
        );
        let pixels = frames.pixels(&context);
        levels.push((linear_level(&pixels, left), linear_level(&pixels, right)));
    }
    eprintln!("linear light under the lights (left fading in, right fading out): {levels:?}");
    let (start, end) = (levels[0], levels[levels.len() - 1]);
    let (rise, fall) = (end.0 - start.0, start.1 - end.1);
    assert!(
        rise > 0.1 && fall > 0.1,
        "the floor under the light entering the budget brightens ({start:?} to {end:?}) and under the one leaving it \
         darkens"
    );
    // Never a pop: every frame moves each side by at most a fifth of its whole change (a tenth expected), the same
    // way all along.
    for (frame, pair) in levels.windows(2).enumerate() {
        let (up, down) = (pair[1].0 - pair[0].0, pair[0].1 - pair[1].1);
        assert!(
            (-0.01..=rise * 0.2).contains(&up),
            "frame {}: the floor under the light fading in changed by {up} of a total {rise} (linear light)",
            frame + 1
        );
        assert!(
            (-0.01..=fall * 0.2).contains(&down),
            "frame {}: the floor under the light fading out changed by {down} of a total {fall} (linear light)",
            frame + 1
        );
    }
}

/// A 2 m cube resting exactly on the night floor at the origin (its bottom on the floor's top), a ramp tilted 30°
/// towards +X beside it, and the camera on the cube's +X side looking back at its foot. The lights of the contact
/// tests stand on the −X side, so the cube's shadow falls towards +X, onto the floor in front of the camera.
fn contact_floor() -> Game {
    let mut game = night_floor();
    game.part(
        Look::new("Block", "Plastic", [163, 162, 165]),
        Vec3::splat(2.0),
        CFrame::new(Vec3::new(0.0, 1.0, 0.0)),
    );
    game.part(
        Look::new("Block", "Plastic", [163, 162, 165]),
        Vec3::new(2.4, 0.4, 2.4),
        CFrame::from_parts(RAMP_CENTRE, Quat::from_rotation_z(-30f32.to_radians())),
    );
    game.set_camera(
        CFrame::look_at(Vec3::new(5.0, 4.5, 3.5), Vec3::new(0.0, 0.4, -1.0), Vec3::Y),
        60.0,
    );
    game
}

/// The contact tests' ramp: beside the cube on the light's side, away from its shadow.
const RAMP_CENTRE: Vec3 = Vec3::new(-1.5, 1.0, -3.5);

/// Floor points along the +X foot of the contact tests' cube: from 3 to 12 cm out, the band the leak lit.
fn contact_band() -> Vec<Vec3> {
    grid_points(&[1.03, 1.06, 1.09, 1.12], &[-0.7, -0.35, 0.0, 0.35, 0.7], |x, z| {
        Vec3::new(x, 0.0, z)
    })
}

/// Floor points well inside the cube's shadow, 0.5 to 0.8 m out from its foot.
fn contact_interior() -> Vec<Vec3> {
    grid_points(&[1.5, 1.6, 1.7, 1.8], &[-0.7, -0.35, 0.0, 0.35, 0.7], |x, z| {
        Vec3::new(x, 0.0, z)
    })
}

/// Floor points as far from the cube's foot, lit beside its shadow.
fn contact_lit() -> Vec<Vec3> {
    grid_points(&[1.06, 1.3, 1.6], &[2.3, 2.5, 2.7], |x, z| Vec3::new(x, 0.0, z))
}

/// Points on the lit surfaces the acne check looks at: the cube's top, the ramp's top and the open floor.
fn lit_surfaces() -> Vec<Vec3> {
    let ramp = Quat::from_rotation_z(-30f32.to_radians());
    let mut points = grid_points(
        &[-0.7, -0.35, 0.0, 0.35, 0.7],
        &[-0.7, -0.35, 0.0, 0.35, 0.7],
        |x, z| Vec3::new(x, 2.0, z),
    );
    points.extend(grid_points(
        &[-0.9, -0.45, 0.0, 0.45, 0.9],
        &[-0.9, -0.45, 0.0, 0.45, 0.9],
        |x, z| RAMP_CENTRE + ramp * Vec3::new(x, 0.2, z),
    ));
    points.extend(contact_lit());
    points
}

/// `point(a, b)` for every `a` in `xs` and `b` in `zs`.
fn grid_points(xs: &[f32], zs: &[f32], point: impl Fn(f32, f32) -> Vec3) -> Vec<Vec3> {
    xs.iter()
        .flat_map(|&x| zs.iter().map(move |&z| (x, z)))
        .map(|(x, z)| point(x, z))
        .collect()
}

/// The pixel world point `point` lands on (one pixel, so a speckle is not averaged away), as the mean of its
/// channels.
fn pixel_level(game: &Game, pixels: &[[u8; 4]], point: Vec3) -> f32 {
    let (x, y) = project(game, point);
    let p = pixels[(y * SIZE + x) as usize];
    (f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2])) / 3.0
}

fn mean_level(game: &Game, pixels: &[[u8; 4]], points: &[Vec3]) -> f32 {
    points.iter().map(|&p| pixel_level(game, pixels, p)).sum::<f32>() / points.len() as f32
}

/// How a light's shadow meets the foot of the contact tests' cube, in sRGB levels (0 to 255).
#[derive(Clone, Copy, Debug)]
struct ContactMeasure {
    /// The floor 3 to 12 cm out from the foot, in the shadow.
    band: f32,
    /// The floor deep in the shadow.
    interior: f32,
    /// The floor beside the shadow, in the light.
    lit: f32,
}

impl ContactMeasure {
    fn of(game: &Game, pixels: &[[u8; 4]]) -> Self {
        Self {
            band: mean_level(game, pixels, &contact_band()),
            interior: mean_level(game, pixels, &contact_interior()),
            lit: mean_level(game, pixels, &contact_lit()),
        }
    }

    /// How much of the light leaks into the band: 0 when it is as dark as the shadow's interior, 1 when as bright as
    /// the lit floor.
    fn leak(&self) -> f32 {
        (self.band - self.interior) / (self.lit - self.interior).max(1.0)
    }
}

/// Saves `pixels` to the test's temporary folder as `name`.png when `METALLIC_SAVE_IMAGES=1`, for looking at.
fn save_image(name: &str, pixels: &[[u8; 4]]) {
    if std::env::var("METALLIC_SAVE_IMAGES").is_ok_and(|v| v == "1") {
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
        scenes::write_png(&out, SIZE, SIZE, pixels);
        eprintln!("wrote {}", out.display());
    }
}

/// The contact scene with the light `make` adds, its `Shadows` as `shadows`, rendered with `settings`.
fn render_contact(
    context: &RenderContext,
    make: fn(&mut Game) -> Id,
    settings: LightSettings,
    shadows: bool,
) -> (Game, Vec<[u8; 4]>) {
    let mut game = contact_floor();
    let id = make(&mut game);
    game.dm
        .set_by_name(id, "Shadows", Value::Bool(shadows))
        .expect("Shadows");
    let mut frames = Frames::new(context, &mut game, settings);
    // Whole fades in one frame: the image does not depend on how fast the machine draws.
    frames.renderer.set_light_fade_step(Some(settings.fade_time));
    for _ in 0..2 {
        frames.frame(context, &mut game);
    }
    assert_eq!(frames.renderer.light_stats().shadowed, u32::from(shadows));
    let pixels = frames.pixels(context);
    (game, pixels)
}

/// Renders the contact scene with the light `make` adds (shadowed), and again with its `Shadows` off, at each
/// preset; checks the contact band against the shadow's interior and the lit surfaces against the unshadowed image
/// (no acne). The vertex-buffer fallback must draw the same.
fn check_contact(name: &str, make: fn(&mut Game) -> Id) {
    let Some(context) = gpu() else { return };
    let presets = [
        ("medium", LightSettings::MEDIUM),
        ("low", LightSettings::LOW),
        ("high", LightSettings::HIGH),
    ];
    for (preset, settings) in presets {
        let (game, shadowed) = render_contact(&context, make, settings, true);
        let (_, unshadowed) = render_contact(&context, make, settings, false);
        if settings == LightSettings::default() {
            save_image(name, &shadowed);
        }
        let contact = ContactMeasure::of(&game, &shadowed);
        eprintln!("{name}, {preset}: {contact:?}, leak {:.1}%", contact.leak() * 100.0);
        // Acne: no lit point is darker than without the shadow (one pixel each, so a speckle shows).
        let (mut worst, mut at) = (0.0f32, Vec3::ZERO);
        for point in lit_surfaces() {
            let drop = pixel_level(&game, &unshadowed, point) - pixel_level(&game, &shadowed, point);
            if drop > worst {
                (worst, at) = (drop, point);
            }
        }
        eprintln!("{name}, {preset}: the lit surfaces' largest drop with the shadow on: {worst:.1} levels at {at}");
        assert!(
            contact.lit > contact.interior + 40.0,
            "{name}, {preset}: the cube casts no clear shadow: {contact:?}"
        );
        assert!(
            contact.leak() < 0.05,
            "{name}, {preset}: light leaks under the cube's foot: the band 3 to 12 cm out is {:.1}% of the way from \
             the shadow to the light ({contact:?})",
            contact.leak() * 100.0
        );
        assert!(
            worst <= 3.0,
            "{name}, {preset}: shadow acne on a lit surface: {worst:.1} levels darker at {at}"
        );
    }
    if let Some(fallback) = fallback() {
        let (_, storage) = render_contact(&context, make, LightSettings::default(), true);
        let (_, vertex) = render_contact(&fallback, make, LightSettings::default(), true);
        let diff = scenes::compare(&vertex, &storage, 1);
        eprintln!("{name}: fallback against storage: {diff:?}");
        assert!(diff.max <= 2, "{diff:?}");
    }
}

/// A spot light 6 m up, 2.5 m to the side of the cube, pointing straight down.
fn contact_spot(game: &mut Game) -> Id {
    let ws = game.workspace;
    let down = CFrame::from_parts(
        Vec3::new(-2.5, 6.0, 0.0),
        Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
    );
    let id = light(game, "SpotLight", ws, down, [255, 255, 255], 6.0, 20.0);
    game.dm.set_by_name(id, "Angle", Value::Number(100.0)).expect("Angle");
    id
}

/// A point light 4 m up, above and to the side of the cube.
fn contact_point(game: &mut Game) -> Id {
    let ws = game.workspace;
    light(
        game,
        "PointLight",
        ws,
        CFrame::new(Vec3::new(-2.5, 4.0, 1.0)),
        [255, 255, 255],
        3.0,
        16.0,
    )
}

/// A point light 12 m away and 4 m up, meeting the cube's top at about 15°: its tiles' texels are some 10 cm
/// across there, where the receivers' offsets reach their cap.
fn contact_far_point(game: &mut Game) -> Id {
    let ws = game.workspace;
    light(
        game,
        "PointLight",
        ws,
        CFrame::new(Vec3::new(-12.0, 4.0, 0.0)),
        [255, 255, 255],
        40.0,
        30.0,
    )
}

/// The same under a far point light, whose tiles are coarse at the cube.
#[test]
fn a_far_point_light_shadow_meets_the_foot_of_a_resting_cube() {
    check_contact("contact_far_point", contact_far_point);
}

/// A cube resting on a plate under a spot light pointing straight down: its shadow reaches its foot, with no lit
/// band between them (#126), and the lit surfaces have no acne.
#[test]
fn a_spot_light_shadow_meets_the_foot_of_a_resting_cube() {
    check_contact("contact_spot", contact_spot);
}

/// The same under a point light above and to the side.
#[test]
fn a_point_light_shadow_meets_the_foot_of_a_resting_cube() {
    check_contact("contact_point", contact_point);
}

/// A scene without local lights holds none of their GPU memory; the first light makes its buffers and the cluster
/// texture, and only the first shadow makes the atlas, at the size the settings say then.
#[test]
fn light_memory_is_made_when_first_needed() {
    let Some(context) = gpu() else { return };
    let mut game = night_floor();
    let mut frames = Frames::new(&context, &mut game, LightSettings::default());
    frames.frame(&context, &mut game);
    assert_eq!(
        frames.renderer.light_stats().gpu_bytes,
        0,
        "GPU memory held by local lights in a scene without any"
    );
    let ws = game.workspace;
    let lamp = light(
        &mut game,
        "PointLight",
        ws,
        CFrame::new(Vec3::new(0.0, 3.0, 0.0)),
        [255, 255, 255],
        1.0,
        10.0,
    );
    frames.frame(&context, &mut game);
    let unshadowed = frames.renderer.light_stats().gpu_bytes;
    assert!(
        unshadowed > 0 && unshadowed < 1 << 20,
        "GPU memory held by one light without a shadow: {unshadowed} bytes, expected the buffers and the cluster \
         texture only"
    );
    game.dm
        .set_by_name(lamp, "Shadows", Value::Bool(true))
        .expect("Shadows");
    frames.frame(&context, &mut game);
    let atlas = u64::from(LightSettings::default().atlas_resolution).pow(2) * 4;
    let shadowed = frames.renderer.light_stats().gpu_bytes;
    assert!(
        shadowed >= unshadowed + atlas,
        "GPU memory held once a light has a shadow: {shadowed} bytes, expected at least {} (the atlas)",
        unshadowed + atlas
    );
    eprintln!("light GPU memory: none, {unshadowed} with a light, {shadowed} with a shadow");
}
