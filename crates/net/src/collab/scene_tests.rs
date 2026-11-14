//! Scene sync between a host and two participants in one process: every kind of edit reaches everyone, conflicts
//! resolve the same everywhere, and drift is repaired.

use metallic_core::glam::Vec3;
use metallic_core::{CFrame, DataModel, Id, Value};

use super::scene::{ClientScene, HostOut, HostScene, NetId, To, kind};
use crate::identity::UserId;

struct Peer {
    dm: DataModel,
    scene: ClientScene,
    outbox: Vec<(u8, Vec<u8>)>,
}

struct World {
    host_dm: DataModel,
    host: HostScene,
    peers: Vec<Peer>,
}

fn game() -> DataModel {
    metallic_classes::new_game().expect("a game")
}

fn workspace(dm: &DataModel) -> Id {
    dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap()
}

fn part(dm: &mut DataModel, name: &str, parent: Id) -> Id {
    let id = dm.create(dm.registry().class_id("Part").unwrap()).unwrap();
    dm.set_name(id, name.into()).unwrap();
    dm.set_parent(id, Some(parent)).unwrap();
    id
}

fn set(dm: &mut DataModel, id: Id, prop: &str, value: Value) {
    let prop = dm.prop_id(id, prop).unwrap();
    dm.set(id, prop, value).unwrap();
}

fn find(dm: &DataModel, name: &str) -> Id {
    let mut stack = vec![workspace(dm)];
    while let Some(at) = stack.pop() {
        if dm.name(at).unwrap() == name {
            return at;
        }
        stack.extend(dm.children(at).unwrap());
    }
    panic!("no {name}")
}

fn names(dm: &DataModel, parent: Id) -> Vec<String> {
    dm.children(parent)
        .unwrap()
        .map(|id| dm.name(id).unwrap().to_owned())
        .collect()
}

impl World {
    /// A host with a few parts, and `peers` participants who joined it (their scene made from the host's state, as
    /// a resync makes it).
    fn new(peers: u32) -> Self {
        let mut host_dm = game();
        let ws = workspace(&host_dm);
        let base = part(&mut host_dm, "Baseplate", ws);
        set(&mut host_dm, base, "Size", Value::Vector3(Vec3::new(64.0, 1.0, 64.0)));
        part(&mut host_dm, "Wall", ws);
        let folder = host_dm.create(host_dm.registry().class_id("Folder").unwrap()).unwrap();
        host_dm.set_name(folder, "Props".into()).unwrap();
        host_dm.set_parent(folder, Some(ws)).unwrap();
        part(&mut host_dm, "Crate", folder);
        let mut host = HostScene::new(&mut host_dm, []).unwrap();
        let records = host.sync.state(&host_dm);
        let peers = (0..peers)
            .map(|i| {
                let mut dm = game();
                let mut scene = ClientScene::new(&mut dm, [], UserId(2 + i)).unwrap();
                scene.sync.reconcile(&mut dm, &records, |_| false);
                Peer {
                    dm,
                    scene,
                    outbox: Vec::new(),
                }
            })
            .collect();
        let mut world = Self { host_dm, host, peers };
        world.assert_converged();
        world
    }

    fn user(i: usize) -> UserId {
        UserId(2 + i as u32)
    }

    /// Each participant's edits since the last call, queued for the host (not delivered yet).
    fn collect(&mut self, i: usize) {
        let peer = &mut self.peers[i];
        if let Some(msg) = peer.scene.collect(&mut peer.dm) {
            peer.outbox.push(msg);
        }
    }

    fn deliver(&mut self, out: HostOut) {
        for (to, kind, bytes) in out.messages {
            for (i, peer) in self.peers.iter_mut().enumerate() {
                let user = Self::user(i);
                let mine = match to {
                    To::All { except } => except != Some(user),
                    To::One(one) => one == user,
                };
                if !mine {
                    continue;
                }
                // A participant collects before applying (as Studio does), so its own edits are in flight.
                if let Some(msg) = peer.scene.collect(&mut peer.dm) {
                    peer.outbox.push(msg);
                }
                if let Some(reply) = peer.scene.receive(&mut peer.dm, kind, &bytes).unwrap() {
                    peer.outbox.push(reply);
                }
            }
        }
    }

