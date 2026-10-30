//! Datagrams: a 3-byte header (`minor: 4 bits, kind: 4 bits`, then `seq: u16` little-endian), then the body.
//!
//! Snapshot and input bodies are bit-packed ([`BitWriter`]); an unreliable remote call is a varint NetId and the
//! postcard arguments. A datagram is at most [`MAX_DATAGRAM`] bytes; decoders refuse larger ones before reading,
//! and every decoder requires the whole datagram to be used (only the last byte's zero padding may be left).

use super::bits::{BitReader, BitWriter};
use super::msg::PROTOCOL_MINOR;
use super::netid::NetId;
use super::quant::{QuantPos, QuantY};
use super::value::NetValue;
use super::varint::{read_u64, write_u64};
use super::{WireError, postcard_exact};
use crate::transport::limits::MAX_DATAGRAM;

/// Most inputs one input packet carries (the 2 newest plus up to 6 unacknowledged ones, spec 4.7).
pub const MAX_INPUT_CMDS: usize = 8;
/// Largest postcard argument body of an unreliable remote call, in bytes (spec 4.9).
pub const MAX_UNRELIABLE_ARGS: usize = 1000;

const HEADER_LEN: usize = 3;

/// What a datagram carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DatagramKind {
    /// Kind 1: [`SnapshotPacket`], server to client.
    Snapshot = 1,
    /// Kind 2: [`InputPacket`], client to server.
    Input = 2,
    /// Kind 3: [`UnreliableRemotePacket`], either way.
    UnreliableRemote = 3,
}

/// One tick of the local player's input (spec 4.7), about 7 bytes on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputCmd {
    /// The client tick this input is for.
    pub tick: u32,
    /// Move direction X, -127..=127 is -1..=1 (the server checks the length).
    pub move_x: i8,
    /// Move direction Z, -127..=127 is -1..=1.
    pub move_z: i8,
    /// Jump held.
    pub jump: bool,
    /// Run held.
    pub run: bool,
    /// Camera yaw, a full turn in 65,536 steps.
    pub yaw: u16,
    /// Action bits (reserved, 0 in M2a).
    pub actions: u16,
}

/// An input datagram: acks for snapshots and 1 to 8 inputs for consecutive ticks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputPacket {
    /// Datagram sequence number.
    pub seq: u16,
    /// Newest snapshot sequence received.
    pub ack: u16,
    /// Bit `i` set: snapshot `ack - 1 - i` was received too.
    pub ack_bits: u32,
    /// The inputs, oldest first, for consecutive ticks.
    pub cmds: Vec<InputCmd>,
}

/// The local character's full-precision server state at `last_input_tick` (spec 4.7 step 4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OwnState {
    /// Root position.
    pub position: [f32; 3],
    /// Vertical speed.
    pub vertical: f32,
    /// Velocity inherited from what it stood on.
    pub inherited: [f32; 3],
    /// Humanoid state number.
    pub state: u8,
    /// The part it stands on.
    pub ground: Option<NetId>,
    /// Facing yaw, radians.
    pub yaw: f32,
    /// Seconds since death.
    pub dead_time: f32,
    /// Velocity.
    pub velocity: [f32; 3],
    /// Position relative to the ground part.
    pub relative: [f32; 3],
}

/// How an entity is rotated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityRot {
    /// A part: smallest-three orientation ([`quantize_orientation`](super::quantize_orientation)).
    Part(u32),
    /// A character: yaw (full turn in 65,536 steps) and Humanoid state.
    Character {
        /// Facing yaw.
        yaw: u16,
        /// Humanoid state number.
        state: u8,
    },
}

/// One moving thing in a snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityState {
    /// The instance.
    pub net: NetId,
    /// Quantized position.
    pub pos: QuantPos,
    /// Rotation.
    pub rot: EntityRot,
    /// Quantized velocity ([`quantize_velocity`](super::quantize_velocity)) while moving.
    pub vel: Option<[i16; 3]>,
    /// Came to rest: nothing more is sent until it moves.
    pub at_rest: bool,
}

/// A snapshot datagram, server to client.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotPacket {
    /// Datagram sequence number.
    pub seq: u16,
    /// Which server (0 when direct).
    pub source: u8,
    /// Server tick of the state.
    pub server_tick: u32,
    /// The newest client input tick the server has used.
    pub last_input_tick: u32,
    /// Input buffer hint: positive asks the client to slow down, negative to speed up.
    pub buffer_hint: i8,
    /// The receiving player's own character state, if it has one.
    pub own: Option<OwnState>,
    /// Moving entities, highest priority first.
    pub entities: Vec<EntityState>,
}

