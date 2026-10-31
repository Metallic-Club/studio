//! A small seeded random number generator (SplitMix64).
//!
//! The building tools need repeatable randomness: the same seed and the same stroke must give
//! the same placements on every machine. SplitMix64 is a public-domain 64-bit mixer with good
//! statistical quality for this use and no platform-dependent behaviour.

/// Deterministic 64-bit generator. Same seed, same sequence, on every platform.
#[derive(Debug, Clone)]
pub struct SeededRng {
    state: u64,
}

impl SeededRng {
    /// Creates a generator from a seed.
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Creates a generator for one sub-stream of a seed (for example one brush dab), so that
    /// streams with different `stream` values do not overlap in practice.
    pub fn for_stream(seed: u64, stream: u64) -> Self {
        let mut mix = Self::new(seed ^ stream.wrapping_mul(0xD1B5_4A32_D192_ED03));
        let state = mix.next_u64();
        Self { state }
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform number in `[0, 1)` with 53 bits of precision.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// A uniform number in `[lo, hi)`; returns `lo` when the range is empty.
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        if hi <= lo {
            lo
        } else {
            lo + (hi - lo) * self.next_f64()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = SeededRng::new(42);
        let mut b = SeededRng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn streams_differ() {
        let mut a = SeededRng::for_stream(7, 0);
        let mut b = SeededRng::for_stream(7, 1);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn unit_interval_and_mean() {
        let mut r = SeededRng::new(1);
        let n = 20_000;
        let mut sum = 0.0;
        for _ in 0..n {
            let x = r.next_f64();
            assert!((0.0..1.0).contains(&x));
            sum += x;
        }
        let mean = sum / n as f64;
        assert!((mean - 0.5).abs() < 0.01, "mean {mean}");
    }

    #[test]
    fn empty_range_returns_low() {
        let mut r = SeededRng::new(3);
        assert_eq!(r.range(2.0, 2.0), 2.0);
        assert_eq!(r.range(5.0, 1.0), 5.0);
    }
}
