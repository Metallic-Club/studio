//! Scene sync and presence between two Studios (ADR 0011, #630), over real TCP on localhost: a move on the host and
//! a create on the participant reach the other, each sees the other's camera and selection (on one part at once
//! too), the host's save keeps the session's edits, and a participant who rejoins gets the scene as it is.

use std::time::Duration;

use egui_kittest::Harness;
use metallic_engine::metallic_core::glam::Vec3;
use metallic_engine::metallic_core::{CFrame, DataModel, Id, Value};

use super::team_create::{host, join, joined, until};
use super::*;

fn workspace(dm: &DataModel) -> Id {
    dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap()
}

fn named(dm: &DataModel, name: &str) -> Option<Id> {
    dm.children(workspace(dm))
        .unwrap()
        .find(|&id| dm.name(id).unwrap() == name)
}

fn cframe(dm: &DataModel, id: Id) -> Value {
    dm.get_by_name(id, "CFrame").unwrap()
}

fn editor<'a>(harness: &'a mut Harness<'_, Test>) -> &'a mut EditorState {
    harness.state_mut().studio.editor_mut().unwrap()
}

/// A part called `name` in the editor's Workspace, made as the Insert menu makes one: through the scene's edits.
fn add_part(harness: &mut Harness<'_, Test>, name: &str) -> Id {
    let editor = editor(harness);
    let ws = workspace(&editor.dm);
    let part = editor
        .dm
        .create(editor.dm.registry().class_id("Part").unwrap())
        .unwrap();
    editor.dm.set_name(part, name.into()).unwrap();
    editor.scene().move_to(part, Some(ws), None).unwrap();
    editor.scene().commit(std::time::Instant::now(), false);
    part
}

#[test]
fn moves_creates_cameras_and_selections_reach_the_other_studio_and_survive_a_save_and_a_rejoin() {
    let settings = settings_file("team-scene-host", None);
    let mut a = editing(settings.clone(), "Harbour");
    let mut b = editing(settings_file("team-scene-join", None), "Scene Scratch");
    a.state_mut().studio.settings.settings_mut().profile.name = "Ada".into();
    let crate_a = add_part(&mut a, "Crate");
    let port = host(&mut a);
    join(&mut b, port, "Bob");
    until(&mut a, &mut b, |_, b| joined(b));
    let crate_b = named(&editor(&mut b).dm, "Crate").expect("the crate came with the project");

    // A moves the crate: B sees it there, bit for bit.
    let moved = Value::CFrame(CFrame::new(Vec3::new(12.0, 3.5, -7.25)));
    editor(&mut a)
        .scene()
        .set_by_name(crate_a, "CFrame", moved.clone())
        .unwrap();
    until(&mut a, &mut b, |_, b| {
        b.editor().is_some_and(|e| cframe(&e.dm, crate_b) == moved)
    });

    // B makes a part: A gets it.
    add_part(&mut b, "Lighthouse");
    until(&mut a, &mut b, |a, _| {
        a.editor().is_some_and(|e| named(&e.dm, "Lighthouse").is_some())
    });
    let lighthouse_a = named(&editor(&mut a).dm, "Lighthouse").unwrap();
    assert!(
        editor(&mut a).is_modified(),
        "the participant's edit is the host's to save"
    );

    // Both select the crate; each sees the other's camera and selection, and the Explorer's dot for it.
    editor(&mut a).selection.set_only(Some(crate_a));
    editor(&mut b).selection.set_only(Some(crate_b));
    until(&mut a, &mut b, |a, b| {
        let seen = |studio: &Studio, crate_: Id| {
            studio.editor().is_some_and(|e| {
                e.presence.users.len() == 1
                    && e.presence.users[0].camera.is_some()
                    && e.presence.users[0].selection == [crate_]
            })
        };
        seen(a, crate_a) && seen(b, crate_b)
    });
    assert_eq!(editor(&mut a).presence.users[0].name, "Bob");
    assert_eq!(editor(&mut b).presence.users[0].name, "Ada");
    assert_eq!(editor(&mut b).presence.dots(crate_b, false).len(), 1);
    // Drawn: the outline and the camera (no panic, and the Explorer has the row).
    settle(&mut a);
    settle(&mut b);

    // B leaves and rejoins: the scene is as the host has it now.
    a.state_mut().studio.team_action(crate::team::menu::TeamAction::Leave);
    b.state_mut().studio.team_action(crate::team::menu::TeamAction::Leave);
    until(&mut a, &mut b, |a, _| a.team().users().len() == 1);
    assert!(
        a.state().studio.editor().unwrap().presence.is_empty(),
        "a user who left is gone from the viewport"
    );
    let moved_again = Value::CFrame(CFrame::new(Vec3::new(0.0, 40.0, 0.0)));
    editor(&mut a)
        .scene()
        .set_by_name(lighthouse_a, "CFrame", moved_again.clone())
        .unwrap();
    for _ in 0..5 {
        a.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    b.state_mut()
        .studio
        .open(make_project_once(&std::env::temp_dir(), "Scene Rejoin"));
    wait_open(&mut b);
    join(&mut b, port, "Bob");
    until(&mut a, &mut b, |_, b| joined(b));
    let lighthouse_b = named(&editor(&mut b).dm, "Lighthouse").expect("the lighthouse after the rejoin");
    assert_eq!(cframe(&editor(&mut b).dm, lighthouse_b), moved_again);

    // The host ends the session: the edits are saved in its file.
    a.state_mut().studio.team_action(crate::team::menu::TeamAction::End);
    let path = editor(&mut a).path().to_path_buf();
    assert!(!editor(&mut a).is_modified(), "saved at the end of the session");
    let mut project = metallic_engine::metallic_project::Project::open(&path).unwrap();
    let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
    project.load_start_scene(&mut dm).unwrap();
    let saved = named(&dm, "Lighthouse").expect("the participant's part is in the saved project");
    assert_eq!(cframe(&dm, saved), moved_again);
    assert_eq!(cframe(&dm, named(&dm, "Crate").unwrap()), moved);
}
