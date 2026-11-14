//! A participant's side of a session, apart from any transport: the join, the project as it arrives, and the
//! operations held until the participant's copy is open.

use std::collections::VecDeque;
use std::sync::Arc;

use super::protocol::{
    ClientMsg, HostMsg, MAX_MEMBERS, MAX_PROJECT_BYTES, PROTOCOL_VERSION, ProjectId, Purpose, SessionInfo,
};
use crate::identity::{Identity, User, UserId};
use crate::transport::Channel;

/// A project as a participant received it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReceivedProject {
    /// Each member's path and contents, in the order they came.
    pub members: Vec<(String, Arc<[u8]>)>,
    /// The start scene's member.
    pub start_scene: String,
}

/// Something the app has to act on.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientEvent {
    /// The host let them in.
    Welcomed {
        /// Their id.
        you: UserId,
        /// The session.
        session: SessionInfo,
    },
    /// The project is arriving: bytes received of the total.
    Progress {
        /// Received so far.
        received: u64,
        /// In all.
        total: u64,
    },
    /// The whole project arrived. Open it, then call [`Client::loaded`]: until then operations are held.
    Project(ReceivedProject),
    /// The host turned them away, and why.
    Refused(String),
    /// The user list changed.
    Users,
    /// The host ended the session, and why.
    Ended(String),
    /// An operation from the host (scene, text or presence), in order, once the copy is open.
    Op {
        /// The channel it came on.
        channel: Channel,
        /// Its kind.
        kind: u8,
        /// Its bytes.
        bytes: Vec<u8>,
    },
    /// The host broke the protocol; the session is over.
    Broken(String),
}

#[derive(Debug, PartialEq)]
enum Stage {
    /// Hello sent, waiting for the welcome.
    Hello,
    /// Welcomed; the project is expected.
    Welcomed,
    /// The project is arriving.
    Receiving,
    /// The project arrived; operations are held until it is open.
    Loading,
    /// Open: operations are delivered.
    Live,
    /// Over.
    Done,
}

/// The participant's session state.
#[derive(Debug)]
pub struct Client {
    stage: Stage,
    me: Option<UserId>,
    name: String,
    session: Option<SessionInfo>,
    users: Vec<User>,
    ready: Vec<UserId>,
    // The project as it arrives.
    expected_members: u32,
    expected_bytes: u64,
    received_bytes: u64,
    project: ReceivedProject,
    member: Option<(String, u64, Vec<u8>)>,
    held: VecDeque<ClientEvent>,
    events: VecDeque<ClientEvent>,
    outbox: Vec<(Channel, Vec<u8>)>,
}

impl Client {
    /// A participant joining as `identity`, expecting `project` if they joined it before. The hello is queued at
    /// once: send it when the connection is up.
    pub fn join(identity: Identity, project: Option<ProjectId>, engine: &str, fingerprint: u64) -> Self {
        let mut client = Self {
            stage: Stage::Hello,
            me: None,
            name: identity.name.clone(),
            session: None,
            users: Vec::new(),
            ready: Vec::new(),
            expected_members: 0,
            expected_bytes: 0,
            received_bytes: 0,
            project: ReceivedProject::default(),
            member: None,
            held: VecDeque::new(),
            events: VecDeque::new(),
            outbox: Vec::new(),
        };
        client.send(&ClientMsg::Hello {
            protocol: PROTOCOL_VERSION,
            engine: engine.to_owned(),
            fingerprint,
            purpose: Purpose::Join { identity, project },
        });
        client
    }

    /// Their id, once welcomed.
    pub fn me(&self) -> Option<UserId> {
        self.me
    }

    /// Their name as the session has it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The session, once welcomed.
    pub fn session(&self) -> Option<&SessionInfo> {
        self.session.as_ref()
    }

    /// Everyone in the session, by id.
    pub fn users(&self) -> &[User] {
        &self.users
    }

    /// Whether user `id` has the project open.
    pub fn is_ready(&self, id: UserId) -> bool {
        self.ready.contains(&id)
    }

