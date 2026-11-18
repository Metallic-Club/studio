//! #833: a PointLight's and a SpotLight's properties in the owner's groups and order, and the Tags section every
//! instance has, which adds and removes tags as undo steps.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use metallic_engine::metallic_core::Id;

use super::*;

/// The top rows of `id`'s Properties, group by group.
fn layout(harness: &Harness<'_, Test>, id: Id) -> Vec<(&'static str, Vec<String>)> {
    let editor = harness.state().studio.editor().unwrap();
    crate::panels::properties::groups(&editor.dm, &[id])
        .iter()
        .map(|group| {
            let rows = group.rows.iter().filter(|row| row.depth == 0);
            (group.name, rows.map(|row| row.label.clone()).collect())
        })
        .collect()
}

/// A new light of `class` in Workspace, selected.
fn light(harness: &mut Harness<'_, Test>, class: &str) -> Id {
    let editor = harness.state_mut().studio.editor_mut().unwrap();
    let dm = &mut editor.dm;
    let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
    let id = dm.create(dm.registry().class_id(class).unwrap()).unwrap();
    dm.set_parent(id, Some(workspace)).unwrap();
    editor.selection = crate::editor::Selection::of([id]);
    id
}

#[test]
fn lights_show_their_properties_in_the_owner_s_order() {
    let mut harness = editing(settings_file("light-properties", None), "Light Game");
    harness.set_size(egui::vec2(1280.0, 1500.0));
    settle(&mut harness);
    let point = light(&mut harness, "PointLight");
    let spot = light(&mut harness, "SpotLight");
    let rows = |names: &[&str]| names.iter().map(|name| name.to_string()).collect::<Vec<_>>();
    assert_eq!(
        layout(&harness, point),
        [
            ("General", rows(&["Name", "Parent", "Archivable", "Locked", "Enabled"])),
            ("Transform", rows(&["Position"])),
            ("Lighting", rows(&["Color", "Brightness", "Range", "Shadows"])),
        ]
    );
    assert_eq!(
        layout(&harness, spot),
        [
            ("General", rows(&["Name", "Parent", "Archivable", "Locked", "Enabled"])),
            ("Transform", rows(&["Position", "Rotation"])),
            ("Lighting", rows(&["Color", "Brightness", "Range", "Angle", "Shadows"])),
        ]
    );
    // Then Attributes and Tags, in the panel.
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Add attribute");
    harness.get_by_role_and_label(Role::Button, "Add tag");
}

#[test]
fn tags_are_added_and_removed_in_properties() {
    let mut harness = editing(settings_file("properties-tags", None), "Tag Game");
    harness.set_size(egui::vec2(1280.0, 1500.0));
    settle(&mut harness);
    let point = light(&mut harness, "PointLight");
    settle(&mut harness);
    let tags = |harness: &Harness<'_, Test>| -> Vec<String> {
        let editor = harness.state().studio.editor().unwrap();
        editor.dm.tags(point).unwrap().iter().map(|t| t.to_string()).collect()
    };
    harness.get_by_role_and_label(Role::Button, "Add tag").click();
    settle(&mut harness);
    let field = harness.get_by_role_and_label(Role::TextInput, "New tag").rect();
    left_click(&mut harness, field);
    harness.event(egui::Event::Text("Lamp".to_owned()));
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Add the tag").click();
    settle(&mut harness);
    assert_eq!(tags(&harness), ["Lamp"]);
    harness.get_by_role_and_label(Role::Button, "Remove tag Lamp").click();
    settle(&mut harness);
    assert!(tags(&harness).is_empty());
    // Each an undo step.
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    settle(&mut harness);
    assert_eq!(tags(&harness), ["Lamp"]);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    settle(&mut harness);
    assert!(tags(&harness).is_empty());
}
