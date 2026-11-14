//! The wire format: a small binary encoding with bounded reads.
//!
//! Every message is written with a [`Writer`] and read back with a [`Reader`]. Numbers are little-endian, lengths and
//! counts are LEB128 varints, floats are their exact bits (so every peer stores the same value the sender had), and
//! text is UTF-8 with a length in front. A reader never trusts a length: it checks it against the bytes left and
//! against the caller's limit before allocating, so a hostile peer cannot make it allocate more than it sent.

use thiserror::Error;

/// Why a message could not be read.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum CodecError {
    /// The message ended in the middle of a value.
    #[error("the message ended too soon")]
    Truncated,
    /// A length or count is larger than the reader allows.
    #[error("{what} is too long ({len}, at most {max})")]
    TooLong {
        /// What was being read.
        what: &'static str,
        /// The length the message gave.
        len: u64,
        /// The largest allowed.
        max: u64,
    },
    /// Text that is not UTF-8.
    #[error("text that is not UTF-8")]
    BadUtf8,
    /// A tag byte that names no variant.
    #[error("{what}: unknown tag {tag}")]
    BadTag {
        /// What was being read.
        what: &'static str,
        /// The tag.
        tag: u8,
    },
    /// A varint longer than its type, or a value out of its type's range.
    #[error("a number out of range")]
    Overflow,
    /// Bytes left after the message's last value.
    #[error("{0} bytes after the end of the message")]
    Trailing(usize),
    /// A value that decodes but breaks a rule of the protocol.
    #[error("{0}")]
    Invalid(&'static str),
}

/// Writes values into a growing buffer.
#[derive(Clone, Debug, Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    /// An empty writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty writer with room for `capacity` bytes.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buf: Vec::with_capacity(capacity),
        }
    }

    /// The bytes written so far.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    /// How many bytes are written.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Whether nothing is written.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// The bytes written.
    pub fn finish(self) -> Vec<u8> {
        self.buf
    }

    /// One byte.
    pub fn u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    /// A boolean as one byte.
    pub fn bool(&mut self, value: bool) {
        self.buf.push(u8::from(value));
    }

    /// An unsigned number as a varint.
    pub fn varint(&mut self, mut value: u64) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                self.buf.push(byte);
                return;
            }
            self.buf.push(byte | 0x80);
        }
    }

    /// A `u32` as a varint.
    pub fn u32(&mut self, value: u32) {
        self.varint(u64::from(value));
    }

    /// A `u64` as a varint.
    pub fn u64(&mut self, value: u64) {
        self.varint(value);
    }

    /// A signed number, zigzag-encoded as a varint.
    pub fn i64(&mut self, value: i64) {
        self.varint(((value << 1) ^ (value >> 63)) as u64);
    }

    /// A length or count.
    pub fn len_of(&mut self, len: usize) {
        self.varint(len as u64);
    }

    /// An `f32`'s bits.
    pub fn f32(&mut self, value: f32) {
        self.buf.extend_from_slice(&value.to_bits().to_le_bytes());
    }

    /// An `f64`'s bits.
    pub fn f64(&mut self, value: f64) {
        self.buf.extend_from_slice(&value.to_bits().to_le_bytes());
    }

    /// Bytes, with their length in front.
    pub fn bytes(&mut self, bytes: &[u8]) {
        self.len_of(bytes.len());
        self.buf.extend_from_slice(bytes);
    }

    /// Text, with its length in bytes in front.
    pub fn str(&mut self, text: &str) {
        self.bytes(text.as_bytes());
    }

    /// An optional value: a flag byte, then the value when there is one.
    pub fn option<T>(&mut self, value: Option<T>, write: impl FnOnce(&mut Self, T)) {
        match value {
            Some(value) => {
                self.u8(1);
                write(self, value);
            }
            None => self.u8(0),
        }
    }
}

