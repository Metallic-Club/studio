//! Gradient noise with analytic derivatives, and curl noise built on it.
//!
//! Curl noise (Bridson, Hourihan and Nordenstam, "Curl-Noise for Procedural Fluid Flow", SIGGRAPH 2007)
//! takes the curl of a vector potential made of three noise fields. The result is divergence free, so
//! particles swirl like smoke instead of bunching into sinks the way plain noise forces make them.
//! Derivatives are analytic (quintic-faded gradient noise), so one curl sample costs three noise samples.

use crate::math::Vec3;
use crate::rng::pcg;

fn hash3(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    pcg(seed ^ pcg((x as u32) ^ pcg((y as u32) ^ pcg(z as u32))))
}

/// Pseudo-random gradient for a lattice corner: one of 16 directions on the cube edges (Perlin's set).
fn gradient(h: u32) -> Vec3 {
    const G: [(f32, f32, f32); 16] = [
        (1.0, 1.0, 0.0),
        (-1.0, 1.0, 0.0),
        (1.0, -1.0, 0.0),
        (-1.0, -1.0, 0.0),
        (1.0, 0.0, 1.0),
        (-1.0, 0.0, 1.0),
        (1.0, 0.0, -1.0),
        (-1.0, 0.0, -1.0),
        (0.0, 1.0, 1.0),
        (0.0, -1.0, 1.0),
        (0.0, 1.0, -1.0),
        (0.0, -1.0, -1.0),
        (1.0, 1.0, 0.0),
        (-1.0, 1.0, 0.0),
        (0.0, -1.0, 1.0),
        (0.0, -1.0, -1.0),
    ];
    let g = G[(h >> 28) as usize];
    Vec3::new(g.0, g.1, g.2)
}

/// Gradient noise at `p` and its gradient `(d/dx, d/dy, d/dz)`. Values lie roughly in `-1..=1`.
#[must_use]
pub fn noise3_grad(p: Vec3, seed: u32) -> (f32, Vec3) {
    let (fx, fy, fz) = (p.x.floor(), p.y.floor(), p.z.floor());
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let f = Vec3::new(p.x - fx, p.y - fy, p.z - fz);
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let dfade = |t: f32| 30.0 * t * t * (t * (t - 2.0) + 1.0);
    let (ux, uy, uz) = (fade(f.x), fade(f.y), fade(f.z));
    let (dux, duy, duz) = (dfade(f.x), dfade(f.y), dfade(f.z));

    let corner = |dx: i32, dy: i32, dz: i32| {
        let g = gradient(hash3(ix + dx, iy + dy, iz + dz, seed));
        let d = f - Vec3::new(dx as f32, dy as f32, dz as f32);
        (g, g.dot(d))
    };
    let (ga, va) = corner(0, 0, 0);
    let (gb, vb) = corner(1, 0, 0);
    let (gc, vc) = corner(0, 1, 0);
    let (gd, vd) = corner(1, 1, 0);
    let (ge, ve) = corner(0, 0, 1);
    let (gf, vf) = corner(1, 0, 1);
    let (gg, vg) = corner(0, 1, 1);
    let (gh, vh) = corner(1, 1, 1);

    let k1 = vb - va;
    let k2 = vc - va;
    let k3 = ve - va;
    let k4 = va - vb - vc + vd;
    let k5 = va - vc - ve + vg;
    let k6 = va - vb - ve + vf;
    let k7 = -va + vb + vc - vd + ve - vf - vg + vh;

    let value = va
        + ux * k1
        + uy * k2
        + uz * k3
        + ux * uy * k4
        + uy * uz * k5
        + uz * ux * k6
        + ux * uy * uz * k7;

    let g1 = gb - ga;
    let g2 = gc - ga;
    let g3 = ge - ga;
    let g4 = ga - gb - gc + gd;
    let g5 = ga - gc - ge + gg;
    let g6 = ga - gb - ge + gf;
    let g7 = -ga + gb + gc - gd + ge - gf - gg + gh;
    let interp_grad = ga
        + g1 * ux
        + g2 * uy
        + g3 * uz
        + g4 * (ux * uy)
        + g5 * (uy * uz)
        + g6 * (uz * ux)
        + g7 * (ux * uy * uz);
    let fade_grad = Vec3::new(
        dux * (k1 + uy * k4 + uz * k6 + uy * uz * k7),
        duy * (k2 + uz * k5 + ux * k4 + uz * ux * k7),
        duz * (k3 + ux * k6 + uy * k5 + ux * uy * k7),
    );
    (value, interp_grad + fade_grad)
}

