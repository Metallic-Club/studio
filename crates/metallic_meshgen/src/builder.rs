//! Triangle soup collected by the shape generators, and the shared finishing passes that turn it into an
//! indexed [`MeshData`]: weld, drop collapsed triangles, shading, vertex sharing, `uv1`, AO, tangents, bounds.
use crate::mesh::{Aabb, MeshData};
use crate::shape::{MeshDesc, Shading};
use glam::{Vec2, Vec3};
use std::collections::HashMap;

/// One triangle corner as emitted by a generator.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Corner {
    pub p: Vec3,
    pub n: Vec3,
    pub uv: Vec2,
}

impl Corner {
    pub(crate) fn new(p: Vec3, n: Vec3, uv: Vec2) -> Corner {
        Corner { p, n, uv }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Tri {
    pub c: [Corner; 3],
    pub chart: u32,
}

/// Collects triangles. Each triangle belongs to a UV chart (a region with continuous `uv0`); `uv1` is later
/// normalised to 0..1 per chart.
#[derive(Debug, Default)]
pub(crate) struct Builder {
    pub tris: Vec<Tri>,
    charts: u32,
}

impl Builder {
    pub(crate) fn new() -> Builder {
        Builder::default()
    }

    /// A fresh chart id.
    pub(crate) fn chart(&mut self) -> u32 {
        self.charts += 1;
        self.charts - 1
    }

    /// Adds a triangle. The winding is fixed so the geometric normal agrees with the corner normals, so
    /// generators never have to think about vertex order.
    pub(crate) fn tri(&mut self, chart: u32, a: Corner, b: Corner, c: Corner) {
        let g = (b.p - a.p).cross(c.p - a.p);
        let s = a.n + b.n + c.n;
        let c3 = if g.dot(s) < 0.0 { [a, c, b] } else { [a, b, c] };
        self.tris.push(Tri { c: c3, chart });
    }

    /// Adds the quad `a b c d` (in order around its border) as two triangles.
    pub(crate) fn quad(&mut self, chart: u32, a: Corner, b: Corner, c: Corner, d: Corner) {
        self.tri(chart, a, b, c);
        self.tri(chart, a, c, d);
    }

    /// Adds a triangle whose normal is its own geometric normal, oriented toward `outward`.
    pub(crate) fn flat_tri(&mut self, chart: u32, outward: Vec3, mut c: [Corner; 3]) {
        let g = (c[1].p - c[0].p).cross(c[2].p - c[0].p);
        let n = if g.dot(outward) < 0.0 { -g } else { g }.normalize_or(outward);
        for k in &mut c {
            k.n = n;
        }
        self.tri(chart, c[0], c[1], c[2]);
    }
}

/// Weld tolerance in studs for a mesh of the given half size: above float noise (relative to the largest
/// coordinate), far below anything visible, and never more than 5% of the thinnest side.
pub(crate) fn weld_tolerance(half: Vec3) -> f32 {
    (1e-7 + half.max_element() * 2e-6).min(half.min_element() * 0.05)
}

/// Snaps positions closer than `tol` to one representative (first seen wins, so the result is deterministic)
/// and drops triangles that collapse to a line or a point.
fn weld(tris: &mut Vec<Tri>, tol: f32) {
    let cell = |p: Vec3| -> (i64, i64, i64) {
        (
            (p.x / tol).floor() as i64,
            (p.y / tol).floor() as i64,
            (p.z / tol).floor() as i64,
        )
    };
    let mut grid: HashMap<(i64, i64, i64), Vec<Vec3>> = HashMap::new();
    let tol2 = tol * tol;
    for t in tris.iter_mut() {
        for c in &mut t.c {
            let (x, y, z) = cell(c.p);
            let mut found = None;
            'search: for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(list) = grid.get(&(x + dx, y + dy, z + dz)) {
                            if let Some(r) = list.iter().find(|r| r.distance_squared(c.p) <= tol2) {
                                found = Some(*r);
                                break 'search;
                            }
                        }
                    }
                }
            }
            match found {
                Some(r) => c.p = r,
                None => grid.entry((x, y, z)).or_default().push(c.p),
            }
        }
    }
    tris.retain(|t| t.c[0].p != t.c[1].p && t.c[1].p != t.c[2].p && t.c[0].p != t.c[2].p);
}

fn face_normal(t: &Tri) -> Vec3 {
    (t.c[1].p - t.c[0].p)
        .cross(t.c[2].p - t.c[0].p)
        .normalize_or(t.c[0].n)
}

fn corner_angle(t: &Tri, k: usize) -> f32 {
    let p = t.c[k].p;
    let a = t.c[(k + 1) % 3].p - p;
    let b = t.c[(k + 2) % 3].p - p;
    a.angle_between(b)
}

