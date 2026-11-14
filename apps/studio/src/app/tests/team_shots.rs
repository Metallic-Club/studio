//! Screenshots of Team Create (ADR 0011), drawn offscreen by two real Studios on one GPU over localhost TCP: the
//! session menu, another user's camera, outline and Explorer dot, their cursor in a shared script, and Play Solo's
//! Server and Game tabs. `cargo test -p metallic_studio -- --ignored screenshots_of_team_create`.

use std::time::{Duration, Instant};

use metallic_engine::metallic_core::glam::Vec3;
use metallic_engine::metallic_core::{CFrame, Value};

use super::offscreen::Offscreen;
use super::*;
use crate::team::Session;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

fn open(studio: &mut Studio, offscreen: &mut Offscreen, path: PathBuf) {
    studio.open(path);
    for _ in 0..10_000 {
        offscreen.frame(studio);
        if !studio.is_opening() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(studio.editor().is_some(), "{:?}", studio.projects().error());
}

/// Frames both Studios until `done`, for at most twenty seconds.
fn until(a: &mut (Studio, Offscreen), b: &mut (Studio, Offscreen), mut done: impl FnMut(&Studio, &Studio) -> bool) {
    let start = Instant::now();
    while !done(&a.0, &b.0) {
        a.1.frame(&mut a.0);
        b.1.frame(&mut b.0);
        assert!(start.elapsed() < Duration::from_secs(20), "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
fn screenshots_of_team_create() {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
    let path_a = config("shot-team-a", Some("[general]\nskip_sign_in = true\n"), None);
    let path_b = config("shot-team-b", Some("[general]\nskip_sign_in = true\n"), None);
    let project = make_project_once(path_a.parent().unwrap(), "Harbour");
    let scratch = make_project_once(path_b.parent().unwrap(), "Shot Scratch");
    let mut a = (
        studio_after_startup(steps_reading(path_a)),
        Offscreen::new(SIZE, 1.0).expect("a GPU"),
    );
    let mut b = (
        studio_after_startup(steps_reading(path_b)),
        Offscreen::new(SIZE, 1.0).expect("a GPU"),
    );
    for _ in 0..3 {
        a.1.frame(&mut a.0);
        b.1.frame(&mut b.0);
    }
    open(&mut a.0, &mut a.1, project);
    open(&mut b.0, &mut b.1, scratch);
    b.0.editor_mut().unwrap().save().unwrap();
    a.0.settings.settings_mut().profile.name = "Ada".into();
    b.0.settings.settings_mut().profile.name = "Bob".into();
    b.0.settings.settings_mut().profile.color = crate::profile::COLORS[3];
    // The menu before hosting (#643): Host and Join.
    egui::Popup::open_id(&a.1.ctx, crate::team::menu::popup_id());
    for _ in 0..30 {
        a.1.frame(&mut a.0);
    }
    write_png(&out.join("643-team-create-host.png"), a.1.read());
    egui::Popup::close_id(&a.1.ctx, crate::team::menu::popup_id());
    // A crate and a script on the host.
    let crate_a = {
        let editor = a.0.editor_mut().unwrap();
        let ws = editor
            .dm
            .service(editor.dm.registry().class_id("Workspace").unwrap())
            .unwrap();
        let part = editor
            .dm
            .create(editor.dm.registry().class_id("Part").unwrap())
            .unwrap();
        editor.dm.set_name(part, "Crate".into()).unwrap();
        editor
            .dm
            .set_by_name(part, "CFrame", Value::CFrame(CFrame::new(Vec3::new(0.0, 2.0, 0.0))))
            .unwrap();
        editor.dm.set_parent(part, Some(ws)).unwrap();
        let script = editor
            .dm
            .create(editor.dm.registry().class_id("Script").unwrap())
            .unwrap();
        editor.dm.set_name(script, "Door".into()).unwrap();
        editor
            .dm
            .set_by_name(script, "Source", Value::String("Scripts/Door.luau".into()))
            .unwrap();
        editor.dm.set_parent(script, Some(ws)).unwrap();
        editor
            .project
            .write(
                "Scripts/Door.luau",
                b"local door = script.Parent\n\nprint(door.Name)\n".to_vec(),
            )
            .unwrap();
        editor.save().unwrap();
        part
    };
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    a.0.team_mut().form.port = port.to_string();
    a.0.team_action(crate::team::menu::TeamAction::Host);
    b.0.team_mut().form.address = format!("127.0.0.1:{port}");
    b.0.team_action(crate::team::menu::TeamAction::Join);
    until(&mut a, &mut b, |_, b| {
        matches!(b.team().session(), Session::Joined(_)) && b.editor().is_some()
    });

    // Bob looks at the crate from the side and selects it, as Ada does.
    let crate_b = {
        let editor = b.0.editor_mut().unwrap();
        editor.viewport.camera =
            crate::viewport::camera::EditorCamera::looking_at(Vec3::new(9.0, 5.0, -3.0), Vec3::new(0.0, 2.0, 0.0));
        let ws = editor
            .dm
            .service(editor.dm.registry().class_id("Workspace").unwrap())
            .unwrap();
        let found = editor
            .dm
            .children(ws)
            .unwrap()
            .find(|&id| editor.dm.name(id).unwrap() == "Crate")
            .unwrap();
        editor.selection.set_only(Some(found));
        found
    };
    a.0.editor_mut().unwrap().selection.set_only(Some(crate_a));
    a.0.editor_mut().unwrap().viewport.camera =
        crate::viewport::camera::EditorCamera::looking_at(Vec3::new(4.0, 7.0, 18.0), Vec3::new(3.0, 2.0, 0.0));
    until(&mut a, &mut b, |a, _| {
        a.editor()
            .is_some_and(|e| e.presence.users.first().is_some_and(|u| u.selection == [crate_a]))
    });
    let _ = crate_b;
    for _ in 0..20 {
        a.1.frame(&mut a.0);
        b.1.frame(&mut b.0);
    }
    egui::Popup::open_id(&a.1.ctx, crate::team::menu::popup_id());
    for _ in 0..30 {
        a.1.frame(&mut a.0);
    }
    write_png(&out.join("632-team-create-session.png"), a.1.read());
    egui::Popup::close_id(&a.1.ctx, crate::team::menu::popup_id());
    // How Play runs, while hosting (#643).
    egui::Popup::open_id(&a.1.ctx, crate::team::play_menu::popup_id());
    for _ in 0..30 {
        a.1.frame(&mut a.0);
    }
    write_png(&out.join("643-play-modes.png"), a.1.read());
    egui::Popup::close_id(&a.1.ctx, crate::team::play_menu::popup_id());

    // Both open the script; Bob types, and Ada sees his cursor.
    let script = |s: &Studio| {
        let editor = s.editor().unwrap();
        crate::code::script_files(&editor.dm).into_iter().next().unwrap().0
    };
    let (script_a, script_b) = (script(&a.0), script(&b.0));
    a.0.editor_mut().unwrap().open_script(script_a);
    b.0.editor_mut().unwrap().open_script(script_b);
    until(&mut a, &mut b, |a, b| {
        [a, b].iter().all(|s| {
            s.editor()
                .is_some_and(|e| e.scripts.collab.members.contains("Scripts/Door.luau"))
        })
    });
    let field = crate::panels::script::field_id(script_b);
    b.1.ctx.memory_mut(|memory| memory.request_focus(field));
    crate::code::view::place_selection(&b.1.ctx, field, 27, 27);
    b.1.frame(&mut b.0);
    b.1.frame_with(&mut b.0, vec![egui::Event::Text("door.Locked = true\n".into())]);
    until(&mut a, &mut b, |a, _| {
        a.editor().is_some_and(|e| {
            e.scripts
                .collab
                .remote
                .get("Scripts/Door.luau")
                .is_some_and(|c| !c.is_empty())
                && e.scripts.loaded().any(|(_, t)| t.contains("Locked"))
        })
    });
    for _ in 0..10 {
        a.1.frame(&mut a.0);
        b.1.frame(&mut b.0);
    }
    write_png(&out.join("632-shared-script.png"), a.1.read());

    // Play Solo on Bob's side: the Server and Game tabs.
    b.0.team_action(crate::team::menu::TeamAction::Leave);
    a.0.team_action(crate::team::menu::TeamAction::End);
    for _ in 0..5 {
        a.1.frame(&mut a.0);
    }
    a.0.play();
    for _ in 0..40 {
        a.1.frame(&mut a.0);
    }
    assert!(a.0.is_playing());
    write_png(&out.join("632-play-solo-server-and-game.png"), a.1.read());
    // The Server tab in front: the running game's world from the editor camera.
    a.0.dock.open(panels::ids::VIEWPORT);
    a.0.dock.set_focused(Some(panels::ids::VIEWPORT));
    for _ in 0..20 {
        a.1.frame(&mut a.0);
    }
    write_png(&out.join("632-play-solo-server-view.png"), a.1.read());
    a.0.stop();
}
