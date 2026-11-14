//! The live part of a session (ADR 0011 §5, §6, §7): the editor's scene and script texts kept in sync, presence
//! sent and shown, and the host's project saved, once a frame.
//!
//! - **Scene.** Local edits are collected at most [`COLLECT`] apart (a drag is coalesced, not sent each frame), and
//!   always just before the network's operations are applied, so an edit made this frame is in flight before
//!   someone else's lands on the same thing.
//! - **Script text** ([`super::text`]): every script file is a document the host made; what the user typed is
//!   compared with it each frame and sent as operations; others' operations land in the text with the cursor kept
//!   on its code. Undo and redo there undo the user's own edits only.
//! - **Presence** (camera, selection, open scripts, the cursor in a script) is sent at most 10 times a second, and
//!   only when it changed; the host gathers everyone's and sends them all together.
//! - **Saving** (the host): through the editor's own save, a few seconds after the last change, at most every
//!   [`SAVE_AT_MOST`] while edits keep coming, never while a game runs. Each save writes the archive beside the
//!   project and renames it over it, so a crash never leaves a broken file.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use metallic_engine::metallic_core::{CFrame, Id};
use metallic_engine::metallic_net::collab::client::Client;
use metallic_engine::metallic_net::collab::docs::{self, ClientDocs, DocOut, HostDocs};
use metallic_engine::metallic_net::collab::host::Host;
use metallic_engine::metallic_net::collab::presence::{self, Presence};
use metallic_engine::metallic_net::collab::scene::{self, ClientScene, HostOut, HostScene, NetId, SceneSync, To};
use metallic_engine::metallic_net::identity::{User, UserId};
use metallic_engine::metallic_net::transport::Channel;
use metallic_engine::metallic_project::SceneIds;

use super::text;
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

