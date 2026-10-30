//! Baked per-vertex ambient occlusion for concave shapes (stairs, arches, pipes, ramps, tori).
//!
//! Each vertex casts a fixed set of cosine-weighted rays over its hemisphere against the mesh itself, through
//! a small bounding volume hierarchy. Occluders count less with distance and stop counting at
//! [`MAX_DISTANCE`] studs, so the darkening sits in the creases and does not depend on the part's size.
use glam::Vec3;

/// Occluders further than this (studs) do not darken.
pub(crate) const MAX_DISTANCE: f32 = 1.0;
/// Rays per vertex.
const RAYS: usize = 48;
/// How dark full occlusion gets (1 = black). Kept soft: lighting adds its own shadows.
const STRENGTH: f32 = 0.85;

#[derive(Clone, Copy)]
struct Bounds {
    min: Vec3,
    max: Vec3,
}

impl Bounds {
    fn empty() -> Bounds {
        Bounds {
            min: Vec3::splat(f32::MAX),
            max: Vec3::splat(f32::MIN),
        }
    }
    fn grow(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }
    fn hit(&self, o: Vec3, inv: Vec3, tmax: f32) -> bool {
        let t1 = (self.min - o) * inv;
        let t2 = (self.max - o) * inv;
        let lo = t1.min(t2).max_element().max(0.0);
        let hi = t1.max(t2).min_element().min(tmax);
        lo <= hi
    }
}

enum Node {
    Leaf {
        bounds: Bounds,
        first: usize,
        count: usize,
    },
    Inner {
        bounds: Bounds,
        left: usize,
        right: usize,
    },
}

struct Bvh {
    nodes: Vec<Node>,
    tris: Vec<[Vec3; 3]>,
}

impl Bvh {
    fn build(mut tris: Vec<[Vec3; 3]>) -> Bvh {
        let mut nodes = Vec::new();
        let n = tris.len();
        Self::split(&mut tris, &mut nodes, 0, n);
        Bvh { nodes, tris }
    }

    fn split(tris: &mut [[Vec3; 3]], nodes: &mut Vec<Node>, first: usize, count: usize) -> usize {
        let mut b = Bounds::empty();
        let mut cb = Bounds::empty();
        for t in &tris[first..first + count] {
            for p in t {
                b.grow(*p);
            }
            cb.grow((t[0] + t[1] + t[2]) / 3.0);
        }
        let id = nodes.len();
        if count <= 4 {
            nodes.push(Node::Leaf {
                bounds: b,
                first,
                count,
            });
            return id;
        }
        let ext = cb.max - cb.min;
        let axis = if ext.x >= ext.y && ext.x >= ext.z {
            0
        } else if ext.y >= ext.z {
            1
        } else {
            2
        };
        let slice = &mut tris[first..first + count];
        slice.sort_by(|a, b| {
            let ca = a[0][axis] + a[1][axis] + a[2][axis];
            let cb = b[0][axis] + b[1][axis] + b[2][axis];
            ca.total_cmp(&cb)
        });
        nodes.push(Node::Leaf {
            bounds: b,
            first,
            count,
        });
        let half = count / 2;
        let left = Self::split(tris, nodes, first, half);
        let right = Self::split(tris, nodes, first + half, count - half);
        nodes[id] = Node::Inner {
            bounds: b,
            left,
            right,
        };
        id
    }

    /// Distance to the nearest hit within `tmax`, if any.
    fn nearest(&self, o: Vec3, d: Vec3, tmax: f32) -> Option<f32> {
        let inv = Vec3::ONE / d;
        let mut best = tmax;
        let mut found = false;
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            match &self.nodes[i] {
                Node::Leaf {
                    bounds,
                    first,
                    count,
                } => {
                    if !bounds.hit(o, inv, best) {
                        continue;
                    }
                    for t in &self.tris[*first..*first + *count] {
                        if let Some(h) = ray_triangle(o, d, t)
                            && h < best
                        {
                            best = h;
                            found = true;
                        }
                    }
                }
                Node::Inner {
                    bounds,
                    left,
                    right,
                } => {
                    if bounds.hit(o, inv, best) {
                        stack.push(*left);
                        stack.push(*right);
                    }
                }
            }
        }
        found.then_some(best)
    }
}

/// Möller-Trumbore, two-sided, hits closer than 1e-4 ignored.
fn ray_triangle(o: Vec3, d: Vec3, t: &[Vec3; 3]) -> Option<f32> {
    let e1 = t[1] - t[0];
    let e2 = t[2] - t[0];
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - t[0];
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let h = e2.dot(q) * inv;
    (h > 1e-4).then_some(h)
}

/// Cosine-weighted directions on the +Z hemisphere (a Fibonacci spiral: even, deterministic).
fn hemisphere() -> [Vec3; RAYS] {
    let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
    std::array::from_fn(|i| {
        let r = ((i as f32 + 0.5) / RAYS as f32).sqrt();
        let a = golden * i as f32;
        Vec3::new(r * a.cos(), r * a.sin(), (1.0 - r * r).max(0.0).sqrt())
    })
}

/// Ambient occlusion per vertex, 1 = open.
pub(crate) fn bake(positions: &[[f32; 3]], normals: &[[f32; 3]], indices: &[u32]) -> Vec<f32> {
    let tris: Vec<[Vec3; 3]> = indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| [0, 1, 2].map(|k| Vec3::from(positions[t[k] as usize])))
        .collect();
    let bvh = Bvh::build(tris);
    let dirs = hemisphere();
    positions
        .iter()
        .zip(normals)
        .map(|(p, n)| {
            let n = Vec3::from(*n);
            let f = crate::math::Frame::for_normal(n, Vec3::ZERO);
            let o = Vec3::from(*p) + n * 2e-3;
            let mut occlusion = 0.0;
            for d in &dirs {
                let w = (f.u * d.x + f.v * d.y + n * d.z).normalize_or(n);
                if let Some(h) = bvh.nearest(o, w, MAX_DISTANCE) {
                    let x = h / MAX_DISTANCE;
                    occlusion += 1.0 - x * x;
                }
            }
            1.0 - STRENGTH * occlusion / RAYS as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_under_a_roof_is_occluded() {
        // A floor quad (normal +Y) under a wider roof quad 0.25 studs above it.
        let positions = [
            [-1.0, 0.0, -1.0],
            [1.0, 0.0, -1.0],
            [1.0, 0.0, 1.0],
            [-1.0, 0.0, 1.0],
            [-3.0, 0.25, -3.0],
            [3.0, 0.25, -3.0],
            [3.0, 0.25, 3.0],
            [-3.0, 0.25, 3.0],
        ];
        let normals = [[0.0, 1.0, 0.0]; 8];
        let indices = [0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7];
        let ao = bake(&positions, &normals, &indices);
        assert!(ao[0] < 0.6, "{ao:?}");
        assert!(ao[4] > 0.99, "{ao:?}");
    }
}
