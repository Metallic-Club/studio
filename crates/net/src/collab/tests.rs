//! Two peers in one process: a host and participants joining it, over memory and over real TCP.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::client::{Client, ClientEvent, ReceivedProject};
use super::host::{Host, HostConfig, HostEvent, PeerId, ProjectSnapshot};
use super::protocol::{ProjectId, SessionInfo};
use super::{ClientLink, HostServer, probe};
use crate::identity::{Identity, UserId};
use crate::transport::{Channel, Listener, no_waker};

const ENGINE: &str = "0.1.0";
const PRINT: u64 = 1234;

fn config() -> HostConfig {
    HostConfig {
        info: SessionInfo {
            session: 7,
            project: ProjectId(42),
            project_name: "Castle".into(),
            host_name: "Ada".into(),
        },
        host: Identity::new("Ada", [10, 20, 30]),
        engine: ENGINE.into(),
        fingerprint: PRINT,
    }
}

fn snapshot() -> ProjectSnapshot {
    ProjectSnapshot {
        members: vec![
            ("metallic.toml".into(), Arc::from(&b"name = \"Castle\""[..])),
            ("Scenes/Main.scene".into(), Arc::from(vec![7u8; 3 << 20])),
            ("Scripts/Door.luau".into(), Arc::from(&b"print('hi')"[..])),
        ],
        start_scene: "Scenes/Main.scene".into(),
    }
}

