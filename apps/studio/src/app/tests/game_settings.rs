//! #782: the Game Settings window, opened by Shift+Alt+S: an offline game's name, author, version and icon, and the
//! graphics it ships with; an online game's page says the platform is not built yet.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use metallic_engine::metallic_project::{GAME_GRAPHICS_FILE, ProjectMode};

use super::*;

#[test]
fn game_settings_edit_the_game_s_manifest_and_graphics() {
    let mut harness = editing(settings_file("game-settings", None), "Settings Game");
    harness.set_size(egui::vec2(1400.0, 1000.0));
    settle(&mut harness);
    assert!(harness.query_by_label("Game Settings").is_none());
    harness.key_press_modifiers(egui::Modifiers::SHIFT | egui::Modifiers::ALT, egui::Key::S);
    settle(&mut harness);
    harness.get_by_label("What the game is called and who made it. An offline game's program shows them.");

    // Its author and version, typed.
    for (field, text) in [("Author", "Ada"), ("Version", "1.2.0")] {
        harness.get_by_role_and_label(Role::TextInput, field).click();
        settle(&mut harness);
        harness.get_by_role_and_label(Role::TextInput, field).type_text(text);
        settle(&mut harness);
    }
    let editor = harness.state_mut().studio.editor_mut().unwrap();
    let manifest = editor.project.manifest().clone();
    assert_eq!((manifest.author.as_str(), manifest.version.as_str()), ("Ada", "1.2.0"));
    harness.get_by_role_and_label(Role::Button, "Choose Icon");

    // The graphics it ships with: the Settings window's Rendering rows, written to the game's file.
    harness.get_by_role_and_label(Role::Tab, "Graphics").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::ComboBox, "Quality preset").click();
    settle(&mut harness);
    harness.get_by_label("Low").click();
    settle(&mut harness);
    let editor = harness.state_mut().studio.editor_mut().unwrap();
    let shipped = editor.project.read_text(GAME_GRAPHICS_FILE).unwrap();
    assert!(shipped.contains("preset = \"low\""), "{shipped}");
    assert!(
        !editor
            .project
            .assets()
            .iter()
            .any(|(path, _)| path == GAME_GRAPHICS_FILE),
        "never an asset"
    );

    // Online: no Graphics page, and a note that the platform is still to come.
    harness.get_by_role_and_label(Role::Tab, "Game").click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Online").click();
    settle(&mut harness);
    let editor = harness.state_mut().studio.editor_mut().unwrap();
    assert_eq!(editor.project.manifest().mode, ProjectMode::Platform);
    assert!(harness.query_by_role_and_label(Role::Tab, "Graphics").is_none());
    harness.get_by_label_contains("Online games are not available yet");

    // Shift+Alt+S again closes it.
    harness.key_press_modifiers(egui::Modifiers::SHIFT | egui::Modifiers::ALT, egui::Key::S);
    settle(&mut harness);
    assert!(
        harness
            .query_by_label_contains("Online games are not available yet")
            .is_none()
    );
}
