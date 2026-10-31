//! Reliable messages (spec 3.3) and the join snapshot body.

use std::fmt;
use std::io::Read;

use serde::{Deserialize, Serialize};

use super::netid::NetId;
use super::value::NetValue;
use super::{WireError, postcard_exact};

/// Protocol minor version: peers with the same ALPN major and any minor interoperate; a newer minor only adds
/// message kinds, which older peers skip.
pub const PROTOCOL_MINOR: u16 = 0;

/// Largest uncompressed join snapshot body a client inflates, in bytes; a larger one is refused before it is read.
pub const MAX_JOIN_SNAPSHOT: usize = 16 << 20;

/// zstd level for join snapshots.
const JOIN_ZSTD_LEVEL: i32 = 3;
/// Largest zstd window (log2 bytes) a client accepts, so the decoder's own buffer stays within the cap.
const JOIN_WINDOW_LOG_MAX: u32 = 24;

/// One replicated instance as sent by `Create` and in the join snapshot. `class` and property indices are the
/// replication registry's numbers (task A2); this layer does not look them up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Create {
    /// The new instance's id.
    pub net: NetId,
    /// Class index in the replication registry.
    pub class: u16,
    /// Parent, or `None` for a root.
    pub parent: Option<NetId>,
    /// Replicated properties that differ from the class default, by property index.
    pub props: Vec<(u16, NetValue)>,
}

/// A reliable message from a client. The frame kind of each variant is in its doc. `Debug` redacts the ticket.
#[derive(Clone, PartialEq)]
pub enum ClientMsg {
    /// Kind 1: the first message on a connection.
    Hello {
        /// The join ticket (a bearer secret: never log it).
        ticket: String,
        /// The client's [`PROTOCOL_MINOR`].
        protocol_minor: u16,
        /// The client engine version, for logs and the update prompt.
        engine_version: String,
        /// The client's replication registry hash (`registry_hash()`, task A2).
        registry_hash: [u8; 32],
    },
    /// Kind 2: `RemoteEvent:FireServer`.
    FireServer {
        /// The RemoteEvent.
        remote: NetId,
        /// The arguments.
        args: Vec<NetValue>,
    },
    /// Kind 3: `RemoteFunction:InvokeServer`.
    InvokeServer {
        /// Call id, echoed by [`ServerMsg::InvokeServerReply`].
        call: u32,
        /// The RemoteFunction.
        remote: NetId,
        /// The arguments.
        args: Vec<NetValue>,
    },
    /// Kind 4: the answer to a [`ServerMsg::InvokeClient`].
    InvokeClientReply {
        /// The call id from the invoke.
        call: u32,
        /// The returned values, or the error text.
        result: Result<Vec<NetValue>, String>,
    },
    /// Kind 5: the client is leaving.
    Bye,
}

impl fmt::Debug for ClientMsg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientMsg::Hello {
                ticket: _,
                protocol_minor,
                engine_version,
                registry_hash,
            } => f
                .debug_struct("Hello")
                .field("ticket", &"<redacted>")
                .field("protocol_minor", protocol_minor)
                .field("engine_version", engine_version)
                .field("registry_hash", registry_hash)
                .finish(),
            ClientMsg::FireServer { remote, args } => f
                .debug_struct("FireServer")
                .field("remote", remote)
                .field("args", args)
                .finish(),
            ClientMsg::InvokeServer { call, remote, args } => f
                .debug_struct("InvokeServer")
                .field("call", call)
                .field("remote", remote)
                .field("args", args)
                .finish(),
            ClientMsg::InvokeClientReply { call, result } => f
                .debug_struct("InvokeClientReply")
                .field("call", call)
                .field("result", result)
                .finish(),
            ClientMsg::Bye => f.write_str("Bye"),
        }
    }
}

