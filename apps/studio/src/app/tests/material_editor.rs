//! The material editor (#680), in the Properties panel (#742, #779): a material selected in the Assets browser shows
//! its ball, its name with Apply and Copy, and its settings; edits are written into its file and undone with Undo in
//! the Assets or the Properties panel; a texture map is a picture field (#762); a click on a material only selects it
//! (#735), and a material dropped on a part in the viewport gives it to that part.

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

/// Scrolls the Properties panel until the node `label` (with `role`) is inside it, and returns its rect.
fn scroll_to(harness: &mut Harness<'_, Test>, role: Role, label: &str) -> egui::Rect {
    for _ in 0..40 {
        let pane = harness.get_by_label("Asset properties").rect();
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
    panic!("{label} is not in the Properties panel's view");
}

#[test]
fn a_material_is_edited_in_the_properties_panel_and_its_edits_are_written_and_undone() {
    let mut harness = materials("material-editor-edit", "Wood");
    // Tall enough for every row to show.
    harness.set_size(egui::vec2(1400.0, 3000.0));
    let id = material_id(&mut harness, "Wood");
    double_click_tile(&mut harness, "Wood");
    assert!(
        harness.query_by_role_and_label(Role::Tab, "Wood").is_none(),
        "no tab of its own"
    );
    harness.get_by_label("Asset properties");
    harness.get_by_role_and_label(Role::Image, "Material preview");
    assert_eq!(
        harness
            .get_by_role_and_label(Role::TextInput, "Name")
            .value()
            .as_deref(),
        Some("Wood")
    );
    // The texture maps first, in order (#762), then the values, typed and dragged as numbers are (#779).
    let maps = [
        "Color map",
        "Normal map",
        "Roughness map",
        "Metalness map",
        "Emissive map",
        "Ambient occlusion map",
    ];
    let mut tops: Vec<f32> = maps
        .iter()
        .map(|label| harness.get_by_role_and_label(Role::TextInput, label).rect().top())
        .collect();
    tops.extend(
        ["Normal strength", "Roughness", "Metalness", "Emissive strength"]
            .iter()
            .map(|label| harness.get_by_role_and_label(Role::SpinButton, label).rect().top()),
    );
    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "{tops:?}");
    // Each map is a field with a button that opens a window to choose a picture; none has a slider.
    for map in maps {
        harness.get_by_role_and_label(Role::Button, &format!("Choose {map}"));
        assert!(harness.query_by_role_and_label(Role::Slider, map).is_none());
    }

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

    // A picture in the colour map, typed by its path.
    scroll_to(&mut harness, Role::TextInput, "Color map");
    harness.get_by_role_and_label(Role::TextInput, "Color map").click();
    settle(&mut harness);
    harness
        .get_by_role_and_label(Role::TextInput, "Color map")
        .type_text("Assets/bark.png");
    harness.key_press(egui::Key::Enter);
    settle(&mut harness);
    assert_eq!(
        harness
            .get_by_role_and_label(Role::TextInput, "Color map")
            .value()
            .as_deref(),
        Some("bark"),
        "the picture's name"
    );
    let bark = editor(&mut harness).project.assets().id("Assets/bark.png").unwrap();
    assert_eq!(
        editor(&mut harness).read_material(id).unwrap().textures.color,
        Some(bark)
    );
    assert_eq!(editor(&mut harness).materials.take_changed(), [id]);

    // Undo, with the Properties panel focused (where the field was typed in), takes the material's edits back one at a
    // time; Redo does the last again.
    assert_eq!(
        harness.state().studio.dock().context(),
        crate::keymap::Context::Properties
    );
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

    // Each map's row has a preview beside its name, as a colour's has its swatch (#780); a click opens its chooser.
    let preview = scroll_to(&mut harness, Role::Button, "Emissive map preview");
    let name = harness.get_by_role_and_label(Role::TextInput, "Emissive map").rect();
    assert!(
        preview.right() < name.left(),
        "in the name's column: {preview:?} {name:?}"
    );
    harness
        .get_by_role_and_label(Role::Button, "Emissive map preview")
        .click();
    settle(&mut harness);
    settle(&mut harness);
    harness.get_by_label("Search the pictures");
    harness.key_press(egui::Key::Escape);
    settle(&mut harness);
    assert!(harness.query_by_label("Search the pictures").is_none());

    // A picture chosen in the window, for the normal map.
    scroll_to(&mut harness, Role::Button, "Choose Normal map");
    harness.get_by_role_and_label(Role::Button, "Choose Normal map").click();
    settle(&mut harness);
    harness.get_by_label("Search the pictures");
    let time = harness.ctx.input(|input| input.time) + 1.0;
    harness.input_mut().time = Some(time);
    harness.get_by_role_and_label(Role::Row, "bark").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Select").click();
    settle(&mut harness);
    assert_eq!(
        editor(&mut harness).read_material(id).unwrap().textures.normal,
        Some(bark)
    );
}

/// #742, #779: the buttons beside a material's name in the Properties panel give it to the selected parts, and make a
/// copy of it, which the panel then shows. Selecting a part shows the part; a click on the material shows it again.
#[test]
fn the_properties_panel_applies_and_copies_a_material() {
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
    assert!(harness.query_by_label("Asset properties").is_none(), "the part shows");
    harness.get_by_role_and_label(Role::GridCell, "Neon").click();
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
            .get_by_role_and_label(Role::TextInput, "Name")
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

/// #805: a drag across the material's ball in the Properties panel spins it, as if the pointer turned it.
#[test]
fn the_ball_spins_when_dragged() {
    let mut harness = materials("material-editor-spin", "Wood");
    harness.get_by_role_and_label(Role::GridCell, "Wood").click();
    settle(&mut harness);
    let ball = harness.get_by_role_and_label(Role::Image, "Material preview").rect();
    assert_eq!(
        editor(&mut harness).materials.preview_turn,
        metallic_engine::metallic_core::glam::Quat::IDENTITY
    );
    let from = ball.center();
    let to = from + egui::vec2(-30.0, 30.0);
    harness.event(egui::Event::PointerMoved(from));
    harness.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.event(egui::Event::PointerMoved(to));
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    settle(&mut harness);
    let turn = editor(&mut harness).materials.preview_turn;
    assert_eq!(
        turn,
        crate::app::thumbnails::spin(metallic_engine::metallic_core::glam::Quat::IDENTITY, to - from),
        "turned by the drag"
    );
}
