//! The material editor (#680), in the Assets browser's inspector (#742): a material selected shows its ball, its name
//! with Apply and Copy, and its settings; edits are written into its file and undone with the Assets panel's Undo; a
//! texture slot takes a project picture; a click on a material only selects it (#735), and a material dropped on a
//! part in the viewport gives it to that part.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use metallic_engine::metallic_core::{Id, Value};
use metallic_engine::metallic_project::AssetId;
use metallic_engine::metallic_project::material::Material;

use super::*;

fn editor<'a>(harness: &'a mut Harness<'_, Test>) -> &'a mut EditorState {
    harness.state_mut().studio.editor_mut().unwrap()
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

fn material_id(harness: &mut Harness<'_, Test>, name: &str) -> AssetId {
    let path = format!("Assets/Materials/{name}.material");
    editor(harness).project.assets().id(&path).unwrap()
}

fn baseplate(harness: &mut Harness<'_, Test>) -> Id {
    let editor = editor(harness);
    let workspace = editor
        .dm
        .service(editor.dm.registry().class_id("Workspace").unwrap())
        .unwrap();
    editor
        .dm
        .find_first_child(workspace, "Baseplate", false)
        .unwrap()
        .unwrap()
}

/// A project in the Assets panel, its materials' folder open, searched for `name` so its tile is in view.
fn materials(test: &str, name: &str) -> Harness<'static, Test> {
    let mut harness = editing(settings_file(test, None), &format!("Materials {test}"));
    harness.set_size(egui::vec2(1400.0, 1000.0));
    let editor_state = editor(&mut harness);
    if !editor_state.project.contains("Assets/bark.png") {
        editor_state
            .project
            .create_member("Assets/bark.png", vec![1; 64])
            .unwrap();
    }
    editor_state.browser.open_folder("Assets/Materials");
    editor_state.browser.query = name.to_owned();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Tab, "Assets").click();
    settle(&mut harness);
    harness
}

/// Scrolls the inspector until the node `label` (with `role`) is inside it, and returns its rect.
fn scroll_to(harness: &mut Harness<'_, Test>, role: Role, label: &str) -> egui::Rect {
    for _ in 0..40 {
        let pane = harness.get_by_label("Inspector").rect();
        let node = harness.get_by_role_and_label(role, label).rect();
        if node.top() >= pane.top() && node.bottom() <= pane.bottom() {
            return node;
        }
        let down = node.bottom() > pane.bottom();
        harness.event(egui::Event::PointerMoved(pane.center()));
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, if down { -40.0 } else { 40.0 }),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
        });
        settle(harness);
    }
    panic!("{label} is not in the inspector's view");
}

#[test]
fn a_material_is_edited_in_the_inspector_and_its_edits_are_written_and_undone() {
    let mut harness = materials("material-editor-edit", "Wood");
    let id = material_id(&mut harness, "Wood");
    double_click_tile(&mut harness, "Wood");
    assert!(
        harness.query_by_role_and_label(Role::Tab, "Wood").is_none(),
        "no tab of its own"
    );
    harness.get_by_label("Inspector");
    harness.get_by_role_and_label(Role::Image, "Material preview");
    assert_eq!(
        harness
            .get_by_role_and_label(Role::TextInput, "Asset name")
            .value()
            .as_deref(),
        Some("Wood")
    );
    // The sections, in order.
    let sliders: Vec<f32> = ["Normal strength", "Roughness", "Metallic", "Emission strength"]
        .iter()
        .map(|label| harness.get_by_role_and_label(Role::Slider, label).rect().top())
        .collect();
    assert!(sliders.windows(2).all(|pair| pair[0] < pair[1]), "{sliders:?}");

    // An edit is written into the file at once, and noted for the renderers.
    let before = editor(&mut harness).read_material(id).unwrap();
    let shiny = Material {
        roughness: 0.1,
        ..before
    };
    editor(&mut harness).edit_material(id, shiny).unwrap();
    settle(&mut harness);
    assert_eq!(editor(&mut harness).read_material(id).unwrap(), shiny);
    // (The viewport's renderer took the change; kittest draws no viewport.)
    let _ = editor(&mut harness).materials.take_changed();

    // A picture in the colour's texture slot, from its picker.
    scroll_to(&mut harness, Role::ComboBox, "Colour texture");
    harness.get_by_role_and_label(Role::ComboBox, "Colour texture").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Assets/bark.png").click();
    settle(&mut harness);
    let bark = editor(&mut harness).project.assets().id("Assets/bark.png").unwrap();
    assert_eq!(
        editor(&mut harness).read_material(id).unwrap().textures.color,
        Some(bark)
    );
    assert_eq!(editor(&mut harness).materials.take_changed(), [id]);

    // Undo, with the Assets panel focused, takes the material's edits back one at a time; Redo does the last again.
    assert_eq!(harness.state().studio.dock().context(), crate::keymap::Context::Assets);
    let scene_steps = editor(&mut harness).history().can_undo();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    settle(&mut harness);
    assert_eq!(editor(&mut harness).read_material(id).unwrap(), shiny);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    settle(&mut harness);
    assert_eq!(editor(&mut harness).read_material(id).unwrap(), before);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Y);
    settle(&mut harness);
    assert_eq!(editor(&mut harness).read_material(id).unwrap(), shiny);
    assert_eq!(
        editor(&mut harness).history().can_undo(),
        scene_steps,
        "the scene's steps untouched"
    );
}