/// A reliable message from the server. The frame kind of each variant is in its doc.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMsg {
    /// Kind 1: the Hello was accepted.
    Welcome {
        /// The joining player's Player instance.
        player: NetId,
        /// Server tick the event stream continues from.
        tick: u64,
        /// The server's [`PROTOCOL_MINOR`].
        protocol_minor: u16,
    },
    /// Kind 2: the client's registry differs; it must update.
    UpdateRequired {
        /// The server's registry hash.
        registry_hash: [u8; 32],
    },
    /// Kind 3: the server closes the session.
    Kick {
        /// Shown to the player.
        reason: String,
    },
    /// Kind 4: the whole replicated tree at `tick`: zstd of the postcard `Vec<Create>` (see [`compress_join`] and
    /// [`decompress_join`]).
    JoinSnapshot {
        /// Tick boundary the snapshot was taken at.
        tick: u64,
        /// The compressed body.
        zstd: Vec<u8>,
    },
    /// Kind 5: an instance became visible to the client.
    Create(Create),
    /// Kind 6: an instance was destroyed or left the client's view.
    Destroy {
        /// The instance.
        net: NetId,
    },
    /// Kind 7: an instance moved to a new parent.
    Reparent {
        /// The instance.
        net: NetId,
        /// The new parent, or `None` for a root.
        parent: Option<NetId>,
    },
    /// Kind 8: replicated properties changed.
    Set {
        /// The instance.
        net: NetId,
        /// New values by property index.
        props: Vec<(u16, NetValue)>,
    },
    /// Kind 9: `RemoteEvent:FireClient`.
    FireClient {
        /// The RemoteEvent.
        remote: NetId,
        /// The arguments.
        args: Vec<NetValue>,
    },
    /// Kind 10: `RemoteFunction:InvokeClient`.
    InvokeClient {
        /// Call id, echoed by [`ClientMsg::InvokeClientReply`].
        call: u32,
        /// The RemoteFunction.
        remote: NetId,
        /// The arguments.
        args: Vec<NetValue>,
    },
    /// Kind 11: the answer to a [`ClientMsg::InvokeServer`].
    InvokeServerReply {
        /// The call id from the invoke.
        call: u32,
        /// The returned values, or the error text.
        result: Result<Vec<NetValue>, String>,
    },
}

fn body<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, WireError> {
    Ok(postcard::to_allocvec(value)?)
}

fn args_too_deep(args: &[NetValue]) -> bool {
    args.iter().any(NetValue::too_deep)
}

fn props_too_deep(props: &[(u16, NetValue)]) -> bool {
    props.iter().any(|(_, v)| v.too_deep())
}

fn result_too_deep(result: &Result<Vec<NetValue>, String>) -> bool {
    result.as_ref().is_ok_and(|v| args_too_deep(v))
}

fn check_depth(too_deep: bool) -> Result<(), WireError> {
    if too_deep {
        Err(WireError::BadValue("tables nested too deeply"))
    } else {
        Ok(())
    }
}

impl ClientMsg {
    /// The frame kind and postcard body.
    pub(crate) fn to_body(&self) -> Result<(u8, Vec<u8>), WireError> {
        Ok(match self {
            ClientMsg::Hello {
                ticket,
                protocol_minor,
                engine_version,
                registry_hash,
            } => (
                1,
                body(&(ticket, protocol_minor, engine_version, registry_hash))?,
            ),
            ClientMsg::FireServer { remote, args } => {
                check_depth(args_too_deep(args))?;
                (2, body(&(remote, args))?)
            }
            ClientMsg::InvokeServer { call, remote, args } => {
                check_depth(args_too_deep(args))?;
                (3, body(&(call, remote, args))?)
            }
            ClientMsg::InvokeClientReply { call, result } => {
                check_depth(result_too_deep(result))?;
                (4, body(&(call, result))?)
            }
            ClientMsg::Bye => (5, Vec::new()),
        })
    }

    /// The message of frame kind `kind` from its body; `Ok(None)` for a kind this build does not know.
    pub(crate) fn from_body(kind: u8, bytes: &[u8]) -> Result<Option<Self>, WireError> {
        Ok(Some(match kind {
            1 => {
                let (ticket, protocol_minor, engine_version, registry_hash) =
                    postcard_exact(bytes)?;
                ClientMsg::Hello {
                    ticket,
                    protocol_minor,
                    engine_version,
                    registry_hash,
                }
            }
            2 => {
                let (remote, args) = postcard_exact(bytes)?;
                ClientMsg::FireServer { remote, args }
            }
            3 => {
                let (call, remote, args) = postcard_exact(bytes)?;
                ClientMsg::InvokeServer { call, remote, args }
            }
            4 => {
                let (call, result) = postcard_exact(bytes)?;
                ClientMsg::InvokeClientReply { call, result }
            }
            5 => {
                postcard_exact::<()>(bytes)?;
                ClientMsg::Bye
            }
            _ => return Ok(None),
        }))
    }
}

