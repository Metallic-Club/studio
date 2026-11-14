//! Playtesting's three modes (ADR 0011 §8, #632): Play Solo's Server and Game tabs; Team Create Play, the host's
//! server with every participant a player in it; and Multiple Clients, a local server and client processes that join
//! it by themselves, all ended by Stop.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use metallic_engine::metallic_core::glam::Vec3;
use metallic_engine::metallic_core::{CFrame, DataModel, Value};
use metallic_engine::metallic_net::play::Connecting;
use metallic_engine::metallic_net::transport::no_waker;

use super::team_create::{host, join, joined, until};
use super::*;
use crate::editor::multi::{ClientProcess, Launcher, PlayKind};
use crate::team::PlayMode;

fn named(dm: &DataModel, name: &str) -> Option<metallic_engine::metallic_core::Id> {
    let ws = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
    dm.children(ws).unwrap().find(|&id| dm.name(id).unwrap() == name)
}

fn press_play(harness: &mut Harness<'_, Test>) {
    harness.get_by_role_and_label(Role::Button, "Play").click();
    settle(harness);
}

#[test]
fn play_solo_shows_a_server_and_a_game_tab_and_the_normal_tabs_come_back() {
    let mut harness = editing(settings_file("play-solo-tabs", None), "Solo Tabs");
    harness.set_size(egui::vec2(1280.0, 800.0));
    assert_eq!(
        harness.state().studio.team().play_mode,
        PlayMode::Solo,
        "Play Solo is the default"
    );
    press_play(&mut harness);
    assert!(harness.state().studio.is_playing());
    harness.get_by_role_and_label(Role::Tab, "Server");
    harness.get_by_role_and_label(Role::Tab, "Game");
    assert!(harness.query_by_role_and_label(Role::Tab, "Viewport").is_none());
    harness.state_mut().studio.stop();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::Tab, "Viewport");
    assert!(harness.query_by_role_and_label(Role::Tab, "Game").is_none());
    assert!(harness.query_by_role_and_label(Role::Tab, "Server").is_none());
    // The arrow beside Play offers the three modes.
    harness.get_by_role_and_label(Role::Button, "Play options").click();
    settle(&mut harness);
    harness.get_by_label("Play Solo");
    harness.get_by_label("Team Create Play");
    harness.get_by_label("Multiple Clients");
}

#[test]
fn team_create_play_puts_the_host_and_a_participant_in_one_game_and_stop_ends_it_for_both() {
    let mut a = editing(settings_file("play-team-host", None), "Arena");
    let mut b = editing(settings_file("play-team-join", None), "Arena Scratch");
    a.state_mut().studio.settings.settings_mut().profile.name = "Ada".into();
    let port = host(&mut a);
    join(&mut b, port, "Bob");
    until(&mut a, &mut b, |_, b| joined(b));

    a.state_mut().studio.team_mut().play_mode = PlayMode::TeamCreate;
    press_play(&mut a);
    assert!(a.state().studio.is_playing(), "{:?}", a.state().studio.notice());
    // Bob's Studio joins the host's game by itself.
    until(&mut a, &mut b, |a, b| {
        let players = a.editor().and_then(|e| {
            e.play
                .as_ref()?
                .net
                .as_ref()?
                .server
                .as_ref()
                .map(|s| s.players().len())
        });
        players == Some(2) && b.is_playing()
    });
    let kind = |s: &Studio| s.editor().and_then(|e| e.play.as_ref()?.net.as_ref().map(|n| n.kind));
    assert_eq!(kind(&a.state().studio), Some(PlayKind::TeamCreate));
    assert_eq!(kind(&b.state().studio), Some(PlayKind::Joined));
    // One world: each player's game has both characters.
    until(&mut a, &mut b, |a, b| {
        let has_both = |s: &Studio| {
            s.editor().and_then(|e| e.play.as_ref()).is_some_and(|p| {
                let dm = p.game_player().engine().data_model();
                named(dm, "Ada").is_some() && named(dm, "Bob").is_some()
            })
        };
        has_both(a) && has_both(b)
    });
    // Ada moves her character: Bob sees it there.
    let to = Vec3::new(4.0, 6.0, -8.0);
    {
        let session = a.state_mut().studio.editor_mut().unwrap().play.as_mut().unwrap();
        let dm = session.game_player_mut().engine_mut().data_model_mut();
        let ada = named(dm, "Ada").unwrap();
        dm.set_by_name(ada, "CFrame", Value::CFrame(CFrame::new(to))).unwrap();
    }
    until(&mut a, &mut b, |_, b| {
        b.editor().and_then(|e| e.play.as_ref()).is_some_and(|p| {
            let dm = p.game_player().engine().data_model();
            named(dm, "Ada").is_some_and(|ada| match dm.get_by_name(ada, "CFrame") {
                Ok(Value::CFrame(at)) => (at.position() - to).length() < 1e-4,
                _ => false,
            })
        })
    });

    // The host stops: both are back to editing.
    a.state_mut().studio.stop();
    until(&mut a, &mut b, |a, b| !a.is_playing() && !b.is_playing());
    assert!(
        b.state().studio.editor().is_some(),
        "the participant is still in the session, editing"
    );
    a.state_mut().studio.team_action(crate::team::menu::TeamAction::End);
}

