//! #802: Ctrl+G groups the selection into a Model, Ctrl+Shift+G into a Folder, Alt+G ungroups; each is one undo step.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use metallic_engine::metallic_core::Id;

use super::*;

/// The class name of `id`'s parent.
fn parent_class(harness: &Harness<'_, Test>, id: Id) -> String {
    let editor = harness.state().studio.editor().unwrap();
    let parent = editor.dm.parent(id).unwrap().unwrap();
    editor.dm.class_name(parent).unwrap_or_default().to_owned()
}

#[test]
fn ctrl_g_groups_ctrl_shift_g_folders_and_alt_g_ungroups() {
    let mut harness = editing(settings_file("grouping", None), "Grouping Game");
    harness.set_size(egui::vec2(1400.0, 1000.0));
    settle(&mut harness);
    let (baseplate, spawn) = {
        let editor = harness.state_mut().studio.editor_mut().unwrap();
        let dm = &editor.dm;
        let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        let find = |name: &str| dm.find_first_child(workspace, name, false).unwrap().unwrap();
        let (baseplate, spawn) = (find("Baseplate"), find("SpawnLocation"));
        editor.selection = crate::editor::Selection::of([baseplate, spawn]);
        (baseplate, spawn)
    };
    // The Explorer has the focus, so the keys are the editor's.
    harness.get_by_role_and_label(Role::TreeItem, "Workspace").click();
    settle(&mut harness);
    {
        let editor = harness.state_mut().studio.editor_mut().unwrap();
        editor.selection = crate::editor::Selection::of([baseplate, spawn]);
    }
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::G);
    settle(&mut harness);
    assert_eq!(parent_class(&harness, baseplate), "Model");
    assert_eq!(parent_class(&harness, spawn), "Model");
    let model = {
        let editor = harness.state().studio.editor().unwrap();
        let model = editor.dm.parent(baseplate).unwrap().unwrap();
        assert_eq!(editor.selection.ids(), [model], "the group is selected");
        assert_eq!(editor.dm.parent(spawn).unwrap(), Some(model), "one group");
        model
    };

    // One undo step takes it back.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    settle(&mut harness);
    assert_eq!(parent_class(&harness, baseplate), "Workspace");
    assert!(
        harness
            .state()
            .studio
            .editor()
            .unwrap()
            .dm
            .parent(model)
            .unwrap()
            .is_none()
    );

    // A Folder, then ungrouped: the parts back in the Workspace, selected, and the folder gone.
    {
        let editor = harness.state_mut().studio.editor_mut().unwrap();
        editor.selection = crate::editor::Selection::of([baseplate, spawn]);
    }
    harness.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::G);
    settle(&mut harness);
    assert_eq!(parent_class(&harness, baseplate), "Folder");
    let folder = harness
        .state()
        .studio
        .editor()
        .unwrap()
        .dm
        .parent(baseplate)
        .unwrap()
        .unwrap();
    harness.key_press_modifiers(egui::Modifiers::ALT, egui::Key::G);
    settle(&mut harness);
    assert_eq!(parent_class(&harness, baseplate), "Workspace");
    assert_eq!(parent_class(&harness, spawn), "Workspace");
    let editor = harness.state().studio.editor().unwrap();
    assert!(editor.dm.parent(folder).unwrap().is_none(), "the folder is gone");
    let mut selected = editor.selection.ids().to_vec();
    selected.sort();
    let mut parts = vec![baseplate, spawn];
    parts.sort();
    assert_eq!(selected, parts);
}