/// An `UnreliableRemoteEvent` call, either way.
#[derive(Debug, Clone, PartialEq)]
pub struct UnreliableRemotePacket {
    /// Datagram sequence number.
    pub seq: u16,
    /// The remote.
    pub remote: NetId,
    /// The arguments (postcard, at most [`MAX_UNRELIABLE_ARGS`] bytes).
    pub args: Vec<NetValue>,
}

fn header(kind: DatagramKind, seq: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    out.push((((PROTOCOL_MINOR & 0x0f) as u8) << 4) | kind as u8);
    out.extend_from_slice(&seq.to_le_bytes());
    out
}

/// The kind of a datagram, from its header. The minor version nibble is not checked: minors are compatible.
///
/// Errors: [`WireError::TooLarge`] over [`MAX_DATAGRAM`], [`WireError::Truncated`] under 3 bytes,
/// [`WireError::UnknownKind`] for another kind.
pub fn datagram_kind(bytes: &[u8]) -> Result<DatagramKind, WireError> {
    if bytes.len() > MAX_DATAGRAM {
        return Err(WireError::TooLarge);
    }
    if bytes.len() < HEADER_LEN {
        return Err(WireError::Truncated);
    }
    match bytes.first().map(|b| b & 0x0f) {
        Some(1) => Ok(DatagramKind::Snapshot),
        Some(2) => Ok(DatagramKind::Input),
        Some(3) => Ok(DatagramKind::UnreliableRemote),
        Some(other) => Err(WireError::UnknownKind(other)),
        None => Err(WireError::Truncated),
    }
}

/// Checks the size and kind; returns the sequence number and the body.
fn open(bytes: &[u8], want: DatagramKind) -> Result<(u16, &[u8]), WireError> {
    let kind = datagram_kind(bytes)?;
    if kind != want {
        return Err(WireError::BadValue("wrong datagram kind"));
    }
    match bytes {
        [_, lo, hi, body @ ..] => Ok((u16::from_le_bytes([*lo, *hi]), body)),
        _ => Err(WireError::Truncated),
    }
}

fn finish(out: Vec<u8>) -> Result<Vec<u8>, WireError> {
    if out.len() > MAX_DATAGRAM {
        Err(WireError::TooLarge)
    } else {
        Ok(out)
    }
}

fn write_signed(w: &mut BitWriter, v: i64, bits: u8) {
    w.write(v as u64, bits);
}

fn read_signed(r: &mut BitReader, bits: u8) -> Result<i64, WireError> {
    let raw = r.read(bits)? as i64;
    let sign = 1i64 << (bits - 1);
    Ok(if raw & sign != 0 {
        raw - (sign << 1)
    } else {
        raw
    })
}

fn fits_signed(v: i64, bits: u8) -> bool {
    let half = 1i64 << (bits - 1);
    (-half..half).contains(&v)
}

fn write_f32(w: &mut BitWriter, v: f32) -> Result<(), WireError> {
    if !v.is_finite() {
        return Err(WireError::BadValue("non-finite number"));
    }
    w.write(u64::from(v.to_bits()), 32);
    Ok(())
}

fn read_f32(r: &mut BitReader) -> Result<f32, WireError> {
    let v = f32::from_bits(r.read(32)? as u32);
    if v.is_finite() {
        Ok(v)
    } else {
        Err(WireError::BadValue("non-finite number"))
    }
}

fn write_f32s(w: &mut BitWriter, vs: &[f32]) -> Result<(), WireError> {
    vs.iter().try_for_each(|&v| write_f32(w, v))
}

fn read_f32x3(r: &mut BitReader) -> Result<[f32; 3], WireError> {
    Ok([read_f32(r)?, read_f32(r)?, read_f32(r)?])
}

// ---------- input ----------

const INPUT_COUNT_BITS: u8 = 3;

/// Encodes an input datagram.
///
/// Errors: [`WireError::BadValue`] unless there are 1 to 8 commands for consecutive ticks.
pub fn encode_input(p: &InputPacket) -> Result<Vec<u8>, WireError> {
    let first = p
        .cmds
        .first()
        .ok_or(WireError::BadValue("input packet without inputs"))?;
    if p.cmds.len() > MAX_INPUT_CMDS {
        return Err(WireError::BadValue("more than 8 inputs in one packet"));
    }
    for (i, c) in p.cmds.iter().enumerate() {
        if first.tick.checked_add(i as u32) != Some(c.tick) {
            return Err(WireError::BadValue("input ticks are not consecutive"));
        }
    }
    let mut w = BitWriter::new();
    w.write(u64::from(p.ack), 16);
    w.write(u64::from(p.ack_bits), 32);
    w.write((p.cmds.len() - 1) as u64, INPUT_COUNT_BITS);
    w.write(u64::from(first.tick), 32);
    for c in &p.cmds {
        w.write(u64::from(c.move_x as u8), 8);
        w.write(u64::from(c.move_z as u8), 8);
        w.write_bool(c.jump);
        w.write_bool(c.run);
        w.write(u64::from(c.yaw), 16);
        w.write(u64::from(c.actions), 16);
    }
    let mut out = header(DatagramKind::Input, p.seq);
    out.extend_from_slice(&w.finish());
    finish(out)
}

