//! Team Create's messages, as they travel.
//!
//! A participant sends [`ClientMsg`]s; the host sends [`HostMsg`]s. Each is one frame: a tag byte, then its fields
//! ([`crate::codec`]). Reading a message checks every length against the limits here, and the decoded message
//! against the protocol's rules, before anything acts on it.
//!
//! # Joining, in order
//!
//! 1. The participant connects and sends [`ClientMsg::Hello`]: the protocol and engine versions, the class
//!    registry's fingerprint, and who they are.
//! 2. The host answers [`HostMsg::Welcome`] (their user id, the session, everyone already in it) or
//!    [`HostMsg::Refused`] and closes.
//! 3. The project: [`HostMsg::ProjectBegin`], each member as [`HostMsg::Member`] then its [`HostMsg::Chunk`]s, and
//!    [`HostMsg::ProjectEnd`]. The start scene among them is the host's scene as it is at that moment.
//! 4. Everything after `ProjectEnd` is live: the host sends edits made after the snapshot, in order, on the same
//!    stream, so nothing between the two is missed. The participant keeps them until its copy is open, then applies
//!    them in order and says [`ClientMsg::Ready`]; the host then tells everyone the user is in.

use std::sync::Arc;

use crate::codec::{CodecError, Reader, Writer};
use crate::identity::{Identity, User, UserId};

/// The version of this protocol. Peers must have the same one.
pub const PROTOCOL_VERSION: u32 = 1;

/// The port Team Create hosts on unless the user picks another.
pub const DEFAULT_PORT: u16 = 7350;

/// The longest member path.
pub const MAX_PATH: usize = 512;
/// The most members a project may send.
pub const MAX_MEMBERS: usize = 20_000;
/// The largest member.
pub const MAX_MEMBER_BYTES: u64 = 256 << 20;
/// The largest project, all members together.
pub const MAX_PROJECT_BYTES: u64 = 1 << 30;
/// How much of a member one chunk carries.
pub const CHUNK: usize = 1 << 20;
/// The longest reason or other free text.
pub const MAX_TEXT: usize = 2048;
/// The most users one message lists.
pub const MAX_USERS: usize = 256;

/// A Team Create project's id: chosen once when a project is first hosted and kept with it, so a participant knows
/// the address they saved still hosts the same project.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProjectId(pub u128);

impl ProjectId {
    /// A new random id. Falls back to the time if the system has no randomness to give.
    pub fn random() -> Self {
        let mut bytes = [0u8; 16];
        if getrandom::fill(&mut bytes).is_err() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default();
            return Self(now ^ 0x9e37_79b9_7f4a_7c15_f39c_c060_5ced_c834);
        }
        Self(u128::from_le_bytes(bytes))
    }

    /// The id as 32 hex digits.
    pub fn to_hex(self) -> String {
        format!("{:032x}", self.0)
    }

    /// The id from [`ProjectId::to_hex`]'s text.
    pub fn from_hex(text: &str) -> Option<Self> {
        (text.len() == 32)
            .then(|| u128::from_str_radix(text, 16).ok().map(Self))
            .flatten()
    }

    fn write(self, w: &mut Writer) {
        w.u64(self.0 as u64);
        w.u64((self.0 >> 64) as u64);
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, CodecError> {
        let low = u128::from(r.u64()?);
        let high = u128::from(r.u64()?);
        Ok(Self(low | (high << 64)))
    }
}

/// What everyone knows about a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    /// A number this hosting run picked: a session that ends and starts again has a new one.
    pub session: u64,
    /// The project hosted.
    pub project: ProjectId,
    /// The project's name.
    pub project_name: String,
    /// The host's display name.
    pub host_name: String,
}

impl SessionInfo {
    fn write(&self, w: &mut Writer) {
        w.u64(self.session);
        self.project.write(w);
        w.str(&self.project_name);
        w.str(&self.host_name);
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            session: r.u64()?,
            project: ProjectId::read(r)?,
            project_name: r.str("a project name", MAX_TEXT)?.to_owned(),
            host_name: r.str("a host name", MAX_TEXT)?.to_owned(),
        })
    }
}

/// Why a participant connects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// To join, as `identity`, expecting `project` when they saved one (a Team Create entry).
    Join {
        /// Who they are.
        identity: Identity,
        /// The project they expect, if they joined it before.
        project: Option<ProjectId>,
    },
    /// Only to ask what is hosted (the projects page's reachability check). The host answers and closes.
    Probe,
}

