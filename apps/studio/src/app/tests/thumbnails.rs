//! Thumbnails and the image preview (#681): a picture's tile shows the picture at its aspect ratio; a double-click
//! opens its preview with its size, format, file size and the materials that use it; material and scene tiles ask
//! for rendered thumbnails, again once their file changes.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use metallic_engine::metallic_project::material::Material;

use super::*;
use crate::editor::pictures::Picture;

fn editor<'a>(harness: &'a mut Harness<'_, Test>) -> &'a mut EditorState {
    harness.state_mut().studio.editor_mut().unwrap()
}

/// A `width` × `height` PNG of one colour.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let texels: Vec<u8> = (0..width * height).flat_map(|_| [200u8, 100, 50, 255]).collect();
    writer.write_image_data(&texels).unwrap();
    writer.finish().unwrap();
    bytes
}

/// Double-clicks the tile `name`, the clicks timed as `double_click`'s are (#662).
fn double_click_tile(harness: &mut Harness<'_, Test>, name: &str) {
    let time = harness.ctx.input(|input| input.time) + 1.0;
    harness.input_mut().time = Some(time);
    harness.get_by_role_and_label(Role::GridCell, name).click();
    harness.step();
    harness.input_mut().time = Some(time + 0.05);
    harness.get_by_role_and_label(Role::GridCell, name).click();
    settle(harness);
    harness.input_mut().time = None;
}

#[test]
fn a_picture_shows_on_its_tile_and_opens_in_a_preview_with_its_facts() {
    let mut harness = editing(settings_file("thumbnails-picture", None), "Pictures Game");
    harness.set_size(egui::vec2(1400.0, 1000.0));
    let editor_state = editor(&mut harness);
    if !editor_state.project.contains("Assets/wide.png") {
        editor_state
            .project
            .create_member("Assets/wide.png", png(40, 10))
            .unwrap();
    }
    let id = editor_state.project.assets().id("Assets/wide.png").unwrap();
    // A material that uses it.
    let brick = editor_state
        .project
        .assets()
        .id("Assets/Materials/Brick.material")
        .unwrap();
    let mut material = editor_state.read_material(brick).unwrap();
    material.textures.color = Some(id);
    editor_state
        .project
        .write("Assets/Materials/Brick.material", material.to_json())
        .unwrap();
    editor_state.browser.open_folder("Assets");
    editor_state.browser.query = "wide".into();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Tab, "Assets").click();
    settle(&mut harness);
    // Decoded for its tile (off the UI thread, #771), a quarter as tall as it is wide.
    until_thumbnail(&mut harness, id);
    let thumbnail = editor(&mut harness)
        .thumbnails
        .get(&id)
        .cloned()
        .flatten()
        .expect("a thumbnail");
    let [width, height] = thumbnail.size();
    assert_eq!(width, height * 4);

    double_click_tile(&mut harness, "wide");
    harness.get_by_role_and_label(Role::Tab, "wide.png");
    harness.get_by_label_contains("40 \u{00d7} 10 px  \u{2022}  PNG  \u{2022}");
    harness.get_by_label("Used by Assets/Materials/Brick.material");
    let picture = harness.get_by_role_and_label(Role::Image, "Assets/wide.png").rect();
    assert!((picture.width() / picture.height() - 4.0).abs() < 0.05, "{picture:?}");
}

#[test]
fn materials_and_scenes_ask_for_thumbnails_and_again_once_they_change() {
    let mut harness = editing(settings_file("thumbnails-rendered", None), "Rendered Game");
    harness.set_size(egui::vec2(1400.0, 1000.0));
    editor(&mut harness).browser.open_folder("Assets/Materials");
    editor(&mut harness).browser.query = "Neon".into();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Tab, "Assets").click();
    settle(&mut harness);
    let editor_state = editor(&mut harness);
    let neon = editor_state
        .project
        .assets()
        .id("Assets/Materials/Neon.material")
        .unwrap();
    // Its tile asks for it (and the tree's rows for theirs, #756).
    let asked = std::iter::from_fn(|| editor_state.pictures.take_next())
        .find(|wanted| wanted.id == neon)
        .expect("asked for");
    assert_eq!(asked.picture, Picture::Material);
    // Drawn (as the renderer would), then the file changes: asked for again, the old one shown meanwhile.
    let texture = egui::TextureId::User(1);
    editor_state.pictures.done(neon, asked.fingerprint, texture);
    while editor_state.pictures.take_next().is_some() {}
    let mut material = editor_state.read_material(neon).unwrap();
    material.roughness = 0.2;
    editor_state
        .project
        .write("Assets/Materials/Neon.material", Material { ..material }.to_json())
        .unwrap();
    settle(&mut harness);
    let editor_state = editor(&mut harness);
    let again = std::iter::from_fn(|| editor_state.pictures.take_next())
        .find(|wanted| wanted.id == neon)
        .expect("asked for again");
    assert_ne!(again.fingerprint, asked.fingerprint);

    // A scene's tile asks for its picture too.
    editor_state.browser.query = "Main".into();
    settle(&mut harness);
    let editor_state = editor(&mut harness);
    let scene = editor_state.project.assets().id("Scenes/Main.scene").unwrap();
    let mut asked = Vec::new();
    while let Some(wanted) = editor_state.pictures.take_next() {
        asked.push((wanted.id, wanted.picture));
    }
    assert!(asked.contains(&(scene, Picture::Scene)), "{asked:?}");
}

#[test]
#[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
fn screenshot_of_the_thumbnails() {
    use super::offscreen::Offscreen;
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
    let path = config("shot-thumbnails", Some("[general]\nskip_sign_in = true\n"), None);
    let project = make_project_once(path.parent().unwrap(), "Gallery");
    let mut studio = studio_after_startup(steps_reading(path));
    let mut offscreen = Offscreen::new(egui::vec2(1280.0, 800.0), 1.0).expect("a GPU");
    studio.open(project);
    for _ in 0..10_000 {
        offscreen.frame(&mut studio);
        if !studio.is_opening() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    studio.editor_mut().unwrap().browser.open_folder("Assets/Materials");
    studio.dock_mut().open(crate::panels::ids::ASSETS);
    // One thumbnail a frame, and again once its assets are here.
    for _ in 0..120 {
        offscreen.frame(&mut studio);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    write_png(&out.join("681-thumbnails.png"), offscreen.read());
}
