//! Material assets (#678, ADR 0012) reaching the renderer: parts naming a project's material by id share it, and
//! both change when its file does; a material named by name is the project's of that name, else the built-in one;
//! a material that is gone is a warning, and its parts draw as plastic. Skips without an adapter unless
//! `METALLIC_REQUIRE_GPU=1` (CI sets it).

use std::path::Path;
use std::time::Duration;

use metallic_engine::metallic_core::{DataModel, Id, Value};
use metallic_engine::metallic_project::material::{Material, builtin, builtin_path};
use metallic_engine::metallic_project::{Project, Template};
use metallic_engine::metallic_render::scene::RenderComponents;
use metallic_engine::metallic_render::{
    ContextOptions, GpuScene, MaterialParams, RenderContext, RenderError, Renderer, RendererSettings, wgpu,
};
use metallic_engine::{MeshAssets, material_params};

/// An id no asset has.
const GONE: &str = "0123456789abcdef0123456789abcdef";

fn gpu() -> Option<RenderContext> {
    match RenderContext::new_offscreen(&ContextOptions::default()) {
        Ok(context) => Some(context),
        Err(error @ RenderError::NoAdapter { .. }) => {
            if std::env::var("METALLIC_REQUIRE_GPU").is_ok_and(|value| value == "1") {
                panic!("{error}, and METALLIC_REQUIRE_GPU=1");
            }
            eprintln!("skipped: {error}");
            None
        }
        Err(error) => panic!("could not create a device: {error}"),
    }
}

fn part(dm: &mut DataModel, workspace: Id, material: &str) {
    let id = dm
        .create(dm.registry().class_id("Part").expect("test setup"))
        .expect("test setup");
    dm.set_by_name(id, "Material", Value::String(material.into()))
        .expect("test setup");
    dm.set_parent(id, Some(workspace)).expect("test setup");
}

#[test]
fn parts_share_a_material_asset_and_follow_its_file() {
    let Some(context) = gpu() else { return };
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("material_assets");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("test setup");
    let template = Template::from_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d"))
        .expect("test setup");
    let mut project = Project::create(dir.join("Game.mtproj"), &template, "Game").expect("test setup");
    let shiny = Material {
        roughness: 0.2,
        metallic: 1.0,
        ..Material::default()
    };
    let id = project
        .create_member("Assets/Materials/Shiny.material", shiny.to_json())
        .expect("test setup")
        .to_string();
    project.refresh_assets().expect("test setup");
    // The project's Neon glows red; Glass is only built in.
    let red_neon = Material {
        emission: [1.0, 0.0, 0.0],
        ..builtin("Neon").expect("built in")
    };
    project
        .write(&builtin_path("Neon"), red_neon.to_json())
        .expect("test setup");
    project.remove(&builtin_path("Glass"));

    let mut dm = metallic_classes::new_game().expect("test setup");
    let workspace = dm
        .service(dm.registry().class_id("Workspace").expect("test setup"))
        .expect("test setup");
    for material in [id.as_str(), id.as_str(), "Neon", "Glass", GONE, "Cheese"] {
        part(&mut dm, workspace, material);
    }
    let mut renderer = Renderer::new(
        &context,
        RendererSettings::new(wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64),
    )
    .expect("test setup");
    let mut scene =
        GpuScene::new(&context, &mut dm, Some(workspace), &RenderComponents::builtin()).expect("test setup");
    scene.sync(&mut dm);
    let mut assets = MeshAssets::new(project.asset_reader());
    assert!(assets.wait(scene.scene(), &mut renderer, Duration::from_secs(60)));

    // Two parts, one material: one key, read once.
    assert_eq!(scene.scene().material_keys().len(), 5);
    let materials = renderer.parts().materials();
    assert_eq!(materials.resolve(&id), material_params(&shiny));
    assert_eq!(
        materials.resolve("Neon"),
        material_params(&red_neon),
        "the project's Neon"
    );
    assert_eq!(
        materials.resolve("Glass"),
        MaterialParams::builtin("Glass").expect("built in")
    );
    assert_eq!(materials.resolve(GONE), MaterialParams::PLASTIC);
    let warnings = assets.warnings().join("\n");
    assert!(warnings.contains(GONE), "{warnings}");
    assert!(warnings.contains("Cheese"), "{warnings}");

    // The file changes (in memory, as Studio edits it): every part using it follows.
    let rough = Material {
        roughness: 0.9,
        ..shiny
    };
    let member = project
        .assets()
        .path(id.parse().expect("an id"))
        .expect("indexed")
        .to_owned();
    project.write(&member, rough.to_json()).expect("test setup");
    assets.read_from(project.asset_reader());
    assets.reload_material(&id);
    assert!(assets.wait(scene.scene(), &mut renderer, Duration::from_secs(60)));
    assert_eq!(renderer.parts().materials().resolve(&id), material_params(&rough));
    let _ = std::fs::remove_dir_all(&dir);
}
