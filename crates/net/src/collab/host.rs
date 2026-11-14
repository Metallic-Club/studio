//! The host's side of a session, apart from any transport: who is connected, the join, and what to send whom.
//!
//! [`Host`] is fed what the transport saw ([`Host::connected`], [`Host::received`], [`Host::disconnected`]) and
//! fills an outbox the transport drains ([`Host::take_outbox`]). It never touches a socket, so the same logic runs
//! over TCP ([`super::HostServer`]), over memory in tests, and later behind a relay of the platform's.
//!
//! The host is the authority: it decides who joins and under what name, numbers users, and forwards operations only
//! from users who joined. What an operation does is the app's to check ([`HostEvent::Op`]); this layer only makes
//! sure it came from a live user within the rate limits.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use super::protocol::{CHUNK, ClientMsg, HostMsg, PROTOCOL_VERSION, Purpose, SessionInfo};
use crate::identity::{Identity, User, UserId, unique_name};
use crate::transport::Channel;

/// A connection, numbered by the transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PeerId(pub u64);

/// The most participants a session takes, the host not counted.
pub const MAX_PARTICIPANTS: usize = 32;

/// How many messages a second a participant may send on average (a burst of [`MESSAGE_BURST`]).
pub const MESSAGES_PER_SECOND: f64 = 400.0;
/// How many messages a participant may send at once.
pub const MESSAGE_BURST: f64 = 4000.0;

/// What a session is: the project, the host, and the versions a participant must match.
#[derive(Clone, Debug)]
pub struct HostConfig {
    /// The session, as everyone sees it.
    pub info: SessionInfo,
    /// The host's identity.
    pub host: Identity,
    /// The engine version participants must have.
    pub engine: String,
    /// The registry fingerprint participants must have ([`crate::registry_fingerprint`]).
    pub fingerprint: u64,
}

/// A project's members, as they are sent to a joining participant.
#[derive(Clone, Debug, Default)]
pub struct ProjectSnapshot {
    /// Each member's path and contents. The start scene's holds the host's scene as it is now.
    pub members: Vec<(String, Arc<[u8]>)>,
    /// The start scene's member.
    pub start_scene: String,
}

/// Something the app has to act on.
#[derive(Clone, Debug, PartialEq)]
pub enum HostEvent {
    /// A participant may join as `user`: make a snapshot of the project as it is now, with every edit so far sent,
    /// and pass it to [`Host::admit`] (or [`Host::refuse`] them).
    JoinRequested {
        /// The connection.
        peer: PeerId,
        /// Who they will be.
        user: User,
    },
    /// A user finished loading the project.
    Ready(UserId),
    /// A user left (disconnected, or was dropped), and why.
    Left {
        /// Who.
        user: User,
        /// Why, in words.
        reason: String,
    },
    /// A user sent an operation (scene, text or presence) of `kind`.
    Op {
        /// Who sent it.
        from: UserId,
        /// The channel it came on.
        channel: Channel,
        /// Its kind.
        kind: u8,
        /// Its bytes, not checked yet.
        bytes: Vec<u8>,
    },
}

#[derive(Debug)]
enum State {
    /// Connected; the first message is awaited.
    New,
    /// Asked to join; waiting for the app to admit them.
    Asking(User),
    /// In the session: receives everything broadcast.
    Live { user: User, ready: bool },
    /// Being closed.
    Closing,
}

#[derive(Debug)]
struct Peer {
    state: State,
    tokens: f64,
    at: Instant,
}

/// The host's session state.
#[derive(Debug)]
pub struct Host {
    config: HostConfig,
    me: User,
    peers: BTreeMap<PeerId, Peer>,
    next_user: UserId,
    outbox: Vec<(PeerId, Channel, Vec<u8>)>,
    closing: Vec<PeerId>,
    events: VecDeque<HostEvent>,
    ended: bool,
}

