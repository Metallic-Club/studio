//! The projects page's Team Create section (ADR 0011, #629): a hosted project shows as available from a real probe
//! and can be joined; a stopped one shows as not hosted and cannot be opened by a participant; its owner can always
//! open it and host it again; the list survives a restart.

use std::time::{Duration, Instant};

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::team_create::{host, join, joined, until};
use super::*;
use crate::projects::View;
use crate::team::store::{Role as Owned, TeamStore};

/// Double-clicks the node whose label contains `label`.
fn double_click_containing(harness: &mut Harness<'_, Test>, label: &str) {
    harness.get_by_label_contains(label).click();
    harness.step();
    let time = harness.ctx.input(|input| input.time);
    harness.input_mut().time = Some(time + 0.05);
    harness.get_by_label_contains(label).click();
    settle(harness);
}

/// Steps `harness` until `done`, for at most fifteen seconds.
fn wait_for(harness: &mut Harness<'_, Test>, what: &str, mut done: impl FnMut(&Harness<'_, Test>) -> bool) {
    let start = Instant::now();
    while !done(harness) {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "timed out waiting for {what}"
        );
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    settle(harness);
}

#[test]
fn team_create_projects_show_whether_they_are_hosted_and_open_by_role() {
    let mut a = editing(settings_file("team-page-host", None), "Moon Base");
    let mut b = editing(settings_file("team-page-join", None), "Page Scratch");
    // B's Team Create list is a real file, to read it again after a "restart".
    let store_path = settings_file("team-page-store", None).with_file_name(crate::team::store::FILE);
    b.state_mut()
        .studio
        .team_mut()
        .set_store(TeamStore::load(store_path.clone()));

    // B joins once by address: the project joins B's Team Create list.
    let port = host(&mut a);
    join(&mut b, port, "Bea");
    until(&mut a, &mut b, |_, b| joined(b));
    let entry = b.state().studio.team().store.projects[0].clone();
    assert_eq!(
        (entry.name.as_str(), entry.role, entry.port),
        ("Moon Base", Owned::Participant, port)
    );
    b.state_mut().studio.show_projects();
    until(&mut a, &mut b, |a, _| a.team().users().len() == 1);

    // On the page: green, from a probe of the host, and a double-click joins.
    b.state_mut().studio.projects_mut().show_view(View::TeamCreate);
    let mut steps = 0;
    while b.query_by_label_contains("Moon Base (Team Create, Available").is_none() {
        assert!(steps < 1500, "the card never said Available");
        steps += 1;
        a.step();
        b.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    double_click_containing(&mut b, "Moon Base (Team Create");
    until(&mut a, &mut b, |_, b| joined(b));
    b.state_mut().studio.show_projects();
    until(&mut a, &mut b, |a, _| a.team().users().len() == 1);

    // The host stops: checked again, it is not hosted, and a participant cannot open it.
    a.state_mut().studio.team_action(crate::team::menu::TeamAction::End);
    assert!(!a.state().studio.team().is_hosting());
    b.state_mut().studio.projects_mut().show_view(View::TeamCreate);
    settle(&mut b);
    b.get_by_role_and_label(Role::Button, "Check again").click();
    wait_for(&mut b, "Not hosted", |b| {
        b.query_by_label_contains("Moon Base (Team Create, Not hosted")
            .is_some()
    });
    double_click_containing(&mut b, "Moon Base (Team Create");
    for _ in 0..20 {
        b.step();
    }
    assert!(
        !b.state().studio.team().is_active(),
        "an unavailable project does not open"
    );
    assert_eq!(b.state().studio.screen(), Screen::Projects);

    // Its owner can always open it, and hosts it again.
    a.state_mut().studio.show_projects();
    a.state_mut().studio.projects_mut().show_view(View::TeamCreate);
    settle(&mut a);
    double_click_containing(&mut a, "Moon Base (Team Create");
    wait_open(&mut a);
    settle(&mut a);
    assert_eq!(a.state().studio.editor().map(EditorState::name), Some("Moon Base"));
    assert!(
        a.state().studio.team().is_hosting(),
        "{:?}",
        a.state().studio.team().note
    );
    a.state_mut().studio.team_action(crate::team::menu::TeamAction::End);

    // The list survives a restart.
    let again = TeamStore::load(store_path);
    assert!(
        again
            .projects
            .iter()
            .any(|p| p.name == "Moon Base" && p.role == Owned::Participant)
    );
}
