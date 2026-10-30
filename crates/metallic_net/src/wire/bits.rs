//! Bit packing for datagrams. Fields are written low bit first, packed back to back with no alignment; the last byte
//! is padded with zero bits.

use super::WireError;
use super::varint::MAX_VARINT_LEN;

/// Packs fields of 1 to 64 bits into bytes.
#[derive(Debug, Default, Clone)]
pub struct BitWriter {
    bytes: Vec<u8>,
    acc: u128,
    pending: u32,
}

impl BitWriter {
    /// An empty writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends the low `bits` bits of `value` (higher bits are ignored). `bits` above 64 is treated as 64.
    pub fn write(&mut self, value: u64, bits: u8) {
        let bits = u32::from(bits.min(64));
        let masked = if bits == 64 {
            value
        } else {
            value & ((1u64 << bits) - 1)
        };
        self.acc |= u128::from(masked) << self.pending;
        self.pending += bits;
        while self.pending >= 8 {
            self.bytes.push(self.acc as u8);
            self.acc >>= 8;
            self.pending -= 8;
        }
    }

    /// Appends one bit.
    pub fn write_bool(&mut self, value: bool) {
        self.write(u64::from(value), 1);
    }

    /// Appends `value` as LEB128 groups (7 value bits and a continuation bit each): small numbers take 8 bits.
    pub fn write_varint(&mut self, mut value: u64) {
        while value >= 0x80 {
            self.write((value & 0x7f) | 0x80, 8);
            value >>= 7;
        }
        self.write(value, 8);
    }

    /// Bits written so far.
    pub fn bit_len(&self) -> usize {
        self.bytes.len() * 8 + self.pending as usize
    }

    /// The packed bytes, the last one padded with zero bits.
    pub fn finish(mut self) -> Vec<u8> {
        if self.pending > 0 {
            self.bytes.push(self.acc as u8);
        }
        self.bytes
    }
}

/// Reads fields written by a [`BitWriter`]. Every read checks the remaining length first.
#[derive(Debug, Clone)]
pub struct BitReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    /// A reader at the first bit of `bytes`.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    /// Bits left to read (including padding).
    pub fn remaining_bits(&self) -> usize {
        self.bytes.len() * 8 - self.pos
    }

    /// Reads a field of `bits` bits (1 to 64).
    ///
    /// Errors: [`WireError::Truncated`] past the end, [`WireError::BadValue`] for more than 64 bits.
    pub fn read(&mut self, bits: u8) -> Result<u64, WireError> {
        if bits > 64 {
            return Err(WireError::BadValue("bit field wider than 64 bits"));
        }
        let bits = usize::from(bits);
        if bits > self.remaining_bits() {
            return Err(WireError::Truncated);
        }
        let mut out: u64 = 0;
        let mut got = 0;
        while got < bits {
            let byte = *self.bytes.get(self.pos / 8).ok_or(WireError::Truncated)?;
            let offset = self.pos % 8;
            let take = (8 - offset).min(bits - got);
            let chunk = (u16::from(byte) >> offset) & ((1u16 << take) - 1);
            out |= u64::from(chunk) << got;
            got += take;
            self.pos += take;
        }
        Ok(out)
    }

    /// Reads one bit.
    pub fn read_bool(&mut self) -> Result<bool, WireError> {
        Ok(self.read(1)? == 1)
    }

    /// Reads a number written by [`BitWriter::write_varint`].
    pub fn read_varint(&mut self) -> Result<u64, WireError> {
        let mut v: u64 = 0;
        for i in 0..MAX_VARINT_LEN {
            let group = self.read(8)?;
            let low = group & 0x7f;
            if i == MAX_VARINT_LEN - 1 && low > 1 {
                return Err(WireError::BadValue("varint overflows 64 bits"));
            }
            v |= low << (7 * i);
            if group & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(WireError::BadValue("varint longer than 10 bytes"))
    }

    /// Checks that only the zero padding of the last byte is left: anything else is an error.
    pub fn finish(self) -> Result<(), WireError> {
        let left = self.remaining_bits();
        if left >= 8 {
            return Err(WireError::BadValue("trailing bytes after the packet"));
        }
        let mut rest = self;
        if left > 0 && rest.read(left as u8)? != 0 {
            return Err(WireError::BadValue("nonzero padding bits"));
        }
        Ok(())
    }
}
