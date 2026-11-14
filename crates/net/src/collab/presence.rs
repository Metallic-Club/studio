//! Presence (ADR 0011 §3): where each user is looking and what they have selected or open, sent latest-only and
//! throttled, never saved.

use super::scene::NetId;
use crate::codec::{CodecError, Reader, Writer};
use crate::identity::UserId;

/// How often a user's presence is sent at most, in hertz.
pub const RATE_HZ: f64 = 10.0;
/// The most selected instances presence carries (more are shown as these).
pub const MAX_SELECTION: usize = 256;
/// The most open scripts presence carries.
pub const MAX_OPEN: usize = 64;

/// The message kinds of presence.
pub mod kind {
    /// A participant's own presence, to the host.
    pub const MINE: u8 = 20;
    /// A user's presence, from the host to the others.
    pub const OF: u8 = 21;
    /// Everyone's presence, from the host: one message, so a latest-only channel never drops one user's behind
    /// another's.
    pub const ALL: u8 = 22;
}

/// A user's presence.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Presence {
    /// The editor camera: its position and rotation, as [`metallic_core::CFrame::to_array`] gives them.
    pub camera: Option<[f32; 7]>,
    /// What they have selected.
    pub selection: Vec<NetId>,
    /// The scripts whose tabs they have open.
    pub open: Vec<NetId>,
    /// The script they are in and their cursor there, as the text sync places it (opaque here).
    pub cursor: Option<(NetId, Vec<u8>)>,
}

impl Presence {
    /// Writes it.
    pub fn write(&self, w: &mut Writer) {
        w.option(self.camera.as_ref(), |w, parts| {
            for part in parts {
                w.f32(*part);
            }
        });
        let selection = &self.selection[..self.selection.len().min(MAX_SELECTION)];
        w.len_of(selection.len());
        for id in selection {
            w.u64(id.0);
        }
        let open = &self.open[..self.open.len().min(MAX_OPEN)];
        w.len_of(open.len());
        for id in open {
            w.u64(id.0);
        }
        w.option(self.cursor.as_ref(), |w, (script, bytes)| {
            w.u64(script.0);
            w.bytes(bytes);
        });
    }

    /// Reads one written by [`Presence::write`].
    pub fn read(r: &mut Reader<'_>) -> Result<Self, CodecError> {
        let camera = r.option(|r| {
            let mut parts = [0.0f32; 7];
            for part in &mut parts {
                *part = r.f32()?;
                if !part.is_finite() {
                    return Err(CodecError::Invalid("a camera that is not finite"));
                }
            }
            Ok(parts)
        })?;
        let count = r.count("a selection", MAX_SELECTION)?;
        let selection = (0..count).map(|_| r.u64().map(NetId)).collect::<Result<_, _>>()?;
        let count = r.count("open scripts", MAX_OPEN)?;
        let open = (0..count).map(|_| r.u64().map(NetId)).collect::<Result<_, _>>()?;
        let cursor = r.option(|r| Ok((NetId(r.u64()?), r.bytes("a cursor", 256)?.to_vec())))?;
        Ok(Self {
            camera,
            selection,
            open,
            cursor,
        })
    }

    /// The host's message carrying `user`'s presence.
    pub fn of(&self, user: UserId) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(user.0);
        self.write(&mut w);
        w.finish()
    }

    /// Reads a message of [`kind::OF`]: whose presence, and it.
    pub fn read_of(bytes: &[u8]) -> Result<(UserId, Self), CodecError> {
        let mut r = Reader::new(bytes);
        let user = UserId(r.u32()?);
        let presence = Self::read(&mut r)?;
        r.finish()?;
        Ok((user, presence))
    }

    /// Reads a message of [`kind::MINE`].
    pub fn read_mine(bytes: &[u8]) -> Result<Self, CodecError> {
        let mut r = Reader::new(bytes);
        let presence = Self::read(&mut r)?;
        r.finish()?;
        Ok(presence)
    }

    /// The message of [`kind::MINE`] carrying it.
    pub fn mine(&self) -> Vec<u8> {
        let mut w = Writer::new();
        self.write(&mut w);
        w.finish()
    }
}

/// The message of [`kind::ALL`] carrying every user's presence in `all`.
pub fn encode_all(all: &[(UserId, &Presence)]) -> Vec<u8> {
    let mut w = Writer::new();
    w.len_of(all.len());
    for (user, presence) in all {
        w.u32(user.0);
        presence.write(&mut w);
    }
    w.finish()
}

/// Reads a message of [`kind::ALL`].
pub fn decode_all(bytes: &[u8]) -> Result<Vec<(UserId, Presence)>, CodecError> {
    let mut r = Reader::new(bytes);
    let count = r.count("presences", 1024)?;
    let all = (0..count)
        .map(|_| Ok((UserId(r.u32()?), Presence::read(&mut r)?)))
        .collect::<Result<_, CodecError>>()?;
    r.finish()?;
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_round_trips_and_is_capped() {
        let presence = Presence {
            camera: Some([1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0]),
            selection: (0..1000).map(NetId).collect(),
            open: vec![NetId(9)],
            cursor: Some((NetId(9), vec![1, 2, 3])),
        };
        let (user, back) = Presence::read_of(&presence.of(UserId(4))).unwrap();
        assert_eq!(user, UserId(4));
        assert_eq!(back.selection.len(), MAX_SELECTION);
        assert_eq!(back.open, presence.open);
        assert_eq!(back.cursor, presence.cursor);
        assert_eq!(back.camera, presence.camera);
    }
}