impl ServerMsg {
    /// The frame kind and postcard body.
    pub(crate) fn to_body(&self) -> Result<(u8, Vec<u8>), WireError> {
        Ok(match self {
            ServerMsg::Welcome {
                player,
                tick,
                protocol_minor,
            } => (1, body(&(player, tick, protocol_minor))?),
            ServerMsg::UpdateRequired { registry_hash } => (2, body(registry_hash)?),
            ServerMsg::Kick { reason } => (3, body(reason)?),
            ServerMsg::JoinSnapshot { tick, zstd } => (4, body(&(tick, zstd))?),
            ServerMsg::Create(create) => {
                check_depth(props_too_deep(&create.props))?;
                (5, body(create)?)
            }
            ServerMsg::Destroy { net } => (6, body(net)?),
            ServerMsg::Reparent { net, parent } => (7, body(&(net, parent))?),
            ServerMsg::Set { net, props } => {
                check_depth(props_too_deep(props))?;
                (8, body(&(net, props))?)
            }
            ServerMsg::FireClient { remote, args } => {
                check_depth(args_too_deep(args))?;
                (9, body(&(remote, args))?)
            }
            ServerMsg::InvokeClient { call, remote, args } => {
                check_depth(args_too_deep(args))?;
                (10, body(&(call, remote, args))?)
            }
            ServerMsg::InvokeServerReply { call, result } => {
                check_depth(result_too_deep(result))?;
                (11, body(&(call, result))?)
            }
        })
    }

    /// The message of frame kind `kind` from its body; `Ok(None)` for a kind this build does not know.
    pub(crate) fn from_body(kind: u8, bytes: &[u8]) -> Result<Option<Self>, WireError> {
        Ok(Some(match kind {
            1 => {
                let (player, tick, protocol_minor) = postcard_exact(bytes)?;
                ServerMsg::Welcome {
                    player,
                    tick,
                    protocol_minor,
                }
            }
            2 => ServerMsg::UpdateRequired {
                registry_hash: postcard_exact(bytes)?,
            },
            3 => ServerMsg::Kick {
                reason: postcard_exact(bytes)?,
            },
            4 => {
                let (tick, zstd) = postcard_exact(bytes)?;
                ServerMsg::JoinSnapshot { tick, zstd }
            }
            5 => ServerMsg::Create(postcard_exact(bytes)?),
            6 => ServerMsg::Destroy {
                net: postcard_exact(bytes)?,
            },
            7 => {
                let (net, parent) = postcard_exact(bytes)?;
                ServerMsg::Reparent { net, parent }
            }
            8 => {
                let (net, props) = postcard_exact(bytes)?;
                ServerMsg::Set { net, props }
            }
            9 => {
                let (remote, args) = postcard_exact(bytes)?;
                ServerMsg::FireClient { remote, args }
            }
            10 => {
                let (call, remote, args) = postcard_exact(bytes)?;
                ServerMsg::InvokeClient { call, remote, args }
            }
            11 => {
                let (call, result) = postcard_exact(bytes)?;
                ServerMsg::InvokeServerReply { call, result }
            }
            _ => return Ok(None),
        }))
    }
}

/// The `zstd` field of a [`ServerMsg::JoinSnapshot`]: postcard `Vec<Create>`, zstd level 3.
///
/// Errors: [`WireError::TooLarge`] when the uncompressed body is over [`MAX_JOIN_SNAPSHOT`] (the client would refuse
/// it), [`WireError::BadValue`] for tables nested too deeply.
pub fn compress_join(creates: &[Create]) -> Result<Vec<u8>, WireError> {
    check_depth(creates.iter().any(|c| props_too_deep(&c.props)))?;
    let raw = body(creates)?;
    if raw.len() > MAX_JOIN_SNAPSHOT {
        return Err(WireError::TooLarge);
    }
    zstd::encode_all(raw.as_slice(), JOIN_ZSTD_LEVEL)
        .map_err(|_| WireError::BadValue("zstd compression failed"))
}

/// The instances in a join snapshot's `zstd` field. Inflates at most [`MAX_JOIN_SNAPSHOT`] bytes (and refuses zstd
/// windows over 16 MiB), so a small hostile body cannot expand without bound.
///
/// Errors: [`WireError::TooLarge`] over the cap, [`WireError::BadValue`] for bytes that are not zstd, postcard errors
/// for a body that does not decode completely.
pub fn decompress_join(zstd: &[u8]) -> Result<Vec<Create>, WireError> {
    let bad = |_| WireError::BadValue("join snapshot is not valid zstd");
    let mut decoder = zstd::stream::read::Decoder::with_buffer(zstd).map_err(bad)?;
    decoder.window_log_max(JOIN_WINDOW_LOG_MAX).map_err(bad)?;
    // Read in chunks and grow the buffer by hand: `read_to_end` doubles capacity and could reach 32 MiB at the cap.
    let mut raw = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let n = decoder.read(&mut chunk).map_err(bad)?;
        if n == 0 {
            break;
        }
        if raw.len() + n > MAX_JOIN_SNAPSHOT {
            return Err(WireError::TooLarge);
        }
        if raw.capacity() - raw.len() < n {
            let want = (raw.capacity() * 2).clamp(raw.len() + n, MAX_JOIN_SNAPSHOT);
            raw.reserve_exact(want - raw.len());
        }
        raw.extend_from_slice(chunk.get(..n).unwrap_or_default());
    }
    postcard_exact(&raw)
}