/// Decodes an input datagram. Range checks on the inputs themselves (move length, tick window) are the server's.
pub fn decode_input(bytes: &[u8]) -> Result<InputPacket, WireError> {
    let (seq, body) = open(bytes, DatagramKind::Input)?;
    let mut r = BitReader::new(body);
    let ack = r.read(16)? as u16;
    let ack_bits = r.read(32)? as u32;
    let count = r.read(INPUT_COUNT_BITS)? as usize + 1;
    let first = r.read(32)? as u32;
    let mut cmds = Vec::with_capacity(count);
    for i in 0..count {
        let tick = first
            .checked_add(i as u32)
            .ok_or(WireError::BadValue("input tick overflows"))?;
        cmds.push(InputCmd {
            tick,
            move_x: r.read(8)? as u8 as i8,
            move_z: r.read(8)? as u8 as i8,
            jump: r.read_bool()?,
            run: r.read_bool()?,
            yaw: r.read(16)? as u16,
            actions: r.read(16)? as u16,
        });
    }
    r.finish()?;
    Ok(InputPacket {
        seq,
        ack,
        ack_bits,
        cmds,
    })
}

// ---------- snapshot ----------

const OFFSET_BITS: u8 = 12;
const Y_BITS: u8 = 20;
const VEL_BITS: u8 = 12;

fn write_entity(w: &mut BitWriter, e: &EntityState) -> Result<(), WireError> {
    w.write_varint(e.net.0);
    w.write_varint(super::zigzag_encode(i64::from(e.pos.cell_x)));
    w.write_varint(super::zigzag_encode(i64::from(e.pos.cell_z)));
    if e.pos.x >= 1 << OFFSET_BITS || e.pos.z >= 1 << OFFSET_BITS {
        return Err(WireError::BadValue("cell offset over 12 bits"));
    }
    w.write(u64::from(e.pos.x), OFFSET_BITS);
    w.write(u64::from(e.pos.z), OFFSET_BITS);
    match e.pos.y {
        QuantY::Fixed(y) => {
            if !fits_signed(i64::from(y), Y_BITS) {
                return Err(WireError::BadValue("fixed height over 20 bits"));
            }
            w.write_bool(false);
            write_signed(w, i64::from(y), Y_BITS);
        }
        QuantY::Raw(y) => {
            w.write_bool(true);
            write_f32(w, y)?;
        }
    }
    match e.rot {
        EntityRot::Part(code) => {
            w.write_bool(false);
            w.write(u64::from(code), 32);
        }
        EntityRot::Character { yaw, state } => {
            w.write_bool(true);
            w.write(u64::from(yaw), 16);
            w.write(u64::from(state), 8);
        }
    }
    w.write_bool(e.vel.is_some());
    if let Some(vel) = e.vel {
        for c in vel {
            if !fits_signed(i64::from(c), VEL_BITS) {
                return Err(WireError::BadValue("velocity over 12 bits"));
            }
            write_signed(w, i64::from(c), VEL_BITS);
        }
    }
    w.write_bool(e.at_rest);
    Ok(())
}

fn read_cell(r: &mut BitReader) -> Result<i32, WireError> {
    i32::try_from(super::zigzag_decode(r.read_varint()?))
        .map_err(|_| WireError::BadValue("grid cell out of range"))
}

fn read_entity(r: &mut BitReader) -> Result<EntityState, WireError> {
    let net = NetId(r.read_varint()?);
    let cell_x = read_cell(r)?;
    let cell_z = read_cell(r)?;
    let x = r.read(OFFSET_BITS)? as u16;
    let z = r.read(OFFSET_BITS)? as u16;
    let y = if r.read_bool()? {
        QuantY::Raw(read_f32(r)?)
    } else {
        QuantY::Fixed(read_signed(r, Y_BITS)? as i32)
    };
    let rot = if r.read_bool()? {
        EntityRot::Character {
            yaw: r.read(16)? as u16,
            state: r.read(8)? as u8,
        }
    } else {
        EntityRot::Part(r.read(32)? as u32)
    };
    let vel = if r.read_bool()? {
        Some([
            read_signed(r, VEL_BITS)? as i16,
            read_signed(r, VEL_BITS)? as i16,
            read_signed(r, VEL_BITS)? as i16,
        ])
    } else {
        None
    };
    Ok(EntityState {
        net,
        pos: QuantPos {
            cell_x,
            cell_z,
            x,
            z,
            y,
        },
        rot,
        vel,
        at_rest: r.read_bool()?,
    })
}

