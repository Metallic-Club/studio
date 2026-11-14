//! Team Create Play's game port (#639): the one after the session's, so it can be forwarded for people outside the
//! host's network, and a clear error when something else holds it.

use egui::Role;
use egui_kittest::kittest::Queryable;

use super::team_create::host;
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
