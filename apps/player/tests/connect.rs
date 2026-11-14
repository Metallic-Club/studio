//! The Player app as Multiple Clients starts it (ADR 0011 §8): real `metallic_player --connect` processes join a game
//! server, get characters, and close when the server ends the game.
//!
//! The processes open windows, so this needs a display and a GPU: it is ignored by default. Run it with
//! `cargo test -p metallic_player --test connect -- --ignored`.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use metallic_engine::metallic_classes;
use metallic_engine::metallic_net::play::GameServer;
use metallic_engine::metallic_net::registry_fingerprint;
use metallic_engine::metallic_net::transport::no_waker;

#[test]
#[ignore = "opens windows: needs a display and a GPU"]
fn player_processes_join_a_server_and_close_when_it_ends() {
    let mut world = metallic_classes::new_game().unwrap();
    let print = registry_fingerprint(world.registry());
    let mut server = GameServer::start(0, &mut world, Vec::new(), metallic_player::VERSION, print, no_waker()).unwrap();
    let port = server.port();
    let mut children: Vec<_> = (1..=2)
        .map(|n| {
            Command::new(env!("CARGO_BIN_EXE_metallic_player"))
                .args([
                    "--connect",
                    &format!("127.0.0.1:{port}"),
                    "--name",
                    &format!("Player{n}"),
                ])
                .stdin(Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    let start = Instant::now();
    while server.players().len() < 2 {
        server.tick(&mut world);
        assert!(start.elapsed() < Duration::from_secs(60), "the players never joined");
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut names: Vec<String> = server.players().into_iter().map(|(_, name)| name).collect();
    names.sort();
    assert_eq!(names, ["Player1", "Player2"]);
    // Let them run a moment: they send their characters' places.
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until {
        server.tick(&mut world);
        std::thread::sleep(Duration::from_millis(10));
    }
    server.end("the test is over");
    for _ in 0..10 {
        server.tick(&mut world);
        std::thread::sleep(Duration::from_millis(10));
    }
    for child in &mut children {
        let start = Instant::now();
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "a player did not close when the game ended"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
