//! The live part of a session (ADR 0011 §5, §7): the editor's scene kept in sync, presence sent and shown, and the
//! host's project saved, once a frame.
//!
//! - **Scene.** Local edits are collected at most [`COLLECT`] apart (a drag is coalesced, not sent each frame), and
//!   always just before the network's operations are applied, so an edit made this frame is in flight before
//!   someone else's lands on the same thing.
//! - **Presence** (camera, selection, open scripts) is sent at most 10 times a second, and only when it changed; the
//!   host gathers everyone's and sends them all together.
//! - **Saving** (the host): through the editor's own save, a few seconds after the last change, at most every
//!   [`SAVE_AT_MOST`] while edits keep coming, never while a game runs. Each save writes the archive beside the
//!   project and renames it over it, so a crash never leaves a broken file.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use metallic_engine::metallic_core::{CFrame, Id};
use metallic_engine::metallic_net::collab::client::Client;
use metallic_engine::metallic_net::collab::host::Host;
use metallic_engine::metallic_net::collab::presence::{self, Presence};
use metallic_engine::metallic_net::collab::scene::{self, ClientScene, HostOut, HostScene, NetId, SceneSync, To};
use metallic_engine::metallic_net::identity::{User, UserId};
use metallic_engine::metallic_net::transport::Channel;
use metallic_engine::metallic_project::SceneIds;

use crate::editor::EditorState;

/// How often local edits are collected at most.
pub const COLLECT: Duration = Duration::from_millis(50);
/// How often presence is sent at most.
pub const PRESENCE: Duration = Duration::from_millis(100);
/// How often the host sends its scene's digest.
pub const DIGEST: Duration = Duration::from_secs(10);
/// How long after the last change the host saves.
pub const SAVE_AFTER: Duration = Duration::from_secs(4);
/// How long changes may go unsaved while edits keep coming.
pub const SAVE_AT_MOST: Duration = Duration::from_secs(30);

/// The net ids of the editor's scene at the start of a session: its scene ids (the file's, or the host's net ids in
/// a participant's copy).
pub fn initial_ids(editor: &EditorState) -> Vec<(Id, NetId)> {
    editor.scene_ids.iter().map(|(id, scene)| (id, NetId(scene))).collect()
}

/// This user's presence now: the editor camera, the editor's selection and the scripts with tabs open.
fn own_presence(editor: &EditorState, sync: &SceneSync, open: &[Id]) -> Presence {
    Presence {
        camera: Some(editor.viewport.camera.cframe().to_array()),
        selection: editor
            .selection
            .ids()
            .iter()
            .filter_map(|&id| sync.net(id))
            .take(presence::MAX_SELECTION)
            .collect(),
        open: open
            .iter()
            .filter_map(|&id| sync.net(id))
            .take(presence::MAX_OPEN)
            .collect(),
        cursor: None,
    }
}

/// Shows `presences` (others', never `me`'s) in the editor.
fn show(editor: &mut EditorState, sync: &SceneSync, users: &[User], presences: &[(UserId, &Presence)], me: UserId) {
    let now = Instant::now();
    let mut shown = std::mem::take(&mut editor.presence);
    let mut present = Vec::new();
    for (id, presence) in presences {
        if *id == me {
            continue;
        }
        let Some(user) = users.iter().find(|u| u.id == *id) else {
            continue;
        };
        present.push(*id);
        let local = |ids: &[NetId]| -> Vec<Id> { ids.iter().filter_map(|&net| sync.local(net)).collect() };
        shown.update(
            user,
            presence.camera.and_then(CFrame::from_array),
            local(&presence.selection),
            local(&presence.open),
            now,
        );
    }
    shown.retain(&present);
    editor.presence = shown;
}

/// The host's live session.
pub struct HostLive {
    /// The scene sync.
    pub scene: HostScene,
    presences: BTreeMap<UserId, Presence>,
    presence_changed: bool,
    last_collect: Instant,
    last_presence: Instant,
    last_digest: Instant,
    /// When the scene last changed, and when it first changed since the last save.
    changed: Option<(Instant, Instant)>,
}

impl std::fmt::Debug for HostLive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostLive").finish_non_exhaustive()
    }
}

