//! A small, fast hasher for the registry's name tables.
//!
//! This is the FxHash algorithm (from Firefox, also used inside rustc): one rotate, xor and multiply per 8 bytes.
//! It is several times faster than the standard SipHash on short keys such as property names. It is not resistant
//! to collision attacks, which is fine here: the tables are filled once from the static class descriptors, and a
//! lookup with an unknown name (from a script) cannot add entries, so at worst it is slower.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// The FxHash state.
#[derive(Clone, Copy, Default)]
pub(crate) struct FxHasher {
    hash: u64,
}

/// The multiplier FxHash uses for 64-bit words.
const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FxHasher {
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for word in words {
            self.add(u64::from_le_bytes(*word));
        }
        if !rest.is_empty() {
            let mut word = [0; 8];
            word[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(word));
        }
    }

    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }

    fn write_u16(&mut self, i: u16) {
        self.add(u64::from(i));
    }

    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }

    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    fn finish(&self) -> u64 {
        self.hash
    }
}

/// A `HashMap` using [`FxHasher`].
pub(crate) type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

#[cfg(test)]
mod tests {
    use std::hash::Hash;

    use super::*;

    fn hash(s: &str) -> u64 {
        let mut h = FxHasher::default();
        s.hash(&mut h);
        h.finish()
    }

    #[test]
    fn distinguishes_close_names() {
        let names = [
            "Size", "Sizf", "CFrame", "CFrame2", "Position", "Positiom", "", "a", "aa",
        ];
        let hashes: std::collections::HashSet<u64> = names.iter().map(|n| hash(n)).collect();
        assert_eq!(hashes.len(), names.len());
    }

    #[test]
    fn map_finds_borrowed_keys() {
        let mut map: FxMap<&'static str, u16> = FxMap::default();
        map.insert("Brightness", 3);
        let key = String::from("Brightness");
        assert_eq!(map.get(key.as_str()), Some(&3));
        assert_eq!(map.get("brightness"), None);
    }
}
