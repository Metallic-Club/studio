//! Team Create (ADR 0011) between two Studios in one process, over real TCP on localhost: hosting from the top
//! bar's menu, joining by address, the user lists, leaving and ending.

use std::time::{Duration, Instant};

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;
use crate::team::Session;

/// A port nothing listens on now.
pub(super) fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A port to host on whose next port (Team Create Play's, #639) is free too. Both below every OS's ephemeral range
/// (Windows from 49152, Linux from 32768): there, the joiner's own outgoing socket can be handed the port after the
/// host's, as Windows hands them out in order.
pub(super) fn free_pair() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static NEXT: AtomicU16 = AtomicU16::new(0);
    let bindable = |port: u16| std::net::TcpListener::bind(("0.0.0.0", port)).is_ok();
    // Spread over the range by process, so test runs side by side rarely try the same ports.
    let start = 20_000 + (std::process::id() % 400) as u16 * 20;
    loop {
        let port = start + NEXT.fetch_add(2, Ordering::Relaxed) % 4_000;
        if bindable(port) && bindable(port + 1) {
            return port;
        }
    }
}

/// Steps both Studios until `done` holds, for at most twenty seconds.
pub(super) fn until(
    a: &mut Harness<'_, Test>,
    b: &mut Harness<'_, Test>,
    mut done: impl FnMut(&Studio, &Studio) -> bool,
) {
    let start = Instant::now();
    loop {
        a.step();
        b.step();
        if done(&a.state().studio, &b.state().studio) {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out: a {:?} {:?} {:?}, b {:?} {:?} {:?} {:?}",
            a.state().studio.team().status_line(),
            names(&a.state().studio),
            a.state().studio.team().note,
            b.state().studio.team().status_line(),
            b.state().studio.team().note,
            b.state().studio.screen(),
            b.state().studio.projects().error(),
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    settle(a);
    settle(b);
}

/// Opens the top bar's Team Create menu.
pub(super) fn open_menu(harness: &mut Harness<'_, Test>) {
    // Already open: its header says Offline.
    if harness.query_by_label("Offline").is_some() {
        return;
    }
    harness.get_by_role_and_label(Role::Button, "Team Create").click();
    settle(harness);
}

/// Hosts from `a`'s menu on a free port; returns it.
pub(super) fn host(a: &mut Harness<'_, Test>) -> u16 {
    open_menu(a);
    let port = free_pair();
    a.state_mut().studio.team_mut().form.port = port.to_string();
    a.get_by_role_and_label(Role::Button, "Start hosting").click();
    settle(a);
    assert!(
        a.state().studio.team().is_hosting(),
        "{:?}",
        a.state().studio.team().note
    );
    port
}

/// Joins `port` on localhost from `b`'s menu as `name`.
pub(super) fn join(b: &mut Harness<'_, Test>, port: u16, name: &str) {
    // A new project counts as unsaved (#228), and joining would close it: saved first.
    if let Some(editor) = b.state_mut().studio.editor_mut()
        && editor.is_modified()
    {
        editor.save().unwrap();
    }
    open_menu(b);
    b.get_by_role_and_label(Role::Tab, "Join").click();
    settle(b);
    let team = b.state_mut().studio.team_mut();
    team.form.address = format!("127.0.0.1:{port}");
    team.form.name = name.to_owned();
    b.get_by_role_and_label(Role::Button, "Join").click();
    settle(b);
}

pub(super) fn joined(studio: &Studio) -> bool {
    matches!(studio.team().session(), Session::Joined(_)) && studio.editor().is_some()
}

pub(super) fn names(studio: &Studio) -> Vec<String> {
    studio
        .team()
        .users()
        .into_iter()
        .map(|(user, _)| user.identity.name)
        .collect()
}

#[test]
fn a_second_studio_joins_by_address_gets_the_project_and_both_see_the_users() {
    let mut a = editing(settings_file("team-host", None), "Castle Keep");
    let mut b = editing(settings_file("team-join", None), "Scratch");
    a.state_mut().studio.settings.settings_mut().profile.name = "Ada".into();
    // The host edits before anyone joins, without saving: the participant gets the scene as it is now.
    {
        let editor = a.state_mut().studio.editor_mut().unwrap();
        let workspace = editor
            .dm
            .service(editor.dm.registry().class_id("Workspace").unwrap())
            .unwrap();
        let part = editor
            .dm
            .create(editor.dm.registry().class_id("Part").unwrap())
            .unwrap();
        editor.dm.set_name(part, "Unsaved Tower".into()).unwrap();
        editor.dm.set_parent(part, Some(workspace)).unwrap();
    }
    let port = host(&mut a);
    // The menu shows the session: hosting, the address to join at, the host.
    a.get_by_label_contains(&format!("Hosting on port {port}"));
    a.get_by_role_and_label(Role::Button, "End session");

    join(&mut b, port, "Bob");
    until(&mut a, &mut b, |a, b| {
        joined(b) && a.team().users().iter().filter(|(_, ready)| *ready).count() == 2
    });
    let editor = b.state().studio.editor().unwrap();
    assert_eq!(editor.name(), "Castle Keep", "the host's project is open");
    assert!(b.state().studio.team().is_copy(editor.path()));
    let workspace = editor
        .dm
        .service(editor.dm.registry().class_id("Workspace").unwrap())
        .unwrap();
    let kids: Vec<String> = editor
        .dm
        .children(workspace)
        .unwrap()
        .map(|id| editor.dm.name(id).unwrap().to_owned())
        .collect();
    assert!(
        kids.iter().any(|k| k == "Unsaved Tower"),
        "the unsaved edit came along: {kids:?}"
    );
    assert_eq!(names(&a.state().studio), ["Ada", "Bob"]);
    assert_eq!(names(&b.state().studio), ["Ada", "Bob"]);
    // Both menus list both people.
    open_menu(&mut b);
    b.get_by_label("Bob (you)");
    b.get_by_label("Ada");
    b.get_by_role_and_label(Role::Button, "Disconnect");
    // A participant's Save is the host's.
    b.state_mut().studio.save();
    assert_eq!(
        b.state().studio.notice().map(|n| n.contains("reach the host")),
        Some(true)
    );

    // Bob leaves: Ada sees one person again, and Bob is back on the projects page.
    b.get_by_role_and_label(Role::Button, "Disconnect").click();
    settle(&mut b);
    until(&mut a, &mut b, |a, _| a.team().users().len() == 1);
    assert_eq!(b.state().studio.screen(), Screen::Projects);
    assert!(!b.state().studio.team().is_active());

    // He rejoins, and Ada ends the session: he is told, and his copy closes.
    b.state_mut()
        .studio
        .open(make_project_once(&std::env::temp_dir(), "Team Scratch"));
    wait_open(&mut b);
    join(&mut b, port, "Bob");
    until(&mut a, &mut b, |_, b| joined(b));
    open_menu(&mut a);
    a.get_by_role_and_label(Role::Button, "End session").click();
    settle(&mut a);
    assert!(!a.state().studio.team().is_hosting());
    until(&mut a, &mut b, |_, b| !b.team().is_active());
    assert_eq!(b.state().studio.screen(), Screen::Projects);
    assert!(b.state().studio.notices.iter().any(|n| n.body.contains("ended")));
}

#[test]
fn a_join_to_an_address_where_nothing_hosts_says_why_in_the_menu() {
    let mut b = editing(settings_file("team-nothing", None), "Lonely");
    join(&mut b, free_port(), "Bob");
    let start = Instant::now();
    while b.state().studio.team().is_active() {
        assert!(start.elapsed() < Duration::from_secs(20), "the join never failed");
        b.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    settle(&mut b);
    assert_eq!(
        b.state().studio.editor().map(EditorState::name),
        Some("Lonely"),
        "the project stays open"
    );
    open_menu(&mut b);
    b.get_by_label_contains("127.0.0.1");
}
