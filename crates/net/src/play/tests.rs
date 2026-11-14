//! A game server and two clients over real TCP in one process: joining, characters, movement, the server's changes
//! replicated, what stays on the server, leaving and ending.

use std::time::{Duration, Instant};

use metallic_core::glam::Vec3;
use metallic_core::{CFrame, DataModel, Id, Value};

use super::{Connecting, GameClient, GameServer};
use crate::transport::{Limits, Listener, no_waker};

const ENGINE: &str = "test";

fn game() -> DataModel {
    metallic_classes::new_game().expect("a game")
}

fn service(dm: &DataModel, name: &str) -> Id {
    dm.service(dm.registry().class_id(name).unwrap()).unwrap()
}

fn find(dm: &DataModel, name: &str) -> Option<Id> {
    let mut stack = vec![dm.root().unwrap()];
    while let Some(at) = stack.pop() {
        if dm.name(at).ok()? == name && at != dm.root().unwrap() {
            return Some(at);
        }
        stack.extend(dm.children(at).ok()?);
    }
    None
}

fn part(dm: &mut DataModel, name: &str, parent: Id) -> Id {
    let id = dm.create(dm.registry().class_id("Part").unwrap()).unwrap();
    dm.set_name(id, name.into()).unwrap();
    dm.set_parent(id, Some(parent)).unwrap();
    id
}

fn at(dm: &DataModel, id: Id) -> Vec3 {
    match dm.get_by_name(id, "CFrame").unwrap() {
        Value::CFrame(cf) => cf.position(),
        other => panic!("{other:?}"),
    }
}

struct Client {
    dm: DataModel,
    game: GameClient,
}

fn join(port: u16, name: &str, server: &mut GameServer, server_dm: &mut DataModel) -> Client {
    let print = crate::registry_fingerprint(server_dm.registry());
    let mut connecting = Connecting::start("127.0.0.1", port, name, ENGINE, print, no_waker());
    let start = Instant::now();
    let welcome = loop {
        server.tick(server_dm);
        if let Some(welcome) = connecting.poll().unwrap() {
            break welcome;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "no welcome");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(
        welcome.sources,
        [("Scripts/Hud.luau".to_owned(), "print('hud')".to_owned())]
    );
    let mut dm = game();
    let game = connecting.into_client(&mut dm, &welcome).unwrap();
    Client { dm, game }
}

/// Runs the server and the clients until `done`, for at most ten seconds.
fn until(
    server: &mut GameServer,
    server_dm: &mut DataModel,
    clients: &mut [&mut Client],
    mut done: impl FnMut(&DataModel, &[&mut Client]) -> bool,
) {
    let start = Instant::now();
    loop {
        server.tick(server_dm);
        for client in clients.iter_mut() {
            client.game.tick(&mut client.dm);
        }
        if done(server_dm, clients) {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn two_players_join_one_server_see_each_other_move_and_leave() {
    let mut server_dm = game();
    let workspace = service(&server_dm, "Workspace");
    let door = part(&mut server_dm, "Door", workspace);
    let storage = service(&server_dm, "ServerStorage");
    part(&mut server_dm, "Secret", storage);
    let listener = Listener::bind_addr(([127, 0, 0, 1], 0).into(), Limits::default(), no_waker()).unwrap();
    let print = crate::registry_fingerprint(server_dm.registry());
    let sources = vec![("Scripts/Hud.luau".to_owned(), "print('hud')".to_owned())];
    let mut server = GameServer::new(listener, &mut server_dm, sources, ENGINE, print).unwrap();
    let port = server.port();

    let mut a = join(port, "Player1", &mut server, &mut server_dm);
    let mut b = join(port, "Player1", &mut server, &mut server_dm);
    assert_eq!(b.game.name, "Player1 (2)", "names are made unique");
    assert_eq!(server.players().len(), 2);
    // Each world has the door and both characters, and nothing of the server's own storage.
    until(&mut server, &mut server_dm, &mut [&mut a, &mut b], |_, clients| {
        clients
            .iter()
            .all(|c| find(&c.dm, "Player1").is_some() && find(&c.dm, "Player1 (2)").is_some())
    });
    for client in [&a, &b] {
        assert!(find(&client.dm, "Door").is_some());
        assert!(
            find(&client.dm, "Secret").is_none(),
            "ServerStorage stays on the server"
        );
    }

    // A moves its character: the server and B see it there.
    let mine = a.game.character().unwrap();
    let to = Vec3::new(3.0, 6.0, -4.0);
    a.dm.set_by_name(mine, "CFrame", Value::CFrame(CFrame::new(to)))
        .unwrap();
    until(
        &mut server,
        &mut server_dm,
        &mut [&mut a, &mut b],
        |server_dm, clients| {
            let on_server = find(server_dm, "Player1").is_some_and(|c| at(server_dm, c) == to);
            let on_b = find(&clients[1].dm, "Player1").is_some_and(|c| at(&clients[1].dm, c) == to);
            on_server && on_b
        },
    );
    // A teleport is refused: the server keeps the character where it was.
    a.dm.set_by_name(mine, "CFrame", Value::CFrame(CFrame::new(Vec3::new(900.0, 0.0, 0.0))))
        .unwrap();
    for _ in 0..20 {
        server.tick(&mut server_dm);
        a.game.tick(&mut a.dm);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(at(&server_dm, find(&server_dm, "Player1").unwrap()), to);

    // The server's game moves the door: everyone sees it.
    let open = Vec3::new(0.0, 20.0, 0.0);
    server_dm
        .set_by_name(door, "CFrame", Value::CFrame(CFrame::new(open)))
        .unwrap();
    until(&mut server, &mut server_dm, &mut [&mut a, &mut b], |_, clients| {
        clients
            .iter()
            .all(|c| find(&c.dm, "Door").is_some_and(|d| at(&c.dm, d) == open))
    });

    // B leaves: its character goes, for the server and for A.
    b.game.leave();
    until(&mut server, &mut server_dm, &mut [&mut a], |server_dm, clients| {
        find(server_dm, "Player1 (2)").is_none() && find(&clients[0].dm, "Player1 (2)").is_none()
    });

    // The server ends the game: A is told.
    server.end("the test is over");
    until(&mut server, &mut server_dm, &mut [&mut a], |_, clients| {
        clients[0].game.ended().is_some()
    });
    assert_eq!(a.game.ended(), Some("the test is over"));
}

#[test]
fn a_client_of_another_version_is_refused() {
    let mut server_dm = game();
    let listener = Listener::bind_addr(([127, 0, 0, 1], 0).into(), Limits::default(), no_waker()).unwrap();
    let mut server = GameServer::new(listener, &mut server_dm, Vec::new(), ENGINE, 1).unwrap();
    let mut connecting = Connecting::start("127.0.0.1", server.port(), "Old", "other", 1, no_waker());
    let start = Instant::now();
    let error = loop {
        server.tick(&mut server_dm);
        match connecting.poll() {
            Err(error) => break error,
            Ok(Some(_)) => panic!("welcomed"),
            Ok(None) => {}
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(error.contains("cannot join"), "{error}");
    assert!(server.players().is_empty());
}