    /// Whether the project is open and operations flow.
    pub fn is_live(&self) -> bool {
        self.stage == Stage::Live
    }

    /// Whether the session is over for them (refused, ended, broken or left).
    pub fn is_done(&self) -> bool {
        self.stage == Stage::Done
    }

    /// The next thing to act on.
    pub fn next_event(&mut self) -> Option<ClientEvent> {
        self.events.pop_front()
    }

    /// Takes what is to be sent.
    pub fn take_outbox(&mut self) -> Vec<(Channel, Vec<u8>)> {
        std::mem::take(&mut self.outbox)
    }

    fn send(&mut self, msg: &ClientMsg) {
        self.outbox.push((Channel::Reliable, msg.encode()));
    }

    /// Queues an operation of `kind` for the host.
    pub fn send_op(&mut self, channel: Channel, kind: u8, bytes: Vec<u8>) {
        if self.stage == Stage::Live {
            self.outbox.push((channel, ClientMsg::Op(kind, bytes).encode()));
        }
    }

    /// Their copy of the project is open: the operations held since the snapshot are delivered, in order, and the
    /// host is told they are ready.
    pub fn loaded(&mut self) {
        if self.stage != Stage::Loading {
            return;
        }
        self.stage = Stage::Live;
        self.events.extend(self.held.drain(..));
        if let Some(me) = self.me
            && !self.ready.contains(&me)
        {
            self.ready.push(me);
        }
        self.send(&ClientMsg::Ready);
    }

    /// Leaves the session.
    pub fn leave(&mut self) {
        if self.stage != Stage::Done {
            self.send(&ClientMsg::Leave);
            self.stage = Stage::Done;
        }
    }

    /// The connection was lost.
    pub fn disconnected(&mut self, reason: &str) {
        if self.stage != Stage::Done {
            self.stage = Stage::Done;
            self.events.push_back(ClientEvent::Broken(reason.to_owned()));
        }
    }

    fn broken(&mut self, why: String) {
        self.stage = Stage::Done;
        self.events.push_back(ClientEvent::Broken(why));
    }