/// This user's presence now: the editor camera, the editor's selection, the scripts with tabs open and the cursor.
fn own_presence(editor: &EditorState, sync: &SceneSync, open: &[Id], cursor: Option<Vec<u8>>) -> Presence {
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
        cursor: cursor.map(|bytes| (NetId(0), bytes)),
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

/// The script files of the scripts with tabs `open`, for asking for their documents.
fn open_members(editor: &EditorState, open: &[Id]) -> Vec<String> {
    open.iter()
        .map(|&script| crate::code::source(&editor.dm, script))
        .filter(|member| docs::is_script_member(member))
        .collect()
}

/// The host's live session.
pub struct HostLive {
    /// The scene sync.
    pub scene: HostScene,
    /// The script documents.
    pub docs: HostDocs,
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
    /// The live session over `editor`'s scene and scripts.
    pub fn new(editor: &mut EditorState) -> Result<Self, String> {
        let ids = initial_ids(editor);
        let scene = HostScene::new(&mut editor.dm, ids)?;
        let now = Instant::now();
        let mut live = Self {
            scene,
            docs: HostDocs::default(),
            presences: BTreeMap::new(),
            presence_changed: false,
            last_collect: now,
            last_presence: now,
            last_digest: now,
            changed: None,
        };
        // Every script's file is a document from the start: nobody is in the session yet to be told.
        let mut out = DocOut::default();
        live.ensure_docs(editor, &mut out);
        Ok(live)
    }

    /// Stops syncing `editor`'s scene and scripts and forgets the others' presence.
    pub fn release(self, editor: &mut EditorState) {
        self.scene.sync.release(&mut editor.dm);
        editor.presence = Default::default();
        editor.scripts.collab = Default::default();
    }

    fn send(host: &mut Host, out: HostOut) {
        for (to, kind, bytes) in out.messages {
            match to {
                To::All { except } => host.broadcast_op(Channel::Reliable, kind, bytes, except),
                To::One(user) => host.send_op(user, Channel::Reliable, kind, bytes),
            }
        }
    }

    fn send_docs(host: &mut Host, out: DocOut) {
        for (kind, bytes, except) in out.all {
            host.broadcast_op(Channel::Reliable, kind, bytes, except);
        }
        for (to, kind, bytes) in out.one {
            host.send_op(to, Channel::Reliable, kind, bytes);
        }
    }

    fn noted(&mut self, out: &HostOut, now: Instant) {
        if out.messages.iter().any(|(_, kind, _)| *kind == scene::kind::APPLY) {
            self.note_change(now);
        }
    }

    fn note_change(&mut self, now: Instant) {
        self.changed = Some(match self.changed {
            Some((_, first)) => (now, first),
            None => (now, now),
        });
    }

    /// A document for every script's file that has none yet.
    fn ensure_docs(&mut self, editor: &mut EditorState, out: &mut DocOut) {
        for (_, member) in crate::code::script_files(&editor.dm) {
            if self.docs.get(&member).is_none() {
                let text = editor.scripts.text(&mut editor.project, &member).clone();
                self.docs.ensure(&member, &text, out);
            }
        }
    }

    /// The host's own typing since the last frame, as operations for everyone.
    fn texts(&mut self, editor: &mut EditorState, host: &mut Host) {
        let mut out = DocOut::default();
        self.ensure_docs(editor, &mut out);
        let changes = text::local_changes(editor, &self.docs);
        if !changes.is_empty() {
            self.note_change(Instant::now());
        }
        for (member, now, merge) in changes {
            self.docs.local_edit(&member, &now, merge, &mut out);
        }
        Self::send_docs(host, out);
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
        self.texts(editor, host);
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

    /// A participant was just admitted: every document's state follows their project.
    pub fn joined(&mut self, user: UserId, host: &mut Host) {
        let mut out = DocOut::default();
        self.docs.send_all(user, &mut out);
        Self::send_docs(host, out);
        self.presence_changed = true;
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
            docs::kind::OPEN | docs::kind::EDIT => {
                // The host's own typing first, so it is ordered before theirs.
                self.texts(editor, host);
                let anchors = text::anchors(editor, &self.docs);
                let mut out = DocOut::default();
                let mut read = |member: &str| editor.scripts.text(&mut editor.project, member).clone();
                let ok = self.docs.receive(from, kind, bytes, &mut read, &mut out).is_ok();
                for member in std::mem::take(&mut out.changed) {
                    text::apply(editor, &mut self.docs, &member, anchors.as_ref());
                    self.note_change(Instant::now());
                }
                Self::send_docs(host, out);
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

    /// Once a frame: the host's edits, undo in scripts, its presence, the digest and the save. `open` are the
    /// scripts with tabs open.
    pub fn tick(&mut self, editor: &mut EditorState, host: &mut Host, open: &[Id]) {
        let now = Instant::now();
        if now.duration_since(self.last_collect) >= COLLECT {
            self.flush(editor, host);
        }
        // Undo and redo in a script are the document's: the host's own edits only.
        for (member, redo) in std::mem::take(&mut editor.scripts.collab.requests) {
            self.texts(editor, host);
            let mut out = DocOut::default();
            if let Some((now_text, at)) = self.docs.undo(&member, redo, &mut out) {
                editor.scripts.set_from_session(&mut editor.project, &member, now_text);
                editor.scripts.collab.moves.insert(member, (at, at));
            }
            Self::send_docs(host, out);
        }
        text::publish(editor, &self.docs);
        if now.duration_since(self.last_presence) >= PRESENCE {
            self.last_presence = now;
            let cursor = text::cursor_bytes(editor, &self.docs);
            let mine = own_presence(editor, &self.scene.sync, open, cursor);
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
        text::show_carets(editor, &mut self.docs, &users, &all, UserId::HOST);
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
    /// The script documents.
    pub docs: ClientDocs,
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
            docs: ClientDocs::new(me),
            remote: Vec::new(),
            sent: None,
            last_collect: now,
            last_presence: now,
        })
    }

    /// Stops syncing `editor`'s scene and scripts and forgets the others' presence.
    pub fn release(self, editor: &mut EditorState) {
        self.scene.sync.release(&mut editor.dm);
        editor.presence = Default::default();
        editor.scripts.collab = Default::default();
    }

    /// The participant's typing since the last frame, as operations for the host.
    fn texts(&mut self, editor: &mut EditorState, client: &mut Client) {
        for (member, now, merge) in text::local_changes(editor, &self.docs) {
            if let Some((kind, bytes)) = self.docs.local_edit(&member, &now, merge) {
                client.send_op(Channel::Reliable, kind, bytes);
            }
        }
    }

    fn flush(&mut self, editor: &mut EditorState, client: &mut Client) {
        if let Some((kind, bytes)) = self.scene.collect(&mut editor.dm) {
            client.send_op(Channel::Reliable, kind, bytes);
        }
        self.texts(editor, client);
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
            docs::kind::STATE | docs::kind::EDITS => {
                // What the participant typed is in its document before others' edits land beside it.
                self.texts(editor, client);
                let anchors = text::anchors(editor, &self.docs);
                match self.docs.receive(kind, bytes) {
                    Ok(events) => {
                        for member in events.replaced.iter().chain(&events.edited) {
                            text::apply(editor, &mut self.docs, member, anchors.as_ref());
                        }
                        true
                    }
                    Err(error) => {
                        log::warn!("team create: the host sent a script edit that is not valid: {error}");
                        false
                    }
                }
            }
            _ => true,
        }
    }

    /// Once a frame: the participant's edits, undo in scripts, presence, and the others' shown.
    pub fn tick(&mut self, editor: &mut EditorState, client: &mut Client, open: &[Id]) {
        let now = Instant::now();
        if now.duration_since(self.last_collect) >= COLLECT {
            self.flush(editor, client);
        }
        // A script opened that has no document yet: the host makes one.
        for member in open_members(editor, open) {
            if let Some((kind, bytes)) = self.docs.open(&member) {
                client.send_op(Channel::Reliable, kind, bytes);
            }
        }
        for (member, redo) in std::mem::take(&mut editor.scripts.collab.requests) {
            self.texts(editor, client);
            if let Some(((kind, bytes), now_text, at)) = self.docs.undo(&member, redo) {
                client.send_op(Channel::Reliable, kind, bytes);
                editor.scripts.set_from_session(&mut editor.project, &member, now_text);
                editor.scripts.collab.moves.insert(member, (at, at));
            }
        }
        text::publish(editor, &self.docs);
        if now.duration_since(self.last_presence) >= PRESENCE {
            self.last_presence = now;
            let cursor = text::cursor_bytes(editor, &self.docs);
            let mine = own_presence(editor, &self.scene.sync, open, cursor);
            if self.sent.as_ref() != Some(&mine) {
                client.send_op(Channel::Presence, presence::kind::MINE, mine.mine());
                self.sent = Some(mine);
            }
        }
        let me = client.me().unwrap_or(UserId(0));
        let users = client.users().to_vec();
        let all: Vec<(UserId, &Presence)> = self.remote.iter().map(|(u, p)| (*u, p)).collect();
        show(editor, &self.scene.sync, &users, &all, me);
        text::show_carets(editor, &mut self.docs, &users, &all, me);
    }
}