impl HostLive {
    /// The live session over `editor`'s scene.
    pub fn new(editor: &mut EditorState) -> Result<Self, String> {
        let ids = initial_ids(editor);
        let scene = HostScene::new(&mut editor.dm, ids)?;
        let now = Instant::now();
        Ok(Self {
            scene,
            presences: BTreeMap::new(),
            presence_changed: false,
            last_collect: now,
            last_presence: now,
            last_digest: now,
            changed: None,
        })
    }

    /// Stops syncing `editor`'s scene and forgets the others' presence.
    pub fn release(self, editor: &mut EditorState) {
        self.scene.sync.release(&mut editor.dm);
        editor.presence = Default::default();
    }

    fn send(host: &mut Host, out: HostOut) {
        for (to, kind, bytes) in out.messages {
            match to {
                To::All { except } => host.broadcast_op(Channel::Reliable, kind, bytes, except),
                To::One(user) => host.send_op(user, Channel::Reliable, kind, bytes),
            }
        }
    }

    fn noted(&mut self, out: &HostOut, now: Instant) {
        if out.messages.iter().any(|(_, kind, _)| *kind == scene::kind::APPLY) {
            self.changed = Some(match self.changed {
                Some((_, first)) => (now, first),
                None => (now, now),
            });
        }
    }

    /// Sends the host's own edits now (before a snapshot is made).
    pub fn flush(&mut self, editor: &mut EditorState, host: &mut Host) {
        let mut out = HostOut::default();
        self.scene.collect(&mut editor.dm, &mut out);
        let now = Instant::now();
        self.noted(&out, now);
        if !out.messages.is_empty() {
            editor.collab_changed = true;
        }
        Self::send(host, out);
        self.last_collect = now;
    }

    /// The editor's scene saved with its net ids as scene ids, for a joining participant's copy.
    pub fn scene_ids(&self) -> SceneIds {
        let mut ids = SceneIds::new();
        for (id, net) in self.scene.sync.pairs() {
            ids.assign(id, net.0);
        }
        ids
    }

    /// An operation from `from`. Returns false when it broke the protocol (drop them).
    pub fn receive(&mut self, editor: &mut EditorState, host: &mut Host, from: UserId, kind: u8, bytes: &[u8]) -> bool {
        match kind {
            presence::kind::MINE => match Presence::read_mine(bytes) {
                Ok(presence) => {
                    self.presences.insert(from, presence);
                    self.presence_changed = true;
                    true
                }
                Err(_) => false,
            },
            scene::kind::PROPOSE | scene::kind::RESYNC_ASK => {
                let mut out = HostOut::default();
                let ok = self.scene.receive(&mut editor.dm, from, kind, bytes, &mut out).is_ok();
                let now = Instant::now();
                self.noted(&out, now);
                if out.messages.iter().any(|(_, k, _)| *k == scene::kind::APPLY) {
                    editor.collab_changed = true;
                }
                Self::send(host, out);
                ok
            }
            _ => false,
        }
    }

    /// A user left: their presence goes.
    pub fn left(&mut self, user: UserId) {
        self.presences.remove(&user);
        self.presence_changed = true;
    }

    /// Once a frame: the host's edits, its presence, the digest and the save. `open` are the scripts with tabs
    /// open.
    pub fn tick(&mut self, editor: &mut EditorState, host: &mut Host, open: &[Id]) {
        let now = Instant::now();
        if now.duration_since(self.last_collect) >= COLLECT {
            self.flush(editor, host);
        }
        if now.duration_since(self.last_presence) >= PRESENCE {
            self.last_presence = now;
            let mine = own_presence(editor, &self.scene.sync, open);
            if self.presences.get(&UserId::HOST) != Some(&mine) {
                self.presences.insert(UserId::HOST, mine);
                self.presence_changed = true;
            }
            if self.presence_changed {
                self.presence_changed = false;
                let all: Vec<(UserId, &Presence)> = self.presences.iter().map(|(u, p)| (*u, p)).collect();
                host.broadcast_op(Channel::Presence, presence::kind::ALL, presence::encode_all(&all), None);
            }
        }
        if now.duration_since(self.last_digest) >= DIGEST {
            self.last_digest = now;
            let mut out = HostOut::default();
            self.scene.digest(&mut editor.dm, &mut out);
            Self::send(host, out);
        }
        let users = host.users();
        let all: Vec<(UserId, &Presence)> = self.presences.iter().map(|(u, p)| (*u, p)).collect();
        show(editor, &self.scene.sync, &users, &all, UserId::HOST);
        // Saving: a few seconds after the last change, or at the latest a while after the first unsaved one.
        if let Some((last, first)) = self.changed
            && (now.duration_since(last) >= SAVE_AFTER || now.duration_since(first) >= SAVE_AT_MOST)
        {
            if editor.is_playing() {
                return;
            }
            self.changed = None;
            if editor.is_modified() {
                match editor.save() {
                    Ok(()) => log::info!("team create: saved the session's edits"),
                    Err(error) => log::warn!("team create: could not save: {error}"),
                }
            }
        }
    }
}