/// A message from a participant to the host.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientMsg {
    /// The first message.
    Hello {
        /// [`PROTOCOL_VERSION`].
        protocol: u32,
        /// The engine version.
        engine: String,
        /// The class registry's fingerprint ([`crate::registry_fingerprint`]).
        fingerprint: u64,
        /// Why they connect.
        purpose: Purpose,
    },
    /// Their copy of the project is open and the edits kept since the snapshot are applied.
    Ready,
    /// They leave the session.
    Leave,
    /// Scene, text and presence messages of the later stages, carried as their own bytes with their own tag
    /// (`crate::collab` reads them).
    Op(u8, Vec<u8>),
}

/// A message from the host to a participant.
#[derive(Clone, Debug, PartialEq)]
pub enum HostMsg {
    /// They are in: their id, the session and its users (them included).
    Welcome {
        /// Their id.
        you: UserId,
        /// Their name as the session has it (made unique).
        name: String,
        /// The session.
        session: SessionInfo,
        /// Everyone in it.
        users: Vec<User>,
    },
    /// They cannot join, and why. The host closes the connection after it.
    Refused(String),
    /// The answer to a [`Purpose::Probe`].
    ProbeInfo {
        /// The session.
        session: SessionInfo,
        /// How many users are in it.
        users: u32,
    },
    /// The project's members follow: how many, and their total size.
    ProjectBegin {
        /// How many members.
        members: u32,
        /// Their total size in bytes.
        bytes: u64,
        /// The member that is the start scene.
        start_scene: String,
    },
    /// A member follows in chunks.
    Member {
        /// Its path in the project.
        path: String,
        /// Its size in bytes.
        len: u64,
    },
    /// The next piece of the member under way.
    Chunk(Arc<[u8]>),
    /// Every member is sent; what follows is live.
    ProjectEnd,
    /// A user joined (and is loading), or finished loading.
    UserJoined(User),
    /// A user finished loading the project.
    UserReady(UserId),
    /// A user left, and why.
    UserLeft {
        /// Who.
        id: UserId,
        /// Why, in words.
        reason: String,
    },
    /// The host ended the session. The host closes every connection after it.
    Ended(String),
    /// A scene, text or presence message (see [`ClientMsg::Op`]).
    Op(u8, Vec<u8>),
}

mod tag {
    pub(super) const HELLO: u8 = 1;
    pub(super) const READY: u8 = 2;
    pub(super) const LEAVE: u8 = 3;
    pub(super) const OP: u8 = 9;

    pub(super) const WELCOME: u8 = 1;
    pub(super) const REFUSED: u8 = 2;
    pub(super) const PROBE_INFO: u8 = 3;
    pub(super) const PROJECT_BEGIN: u8 = 4;
    pub(super) const MEMBER: u8 = 5;
    pub(super) const CHUNK: u8 = 6;
    pub(super) const PROJECT_END: u8 = 7;
    pub(super) const USER_JOINED: u8 = 8;
    pub(super) const USER_READY: u8 = 10;
    pub(super) const USER_LEFT: u8 = 11;
    pub(super) const ENDED: u8 = 12;
    pub(super) const HOST_OP: u8 = 13;
}

/// Whether `path` is a member path a participant may write: relative, `/`-separated, no `.` or `..` parts, no
/// drive letters, backslashes or control characters, at most [`MAX_PATH`] bytes.
pub fn is_safe_member_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_PATH
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

