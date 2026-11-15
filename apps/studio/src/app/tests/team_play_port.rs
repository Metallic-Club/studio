//! Team Create Play's game port (#639): the one after the session's, so it can be forwarded for people outside the
//! host's network, and a clear error when something else holds it.

use egui::Role;
use egui_kittest::kittest::Queryable;

use super::team_create::{free_pairs, host, hosted_port, open_menu};
use super::*;
use crate::team::PlayMode;

fn game_port(studio: &Studio) -> Option<u16> {
    studio
        .editor()?
        .play
        .as_ref()?
        .net
        .as_ref()?
        .server
        .as_ref()
        .map(|server| server.port())
}

#[test]
fn team_create_play_serves_on_the_port_after_the_sessions() {
    let mut a = editing(settings_file("play-port-host", None), "Port Pair");
    let port = host(&mut a);
    assert_eq!(a.state().studio.team().play_port(), Some(port + 1));
    // The menu says which ports to forward.
    a.get_by_label_contains(&format!("TCP ports {port} and {}", port + 1));

    a.state_mut().studio.team_mut().play_mode = PlayMode::TeamCreate;
    a.get_by_role_and_label(Role::Button, "Play").click();
    settle(&mut a);
    assert!(a.state().studio.is_playing(), "{:?}", a.state().studio.notice());
    assert_eq!(game_port(&a.state().studio), Some(port + 1));
    a.state_mut().studio.stop();
    settle(&mut a);
    // Straight after Stop the port is free again: a second game starts on it.
    a.get_by_role_and_label(Role::Button, "Play").click();
    settle(&mut a);
    assert!(a.state().studio.is_playing(), "{:?}", a.state().studio.notice());
    assert_eq!(game_port(&a.state().studio), Some(port + 1));
    a.state_mut().studio.stop();
    settle(&mut a);

    // Something else holds the play port: the game does not start, and the notice says why.
    let _taken = std::net::TcpListener::bind(("0.0.0.0", port + 1)).unwrap();
    a.get_by_role_and_label(Role::Button, "Play").click();
    settle(&mut a);
    assert!(!a.state().studio.is_playing());
    let notice = a.state().studio.notice().unwrap_or_default().to_owned();
    assert!(
        notice.contains("the port after the session's") && notice.contains(&format!("port {}", port + 1)),
        "{notice}"
    );
    a.state_mut().studio.team_action(crate::team::menu::TeamAction::End);
}

/// #651: a session asked for on a port something else holds moves up to the next free pair, and the menu says so.
#[test]
fn hosting_on_a_taken_port_moves_to_the_next_free_pair() {
    let mut a = editing(settings_file("busy-port-host", None), "Busy Port");
    // Two pairs, so the one the session moves to is no other test's.
    let asked = free_pairs(2);
    let _held = std::net::TcpListener::bind(("0.0.0.0", asked)).unwrap();
    open_menu(&mut a);
    a.state_mut().studio.team_mut().form.port = asked.to_string();
    a.get_by_role_and_label(Role::Button, "Start hosting").click();
    settle(&mut a);
    let port = hosted_port(&a);
    assert_eq!(port, asked + 2);
    a.get_by_label_contains(&format!(
        "Port {asked} or the one after it was in use, so the session is on {port}."
    ));
    assert_eq!(a.state().studio.team().form.port, port.to_string());
}