/// Encodes a snapshot datagram.
///
/// Errors: [`WireError::TooLarge`] over [`MAX_DATAGRAM`] (the sender fits entities to the budget first),
/// [`WireError::BadValue`] for a non-finite own state or a quantized field out of its bit range.
pub fn encode_snapshot(p: &SnapshotPacket) -> Result<Vec<u8>, WireError> {
    let mut w = BitWriter::new();
    w.write(u64::from(p.source), 8);
    w.write(u64::from(p.server_tick), 32);
    w.write(u64::from(p.last_input_tick), 32);
    w.write(u64::from(p.buffer_hint as u8), 8);
    w.write_bool(p.own.is_some());
    if let Some(own) = &p.own {
        write_f32s(&mut w, &own.position)?;
        write_f32(&mut w, own.vertical)?;
        write_f32s(&mut w, &own.inherited)?;
        w.write(u64::from(own.state), 8);
        w.write_bool(own.ground.is_some());
        if let Some(g) = own.ground {
            w.write_varint(g.0);
        }
        write_f32(&mut w, own.yaw)?;
        write_f32(&mut w, own.dead_time)?;
        write_f32s(&mut w, &own.velocity)?;
        write_f32s(&mut w, &own.relative)?;
    }
    w.write_varint(p.entities.len() as u64);
    for e in &p.entities {
        write_entity(&mut w, e)?;
        if w.bit_len() > MAX_DATAGRAM * 8 {
            return Err(WireError::TooLarge);
        }
    }
    let mut out = header(DatagramKind::Snapshot, p.seq);
    out.extend_from_slice(&w.finish());
    finish(out)
}

/// Decodes a snapshot datagram.
pub fn decode_snapshot(bytes: &[u8]) -> Result<SnapshotPacket, WireError> {
    let (seq, body) = open(bytes, DatagramKind::Snapshot)?;
    let mut r = BitReader::new(body);
    let source = r.read(8)? as u8;
    let server_tick = r.read(32)? as u32;
    let last_input_tick = r.read(32)? as u32;
    let buffer_hint = r.read(8)? as u8 as i8;
    let own = if r.read_bool()? {
        let position = read_f32x3(&mut r)?;
        let vertical = read_f32(&mut r)?;
        let inherited = read_f32x3(&mut r)?;
        let state = r.read(8)? as u8;
        let ground = if r.read_bool()? {
            Some(NetId(r.read_varint()?))
        } else {
            None
        };
        Some(OwnState {
            position,
            vertical,
            inherited,
            state,
            ground,
            yaw: read_f32(&mut r)?,
            dead_time: read_f32(&mut r)?,
            velocity: read_f32x3(&mut r)?,
            relative: read_f32x3(&mut r)?,
        })
    } else {
        None
    };
    let count = r.read_varint()?;
    // Each entity takes well over 8 bits, so a count above the bits left is a lie; refuse it before allocating.
    if count > r.remaining_bits() as u64 / 8 {
        return Err(WireError::Truncated);
    }
    let mut entities = Vec::with_capacity(count as usize);
    for _ in 0..count {
        entities.push(read_entity(&mut r)?);
    }
    r.finish()?;
    Ok(SnapshotPacket {
        seq,
        source,
        server_tick,
        last_input_tick,
        buffer_hint,
        own,
        entities,
    })
}

// ---------- unreliable remote ----------

/// Encodes an unreliable remote call.
///
/// Errors: [`WireError::TooLarge`] when the arguments take more than [`MAX_UNRELIABLE_ARGS`] bytes,
/// [`WireError::BadValue`] for tables nested too deeply.
pub fn encode_unreliable_remote(p: &UnreliableRemotePacket) -> Result<Vec<u8>, WireError> {
    if p.args.iter().any(NetValue::too_deep) {
        return Err(WireError::BadValue("tables nested too deeply"));
    }
    let args = postcard::to_allocvec(&p.args)?;
    if args.len() > MAX_UNRELIABLE_ARGS {
        return Err(WireError::TooLarge);
    }
    let mut out = header(DatagramKind::UnreliableRemote, p.seq);
    write_u64(&mut out, p.remote.0);
    out.extend_from_slice(&args);
    finish(out)
}

/// Decodes an unreliable remote call; the arguments must use the rest of the datagram exactly.
pub fn decode_unreliable_remote(bytes: &[u8]) -> Result<UnreliableRemotePacket, WireError> {
    let (seq, body) = open(bytes, DatagramKind::UnreliableRemote)?;
    let (remote, used) = read_u64(body)?;
    let args = body.get(used..).ok_or(WireError::Truncated)?;
    if args.len() > MAX_UNRELIABLE_ARGS {
        return Err(WireError::TooLarge);
    }
    Ok(UnreliableRemotePacket {
        seq,
        remote: NetId(remote),
        args: postcard_exact(args)?,
    })
}