    /// Delivers everything queued, in the host's order, until nothing is left.
    fn settle(&mut self) {
        loop {
            let mut out = HostOut::default();
            self.host.collect(&mut self.host_dm, &mut out);
            let mut moved = !out.messages.is_empty();
            for i in 0..self.peers.len() {
                for (kind, bytes) in std::mem::take(&mut self.peers[i].outbox) {
                    moved = true;
                    self.host
                        .receive(&mut self.host_dm, Self::user(i), kind, &bytes, &mut out)
                        .unwrap();
                }
            }
            self.deliver(out);
            for i in 0..self.peers.len() {
                self.collect(i);
                moved |= !self.peers[i].outbox.is_empty();
            }
            if !moved {
                break;
            }
        }
    }

    fn digests(&self) -> Vec<u64> {
        std::iter::once(self.host.sync.digest(&self.host_dm))
            .chain(self.peers.iter().map(|p| p.scene.sync.digest(&p.dm)))
            .collect()
    }

    fn assert_converged(&mut self) {
        self.settle();
        let digests = self.digests();
        if !digests.windows(2).all(|w| w[0] == w[1]) {
            let host = self.host.sync.state(&self.host_dm);
            for (i, peer) in self.peers.iter_mut().enumerate() {
                let theirs = peer.scene.sync.state(&peer.dm);
                for (a, b) in host.iter().zip(&theirs) {
                    assert_eq!(a, b, "peer {i} differs");
                }
                assert_eq!(host.len(), theirs.len(), "peer {i} has another number of instances");
            }
            panic!("the scenes differ: {digests:?}");
        }
        for peer in &self.peers {
            assert_eq!(
                names(&peer.dm, workspace(&peer.dm)),
                names(&self.host_dm, workspace(&self.host_dm))
            );
        }
    }
}

#[test]
fn a_move_rotate_scale_and_property_edit_on_the_host_reach_everyone() {
    let mut w = World::new(2);
    let wall = find(&w.host_dm, "Wall");
    set(
        &mut w.host_dm,
        wall,
        "CFrame",
        Value::CFrame(CFrame::from_parts(
            Vec3::new(3.0, 4.5, -2.25),
            metallic_core::glam::Quat::from_rotation_y(0.7),
        )),
    );
    set(&mut w.host_dm, wall, "Size", Value::Vector3(Vec3::new(1.0, 8.0, 20.0)));
    set(&mut w.host_dm, wall, "Transparency", Value::Number(0.25));
    w.assert_converged();
    let peer_wall = find(&w.peers[1].dm, "Wall");
    let prop = w.peers[1].dm.prop_id(peer_wall, "CFrame").unwrap();
    let host_prop = w.host_dm.prop_id(wall, "CFrame").unwrap();
    assert_eq!(
        w.peers[1].dm.get(peer_wall, prop).unwrap(),
        w.host_dm.get(wall, host_prop).unwrap(),
        "bit for bit"
    );
}

