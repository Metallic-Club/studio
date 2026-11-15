//! Materials' textures (#679) on a real adapter: a base-colour checker, a grooved normal map and an
//! occlusion-roughness-metallic map on a block, a ball and a long part (which shows the tiling in metres), against a
//! committed reference image, and the same image on the vertex-buffer fallback. A material without textures draws as
//! it did. Skips when the machine has no adapter (see `support::gpu`); `METALLIC_UPDATE_REFERENCES=1` writes the
//! reference after a deliberate change.

mod support;

use std::path::PathBuf;

use metallic_core::glam::{Quat, Vec3};
use metallic_core::{CFrame, Value};
use metallic_render::scene::RenderComponents;
use metallic_render::{
    CameraReader, ContextOptions, GpuScene, MATERIAL_TEXTURE_SIZE, MaterialParams, MaterialTextureSet, OffscreenTarget,
    PassId, PassSet, RenderContext, Renderer, RendererSettings, material_image, wgpu,
};
use support::scenes::{self, Game, Look};
use support::{assert_no_wgpu_errors, collect_wgpu_messages, gpu, gpu_with, read_back_rgba8, reference_max_mean};

const SIZE: u32 = 256;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const PASSES: PassSet = PassSet::EMPTY
    .with(PassId::Opaque)
    .with(PassId::Transparent)
    .with(PassId::Post)
    .with(PassId::Ui);
const THRESHOLD: u8 = 12;
const MAX_OVER: f64 = 0.01;

/// The textured material's key, as parts name it.
const TEXTURED: &str = "Checker";

/// A square picture of `MATERIAL_TEXTURE_SIZE` from `texel(x, y)`.
fn picture(texel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let side = MATERIAL_TEXTURE_SIZE;
    (0..side * side).flat_map(|i| texel(i % side, i / side)).collect()
}

/// A checker of 8 × 8 squares in red and cream, grooves along V, and a matte occlusion map darker in the grooves.
fn textures() -> MaterialTextureSet {
    let side = MATERIAL_TEXTURE_SIZE;
    let square = side / 8;
    let color = picture(|x, y| {
        if (x / square + y / square) % 2 == 0 {
            [200, 40, 30, 255]
        } else {
            [240, 230, 210, 255]
        }
    });
    let groove = |x: u32| (x as f32 / side as f32 * std::f32::consts::TAU * 8.0).sin();
    let normal = picture(|x, _| {
        let slope = groove(x) * 0.8;
        let n = Vec3::new(-slope, 0.0, 1.0).normalize();
        let byte = |v: f32| ((v * 0.5 + 0.5) * 255.0).round() as u8;
        [byte(n.x), byte(n.y), byte(n.z), 255]
    });
    let orm = picture(|x, _| {
        let occlusion = (190.0 + 65.0 * groove(x)) as u8;
        [occlusion, 255, 0, 255]
    });
    MaterialTextureSet {
        color: material_image(side, side, &color),
        normal: material_image(side, side, &normal),
        orm: material_image(side, side, &orm),
        emission: None,
    }
}

fn game() -> Game {
    let mut game = Game::new();
    game.part(
        Look::new("Block", "Concrete", [120, 124, 128]),
        Vec3::new(40.0, 1.0, 40.0),
        CFrame::new(Vec3::new(0.0, -0.5, 0.0)),
    );
    let white = [255, 255, 255];
    let parts = [
        ("Block", Vec3::new(4.0, 4.0, 4.0), Vec3::new(-4.0, 2.0, 0.0), 30.0),
        ("Ball", Vec3::new(4.0, 4.0, 4.0), Vec3::new(2.0, 2.0, 1.0), 0.0),
        ("Block", Vec3::new(16.0, 1.0, 2.0), Vec3::new(0.0, 0.5, -5.0), 0.0),
    ];
    for (shape, size, at, yaw) in parts {
        let id = game.part(
            Look::new(shape, "Plastic", white),
            size,
            CFrame::from_parts(at, Quat::from_rotation_y(f32::to_radians(yaw))),
        );
        game.dm
            .set_by_name(id, "Material", Value::String(TEXTURED.into()))
            .expect("the material");
    }
    game.set_camera(
        CFrame::look_at(Vec3::new(9.0, 9.0, 12.0), Vec3::new(0.0, 1.0, -1.0), Vec3::Y),
        60.0,
    );
    game
}

fn render(context: &RenderContext, textured: bool) -> Vec<[u8; 4]> {
    collect_wgpu_messages();
    let mut game = game();
    let target = OffscreenTarget::new(context, SIZE, SIZE, FORMAT).expect("a target");
    let mut renderer = Renderer::new(
        context,
        RendererSettings {
            passes: PASSES,
            gpu_timing: false,
            ..RendererSettings::new(FORMAT, SIZE, SIZE)
        },
    )
    .expect("a renderer");
    renderer.wait_for_pipelines();
    let parts = renderer.parts_mut();
    parts.materials_mut().set_material(
        TEXTURED,
        MaterialParams {
            roughness: 0.5,
            tile_size: 4.0,
            ..MaterialParams::PLASTIC
        },
    );
    if textured {
        parts.add_material_textures(TEXTURED, &textures());
    }
    let mut scene = GpuScene::new(
        context,
        &mut game.dm,
        Some(game.workspace),
        &RenderComponents::builtin(),
    )
    .expect("a scene");
    let mut reader = CameraReader::new();
    for _ in 0..2 {
        let camera = reader.read(&game.dm, game.workspace);
        if let Some(camera) = camera {
            scene.set_camera_position(&game.dm, camera.position);
        }
        scene.sync(&mut game.dm);
        renderer.render_scene(target.view(), &mut scene, camera.as_ref());
    }
    let pixels = read_back_rgba8(context, target.texture());
    assert_no_wgpu_errors();
    pixels
}

fn reference_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/reference/material_textures.png")
}

#[test]
fn textured_materials_match_the_reference_on_both_paths() {
    let Some(context) = gpu() else { return };
    let pixels = render(&context, true);
    let path = reference_path();
    if std::env::var("METALLIC_UPDATE_REFERENCES").is_ok_and(|v| v == "1") {
        scenes::write_png(&path, SIZE, SIZE, &pixels);
        eprintln!("wrote {}", path.display());
    } else {
        let (_, _, reference) = scenes::read_png(&path)
            .unwrap_or_else(|| panic!("{} is missing; run with METALLIC_UPDATE_REFERENCES=1", path.display()));
        let diff = scenes::compare(&pixels, &reference, THRESHOLD);
        if diff.over > MAX_OVER || diff.mean > reference_max_mean(&context) {
            let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("material_textures.png");
            scenes::write_png(&out, SIZE, SIZE, &pixels);
            panic!(
                "differs from the reference ({diff:?}); the image is in {}",
                out.display()
            );
        }
    }
    // The textures show: the same scene untextured is another picture.
    let plain = render(&context, false);
    assert!(scenes::compare(&pixels, &plain, THRESHOLD).over > 0.05);

    let Some(fallback) = gpu_with(&ContextOptions {
        force_vertex_instances: true,
        ..ContextOptions::default()
    }) else {
        return;
    };
    let diff = scenes::compare(&pixels, &render(&fallback, true), THRESHOLD);
    assert!(diff.over <= MAX_OVER, "the fallback draws another image ({diff:?})");
}