/// A participant's live session.
pub struct ClientLive {
    /// The scene sync.
    pub scene: ClientScene,
    remote: Vec<(UserId, Presence)>,
    sent: Option<Presence>,
    last_collect: Instant,
    last_presence: Instant,
}

impl std::fmt::Debug for ClientLive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientLive").finish_non_exhaustive()
    }
}

impl ClientLive {
    /// The live session of participant `me` over `editor`'s scene (their copy, whose scene ids are the net ids).
    pub fn new(editor: &mut EditorState, me: UserId) -> Result<Self, String> {
        let ids = initial_ids(editor);
        let scene = ClientScene::new(&mut editor.dm, ids, me)?;
        let now = Instant::now();
        Ok(Self {
            scene,
            remote: Vec::new(),
            sent: None,
            last_collect: now,
            last_presence: now,
        })
    }

    /// Stops syncing `editor`'s scene and forgets the others' presence.
    pub fn release(self, editor: &mut EditorState) {
        self.scene.sync.release(&mut editor.dm);
        editor.presence = Default::default();
    }

    fn flush(&mut self, editor: &mut EditorState, client: &mut Client) {
        if let Some((kind, bytes)) = self.scene.collect(&mut editor.dm) {
            client.send_op(Channel::Reliable, kind, bytes);
        }
        self.last_collect = Instant::now();
    }

    /// An operation from the host. Returns false when it broke the protocol (the session is broken).
    pub fn receive(&mut self, editor: &mut EditorState, client: &mut Client, kind: u8, bytes: &[u8]) -> bool {
        match kind {
            presence::kind::ALL => match presence::decode_all(bytes) {
                Ok(all) => {
                    self.remote = all;
                    true
                }
                Err(_) => false,
            },
            scene::kind::APPLY | scene::kind::ACK | scene::kind::DIGEST | scene::kind::RESYNC => {
                // This frame's own edits go out first, so the host orders them before what comes after.
                self.flush(editor, client);
                match self.scene.receive(&mut editor.dm, kind, bytes) {
                    Ok(reply) => {
                        if let Some((kind, bytes)) = reply {
                            client.send_op(Channel::Reliable, kind, bytes);
                        }
                        true
                    }
                    Err(error) => {
                        log::warn!("team create: the host sent a scene message that is not valid: {error}");
                        false
                    }
                }
            }
            _ => true,
        }
    }

    /// Once a frame: the participant's edits and presence, and the others' shown.
    pub fn tick(&mut self, editor: &mut EditorState, client: &mut Client, open: &[Id]) {
        let now = Instant::now();
        if now.duration_since(self.last_collect) >= COLLECT {
            self.flush(editor, client);
        }
        if now.duration_since(self.last_presence) >= PRESENCE {
            self.last_presence = now;
            let mine = own_presence(editor, &self.scene.sync, open);
            if self.sent.as_ref() != Some(&mine) {
                client.send_op(Channel::Presence, presence::kind::MINE, mine.mine());
                self.sent = Some(mine);
            }
        }
        let me = client.me().unwrap_or(UserId(0));
        let users = client.users().to_vec();
        let all: Vec<(UserId, &Presence)> = self.remote.iter().map(|(u, p)| (*u, p)).collect();
        show(editor, &self.scene.sync, &users, &all, me);
    }
}