#[test]
fn a_participant_s_create_rename_reparent_and_delete_reach_the_host_and_the_others() {
    let mut w = World::new(2);
    let ws = workspace(&w.peers[0].dm);
    let tower = part(&mut w.peers[0].dm, "Tower", ws);
    let roof = part(&mut w.peers[0].dm, "Roof", tower);
    w.peers[0]
        .dm
        .set_attribute(roof, "Colour", Some(Value::String("red".into())))
        .unwrap();
    w.collect(0);
    w.assert_converged();
    let host_roof = find(&w.host_dm, "Roof");
    assert_eq!(
        w.host_dm.name(w.host_dm.parent(host_roof).unwrap().unwrap()).unwrap(),
        "Tower"
    );
    assert_eq!(
        w.host_dm.attribute(host_roof, "Colour").unwrap(),
        Some(&Value::String("red".into()))
    );
    // Rename and move into the folder.
    let props = find(&w.peers[0].dm, "Props");
    w.peers[0].dm.set_name(roof, "Lid".into()).unwrap();
    w.peers[0].dm.set_parent(roof, Some(props)).unwrap();
    w.collect(0);
    w.assert_converged();
    assert_eq!(names(&w.peers[1].dm, find(&w.peers[1].dm, "Props")), ["Crate", "Lid"]);
    // Reorder: the lid before the crate.
    let crate_ = find(&w.peers[1].dm, "Crate");
    let lid = find(&w.peers[1].dm, "Lid");
    let props1 = find(&w.peers[1].dm, "Props");
    w.peers[1].dm.set_parent_before(lid, props1, Some(crate_)).unwrap();
    w.collect(1);
    w.assert_converged();
    assert_eq!(names(&w.host_dm, find(&w.host_dm, "Props")), ["Lid", "Crate"]);
    // Delete (out of the tree, as Studio's delete is), then undo it (back in).
    w.peers[0].dm.set_parent(tower, None).unwrap();
    w.collect(0);
    w.assert_converged();
    assert!(!names(&w.host_dm, workspace(&w.host_dm)).contains(&"Tower".to_owned()));
    w.peers[0].dm.set_parent(tower, Some(ws)).unwrap();
    w.collect(0);
    w.assert_converged();
    assert!(names(&w.peers[1].dm, workspace(&w.peers[1].dm)).contains(&"Tower".to_owned()));
}

#[test]
fn concurrent_edits_of_one_property_end_at_the_host_s_last() {
    let mut w = World::new(2);
    for (i, value) in [(0, 0.1), (1, 0.9)] {
        let wall = find(&w.peers[i].dm, "Wall");
        set(&mut w.peers[i].dm, wall, "Transparency", Value::Number(value));
        w.collect(i);
    }
    w.assert_converged();
    let wall = find(&w.host_dm, "Wall");
    // The host took the first participant's first, then the second's.
    assert_eq!(w.host_dm.get_by_name(wall, "Transparency").unwrap(), Value::Number(0.9));
}

#[test]
fn moving_two_instances_into_each_other_at_once_converges_without_a_cycle() {
    let mut w = World::new(2);
    let ws = workspace(&w.host_dm);
    let a = w
        .host_dm
        .create(w.host_dm.registry().class_id("Folder").unwrap())
        .unwrap();
    w.host_dm.set_name(a, "A".into()).unwrap();
    w.host_dm.set_parent(a, Some(ws)).unwrap();
    let b = w
        .host_dm
        .create(w.host_dm.registry().class_id("Folder").unwrap())
        .unwrap();
    w.host_dm.set_name(b, "B".into()).unwrap();
    w.host_dm.set_parent(b, Some(ws)).unwrap();
    w.assert_converged();
    // Participant 0 puts A in B; participant 1 puts B in A.
    let (a0, b0) = (find(&w.peers[0].dm, "A"), find(&w.peers[0].dm, "B"));
    w.peers[0].dm.set_parent(a0, Some(b0)).unwrap();
    let (a1, b1) = (find(&w.peers[1].dm, "A"), find(&w.peers[1].dm, "B"));
    w.peers[1].dm.set_parent(b1, Some(a1)).unwrap();
    w.collect(0);
    w.collect(1);
    w.assert_converged();
    let (a, b) = (find(&w.host_dm, "A"), find(&w.host_dm, "B"));
    assert_eq!(w.host_dm.parent(a).unwrap(), Some(b), "the first move won");
    assert_eq!(w.host_dm.parent(b).unwrap(), Some(ws));
}

