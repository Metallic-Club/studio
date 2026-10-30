//! Mesh checks shared by the integration tests.
#![allow(dead_code)]

use metallic_meshgen::{MeshData, MeshDesc};
use std::collections::HashMap;

pub type V3 = [f32; 3];

pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn len(a: V3) -> f32 {
    dot(a, a).sqrt()
}

pub fn tri(m: &MeshData, t: usize) -> [usize; 3] {
    [0, 1, 2].map(|k| m.indices[t * 3 + k] as usize)
}

/// Triangle area and unit normal.
pub fn tri_geom(m: &MeshData, t: usize) -> (f32, V3) {
    let [a, b, c] = tri(m, t);
    let g = cross(
        sub(m.positions[b], m.positions[a]),
        sub(m.positions[c], m.positions[a]),
    );
    let l = len(g);
    (0.5 * l, if l > 0.0 { g.map(|x| x / l) } else { [0.0; 3] })
}

pub fn uv_area(m: &MeshData, t: usize) -> f32 {
    let [a, b, c] = tri(m, t);
    let (p, q, r) = (m.uv0[a], m.uv0[b], m.uv0[c]);
    0.5 * ((q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]))
}

/// Every directed edge appears once and its reverse once (positions compared exactly: the generator welds).
pub fn assert_watertight(m: &MeshData, what: &str) {
    let mut ids: HashMap<[u32; 3], usize> = HashMap::new();
    let canon: Vec<usize> = m
        .positions
        .iter()
        .map(|p| {
            let n = ids.len();
            *ids.entry(p.map(|x| (x + 0.0).to_bits())).or_insert(n)
        })
        .collect();
    let mut edges: HashMap<(usize, usize), u32> = HashMap::new();
    for t in 0..m.triangle_count() {
        let [a, b, c] = tri(m, t).map(|i| canon[i]);
        for (x, y) in [(a, b), (b, c), (c, a)] {
            *edges.entry((x, y)).or_default() += 1;
        }
    }
    for (&(x, y), &n) in &edges {
        assert_eq!(n, 1, "{what}: edge {x}->{y} used {n} times");
        assert_eq!(
            edges.get(&(y, x)).copied().unwrap_or(0),
            1,
            "{what}: edge {x}->{y} ({:?} -> {:?}) has no partner",
            m.positions
                .iter()
                .zip(&canon)
                .find(|(_, c)| **c == x)
                .map(|(p, _)| *p),
            m.positions
                .iter()
                .zip(&canon)
                .find(|(_, c)| **c == y)
                .map(|(p, _)| *p),
        );
    }
}

pub fn signed_volume(m: &MeshData) -> f32 {
    (0..m.triangle_count())
        .map(|t| {
            let [a, b, c] = tri(m, t);
            dot(m.positions[a], cross(m.positions[b], m.positions[c])) / 6.0
        })
        .sum()
}

/// Structural checks every mesh must pass.
pub fn assert_valid(m: &MeshData, desc: &MeshDesc, what: &str) {
    let n = m.positions.len();
    assert!(n > 0 && m.triangle_count() > 0, "{what}: empty");
    for arr_len in [
        m.normals.len(),
        m.tangents.len(),
        m.uv0.len(),
        m.uv1.len(),
        m.ao.len(),
    ] {
        assert_eq!(arr_len, n, "{what}: attribute length");
    }
    assert_eq!(m.indices.len() % 3, 0);
    assert!(
        m.indices.iter().all(|i| (*i as usize) < n),
        "{what}: index range"
    );
    // Compare against the quantised size the mesh was really built for.
    let size = desc.key().map(|k| k.desc().size).unwrap_or(desc.size);
    let scale = size.iter().fold(0.0f32, |a, b| a.max(*b));

    for i in 0..n {
        let nn = m.normals[i];
        assert!(
            (len(nn) - 1.0).abs() < 1e-3,
            "{what}: normal {i} not unit: {nn:?}"
        );
        let t = m.tangents[i];
        let tv = [t[0], t[1], t[2]];
        assert!(
            (len(tv) - 1.0).abs() < 1e-3,
            "{what}: tangent {i} not unit: {t:?}"
        );
        assert!(
            dot(tv, nn).abs() < 1e-3,
            "{what}: tangent {i} not orthogonal to normal"
        );
        assert!(t[3] == 1.0 || t[3] == -1.0, "{what}: tangent sign {}", t[3]);
        assert!((0.0..=1.0).contains(&m.ao[i]), "{what}: ao {}", m.ao[i]);
        for k in 0..2 {
            assert!(m.uv0[i][k].is_finite());
            assert!(
                (-1e-4..=1.0 + 1e-4).contains(&m.uv1[i][k]),
                "{what}: uv1 {:?}",
                m.uv1[i]
            );
        }
        for (k, &p) in m.positions[i].iter().enumerate() {
            assert!(
                p.abs() <= size[k] * 0.5 * (1.0 + 1e-5) + 1e-5,
                "{what}: vertex {i} outside the box: {:?}",
                m.positions[i]
            );
        }
    }

    // Degenerate = a triangle whose height is below float precision for this size.
    let min_height = 1e-7 + scale * 1e-8;
    for t in 0..m.triangle_count() {
        let (area, g) = tri_geom(m, t);
        let [a, b, c] = tri(m, t).map(|i| m.positions[i]);
        let longest = len(sub(b, a)).max(len(sub(c, b))).max(len(sub(a, c)));
        let height = 2.0 * area / longest.max(1e-30);
        assert!(
            area > 0.0 && height > min_height,
            "{what}: degenerate triangle {t} (area {area}, height {height})"
        );
        for v in tri(m, t) {
            assert!(
                dot(g, m.normals[v]) > 0.0,
                "{what}: triangle {t} faces against vertex normal {:?} (geometric {g:?})",
                m.normals[v]
            );
        }
    }
    assert_watertight(m, what);
    assert!(
        signed_volume(m) > 0.0,
        "{what}: not outward (volume {})",
        signed_volume(m)
    );
}

/// Median of (uv0 area / 3D area) over triangles, weighted by nothing.
pub fn median_uv_density(m: &MeshData) -> f32 {
    let mut r: Vec<f32> = (0..m.triangle_count())
        .filter_map(|t| {
            let (a, _) = tri_geom(m, t);
            (a > 1e-9).then(|| uv_area(m, t).abs() / a)
        })
        .collect();
    r.sort_by(f32::total_cmp);
    r[r.len() / 2]
}

/// Tiny deterministic random numbers (xorshift64*), for property-style tests without extra crates.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    /// Uniform in `lo..hi`.
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        let u = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
        lo + (hi - lo) * u
    }
    /// Log-uniform in `lo..hi` (sizes spread over orders of magnitude).
    pub fn log_range(&mut self, lo: f32, hi: f32) -> f32 {
        (self.range(lo.ln(), hi.ln())).exp()
    }
}