/// A client process in this process: a game client on a thread of its own, as the Player's `--connect` runs one.
struct InProcess {
    name: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ClientProcess for InProcess {
    fn name(&self) -> &str {
        &self.name
    }

    fn running(&mut self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[test]
fn multiple_clients_start_a_server_and_n_clients_that_join_it_and_stop_ends_them_all() {
    let mut harness = editing(settings_file("play-clients", None), "Many Players");
    let joined = Arc::new(AtomicUsize::new(0));
    let names = Arc::new(Mutex::new(Vec::new()));
    let stops = Arc::new(AtomicUsize::new(0));
    let launcher: Launcher = {
        let (joined, names, stops) = (Arc::clone(&joined), Arc::clone(&names), Arc::clone(&stops));
        Arc::new(move |name: &str, port: u16| {
            let stop = Arc::new(AtomicBool::new(false));
            let (thread_stop, joined, stops) = (Arc::clone(&stop), Arc::clone(&joined), Arc::clone(&stops));
            names.lock().unwrap().push(name.to_owned());
            let name_owned = name.to_owned();
            let thread = std::thread::spawn(move || {
                let print = metallic_engine::metallic_net::registry_fingerprint(
                    &metallic_engine::metallic_classes::shared_registry().unwrap(),
                );
                let connecting =
                    Connecting::start("127.0.0.1", port, &name_owned, crate::team::ENGINE, print, no_waker());
                let Ok((welcome, connecting)) = connecting.wait() else {
                    return;
                };
                let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
                let mut client = connecting.into_client(&mut dm, &welcome).unwrap();
                joined.fetch_add(1, Ordering::SeqCst);
                while !thread_stop.load(Ordering::Acquire) && client.ended().is_none() {
                    client.tick(&mut dm);
                    std::thread::sleep(Duration::from_millis(5));
                }
                client.leave();
                stops.fetch_add(1, Ordering::SeqCst);
            });
            Ok(Box::new(InProcess {
                name: name.to_owned(),
                stop,
                thread: Some(thread),
            }) as Box<dyn ClientProcess>)
        })
    };
    harness.state_mut().studio.set_launcher(launcher);
    {
        let team = harness.state_mut().studio.team_mut();
        team.play_mode = PlayMode::Clients;
        team.store.clients = 3;
    }
    press_play(&mut harness);
    assert!(
        harness.state().studio.is_playing(),
        "{:?}",
        harness.state().studio.notice()
    );
    assert_eq!(*names.lock().unwrap(), ["Player1", "Player2", "Player3"]);
    // Studio shows the server; the clients are windows of their own.
    harness.get_by_role_and_label(Role::Tab, "Server");
    assert!(harness.query_by_role_and_label(Role::Tab, "Game").is_none());
    let start = Instant::now();
    loop {
        harness.step();
        let players = harness.state().studio.editor().and_then(|e| {
            e.play
                .as_ref()?
                .net
                .as_ref()?
                .server
                .as_ref()
                .map(|s| s.players().len())
        });
        if players == Some(3) && joined.load(Ordering::SeqCst) == 3 {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "the clients never all joined"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // The server's world has the three players' characters.
    let session = harness.state().studio.editor().unwrap().play.as_ref().unwrap();
    for name in ["Player1", "Player2", "Player3"] {
        assert!(named(session.player.engine().data_model(), name).is_some(), "{name}");
    }
    // Stop ends the server and every client.
    harness.state_mut().studio.stop();
    settle(&mut harness);
    assert!(!harness.state().studio.is_playing());
    assert_eq!(
        stops.load(Ordering::SeqCst),
        3,
        "every client was stopped and waited for"
    );
}