impl ClientMsg {
    /// The message as one frame's payload.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::Hello {
                protocol,
                engine,
                fingerprint,
                purpose,
            } => {
                w.u8(tag::HELLO);
                w.u32(*protocol);
                w.str(engine);
                w.u64(*fingerprint);
                match purpose {
                    Purpose::Join { identity, project } => {
                        w.u8(0);
                        identity.write(&mut w);
                        w.option(*project, |w, p| p.write(w));
                    }
                    Purpose::Probe => w.u8(1),
                }
            }
            Self::Ready => w.u8(tag::READY),
            Self::Leave => w.u8(tag::LEAVE),
            Self::Op(kind, bytes) => {
                w.u8(tag::OP);
                w.u8(*kind);
                return [w.finish(), bytes.clone()].concat();
            }
        }
        w.finish()
    }

    /// Reads a frame's payload.
    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        let mut r = Reader::new(bytes);
        let msg = match r.u8()? {
            tag::HELLO => {
                let protocol = r.u32()?;
                let engine = r.str("an engine version", 64)?.to_owned();
                let fingerprint = r.u64()?;
                let purpose = match r.u8()? {
                    0 => Purpose::Join {
                        identity: Identity::read(&mut r)?,
                        project: r.option(ProjectId::read)?,
                    },
                    1 => Purpose::Probe,
                    tag => return Err(CodecError::BadTag { what: "a purpose", tag }),
                };
                Self::Hello {
                    protocol,
                    engine,
                    fingerprint,
                    purpose,
                }
            }
            tag::READY => Self::Ready,
            tag::LEAVE => Self::Leave,
            tag::OP => {
                let kind = r.u8()?;
                return Ok(Self::Op(kind, bytes[2..].to_vec()));
            }
            tag => return Err(CodecError::BadTag { what: "a message", tag }),
        };
        r.finish()?;
        Ok(msg)
    }
}