impl Host {
    /// A session hosted as `config` says, with nobody else in it yet.
    pub fn new(config: HostConfig) -> Self {
        let me = User {
            id: UserId::HOST,
            identity: config.host.clone(),
            host: true,
        };
        Self {
            config,
            me,
            peers: BTreeMap::new(),
            next_user: UserId::HOST.next(),
            outbox: Vec::new(),
            closing: Vec::new(),
            events: VecDeque::new(),
            ended: false,
        }
    }

    /// The session.
    pub fn info(&self) -> &SessionInfo {
        &self.config.info
    }

    /// The host as a user.
    pub fn me(&self) -> &User {
        &self.me
    }

    /// Everyone in the session: the host, then participants by id (those still asking to join left out).
    pub fn users(&self) -> Vec<User> {
        let mut users = vec![self.me.clone()];
        users.extend(self.peers.values().filter_map(|peer| match &peer.state {
            State::Live { user, .. } => Some(user.clone()),
            _ => None,
        }));
        users.sort_by_key(|user| user.id);
        users
    }

    /// Whether the user has loaded the project (the host always has).
    pub fn is_ready(&self, id: UserId) -> bool {
        id == UserId::HOST
            || self
                .peers
                .values()
                .any(|peer| matches!(&peer.state, State::Live { user, ready } if user.id == id && *ready))
    }

    /// Whether [`Host::end`] was called.
    pub fn is_ended(&self) -> bool {
        self.ended
    }

    /// The next thing to act on.
    pub fn next_event(&mut self) -> Option<HostEvent> {
        self.events.pop_front()
    }

    /// Takes what is to be sent.
    pub fn take_outbox(&mut self) -> Vec<(PeerId, Channel, Vec<u8>)> {
        std::mem::take(&mut self.outbox)
    }

    /// Takes the connections to close, once their messages are sent.
    pub fn take_closing(&mut self) -> Vec<PeerId> {
        std::mem::take(&mut self.closing)
    }

    /// The transport has a new connection.
    pub fn connected(&mut self, peer: PeerId) {
        self.peers.insert(
            peer,
            Peer {
                state: State::New,
                tokens: MESSAGE_BURST,
                at: Instant::now(),
            },
        );
        if self.ended {
            self.close_with(peer, HostMsg::Refused("the session has ended".to_owned()));
        }
    }

    /// The transport received `bytes` from `peer` on `channel`.
    pub fn received(&mut self, peer: PeerId, channel: Channel, bytes: &[u8]) {
        let Some(state) = self.peers.get_mut(&peer) else {
            return;
        };
        // The rate limit: a flood is dropped, not queued.
        let now = Instant::now();
        state.tokens =
            (state.tokens + now.duration_since(state.at).as_secs_f64() * MESSAGES_PER_SECOND).min(MESSAGE_BURST);
        state.at = now;
        state.tokens -= 1.0;
        if state.tokens < 0.0 {
            self.drop_peer(peer, "sent too many messages");
            return;
        }
        let msg = match ClientMsg::decode(bytes) {
            Ok(msg) => msg,
            Err(error) => {
                log::warn!("team create: a bad message from {peer:?}: {error}");
                self.drop_peer(peer, "sent a message that is not valid");
                return;
            }
        };
        match msg {
            ClientMsg::Hello {
                protocol,
                engine,
                fingerprint,
                purpose,
            } => self.hello(peer, protocol, &engine, fingerprint, purpose),
            ClientMsg::Ready => {
                if let Some(Peer {
                    state: State::Live { user, ready },
                    ..
                }) = self.peers.get_mut(&peer)
                    && !*ready
                {
                    *ready = true;
                    let id = user.id;
                    self.broadcast(Channel::Reliable, &HostMsg::UserReady(id), Some(peer));
                    self.events.push_back(HostEvent::Ready(id));
                }
            }
            ClientMsg::Leave => self.drop_peer(peer, "left"),
            ClientMsg::Op(kind, bytes) => {
                // An operation before joining is ignored: the participant has nothing to edit yet.
                if let Some(State::Live { user, .. }) = self.peers.get(&peer).map(|p| &p.state) {
                    self.events.push_back(HostEvent::Op {
                        from: user.id,
                        channel,
                        kind,
                        bytes,
                    });
                }
            }
        }
    }

