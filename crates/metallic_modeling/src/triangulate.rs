//! Triangulation of the polygon mesh into GPU-ready [`MeshData`], with normals that respect sharp edges.
//!
//! `MeshData` mirrors the field names of `metallic_meshgen::MeshData` (positions, normals, uv0, indices, bounds) so
//! that switching to the shared type once `metallic_meshgen` (#35) is merged is a rename; the meshgen type also carries
//! tangents, a second UV set and baked AO, which the modeller does not produce yet.

use std::collections::HashMap;

use glam::{Vec2, Vec3};

use crate::geom;
use crate::ids::{FaceId, VertId};
use crate::mesh::{FxBuild, Mesh};

/// Axis-aligned bounds.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Bounds {
    /// Smallest corner.
    pub min: [f32; 3],
    /// Largest corner.
    pub max: [f32; 3],
}

/// An indexed triangle mesh ready for upload or export.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshData {
    /// Vertex positions.
    pub positions: Vec<[f32; 3]>,
    /// Unit vertex normals.
    pub normals: Vec<[f32; 3]>,
    /// Texture coordinates (from the faces' corner UVs, v flipped to point down as glTF and GPUs expect).
    pub uv0: Vec<[f32; 2]>,
    /// Triangle list, counter-clockwise seen from the front.
    pub indices: Vec<u32>,
    /// Bounds of `positions`.
    pub bounds: Bounds,
}

impl MeshData {
    /// Number of triangles.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// How vertex normals are shared between neighbouring faces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NormalMode {
    /// Every face is flat (no sharing).
    Flat,
    /// Normals are shared across every edge that is not marked sharp and not a boundary.
    Smooth,
    /// Like `Smooth`, but faces meeting at more than `angle_degrees` are also split ("auto smooth").
    AutoSmooth {
        /// Largest angle between face normals that still shades smoothly.
        angle_degrees: f32,
    },
}

impl Default for NormalMode {
    fn default() -> Self {
        NormalMode::AutoSmooth {
            angle_degrees: 30.0,
        }
    }
}

/// Triangles plus which polygon each came from (for picking and highlighting).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Triangulated {
    /// The GPU-ready mesh.
    pub data: MeshData,
    /// `triangle_faces[t]` is the face that triangle `t` belongs to.
    pub triangle_faces: Vec<FaceId>,
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Triangulates every face (ear clipping for n-gons) and computes normals per `mode`. Output vertices are split
/// wherever the normal group or the UV differs.
#[must_use]
pub fn triangulate(mesh: &Mesh, mode: NormalMode) -> Triangulated {
    let (_, _, face_slots) = mesh.slot_counts();
    let mut weighted = vec![Vec3::ZERO; face_slots];
    let mut unit = vec![Vec3::ZERO; face_slots];
    for (f, _) in mesh.faces() {
        let n = geom::newell(&mesh.face_positions(f));
        weighted[f.index()] = n;
        unit[f.index()] = n.normalize_or_zero();
    }
    // Per (vertex, face): representative face of its smoothing group and the group normal.
    let mut corner: HashMap<(VertId, FaceId), (FaceId, Vec3), FxBuild> = HashMap::default();
    if !matches!(mode, NormalMode::Flat) {
        let cos_limit = match mode {
            NormalMode::AutoSmooth { angle_degrees } => angle_degrees.to_radians().cos(),
            _ => -2.0,
        };
        for (v, _) in mesh.verts() {
            let faces = mesh.vert_faces(v);
            if faces.is_empty() {
                continue;
            }
            let mut parent: Vec<usize> = (0..faces.len()).collect();
            for &e in mesh.vert_edges(v) {
                let around = mesh.edge_faces(e);
                if around.len() != 2 || mesh.edge(e).is_some_and(|x| x.flags.sharp) {
                    continue;
                }
                if unit[around[0].index()].dot(unit[around[1].index()]) < cos_limit {
                    continue;
                }
                if let (Ok(a), Ok(b)) = (
                    faces.binary_search(&around[0]),
                    faces.binary_search(&around[1]),
                ) {
                    let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                    if ra != rb {
                        parent[ra.max(rb)] = ra.min(rb);
                    }
                }
            }
            let mut sums = vec![Vec3::ZERO; faces.len()];
            for (i, f) in faces.iter().enumerate() {
                let r = find(&mut parent, i);
                sums[r] += weighted[f.index()];
            }
            for (i, &f) in faces.iter().enumerate() {
                let r = find(&mut parent, i);
                let n = sums[r].normalize_or(unit[f.index()]);
                corner.insert((v, f), (faces[r], n));
            }
        }
    }
    let mut out = Triangulated::default();
    let mut index: HashMap<(u32, u32, u32, u32), u32, FxBuild> = HashMap::default();
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for (f, face) in mesh.faces() {
        let pts = mesh.face_positions(f);
        let ids: Vec<u32> = face
            .verts
            .iter()
            .zip(&face.uvs)
            .zip(&pts)
            .map(|((&v, &uv), &p)| {
                let (group, n) = corner.get(&(v, f)).copied().unwrap_or((f, unit[f.index()]));
                let key = (v.0, group.0, uv.x.to_bits(), uv.y.to_bits());
                *index.entry(key).or_insert_with(|| {
                    out.data.positions.push(p.to_array());
                    out.data.normals.push(n.to_array());
                    out.data.uv0.push(uv_flip(uv));
                    lo = lo.min(p);
                    hi = hi.max(p);
                    (out.data.positions.len() - 1) as u32
                })
            })
            .collect();
        for t in geom::triangulate_polygon(&pts) {
            out.data.indices.extend([ids[t[0]], ids[t[1]], ids[t[2]]]);
            out.triangle_faces.push(f);
        }
    }
    if !out.data.positions.is_empty() {
        out.data.bounds = Bounds {
            min: lo.to_array(),
            max: hi.to_array(),
        };
    }
    out
}

/// UVs are stored with v up (maths convention); glTF and GPUs expect v down.
fn uv_flip(uv: Vec2) -> [f32; 2] {
    [uv.x, 1.0 - uv.y]
}
