//! Collaborative script editing between two Studios (ADR 0011 §6, #631), over real TCP on localhost: both type into
//! one script at once and end with the same text, nothing lost; each sees the other's cursor; undo takes back only
//! one's own typing; and a participant who rejoins while the script is edited gets it as it is.

use std::time::Duration;

use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use metallic_engine::metallic_core::{Id, Value};

use super::team_create::{host, join, joined, until};
use super::*;

const DOOR: &str = "Scripts/Door.luau";

fn editor<'a>(harness: &'a mut Harness<'_, Test>) -> &'a mut EditorState {
    harness.state_mut().studio.editor_mut().unwrap()
}

/// A Script in the Workspace whose code is `DOOR`, holding `code`.
fn add_script(harness: &mut Harness<'_, Test>, code: &str) -> Id {
    let editor = editor(harness);
    let ws = editor
        .dm
        .service(editor.dm.registry().class_id("Workspace").unwrap())
        .unwrap();
    let script = editor
        .dm
        .create(editor.dm.registry().class_id("Script").unwrap())
        .unwrap();
    editor.dm.set_name(script, "Door".into()).unwrap();
    editor
        .dm
        .set_by_name(script, "Source", Value::String(DOOR.into()))
        .unwrap();
    editor.dm.set_parent(script, Some(ws)).unwrap();
    editor.project.write(DOOR, code.as_bytes().to_vec()).unwrap();
    editor.save().unwrap();
    script
}

fn script_in(harness: &mut Harness<'_, Test>) -> Id {
    let editor = editor(harness);
    crate::code::script_files(&editor.dm)
        .into_iter()
        .find(|(_, member)| member == DOOR)
        .map(|(script, _)| script)
        .expect("the script")
}

fn text(harness: &mut Harness<'_, Test>) -> String {
    let editor = editor(harness);
    editor.scripts.text(&mut editor.project, DOOR).clone()
}

/// Opens the script's tab and gives its field the keyboard, the cursor at `at` (characters).
fn focus_at(harness: &mut Harness<'_, Test>, script: Id, at: usize) {
    editor(harness).open_script(script);
    settle(harness);
    let id = crate::panels::script::field_id(script);
    harness.ctx.memory_mut(|memory| memory.request_focus(id));
    crate::code::view::place_selection(&harness.ctx, id, at, at);
    settle(harness);
}

/// Types `typed` into the focused field, as the keyboard would.
fn type_text(harness: &mut Harness<'_, Test>, typed: &str) {
    harness.input_mut().events.push(Event::Text(typed.to_owned()));
    harness.step();
}

#[test]
fn two_users_type_into_one_script_at_once_see_each_other_and_undo_only_their_own() {
    let mut a = editing(settings_file("team-scripts-host", None), "Script Island");
    let mut b = editing(settings_file("team-scripts-join", None), "Script Scratch");
    a.state_mut().studio.settings.settings_mut().profile.name = "Ada".into();
    let code = "local door = script.Parent\nprint(door)\n";
    let script_a = add_script(&mut a, code);
    let port = host(&mut a);
    join(&mut b, port, "Bob");
    until(&mut a, &mut b, |_, b| joined(b));
    let script_b = script_in(&mut b);
    // Both have it open: it is a shared document on both sides.
    focus_at(&mut a, script_a, 0);
    focus_at(&mut b, script_b, code.len());
    until(&mut a, &mut b, |a, b| {
        [a, b]
            .iter()
            .all(|s| s.editor().is_some_and(|e| e.scripts.collab.members.contains(DOOR)))
    });
    focus_at(&mut a, script_a, 0);
    focus_at(&mut b, script_b, code.len());

    // They type at once: Ada at the top, Bob at the end, each through the field as the keyboard does.
    type_text(&mut a, "-- Ada was here\n");
    type_text(&mut b, "door.Name = \"Front\"\n");
    until(&mut a, &mut b, |a, b| {
        let mut texts = [a, b].map(|s| {
            s.editor()
                .map(|e| e.scripts.loaded().find(|(m, _)| *m == DOOR).map(|(_, t)| t.to_owned()))
        });
        let (x, y) = (texts[0].take().flatten(), texts[1].take().flatten());
        x.is_some()
            && x == y
            && x.as_deref()
                .is_some_and(|t| t.contains("Ada was here") && t.contains("Front"))
    });
    let both = text(&mut a);
    assert!(both.starts_with("-- Ada was here\nlocal door"), "{both}");
    assert!(both.ends_with("door.Name = \"Front\"\n"), "{both}");

    // Each sees the other's cursor, with their name, where it is.
    until(&mut a, &mut b, |a, b| {
        let seen = |s: &Studio, name: &str| {
            s.editor().is_some_and(|e| {
                e.scripts
                    .collab
                    .remote
                    .get(DOOR)
                    .is_some_and(|carets| carets.iter().any(|c| c.name == name))
            })
        };
        seen(a, "Bob") && seen(b, "Ada")
    });
    let bob = editor(&mut a).scripts.collab.remote[DOOR][0].clone();
    assert_eq!(bob.head, both.chars().count(), "Bob's cursor is after what he typed");
    // Drawn without trouble.
    settle(&mut a);
    settle(&mut b);

    // Ada undoes: her line goes, Bob's stays, everywhere.
    a.input_mut().events.push(Event::Key {
        key: Key::Z,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    });
    a.step();
    until(&mut a, &mut b, |a, b| {
        let t = |s: &Studio| {
            s.editor()
                .and_then(|e| e.scripts.loaded().find(|(m, _)| *m == DOOR).map(|(_, t)| t.to_owned()))
                .unwrap_or_default()
        };
        let (x, y) = (t(a), t(b));
        x == y && !x.contains("Ada was here") && x.contains("Front")
    });

    // Bob leaves and rejoins while Ada keeps typing: he gets the script as it is.
    b.state_mut().studio.team_action(crate::team::menu::TeamAction::Leave);
    until(&mut a, &mut b, |a, _| a.team().users().len() == 1);
    focus_at(&mut a, script_a, 0);
    type_text(&mut a, "-- again\n");
    for _ in 0..5 {
        a.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    b.state_mut()
        .studio
        .open(make_project_once(&std::env::temp_dir(), "Script Rejoin"));
    wait_open(&mut b);
    join(&mut b, port, "Bob");
    until(&mut a, &mut b, |_, b| joined(b));
    let script_b = script_in(&mut b);
    editor(&mut b).open_script(script_b);
    until(&mut a, &mut b, |a, b| {
        let t = |s: &Studio| {
            s.editor()
                .and_then(|e| e.scripts.loaded().find(|(m, _)| *m == DOOR).map(|(_, t)| t.to_owned()))
                .unwrap_or_default()
        };
        let (x, y) = (t(a), t(b));
        x == y && x.starts_with("-- again\n") && x.contains("Front")
    });
    a.state_mut().studio.team_action(crate::team::menu::TeamAction::End);
}