/// Moves every queued message between the host and the participants over memory until nothing is left, admitting
/// whoever asks with `snapshot()`.
fn settle(host: &mut Host, clients: &mut [(PeerId, &mut Client)]) {
    loop {
        let mut moved = false;
        for (peer, client) in clients.iter_mut() {
            for (channel, bytes) in client.take_outbox() {
                host.received(*peer, channel, &bytes);
                moved = true;
            }
        }
        while let Some(event) = host.next_event() {
            if let HostEvent::JoinRequested { peer, .. } = event {
                host.admit(peer, &snapshot());
            }
        }
        for (peer, channel, bytes) in host.take_outbox() {
            if let Some((_, client)) = clients.iter_mut().find(|(p, _)| *p == peer) {
                client.received(channel, &bytes);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
}

fn project_of(client: &mut Client) -> Option<ReceivedProject> {
    let mut found = None;
    while let Some(event) = client.next_event() {
        if let ClientEvent::Project(project) = event {
            found = Some(project);
        }
    }
    found
}

#[test]
fn a_participant_joins_receives_the_project_and_both_see_the_user_list() {
    let mut host = Host::new(config());
    let mut bob = Client::join(Identity::new("Bob", [1, 2, 3]), None, ENGINE, PRINT);
    host.connected(PeerId(1));
    settle(&mut host, &mut [(PeerId(1), &mut bob)]);
    let project = project_of(&mut bob).expect("the project arrived");
    assert_eq!(project.start_scene, "Scenes/Main.scene");
    assert_eq!(project.members.len(), 3);
    assert_eq!(
        project.members[1].1.len(),
        3 << 20,
        "a member larger than a chunk is whole"
    );
    assert_eq!(bob.me(), Some(UserId(2)));
    let names: Vec<&str> = bob.users().iter().map(|u| u.identity.name.as_str()).collect();
    assert_eq!(names, ["Ada", "Bob"]);
    assert_eq!(host.users().len(), 2);
    assert!(!host.is_ready(UserId(2)));
    bob.loaded();
    settle(&mut host, &mut [(PeerId(1), &mut bob)]);
    assert!(host.is_ready(UserId(2)));
}

#[test]
fn operations_after_the_snapshot_are_held_until_the_copy_is_open() {
    let mut host = Host::new(config());
    let mut bob = Client::join(Identity::new("Bob", [1, 2, 3]), None, ENGINE, PRINT);
    host.connected(PeerId(1));
    settle(&mut host, &mut [(PeerId(1), &mut bob)]);
    // An edit made after the snapshot, while Bob is still opening the project.
    host.broadcast_op(Channel::Reliable, 1, vec![9, 9], None);
    settle(&mut host, &mut [(PeerId(1), &mut bob)]);
    let mut ops = 0;
    while let Some(event) = bob.next_event() {
        if matches!(event, ClientEvent::Op { .. }) {
            ops += 1;
        }
    }
    assert_eq!(ops, 0, "nothing is applied before the copy is open");
    bob.loaded();
    assert_eq!(
        bob.next_event(),
        Some(ClientEvent::Op {
            channel: Channel::Reliable,
            kind: 1,
            bytes: vec![9, 9]
        })
    );
}

#[test]
fn names_are_made_unique_and_leaving_is_broadcast() {
    let mut host = Host::new(config());
    let mut a = Client::join(Identity::new("Bob", [1, 2, 3]), None, ENGINE, PRINT);
    let mut b = Client::join(Identity::new("bob", [4, 5, 6]), None, ENGINE, PRINT);
    host.connected(PeerId(1));
    host.connected(PeerId(2));
    settle(&mut host, &mut [(PeerId(1), &mut a), (PeerId(2), &mut b)]);
    assert_eq!(b.name(), "bob (2)");
    assert_eq!(a.users().len(), 3, "the first participant hears of the second");
    host.disconnected(PeerId(2), "gone");
    settle(&mut host, &mut [(PeerId(1), &mut a)]);
    assert_eq!(a.users().len(), 2);
    assert!(a.users().iter().all(|u| u.id != b.me().unwrap()));
}

#[test]
fn the_wrong_version_or_project_is_refused_with_a_reason() {
    let mut host = Host::new(config());
    let mut old = Client::join(Identity::new("Old", [1, 2, 3]), None, "0.0.9", PRINT);
    let mut other = Client::join(Identity::new("Other", [1, 2, 3]), Some(ProjectId(1)), ENGINE, PRINT);
    host.connected(PeerId(1));
    host.connected(PeerId(2));
    settle(&mut host, &mut [(PeerId(1), &mut old), (PeerId(2), &mut other)]);
    assert!(matches!(old.next_event(), Some(ClientEvent::Refused(reason)) if reason.contains("same version")));
    assert!(matches!(other.next_event(), Some(ClientEvent::Refused(reason)) if reason.contains("another project")));
    assert_eq!(host.take_closing().len(), 2);
    assert_eq!(host.users().len(), 1);
}

#[test]
fn garbage_drops_the_peer() {
    let mut host = Host::new(config());
    host.connected(PeerId(1));
    host.received(PeerId(1), Channel::Reliable, &[200, 1, 2]);
    assert_eq!(host.take_closing(), [PeerId(1)]);
    // An operation before joining does nothing.
    let mut host = Host::new(config());
    host.connected(PeerId(1));
    host.received(
        PeerId(1),
        Channel::Reliable,
        &super::protocol::ClientMsg::Op(1, vec![]).encode(),
    );
    assert!(host.next_event().is_none());
}

#[test]
fn ending_tells_everyone() {
    let mut host = Host::new(config());
    let mut bob = Client::join(Identity::new("Bob", [1, 2, 3]), None, ENGINE, PRINT);
    host.connected(PeerId(1));
    settle(&mut host, &mut [(PeerId(1), &mut bob)]);
    bob.loaded();
    host.end("the host ended the session");
    settle(&mut host, &mut [(PeerId(1), &mut bob)]);
    let mut ended = false;
    while let Some(event) = bob.next_event() {
        ended |= matches!(event, ClientEvent::Ended(_));
    }
    assert!(ended && bob.is_done());
    assert_eq!(host.take_closing(), [PeerId(1)]);
}

/// Polls until `done`, for at most ten seconds.
fn until(mut step: impl FnMut() -> bool) {
    let start = Instant::now();
    while !step() {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn over_tcp_a_participant_joins_and_a_probe_sees_the_session() {
    let listener = Listener::bind_addr(([127, 0, 0, 1], 0).into(), super::host_limits(), no_waker()).unwrap();
    let mut server = HostServer::with_listener(listener, Host::new(config()));
    let port = server.port();
    let mut link = ClientLink::connect(
        "127.0.0.1",
        port,
        Client::join(Identity::new("Bob", [1, 2, 3]), None, ENGINE, PRINT),
        no_waker(),
    );
    let mut project = None;
    until(|| {
        server.receive();
        while let Some(event) = server.host.next_event() {
            if let HostEvent::JoinRequested { peer, .. } = event {
                server.host.admit(peer, &snapshot());
            }
        }
        server.flush();
        link.receive();
        if let Some(found) = project_of(&mut link.client) {
            project = Some(found);
        }
        project.is_some()
    });
    assert_eq!(project.unwrap().members.len(), 3);
    link.client.loaded();
    link.flush();
    until(|| {
        server.receive();
        server.host.is_ready(UserId(2))
    });

    // The projects page's check, from another thread as Studio runs it.
    let probed = std::thread::spawn(move || probe("127.0.0.1", port, ENGINE, PRINT, Duration::from_secs(2)));
    until(|| {
        server.receive();
        server.flush();
        probed.is_finished()
    });
    let probed = probed.join().unwrap().expect("the probe answered");
    assert_eq!(probed.session.project, ProjectId(42));
    assert_eq!(probed.users, 2);

    // The participant leaves cleanly.
    link.leave();
    until(|| {
        server.receive();
        server.host.users().len() == 1
    });
}

#[test]
fn a_probe_of_nothing_fails_quickly() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let started = Instant::now();
    let error = probe("127.0.0.1", port, ENGINE, PRINT, Duration::from_millis(500)).unwrap_err();
    // Windows retries a refused connection for a while, so it may read as no answer.
    assert!(
        error.contains("nothing is hosting") || error.contains("did not answer"),
        "{error}"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
}
