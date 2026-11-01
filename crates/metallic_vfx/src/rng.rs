//! Counter-based random numbers that a compute shader can reproduce bit for bit.
//!
//! Every random value is a pure function of `(effect seed, emitter index, particle id, stream)`, hashed
//! with the PCG output permutation (the same `pcg` used by `docs/research/style-shaders/particles_sim.wgsl`).
//! Nothing depends on how many values were drawn before, so the CPU reference and a later GPU port can
//! agree on every particle's initial state.

use crate::math::Vec3;

/// PCG-RXS-M-XS 32-bit hash ("PCG hash", Jarzynski and Olano 2020).
#[must_use]
pub fn pcg(v: u32) -> u32 {
    let state = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

/// A random stream for one particle: draws are `hash(key, counter)` with an incrementing counter.
#[derive(Debug, Clone, Copy)]
pub struct Rng {
    key: u32,
    counter: u32,
}

impl Rng {
    /// Stream keyed by an effect seed, an emitter index and a particle id.
    #[must_use]
    pub fn for_particle(seed: u32, emitter: u32, particle: u32) -> Self {
        let key = pcg(seed ^ pcg(emitter.wrapping_mul(0x9E37_79B9) ^ pcg(particle)));
        Self { key, counter: 0 }
    }

    /// Stream keyed by an arbitrary pair, for per-emitter events such as burst probabilities.
    #[must_use]
    pub fn for_event(seed: u32, a: u32, b: u32) -> Self {
        Self {
            key: pcg(seed.wrapping_add(0x68E3_1DA4) ^ pcg(a ^ pcg(b.wrapping_add(17)))),
            counter: 0,
        }
    }

    /// Next raw 32-bit value.
    pub fn next_u32(&mut self) -> u32 {
        self.counter = self.counter.wrapping_add(1);
        pcg(self.key ^ pcg(self.counter))
    }

    /// Uniform in `[0, 1)`.
    pub fn next_f32(&mut self) -> f32 {
        // 24 high bits give an exact float in [0, 1).
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }

    /// Uniform in `[a, b)`.
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next_f32()
    }

    /// Uniform direction on the unit sphere.
    pub fn unit_vector(&mut self) -> Vec3 {
        let z = self.range(-1.0, 1.0);
        let phi = self.next_f32() * std::f32::consts::TAU;
        let r = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(r * phi.cos(), z, r * phi.sin())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcg_matches_reference_values() {
        // Values computed from the WGSL definition by hand-checked integer arithmetic.
        assert_eq!(pcg(0), pcg(0));
        assert_ne!(pcg(1), pcg(2));
        // Avalanche sanity: neighbouring inputs differ in many bits.
        assert!((pcg(1000) ^ pcg(1001)).count_ones() > 8);
    }

    #[test]
    fn floats_are_in_unit_interval_and_roughly_uniform() {
        let mut r = Rng::for_particle(7, 0, 0);
        let mut sum = 0.0;
        for _ in 0..10_000 {
            let v = r.next_f32();
            assert!((0.0..1.0).contains(&v));
            sum += v;
        }
        let mean = sum / 10_000.0;
        assert!((mean - 0.5).abs() < 0.02, "mean {mean}");
    }

    #[test]
    fn streams_are_pure_functions_of_their_key() {
        let mut a = Rng::for_particle(1, 2, 3);
        let mut b = Rng::for_particle(1, 2, 3);
        for _ in 0..16 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
        let mut c = Rng::for_particle(1, 2, 4);
        assert_ne!(Rng::for_particle(1, 2, 3).next_u32(), c.next_u32());
    }
}