    fn hello(&mut self, peer: PeerId, protocol: u32, engine: &str, fingerprint: u64, purpose: Purpose) {
        if !matches!(self.peers.get(&peer).map(|p| &p.state), Some(State::New)) {
            self.drop_peer(peer, "said hello twice");
            return;
        }
        if protocol != PROTOCOL_VERSION || engine != self.config.engine || fingerprint != self.config.fingerprint {
            let reason = format!(
                "this Studio ({engine}) cannot join a session hosted by Studio {}: both need the same version",
                self.config.engine
            );
            self.close_with(peer, HostMsg::Refused(reason));
            return;
        }
        let identity = match purpose {
            Purpose::Probe => {
                let info = HostMsg::ProbeInfo {
                    session: self.config.info.clone(),
                    users: self.users().len() as u32,
                };
                self.close_with(peer, info);
                return;
            }
            Purpose::Join { identity, project } => {
                if let Some(project) = project
                    && project != self.config.info.project
                {
                    let reason = format!(
                        "this address now hosts another project ({}), not the one you joined before",
                        self.config.info.project_name
                    );
                    self.close_with(peer, HostMsg::Refused(reason));
                    return;
                }
                identity
            }
        };
        let participants = self
            .peers
            .values()
            .filter(|p| matches!(p.state, State::Live { .. } | State::Asking(_)))
            .count();
        if participants >= MAX_PARTICIPANTS {
            self.close_with(
                peer,
                HostMsg::Refused(format!("the session is full ({MAX_PARTICIPANTS} people)")),
            );
            return;
        }
        let taken: Vec<String> = std::iter::once(self.me.identity.name.clone())
            .chain(self.peers.values().filter_map(|p| match &p.state {
                State::Live { user, .. } | State::Asking(user) => Some(user.identity.name.clone()),
                _ => None,
            }))
            .collect();
        let name = unique_name(&identity.name, taken.iter().map(String::as_str));
        let user = User {
            id: self.next_user,
            identity: Identity {
                name,
                color: identity.color,
            },
            host: false,
        };
        self.next_user = self.next_user.next();
        if let Some(p) = self.peers.get_mut(&peer) {
            p.state = State::Asking(user.clone());
        }
        self.events.push_back(HostEvent::JoinRequested { peer, user });
    }

    /// Lets the participant asking on `peer` in: sends them the welcome and `snapshot`, then makes them live, so
    /// every message broadcast after this follows the snapshot on their stream. Tells everyone else they joined.
    pub fn admit(&mut self, peer: PeerId, snapshot: &ProjectSnapshot) {
        let Some(Peer {
            state: State::Asking(user),
            ..
        }) = self.peers.get(&peer)
        else {
            return;
        };
        let user = user.clone();
        let mut users = self.users();
        users.push(user.clone());
        users.sort_by_key(|u| u.id);
        self.send(
            peer,
            Channel::Reliable,
            &HostMsg::Welcome {
                you: user.id,
                name: user.identity.name.clone(),
                session: self.config.info.clone(),
                users,
            },
        );
        let bytes: u64 = snapshot.members.iter().map(|(_, data)| data.len() as u64).sum();
        self.send(
            peer,
            Channel::Reliable,
            &HostMsg::ProjectBegin {
                members: snapshot.members.len() as u32,
                bytes,
                start_scene: snapshot.start_scene.clone(),
            },
        );
        for (path, data) in &snapshot.members {
            self.send(
                peer,
                Channel::Reliable,
                &HostMsg::Member {
                    path: path.clone(),
                    len: data.len() as u64,
                },
            );
            for piece in data.chunks(CHUNK) {
                self.send(peer, Channel::Reliable, &HostMsg::Chunk(Arc::from(piece)));
            }
        }
        self.send(peer, Channel::Reliable, &HostMsg::ProjectEnd);
        self.broadcast(Channel::Reliable, &HostMsg::UserJoined(user.clone()), Some(peer));
        if let Some(p) = self.peers.get_mut(&peer) {
            p.state = State::Live { user, ready: false };
        }
        log::info!("team create: {peer:?} joined");
    }