/// Fractal (octave-summed) noise with gradient. Each octave doubles frequency and halves amplitude.
#[must_use]
pub fn fbm3_grad(p: Vec3, seed: u32, octaves: u32) -> (f32, Vec3) {
    let mut value = 0.0;
    let mut grad = Vec3::ZERO;
    let mut amp = 1.0;
    let mut freq = 1.0;
    let mut norm = 0.0;
    for o in 0..octaves.clamp(1, 6) {
        let (v, g) = noise3_grad(p * freq, seed.wrapping_add(o * 1013));
        value += v * amp;
        grad += g * (amp * freq);
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    (value / norm, grad / norm)
}

/// Divergence-free curl-noise velocity at `p`.
#[must_use]
pub fn curl3(p: Vec3, seed: u32, octaves: u32) -> Vec3 {
    let (_, a) = fbm3_grad(p, seed, octaves);
    let (_, b) = fbm3_grad(p + Vec3::new(31.4, 47.2, 12.9), seed ^ 0x5bd1_e995, octaves);
    let (_, c) = fbm3_grad(
        p + Vec3::new(-19.1, 33.7, 71.3),
        seed ^ 0x27d4_eb2d,
        octaves,
    );
    // Potential psi = (a, b, c); curl = (dc/dy - db/dz, da/dz - dc/dx, db/dx - da/dy).
    Vec3::new(c.y - b.z, a.z - c.x, b.x - a.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_zero_on_lattice_and_bounded() {
        assert_eq!(noise3_grad(Vec3::new(3.0, -2.0, 5.0), 1).0, 0.0);
        for i in 0..2000 {
            let p = Vec3::new(i as f32 * 0.137, i as f32 * 0.071, i as f32 * 0.029);
            let (v, _) = noise3_grad(p, 9);
            assert!(v.abs() <= 1.5, "value {v}");
        }
    }

    #[test]
    fn analytic_gradient_matches_finite_difference() {
        let h = 1e-3;
        for i in 0..50 {
            let p = Vec3::new(
                0.3 + i as f32 * 0.21,
                1.7 - i as f32 * 0.13,
                0.45 * i as f32,
            );
            let (_, g) = noise3_grad(p, 4);
            let fd =
                |d: Vec3| (noise3_grad(p + d * h, 4).0 - noise3_grad(p - d * h, 4).0) / (2.0 * h);
            let n = Vec3::new(fd(Vec3::X), fd(Vec3::Y), fd(Vec3::Z));
            assert!((g - n).length() < 2e-2, "analytic {g:?} vs numeric {n:?}");
        }
    }

    #[test]
    fn curl_is_divergence_free() {
        let h = 1e-2;
        for i in 0..20 {
            let p = Vec3::new(
                0.37 * i as f32,
                0.5 + 0.11 * i as f32,
                1.3 - 0.07 * i as f32,
            );
            let div = (curl3(p + Vec3::X * h, 2, 1).x - curl3(p - Vec3::X * h, 2, 1).x
                + curl3(p + Vec3::Y * h, 2, 1).y
                - curl3(p - Vec3::Y * h, 2, 1).y
                + curl3(p + Vec3::Z * h, 2, 1).z
                - curl3(p - Vec3::Z * h, 2, 1).z)
                / (2.0 * h);
            let mag = curl3(p, 2, 1).length();
            assert!(div.abs() < 0.05 * mag.max(1.0), "divergence {div} at {p:?}");
        }
    }
}