fn pos_key(p: Vec3) -> [u32; 3] {
    // `+ 0.0` turns -0.0 into +0.0 so both hash the same.
    [
        (p.x + 0.0).to_bits(),
        (p.y + 0.0).to_bits(),
        (p.z + 0.0).to_bits(),
    ]
}

/// Recomputes normals from the triangles: corners average the angle-weighted normals of neighbours (sharing
/// the welded position) within `angle` radians of their own triangle.
fn crease_normals(tris: &mut [Tri], angle: f32) {
    let cos_a = angle.cos();
    let normals: Vec<Vec3> = tris.iter().map(face_normal).collect();
    let mut around: HashMap<[u32; 3], Vec<(usize, usize)>> = HashMap::new();
    for (ti, t) in tris.iter().enumerate() {
        for k in 0..3 {
            around.entry(pos_key(t.c[k].p)).or_default().push((ti, k));
        }
    }
    let mut out = vec![[Vec3::ZERO; 3]; tris.len()];
    for (ti, t) in tris.iter().enumerate() {
        for k in 0..3 {
            let own = normals[ti];
            let mut sum = Vec3::ZERO;
            if let Some(list) = around.get(&pos_key(t.c[k].p)) {
                for &(oi, ok) in list {
                    if normals[oi].dot(own) >= cos_a - 1e-6 {
                        sum += normals[oi] * corner_angle(&tris[oi], ok);
                    }
                }
            }
            out[ti][k] = sum.normalize_or(own);
        }
    }
    for (t, n) in tris.iter_mut().zip(out) {
        for k in 0..3 {
            t.c[k].n = n[k];
        }
    }
}

/// Runs every finishing pass and returns the indexed mesh.
pub(crate) fn finish(mut b: Builder, desc: &MeshDesc) -> MeshData {
    let half = desc.half();
    weld(&mut b.tris, weld_tolerance(half));

    match desc.shading {
        Shading::Smooth => {}
        Shading::Flat => {
            for t in &mut b.tris {
                let n = face_normal(t);
                for c in &mut t.c {
                    c.n = n;
                }
            }
        }
        Shading::Crease { angle_deg } => crease_normals(&mut b.tris, angle_deg.to_radians()),
    }

    // Share identical corners.
    let mut mesh = MeshData::default();
    let mut charts: Vec<u32> = Vec::new();
    let mut lookup: HashMap<[u32; 9], u32> = HashMap::new();
    mesh.indices.reserve(b.tris.len() * 3);
    for t in &b.tris {
        for c in &t.c {
            let n = c.n.normalize_or(Vec3::Y);
            let key = [
                (c.p.x + 0.0).to_bits(),
                (c.p.y + 0.0).to_bits(),
                (c.p.z + 0.0).to_bits(),
                (n.x + 0.0).to_bits(),
                (n.y + 0.0).to_bits(),
                (n.z + 0.0).to_bits(),
                (c.uv.x + 0.0).to_bits(),
                (c.uv.y + 0.0).to_bits(),
                t.chart,
            ];
            let next = mesh.positions.len() as u32;
            let idx = *lookup.entry(key).or_insert_with(|| {
                mesh.positions.push(c.p.to_array());
                mesh.normals.push(n.to_array());
                mesh.uv0.push(c.uv.to_array());
                charts.push(t.chart);
                next
            });
            mesh.indices.push(idx);
        }
    }

    // uv1: each chart's uv0 range stretched to 0..1.
    let chart_count = charts.iter().max().map_or(0, |m| *m as usize + 1);
    let mut lo = vec![Vec2::splat(f32::MAX); chart_count];
    let mut hi = vec![Vec2::splat(f32::MIN); chart_count];
    for (uv, c) in mesh.uv0.iter().zip(&charts) {
        let uv = Vec2::from(*uv);
        lo[*c as usize] = lo[*c as usize].min(uv);
        hi[*c as usize] = hi[*c as usize].max(uv);
    }
    mesh.uv1 = mesh
        .uv0
        .iter()
        .zip(&charts)
        .map(|(uv, c)| {
            let (l, h) = (lo[*c as usize], hi[*c as usize]);
            let ext = h - l;
            let f = |v: f32, l: f32, e: f32| if e > 1e-9 { (v - l) / e } else { 0.0 };
            [f(uv[0], l.x, ext.x), f(uv[1], l.y, ext.y)]
        })
        .collect();

    mesh.ao = if desc.ao {
        crate::ao::bake(&mesh.positions, &mesh.normals, &mesh.indices)
    } else {
        vec![1.0; mesh.positions.len()]
    };

    crate::tangent::generate(&mut mesh);

    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for p in &mesh.positions {
        min = min.min(Vec3::from(*p));
        max = max.max(Vec3::from(*p));
    }
    if mesh.positions.is_empty() {
        min = Vec3::ZERO;
        max = Vec3::ZERO;
    }
    mesh.bounds = Aabb {
        min: min.to_array(),
        max: max.to_array(),
    };
    mesh
}