    /// Turns away the participant asking on `peer`.
    pub fn refuse(&mut self, peer: PeerId, reason: &str) {
        self.close_with(peer, HostMsg::Refused(reason.to_owned()));
    }

    /// The transport lost `peer`.
    pub fn disconnected(&mut self, peer: PeerId, reason: &str) {
        if let Some(gone) = self.peers.remove(&peer)
            && let State::Live { user, .. } = gone.state
        {
            self.left(user, reason);
        }
    }

    fn left(&mut self, user: User, reason: &str) {
        self.broadcast(
            Channel::Reliable,
            &HostMsg::UserLeft {
                id: user.id,
                reason: reason.to_owned(),
            },
            None,
        );
        self.events.push_back(HostEvent::Left {
            user,
            reason: reason.to_owned(),
        });
    }

    /// Drops `peer` (a broken rule, a flood), telling everyone they left.
    pub fn drop_peer(&mut self, peer: PeerId, reason: &str) {
        let Some(gone) = self.peers.get_mut(&peer) else {
            return;
        };
        let state = std::mem::replace(&mut gone.state, State::Closing);
        if let State::Live { user, .. } = state {
            self.left(user, reason);
        }
        self.closing.push(peer);
    }

    /// Drops the user with id `id`.
    pub fn drop_user(&mut self, id: UserId, reason: &str) {
        if let Some(peer) = self.peer_of(id) {
            self.drop_peer(peer, reason);
        }
    }

    fn close_with(&mut self, peer: PeerId, msg: HostMsg) {
        self.send(peer, Channel::Reliable, &msg);
        if let Some(p) = self.peers.get_mut(&peer) {
            p.state = State::Closing;
        }
        self.closing.push(peer);
    }

    /// The connection of user `id`, while they are live.
    pub fn peer_of(&self, id: UserId) -> Option<PeerId> {
        self.peers.iter().find_map(|(peer, p)| match &p.state {
            State::Live { user, .. } if user.id == id => Some(*peer),
            _ => None,
        })
    }

    fn send(&mut self, peer: PeerId, channel: Channel, msg: &HostMsg) {
        self.outbox.push((peer, channel, msg.encode()));
    }

    /// Sends `msg` to every live participant but `except`.
    pub fn broadcast(&mut self, channel: Channel, msg: &HostMsg, except: Option<PeerId>) {
        let bytes = msg.encode();
        for (peer, p) in &self.peers {
            if Some(*peer) != except && matches!(p.state, State::Live { .. }) {
                self.outbox.push((*peer, channel, bytes.clone()));
            }
        }
    }

    /// Sends an operation of `kind` to every live participant but `except` (`None`: everyone).
    pub fn broadcast_op(&mut self, channel: Channel, kind: u8, bytes: Vec<u8>, except: Option<UserId>) {
        let except = except.and_then(|id| self.peer_of(id));
        self.broadcast(channel, &HostMsg::Op(kind, bytes), except);
    }

    /// Sends an operation of `kind` to user `to` alone.
    pub fn send_op(&mut self, to: UserId, channel: Channel, kind: u8, bytes: Vec<u8>) {
        if let Some(peer) = self.peer_of(to) {
            self.send(peer, channel, &HostMsg::Op(kind, bytes));
        }
    }

    /// Ends the session: tells everyone why and closes every connection. New connections are turned away.
    pub fn end(&mut self, reason: &str) {
        if self.ended {
            return;
        }
        self.ended = true;
        self.broadcast(Channel::Reliable, &HostMsg::Ended(reason.to_owned()), None);
        let peers: Vec<PeerId> = self.peers.keys().copied().collect();
        for peer in peers {
            if let Some(p) = self.peers.get_mut(&peer) {
                p.state = State::Closing;
            }
            self.closing.push(peer);
        }
    }
}