/// #742: the inspector's buttons beside the name give the material to the selected parts, and make a copy of it,
/// which the inspector then shows.
#[test]
fn the_inspector_applies_and_copies_a_material() {
    let mut harness = materials("material-editor-buttons", "Neon");
    let neon = material_id(&mut harness, "Neon");
    let part = baseplate(&mut harness);
    harness.get_by_role_and_label(Role::GridCell, "Neon").click();
    settle(&mut harness);
    let apply = "Apply to the selected parts";
    assert!(
        egui_kittest::kittest::NodeT::accesskit_node(&harness.get_by_role_and_label(Role::Button, apply)).is_disabled(),
        "no part selected"
    );
    editor(&mut harness).select(Some(part));
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, apply).click();
    settle(&mut harness);
    assert_eq!(
        editor(&mut harness).dm.get_by_name(part, "Material"),
        Ok(Value::String(neon.to_string().into()))
    );
    harness.get_by_role_and_label(Role::Button, "Make a copy").click();
    settle(&mut harness);
    assert!(
        editor(&mut harness)
            .project
            .contains("Assets/Materials/Neon 2.material")
    );
    assert_eq!(
        harness
            .get_by_role_and_label(Role::TextInput, "Asset name")
            .value()
            .as_deref(),
        Some("Neon 2")
    );
}

/// #735: a click on a material selects it and leaves the selected parts as they are.
#[test]
fn a_click_on_a_material_only_selects_it() {
    let mut harness = materials("material-editor-apply", "Neon");
    let part = baseplate(&mut harness);
    let before = editor(&mut harness).dm.get_by_name(part, "Material").unwrap();
    editor(&mut harness).select(Some(part));
    settle(&mut harness);
    harness.get_by_role_and_label(Role::GridCell, "Neon").click();
    settle(&mut harness);
    assert_eq!(editor(&mut harness).dm.get_by_name(part, "Material"), Ok(before));
    assert!(
        editor(&mut harness)
            .browser
            .selected
            .contains("Assets/Materials/Neon.material")
    );
}

#[test]
fn a_material_dropped_on_a_part_in_the_viewport_is_its_material() {
    let mut harness = materials("material-editor-drop", "Glass");
    let glass = material_id(&mut harness, "Glass");
    let part = baseplate(&mut harness);
    // The template's camera looks at the baseplate: the viewport's middle is on it.
    let viewport = harness.get_by_role_and_label(Role::Tab, "Viewport").rect();
    let from = harness.get_by_role_and_label(Role::GridCell, "Glass").rect().center();
    let panel = egui::pos2(viewport.center().x, viewport.bottom() + 200.0);
    drag(&mut harness, from, panel);
    assert_eq!(
        editor(&mut harness).dm.get_by_name(part, "Material"),
        Ok(Value::String(glass.to_string().into()))
    );
}

#[test]
#[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
fn screenshot_of_the_material_editor() {
    use super::offscreen::Offscreen;
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
    let path = config("shot-material", Some("[general]\nskip_sign_in = true\n"), None);
    let project = make_project_once(path.parent().unwrap(), "Workshop");
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
    let editor = studio.editor_mut().unwrap();
    editor.browser.open_folder("Assets/Materials");
    editor.browser.selected = ["Assets/Materials/Metal.material".to_owned()].into();
    let id = editor.project.assets().id("Assets/Materials/Metal.material").unwrap();
    let gold = Material {
        color: [1.0, 0.78, 0.34],
        roughness: 0.3,
        ..editor.read_material(id).unwrap()
    };
    studio.dock_mut().open(crate::panels::ids::ASSETS);
    for _ in 0..3 {
        offscreen.frame(&mut studio);
    }
    studio.editor_mut().unwrap().edit_material(id, gold).unwrap();
    for _ in 0..60 {
        offscreen.frame(&mut studio);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    write_png(&out.join("680-material-editor.png"), offscreen.read());
}
