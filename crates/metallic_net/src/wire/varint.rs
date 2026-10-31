//! LEB128 variable-length integers (7 bits per byte, low bits first) and zigzag for signed values. The same encoding
//! postcard uses for its integers and lengths.

use super::WireError;

/// Most bytes a `u64` takes.
pub const MAX_VARINT_LEN: usize = 10;

/// Appends `v` as LEB128.
pub fn write_u64(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// Reads a LEB128 `u64` from the start of `bytes`; returns it and how many bytes it took.
///
/// Errors: [`WireError::Truncated`] when the bytes end inside the number, [`WireError::BadValue`] when it is longer
/// than 10 bytes or does not fit in 64 bits.
pub fn read_u64(bytes: &[u8]) -> Result<(u64, usize), WireError> {
    let mut v: u64 = 0;
    for (i, &b) in bytes.iter().take(MAX_VARINT_LEN).enumerate() {
        let low = u64::from(b & 0x7f);
        if i == MAX_VARINT_LEN - 1 && low > 1 {
            return Err(WireError::BadValue("varint overflows 64 bits"));
        }
        v |= low << (7 * i);
        if b & 0x80 == 0 {
            return Ok((v, i + 1));
        }
    }
    if bytes.len() >= MAX_VARINT_LEN {
        Err(WireError::BadValue("varint longer than 10 bytes"))
    } else {
        Err(WireError::Truncated)
    }
}

/// Zigzag-maps a signed number so small magnitudes of either sign stay small (0, -1, 1, -2 -> 0, 1, 2, 3).
pub fn zigzag_encode(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// Inverse of [`zigzag_encode`].
pub fn zigzag_decode(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

/// Appends `v` as a zigzag LEB128 number.
pub fn write_i64(out: &mut Vec<u8>, v: i64) {
    write_u64(out, zigzag_encode(v));
}

/// Reads a zigzag LEB128 `i64`; returns it and how many bytes it took. Errors as [`read_u64`].
pub fn read_i64(bytes: &[u8]) -> Result<(i64, usize), WireError> {
    read_u64(bytes).map(|(v, n)| (zigzag_decode(v), n))
}