impl HostMsg {
    /// The message as one frame's payload.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::Welcome {
                you,
                name,
                session,
                users,
            } => {
                w.u8(tag::WELCOME);
                w.u32(you.0);
                w.str(name);
                session.write(&mut w);
                w.len_of(users.len());
                for user in users {
                    user.write(&mut w);
                }
            }
            Self::Refused(reason) => {
                w.u8(tag::REFUSED);
                w.str(reason);
            }
            Self::ProbeInfo { session, users } => {
                w.u8(tag::PROBE_INFO);
                session.write(&mut w);
                w.u32(*users);
            }
            Self::ProjectBegin {
                members,
                bytes,
                start_scene,
            } => {
                w.u8(tag::PROJECT_BEGIN);
                w.u32(*members);
                w.u64(*bytes);
                w.str(start_scene);
            }
            Self::Member { path, len } => {
                w.u8(tag::MEMBER);
                w.str(path);
                w.u64(*len);
            }
            Self::Chunk(bytes) => {
                w = Writer::with_capacity(bytes.len() + 8);
                w.u8(tag::CHUNK);
                w.bytes(bytes);
            }
            Self::ProjectEnd => w.u8(tag::PROJECT_END),
            Self::UserJoined(user) => {
                w.u8(tag::USER_JOINED);
                user.write(&mut w);
            }
            Self::UserReady(id) => {
                w.u8(tag::USER_READY);
                w.u32(id.0);
            }
            Self::UserLeft { id, reason } => {
                w.u8(tag::USER_LEFT);
                w.u32(id.0);
                w.str(reason);
            }
            Self::Ended(reason) => {
                w.u8(tag::ENDED);
                w.str(reason);
            }
            Self::Op(kind, bytes) => {
                w.u8(tag::HOST_OP);
                w.u8(*kind);
                return [w.finish(), bytes.clone()].concat();
            }
        }
        w.finish()
    }

    /// Reads a frame's payload.
    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        let mut r = Reader::new(bytes);
        let msg = match r.u8()? {
            tag::WELCOME => {
                let you = UserId(r.u32()?);
                let name = r.str("a name", MAX_TEXT)?.to_owned();
                let session = SessionInfo::read(&mut r)?;
                let count = r.count("users", MAX_USERS)?;
                let users = (0..count).map(|_| User::read(&mut r)).collect::<Result<_, _>>()?;
                Self::Welcome {
                    you,
                    name,
                    session,
                    users,
                }
            }
            tag::REFUSED => Self::Refused(r.str("a reason", MAX_TEXT)?.to_owned()),
            tag::PROBE_INFO => Self::ProbeInfo {
                session: SessionInfo::read(&mut r)?,
                users: r.u32()?,
            },
            tag::PROJECT_BEGIN => {
                let members = r.u32()?;
                let bytes = r.u64()?;
                if members as usize > MAX_MEMBERS {
                    return Err(CodecError::Invalid("a project with too many members"));
                }
                if bytes > MAX_PROJECT_BYTES {
                    return Err(CodecError::Invalid("a project too large"));
                }
                let start_scene = r.str("a path", MAX_PATH)?.to_owned();
                if !is_safe_member_path(&start_scene) {
                    return Err(CodecError::Invalid("an unsafe member path"));
                }
                Self::ProjectBegin {
                    members,
                    bytes,
                    start_scene,
                }
            }
            tag::MEMBER => {
                let path = r.str("a path", MAX_PATH)?.to_owned();
                if !is_safe_member_path(&path) {
                    return Err(CodecError::Invalid("an unsafe member path"));
                }
                let len = r.u64()?;
                if len > MAX_MEMBER_BYTES {
                    return Err(CodecError::Invalid("a member too large"));
                }
                Self::Member { path, len }
            }
            tag::CHUNK => Self::Chunk(Arc::from(r.bytes("a chunk", CHUNK)?)),
            tag::PROJECT_END => Self::ProjectEnd,
            tag::USER_JOINED => Self::UserJoined(User::read(&mut r)?),
            tag::USER_READY => Self::UserReady(UserId(r.u32()?)),
            tag::USER_LEFT => Self::UserLeft {
                id: UserId(r.u32()?),
                reason: r.str("a reason", MAX_TEXT)?.to_owned(),
            },
            tag::ENDED => Self::Ended(r.str("a reason", MAX_TEXT)?.to_owned()),
            tag::HOST_OP => {
                let kind = r.u8()?;
                return Ok(Self::Op(kind, bytes[2..].to_vec()));
            }
            tag => return Err(CodecError::BadTag { what: "a message", tag }),
        };
        r.finish()?;
        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> SessionInfo {
        SessionInfo {
            session: 99,
            project: ProjectId(u128::MAX - 5),
            project_name: "Castle".into(),
            host_name: "Ada".into(),
        }
    }

    #[test]
    fn messages_round_trip() {
        let client = [
            ClientMsg::Hello {
                protocol: PROTOCOL_VERSION,
                engine: "0.1.0".into(),
                fingerprint: 42,
                purpose: Purpose::Join {
                    identity: Identity::new("Bob", [1, 2, 3]),
                    project: Some(ProjectId(7)),
                },
            },
            ClientMsg::Hello {
                protocol: 1,
                engine: "x".into(),
                fingerprint: 0,
                purpose: Purpose::Probe,
            },
            ClientMsg::Ready,
            ClientMsg::Leave,
            ClientMsg::Op(4, vec![1, 2, 3]),
        ];
        for msg in client {
            assert_eq!(ClientMsg::decode(&msg.encode()).unwrap(), msg);
        }
        let host = [
            HostMsg::Welcome {
                you: UserId(2),
                name: "Bob".into(),
                session: session(),
                users: vec![User {
                    id: UserId::HOST,
                    identity: Identity::new("Ada", [9, 9, 9]),
                    host: true,
                }],
            },
            HostMsg::Refused("full".into()),
            HostMsg::ProbeInfo {
                session: session(),
                users: 3,
            },
            HostMsg::ProjectBegin {
                members: 2,
                bytes: 10,
                start_scene: "Scenes/Main.scene".into(),
            },
            HostMsg::Member {
                path: "Scripts/A.luau".into(),
                len: 4,
            },
            HostMsg::Chunk(Arc::from(&b"abcd"[..])),
            HostMsg::ProjectEnd,
            HostMsg::UserReady(UserId(3)),
            HostMsg::UserLeft {
                id: UserId(3),
                reason: "left".into(),
            },
            HostMsg::Ended("bye".into()),
            HostMsg::Op(1, vec![5]),
        ];
        for msg in host {
            assert_eq!(HostMsg::decode(&msg.encode()).unwrap(), msg);
        }
    }

    #[test]
    fn unsafe_paths_are_refused() {
        for path in [
            "",
            "/etc/passwd",
            "../x",
            "a/../b",
            "a//b",
            "C:/x",
            "a\\b",
            "./a",
            "a\u{0}b",
        ] {
            assert!(!is_safe_member_path(path), "{path:?}");
            let msg = HostMsg::Member {
                path: path.to_owned(),
                len: 1,
            };
            assert!(HostMsg::decode(&msg.encode()).is_err(), "{path:?}");
        }
        assert!(is_safe_member_path("Scripts/Door.luau"));
    }

    #[test]
    fn project_ids_print_and_parse() {
        let id = ProjectId::random();
        assert_eq!(ProjectId::from_hex(&id.to_hex()), Some(id));
        assert_eq!(ProjectId::from_hex("zz"), None);
    }
}