/// Reads values from a message, never past its end and never more than its limits allow.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// A reader over `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// How many bytes are left.
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// Fails unless every byte was read.
    pub fn finish(&self) -> Result<(), CodecError> {
        match self.remaining() {
            0 => Ok(()),
            left => Err(CodecError::Trailing(left)),
        }
    }

    /// One byte.
    pub fn u8(&mut self) -> Result<u8, CodecError> {
        let byte = *self.data.get(self.pos).ok_or(CodecError::Truncated)?;
        self.pos += 1;
        Ok(byte)
    }

    /// A boolean: 0 or 1.
    pub fn bool(&mut self) -> Result<bool, CodecError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            tag => Err(CodecError::BadTag { what: "a boolean", tag }),
        }
    }

    /// A varint, at most ten bytes.
    pub fn varint(&mut self) -> Result<u64, CodecError> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.u8()?;
            let bits = u64::from(byte & 0x7f);
            if shift == 63 && bits > 1 {
                return Err(CodecError::Overflow);
            }
            value |= bits << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(CodecError::Overflow)
    }

    /// A `u32` varint.
    pub fn u32(&mut self) -> Result<u32, CodecError> {
        u32::try_from(self.varint()?).map_err(|_| CodecError::Overflow)
    }

    /// A `u16` varint.
    pub fn u16(&mut self) -> Result<u16, CodecError> {
        u16::try_from(self.varint()?).map_err(|_| CodecError::Overflow)
    }

    /// A `u64` varint.
    pub fn u64(&mut self) -> Result<u64, CodecError> {
        self.varint()
    }

    /// A zigzag-encoded signed varint.
    pub fn i64(&mut self) -> Result<i64, CodecError> {
        let raw = self.varint()?;
        Ok(((raw >> 1) as i64) ^ -((raw & 1) as i64))
    }

    /// A count of items that each take at least one byte, at most `max`: never more than the bytes left, so a
    /// collection read with it allocates no more than the message is long.
    pub fn count(&mut self, what: &'static str, max: usize) -> Result<usize, CodecError> {
        let len = self.varint()?;
        let limit = max.min(self.remaining()) as u64;
        if len > limit {
            return Err(CodecError::TooLong {
                what,
                len,
                max: max as u64,
            });
        }
        Ok(len as usize)
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], CodecError> {
        let end = self.pos.checked_add(len).ok_or(CodecError::Truncated)?;
        let bytes = self.data.get(self.pos..end).ok_or(CodecError::Truncated)?;
        self.pos = end;
        Ok(bytes)
    }

    /// An `f32` from its bits.
    pub fn f32(&mut self) -> Result<f32, CodecError> {
        let bytes = self.take(4)?;
        let mut raw = [0; 4];
        raw.copy_from_slice(bytes);
        Ok(f32::from_bits(u32::from_le_bytes(raw)))
    }

    /// An `f64` from its bits.
    pub fn f64(&mut self) -> Result<f64, CodecError> {
        let bytes = self.take(8)?;
        let mut raw = [0; 8];
        raw.copy_from_slice(bytes);
        Ok(f64::from_bits(u64::from_le_bytes(raw)))
    }

    /// Bytes with their length in front, at most `max`.
    pub fn bytes(&mut self, what: &'static str, max: usize) -> Result<&'a [u8], CodecError> {
        let len = self.count(what, max)?;
        self.take(len)
    }

    /// Text with its length in front, at most `max` bytes.
    pub fn str(&mut self, what: &'static str, max: usize) -> Result<&'a str, CodecError> {
        let bytes = self.bytes(what, max)?;
        std::str::from_utf8(bytes).map_err(|_| CodecError::BadUtf8)
    }

    /// An optional value written by [`Writer::option`].
    pub fn option<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, CodecError>,
    ) -> Result<Option<T>, CodecError> {
        if self.bool()? { Ok(Some(read(self)?)) } else { Ok(None) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_read_back_as_written() {
        let mut w = Writer::new();
        w.u8(7);
        w.bool(true);
        w.u64(u64::MAX);
        w.u32(300);
        w.i64(-5);
        w.i64(i64::MIN);
        w.f64(0.1);
        w.f32(-0.0);
        w.str("héllo");
        w.option(Some(3u32), |w, v| w.u32(v));
        w.option(None::<u32>, |w, v| w.u32(v));
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        assert_eq!(r.u8().unwrap(), 7);
        assert!(r.bool().unwrap());
        assert_eq!(r.u64().unwrap(), u64::MAX);
        assert_eq!(r.u32().unwrap(), 300);
        assert_eq!(r.i64().unwrap(), -5);
        assert_eq!(r.i64().unwrap(), i64::MIN);
        assert_eq!(r.f64().unwrap().to_bits(), 0.1f64.to_bits());
        assert_eq!(r.f32().unwrap().to_bits(), (-0.0f32).to_bits());
        assert_eq!(r.str("text", 16).unwrap(), "héllo");
        assert_eq!(r.option(|r| r.u32()).unwrap(), Some(3));
        assert_eq!(r.option(|r| r.u32()).unwrap(), None);
        r.finish().unwrap();
    }

    #[test]
    fn hostile_lengths_are_refused_before_allocating() {
        let mut w = Writer::new();
        w.varint(1 << 40);
        let bytes = w.finish();
        assert!(matches!(
            Reader::new(&bytes).count("items", usize::MAX),
            Err(CodecError::TooLong { .. })
        ));
        let mut w = Writer::new();
        w.str("abcdef");
        let bytes = w.finish();
        assert!(matches!(
            Reader::new(&bytes).str("name", 3),
            Err(CodecError::TooLong { .. })
        ));
        // A varint that never ends.
        assert_eq!(Reader::new(&[0xff; 11]).varint(), Err(CodecError::Overflow));
        assert_eq!(Reader::new(&[0x80]).varint(), Err(CodecError::Truncated));
        assert_eq!(
            Reader::new(&[2]).bool(),
            Err(CodecError::BadTag {
                what: "a boolean",
                tag: 2
            })
        );
        assert_eq!(Reader::new(&[1, 2]).finish(), Err(CodecError::Trailing(2)));
    }
}