#[test]
fn an_edit_of_an_instance_deleted_meanwhile_does_not_bring_it_back_wrongly() {
    let mut w = World::new(2);
    let wall0 = find(&w.peers[0].dm, "Wall");
    w.peers[0].dm.set_parent(wall0, None).unwrap();
    let wall1 = find(&w.peers[1].dm, "Wall");
    set(&mut w.peers[1].dm, wall1, "Transparency", Value::Number(0.5));
    w.collect(0);
    w.collect(1);
    w.assert_converged();
    assert!(!names(&w.host_dm, workspace(&w.host_dm)).contains(&"Wall".to_owned()));
}

#[test]
fn a_create_with_someone_else_s_id_is_refused_and_taken_back() {
    let mut w = World::new(1);
    // Forge an operation by hand: a part with the host's id range.
    let ws = w.host.sync.net(workspace(&w.host_dm)).unwrap();
    let forged = super::scene::SceneOp::Create {
        parent: ws,
        before: None,
        records: vec![super::scene::Record {
            id: NetId::made_by(UserId::HOST, 999),
            parent: Some(ws),
            class: w.host_dm.registry().class_id("Part").unwrap().raw(),
            name: "Forged".into(),
            props: Vec::new(),
            attrs: Vec::new(),
        }],
    };
    let mut writer = crate::codec::Writer::new();
    writer.u64(1);
    super::scene::encode_ops(&[forged], &mut writer);
    let mut out = HostOut::default();
    // The participant never proposed seq 1 itself; feed its ack path as a well-behaved peer would see it.
    let result = w
        .host
        .receive(&mut w.host_dm, UserId(2), kind::PROPOSE, &writer.finish(), &mut out);
    assert!(result.is_ok());
    assert!(
        out.messages
            .iter()
            .all(|(to, kind, _)| *to == To::One(UserId(2)) && *kind == kind::ACK),
        "nothing accepted, nothing passed on"
    );
    assert!(!names(&w.host_dm, workspace(&w.host_dm)).contains(&"Forged".to_owned()));
    // Garbage is an error the session drops the peer for.
    let mut out = HostOut::default();
    assert!(
        w.host
            .receive(&mut w.host_dm, UserId(2), kind::PROPOSE, &[1, 2, 3], &mut out)
            .is_err()
    );
}

#[test]
fn drift_is_seen_by_the_digest_and_repaired_by_a_resync() {
    let mut w = World::new(1);
    // An edit the participant never sent (as a lost message would leave it).
    let wall = find(&w.peers[0].dm, "Wall");
    w.peers[0].dm.set_name(wall, "Drifted".into()).unwrap();
    let peer = &mut w.peers[0];
    let _lost = peer.scene.sync.collect(&mut peer.dm);
    assert_ne!(w.digests()[0], w.digests()[1]);
    let mut out = HostOut::default();
    w.host.digest(&mut w.host_dm, &mut out);
    w.deliver(out);
    assert_eq!(w.peers[0].outbox.len(), 1, "a resync is asked for");
    w.assert_converged();
    assert_eq!(
        names(&w.peers[0].dm, workspace(&w.peers[0].dm)),
        names(&w.host_dm, workspace(&w.host_dm))
    );
}

#[test]
fn the_current_camera_is_each_user_s_own() {
    let mut w = World::new(1);
    let camera = |dm: &DataModel| match dm.get_by_name(workspace(dm), "CurrentCamera").unwrap() {
        Value::Ref(Some(camera)) => camera,
        other => panic!("{other:?}"),
    };
    let host_camera = camera(&w.host_dm);
    set(
        &mut w.host_dm,
        host_camera,
        "CFrame",
        Value::CFrame(CFrame::new(Vec3::new(100.0, 50.0, 0.0))),
    );
    w.settle();
    let peer_camera = camera(&w.peers[0].dm);
    assert_ne!(
        w.peers[0].dm.get_by_name(peer_camera, "CFrame").unwrap(),
        Value::CFrame(CFrame::new(Vec3::new(100.0, 50.0, 0.0)))
    );
    assert_eq!(w.digests()[0], w.digests()[1], "views are not part of the scene");
}