    /// The transport received `bytes` from the host on `channel`.
    pub fn received(&mut self, channel: Channel, bytes: &[u8]) {
        if self.stage == Stage::Done {
            return;
        }
        let msg = match HostMsg::decode(bytes) {
            Ok(msg) => msg,
            Err(error) => {
                self.broken(format!("the host sent a message that is not valid ({error})"));
                return;
            }
        };
        match (msg, &self.stage) {
            (HostMsg::Refused(reason), _) => {
                self.stage = Stage::Done;
                self.events.push_back(ClientEvent::Refused(reason));
            }
            (HostMsg::Ended(reason), _) => {
                self.stage = Stage::Done;
                self.events.push_back(ClientEvent::Ended(reason));
            }
            (
                HostMsg::Welcome {
                    you,
                    name,
                    session,
                    users,
                },
                Stage::Hello,
            ) => {
                self.me = Some(you);
                self.name = name;
                self.ready = users.iter().filter(|u| u.host).map(|u| u.id).collect();
                self.users = users;
                self.session = Some(session.clone());
                self.stage = Stage::Welcomed;
                self.events.push_back(ClientEvent::Welcomed { you, session });
                self.events.push_back(ClientEvent::Users);
            }
            (
                HostMsg::ProjectBegin {
                    members,
                    bytes,
                    start_scene,
                },
                Stage::Welcomed,
            ) => {
                if members as usize > MAX_MEMBERS || bytes > MAX_PROJECT_BYTES {
                    self.broken("the project is too large".to_owned());
                    return;
                }
                self.expected_members = members;
                self.expected_bytes = bytes;
                self.project.start_scene = start_scene;
                self.stage = Stage::Receiving;
            }
            (HostMsg::Member { path, len }, Stage::Receiving) => {
                if self.member.is_some() {
                    self.broken("a member began before the last one ended".to_owned());
                    return;
                }
                if self.project.members.len() >= self.expected_members as usize
                    || self.received_bytes + len > self.expected_bytes
                {
                    self.broken("the project is larger than announced".to_owned());
                    return;
                }
                if self.project.members.iter().any(|(p, _)| *p == path) {
                    self.broken(format!("the member {path} came twice"));
                    return;
                }
                // Capacity only up to what arrives: `len` is the host's word.
                self.member = Some((path, len, Vec::new()));
                self.finish_member();
            }
            (HostMsg::Chunk(piece), Stage::Receiving) => {
                let Some((_, len, data)) = self.member.as_mut() else {
                    self.broken("a chunk outside a member".to_owned());
                    return;
                };
                if data.len() as u64 + piece.len() as u64 > *len {
                    self.broken("a member is longer than announced".to_owned());
                    return;
                }
                data.extend_from_slice(&piece);
                self.received_bytes += piece.len() as u64;
                self.events.push_back(ClientEvent::Progress {
                    received: self.received_bytes,
                    total: self.expected_bytes,
                });
                self.finish_member();
            }
            (HostMsg::ProjectEnd, Stage::Receiving) => {
                if self.member.is_some() || self.project.members.len() != self.expected_members as usize {
                    self.broken("the project ended before all of it came".to_owned());
                    return;
                }
                if !self.project.members.iter().any(|(p, _)| *p == self.project.start_scene) {
                    self.broken("the project has no start scene".to_owned());
                    return;
                }
                self.stage = Stage::Loading;
                self.events
                    .push_back(ClientEvent::Project(std::mem::take(&mut self.project)));
            }
            (HostMsg::UserJoined(user), Stage::Receiving | Stage::Loading | Stage::Live) => {
                self.users.retain(|u| u.id != user.id);
                self.users.push(user);
                self.users.sort_by_key(|u| u.id);
                self.events.push_back(ClientEvent::Users);
            }
            (HostMsg::UserReady(id), Stage::Receiving | Stage::Loading | Stage::Live) => {
                if !self.ready.contains(&id) {
                    self.ready.push(id);
                }
                self.events.push_back(ClientEvent::Users);
            }
            (HostMsg::UserLeft { id, .. }, Stage::Receiving | Stage::Loading | Stage::Live) => {
                self.users.retain(|u| u.id != id);
                self.ready.retain(|&r| r != id);
                self.events.push_back(ClientEvent::Users);
            }
            (HostMsg::Op(kind, bytes), Stage::Loading | Stage::Receiving) => {
                // Live edits made after the snapshot: kept until the copy is open.
                self.held.push_back(ClientEvent::Op { channel, kind, bytes });
            }
            (HostMsg::Op(kind, bytes), Stage::Live) => {
                self.events.push_back(ClientEvent::Op { channel, kind, bytes });
            }
            (msg, stage) => {
                let why = format!("the host sent {} while {stage:?}", describe(&msg));
                self.broken(why);
            }
        }
    }

    fn finish_member(&mut self) {
        if let Some((_, len, data)) = &self.member
            && data.len() as u64 == *len
            && let Some((path, _, data)) = self.member.take()
        {
            self.project.members.push((path, Arc::from(data)));
        }
    }
}

fn describe(msg: &HostMsg) -> &'static str {
    match msg {
        HostMsg::Welcome { .. } => "a welcome",
        HostMsg::Refused(_) => "a refusal",
        HostMsg::ProbeInfo { .. } => "a probe answer",
        HostMsg::ProjectBegin { .. } => "a project",
        HostMsg::Member { .. } => "a member",
        HostMsg::Chunk(_) => "a chunk",
        HostMsg::ProjectEnd => "a project's end",
        HostMsg::UserJoined(_) => "a user",
        HostMsg::UserReady(_) => "a user ready",
        HostMsg::UserLeft { .. } => "a user leaving",
        HostMsg::Ended(_) => "an end",
        HostMsg::Op(..) => "an operation",
    }
}
