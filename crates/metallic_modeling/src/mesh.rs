//! The editable polygon mesh: vertices, edges and n-gon faces with stable ids and derived adjacency.
//!
//! Representation (see `docs/agents/136-modeling.md` for the comparison): a BMesh-style radial-edge structure kept in
//! index arenas instead of linked lists.
//! - **Primary data** is what the user edits and what undo restores: vertices (position), edges (two vertices, flags)
//!   and faces (an ordered loop of vertices plus one UV per corner).
//! - **Derived adjacency** is rebuilt incrementally by the only three mutation primitives and never stored in undo
//!   data: vertex → edges (the "disk"), edge → faces (the "radial" list) and a vertex-pair → edge lookup. Lists are
//!   kept sorted, so adjacency is a pure function of the primary data and every algorithm is deterministic.
//! - Non-manifold geometry (wire edges, loose vertices, edges with three or more faces) is representable; operations
//!   that need manifold input check for it and return [`ModelError::NonManifold`].
//!
//! Every mutation goes through a journaling primitive. [`Mesh::record`] runs an operation with the journal on and
//! returns a [`ChangeSet`] holding the before and after state of every touched element; applying it backwards is undo
//! and forwards is redo, and both give back the exact same ids.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{BuildHasherDefault, Hasher};

use glam::{Vec2, Vec3};

use crate::error::{ModelError, ModelResult};
use crate::geom;
use crate::ids::{Arena, EdgeId, FaceId, VertId};

/// A vertex: just a position. Normals are derived when triangulating.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    /// Position in model space (studs; Y is up).
    pub position: Vec3,
}

/// Per-edge marks used by shading and UV unwrapping.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct EdgeFlags {
    /// Shading splits normals across this edge (a hard edge).
    pub sharp: bool,
    /// UV unwrapping cuts the surface along this edge.
    pub seam: bool,
}

/// An edge between two distinct vertices. `verts` is stored in ascending id order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edge {
    /// The two end vertices, lower id first.
    pub verts: [VertId; 2],
    /// Sharp and seam marks.
    pub flags: EdgeFlags,
}

impl Edge {
    /// The end that is not `v` (or `v`'s partner when `v` is not on this edge: the first vertex).
    #[must_use]
    pub fn other(&self, v: VertId) -> VertId {
        if self.verts[0] == v { self.verts[1] } else { self.verts[0] }
    }

    /// Whether `v` is one of the ends.
    #[must_use]
    pub fn has(&self, v: VertId) -> bool {
        self.verts[0] == v || self.verts[1] == v
    }
}

/// A polygon face: an ordered loop of at least three distinct vertices, counter-clockwise seen from the front, and one
/// UV per corner.
#[derive(Debug, Clone, PartialEq)]
pub struct Face {
    /// Corner vertices in winding order.
    pub verts: Vec<VertId>,
    /// One UV per corner, same length as `verts`.
    pub uvs: Vec<Vec2>,
}

impl Face {
    /// Index of `v` in the loop.
    #[must_use]
    pub fn position_of(&self, v: VertId) -> Option<usize> {
        self.verts.iter().position(|&x| x == v)
    }

    /// The corner before and after `v` in winding order.
    #[must_use]
    pub fn neighbours(&self, v: VertId) -> Option<(VertId, VertId)> {
        let i = self.position_of(v)?;
        let n = self.verts.len();
        Some((self.verts[(i + n - 1) % n], self.verts[(i + 1) % n]))
    }

    /// Whether the loop walks from `a` straight to `b`.
    #[must_use]
    pub fn has_directed_edge(&self, a: VertId, b: VertId) -> bool {
        self.position_of(a).is_some_and(|i| self.verts[(i + 1) % self.verts.len()] == b)
    }

    /// Consecutive vertex pairs `(v[i], v[i+1])`, wrapping around.
    pub fn directed_edges(&self) -> impl Iterator<Item = (VertId, VertId)> + '_ {
        let n = self.verts.len();
        (0..n).map(move |i| (self.verts[i], self.verts[(i + 1) % n]))
    }
}

/// The before and after state of every element an operation touched. Produced by [`Mesh::record`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChangeSet {
    pub(crate) verts: BTreeMap<VertId, (Option<Vertex>, Option<Vertex>)>,
    pub(crate) edges: BTreeMap<EdgeId, (Option<Edge>, Option<Edge>)>,
    pub(crate) faces: BTreeMap<FaceId, (Option<Face>, Option<Face>)>,
}

impl ChangeSet {
    /// Whether nothing changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.verts.is_empty() && self.edges.is_empty() && self.faces.is_empty()
    }

    /// Number of touched elements (a rough size measure for history limits).
    #[must_use]
    pub fn len(&self) -> usize {
        self.verts.len() + self.edges.len() + self.faces.len()
    }

    /// Drops entries whose before and after are equal.
    fn prune(&mut self) {
        self.verts.retain(|_, (a, b)| a != b);
        self.edges.retain(|_, (a, b)| a != b);
        self.faces.retain(|_, (a, b)| a != b);
    }
}

/// The ordered ring of faces around a vertex, from [`Mesh::vert_fan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fan {
    /// Faces in winding order around the vertex (each face's outgoing edge is the next face's incoming edge).
    pub faces: Vec<FaceId>,
    /// `edges[i]` is the incoming edge of `faces[i]` at the vertex; an open fan has one extra edge at the end (the
    /// outgoing edge of the last face).
    pub edges: Vec<EdgeId>,
    /// Whether the ring closes (interior vertex) or is open (boundary vertex).
    pub closed: bool,
}

/// FxHash-style hasher for the vertex-pair lookup (fast; the keys are small integers, not attacker controlled).
#[derive(Default, Clone, Copy)]
pub(crate) struct FxHasher(u64);

impl Hasher for FxHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(u64::from(b));
        }
    }
    fn write_u32(&mut self, i: u32) {
        self.write_u64(u64::from(i));
    }
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

pub(crate) type FxBuild = BuildHasherDefault<FxHasher>;

fn key(a: VertId, b: VertId) -> (VertId, VertId) {
    if a <= b { (a, b) } else { (b, a) }
}

fn insert_sorted<T: Ord + Copy>(list: &mut Vec<T>, x: T) {
    if let Err(i) = list.binary_search(&x) {
        list.insert(i, x);
    }
}

fn remove_sorted<T: Ord + Copy>(list: &mut Vec<T>, x: T) {
    if let Ok(i) = list.binary_search(&x) {
        list.remove(i);
    }
}

/// An editable polygon mesh. See the module docs for the representation.
#[derive(Debug, Clone, Default)]
pub struct Mesh {
    verts: Arena<Vertex>,
    edges: Arena<Edge>,
    faces: Arena<Face>,
    vert_edges: Vec<Vec<EdgeId>>,
    edge_faces: Vec<Vec<FaceId>>,
    edge_lookup: HashMap<(VertId, VertId), EdgeId, FxBuild>,
    journal: Option<ChangeSet>,
}

impl PartialEq for Mesh {
    /// Content equality: same live elements under the same ids (tombstones and adjacency order are ignored).
    fn eq(&self, other: &Self) -> bool {
        self.verts.iter().eq(other.verts.iter())
            && self.edges.iter().eq(other.edges.iter())
            && self.faces.iter().eq(other.faces.iter())
    }
}

impl Mesh {
    /// An empty mesh.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a mesh from positions and polygons (each a list of indices into `positions`, counter-clockwise).
    ///
    /// # Errors
    /// [`ModelError::InvalidFace`] when a polygon has fewer than three distinct or out-of-range indices.
    pub fn from_polygons(positions: &[Vec3], polygons: &[Vec<u32>]) -> ModelResult<Self> {
        let mut mesh = Self::new();
        let ids: Vec<VertId> = positions.iter().map(|&p| mesh.add_vert(p)).collect();
        for poly in polygons {
            let verts = poly
                .iter()
                .map(|&i| ids.get(i as usize).copied().ok_or_else(|| ModelError::InvalidFace(format!("index {i} out of range"))))
                .collect::<ModelResult<Vec<_>>>()?;
            mesh.add_face(&verts)?;
        }
        Ok(mesh)
    }

    // ----------------------------------------------------------------------------------------------------------------
    // Queries
    // ----------------------------------------------------------------------------------------------------------------

    /// Number of live vertices.
    #[must_use]
    pub fn vert_count(&self) -> usize {
        self.verts.len()
    }

    /// Number of live edges.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Number of live faces.
    #[must_use]
    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    /// Slots in use including tombstones (vertices, edges, faces); [`Mesh::compacted`] drops the tombstones.
    #[must_use]
    pub fn slot_counts(&self) -> (usize, usize, usize) {
        (self.verts.slot_count(), self.edges.slot_count(), self.faces.slot_count())
    }

    /// The vertex with this id, if live.
    #[must_use]
    pub fn vert(&self, id: VertId) -> Option<&Vertex> {
        self.verts.get(id.0)
    }

    /// The edge with this id, if live.
    #[must_use]
    pub fn edge(&self, id: EdgeId) -> Option<&Edge> {
        self.edges.get(id.0)
    }

    /// The face with this id, if live.
    #[must_use]
    pub fn face(&self, id: FaceId) -> Option<&Face> {
        self.faces.get(id.0)
    }

    /// Live vertices in id order.
    pub fn verts(&self) -> impl Iterator<Item = (VertId, &Vertex)> + '_ {
        self.verts.iter().map(|(i, v)| (VertId(i), v))
    }

    /// Live edges in id order.
    pub fn edges(&self) -> impl Iterator<Item = (EdgeId, &Edge)> + '_ {
        self.edges.iter().map(|(i, e)| (EdgeId(i), e))
    }

    /// Live faces in id order.
    pub fn faces(&self) -> impl Iterator<Item = (FaceId, &Face)> + '_ {
        self.faces.iter().map(|(i, f)| (FaceId(i), f))
    }

    /// Ids of live vertices in order.
    #[must_use]
    pub fn vert_ids(&self) -> Vec<VertId> {
        self.verts().map(|(id, _)| id).collect()
    }

    /// Ids of live edges in order.
    #[must_use]
    pub fn edge_ids(&self) -> Vec<EdgeId> {
        self.edges().map(|(id, _)| id).collect()
    }

    /// Ids of live faces in order.
    #[must_use]
    pub fn face_ids(&self) -> Vec<FaceId> {
        self.faces().map(|(id, _)| id).collect()
    }

    /// Position of a vertex.
    ///
    /// # Errors
    /// [`ModelError::MissingVert`] when the id is not live.
    pub fn position(&self, id: VertId) -> ModelResult<Vec3> {
        self.vert(id).map(|v| v.position).ok_or(ModelError::MissingVert(id))
    }

    /// Position of a vertex, or the origin when it is missing (for drawing code that must not fail).
    #[must_use]
    pub fn position_or_zero(&self, id: VertId) -> Vec3 {
        self.vert(id).map_or(Vec3::ZERO, |v| v.position)
    }

    pub(crate) fn face_ref(&self, id: FaceId) -> ModelResult<&Face> {
        self.face(id).ok_or(ModelError::MissingFace(id))
    }

    pub(crate) fn edge_ref(&self, id: EdgeId) -> ModelResult<&Edge> {
        self.edge(id).ok_or(ModelError::MissingEdge(id))
    }

    /// The edge joining `a` and `b`, if any.
    #[must_use]
    pub fn find_edge(&self, a: VertId, b: VertId) -> Option<EdgeId> {
        self.edge_lookup.get(&key(a, b)).copied()
    }

    /// Edges touching a vertex, ascending.
    #[must_use]
    pub fn vert_edges(&self, v: VertId) -> &[EdgeId] {
        self.vert_edges.get(v.index()).map_or(&[], Vec::as_slice)
    }

    /// Faces using an edge, ascending.
    #[must_use]
    pub fn edge_faces(&self, e: EdgeId) -> &[FaceId] {
        if self.edge(e).is_none() {
            return &[];
        }
        self.edge_faces.get(e.index()).map_or(&[], Vec::as_slice)
    }

    /// Faces using a vertex, ascending and without duplicates.
    #[must_use]
    pub fn vert_faces(&self, v: VertId) -> Vec<FaceId> {
        let mut out: Vec<FaceId> = self.vert_edges(v).iter().flat_map(|&e| self.edge_faces(e).iter().copied()).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Vertices joined to `v` by an edge, ascending.
    #[must_use]
    pub fn vert_neighbours(&self, v: VertId) -> Vec<VertId> {
        let mut out: Vec<VertId> = self.vert_edges(v).iter().filter_map(|&e| self.edge(e).map(|e| e.other(v))).collect();
        out.sort_unstable();
        out
    }

    /// The edges of a face in winding order (`edges[i]` joins corner `i` and `i + 1`).
    #[must_use]
    pub fn face_edges(&self, f: FaceId) -> Vec<EdgeId> {
        self.face(f).map_or_else(Vec::new, |face| face.directed_edges().filter_map(|(a, b)| self.find_edge(a, b)).collect())
    }

    /// Corner positions of a face.
    #[must_use]
    pub fn face_positions(&self, f: FaceId) -> Vec<Vec3> {
        self.face(f).map_or_else(Vec::new, |face| face.verts.iter().map(|&v| self.position_or_zero(v)).collect())
    }

    /// Unit normal of a face (Newell), zero when degenerate or missing.
    #[must_use]
    pub fn face_normal(&self, f: FaceId) -> Vec3 {
        geom::polygon_normal(&self.face_positions(f))
    }

    /// Centre (vertex average) of a face.
    #[must_use]
    pub fn face_center(&self, f: FaceId) -> Vec3 {
        geom::centroid(&self.face_positions(f))
    }

    /// Area of a face.
    #[must_use]
    pub fn face_area(&self, f: FaceId) -> f32 {
        geom::polygon_area(&self.face_positions(f))
    }

    /// Area-weighted average normal of the faces around a vertex.
    #[must_use]
    pub fn vert_normal(&self, v: VertId) -> Vec3 {
        self.vert_faces(v)
            .iter()
            .map(|&f| geom::newell(&self.face_positions(f)))
            .sum::<Vec3>()
            .normalize_or_zero()
    }

    /// Midpoint of an edge.
    #[must_use]
    pub fn edge_midpoint(&self, e: EdgeId) -> Vec3 {
        self.edge(e).map_or(Vec3::ZERO, |e| (self.position_or_zero(e.verts[0]) + self.position_or_zero(e.verts[1])) * 0.5)
    }

    /// Whether an edge has exactly one face.
    #[must_use]
    pub fn is_boundary_edge(&self, e: EdgeId) -> bool {
        self.edge_faces(e).len() == 1
    }

    /// Axis-aligned bounds of all vertices (`None` for an empty mesh).
    #[must_use]
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut it = self.verts().map(|(_, v)| v.position);
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
    }

    /// The ordered ring of faces around `v`, or `None` when the neighbourhood is not a single manifold, consistently
    /// wound fan (wire edges, three faces on an edge, a bow-tie, or flipped neighbours), or `v` has no faces.
    #[must_use]
    pub fn vert_fan(&self, v: VertId) -> Option<Fan> {
        let faces = self.vert_faces(v);
        if faces.is_empty() {
            return None;
        }
        let mut info: Vec<(FaceId, EdgeId, EdgeId)> = Vec::with_capacity(faces.len());
        let mut by_in: BTreeMap<EdgeId, FaceId> = BTreeMap::new();
        for &f in &faces {
            let face = self.face(f)?;
            let (prev, next) = face.neighbours(v)?;
            let e_in = self.find_edge(prev, v)?;
            let e_out = self.find_edge(v, next)?;
            if by_in.insert(e_in, f).is_some() {
                return None;
            }
            info.push((f, e_in, e_out));
        }
        let lookup = |f: FaceId| info.iter().find(|x| x.0 == f).copied();
        let boundary_starts: Vec<&(FaceId, EdgeId, EdgeId)> =
            info.iter().filter(|(_, e_in, _)| self.edge_faces(*e_in).len() == 1).collect();
        let start = match boundary_starts.len() {
            0 => info[0],
            1 => *boundary_starts[0],
            _ => return None,
        };
        let mut fan = Fan { faces: Vec::new(), edges: Vec::new(), closed: false };
        let mut cur = start;
        loop {
            fan.faces.push(cur.0);
            fan.edges.push(cur.1);
            let around = self.edge_faces(cur.2);
            match around.len() {
                1 => {
                    fan.edges.push(cur.2);
                    break;
                }
                2 => {
                    let next = if around[0] == cur.0 { around[1] } else { around[0] };
                    if by_in.get(&cur.2) != Some(&next) {
                        return None;
                    }
                    if next == start.0 {
                        fan.closed = true;
                        break;
                    }
                    if fan.faces.len() > faces.len() {
                        return None;
                    }
                    cur = lookup(next)?;
                }
                _ => return None,
            }
        }
        if fan.faces.len() != faces.len() || fan.edges.len() != self.vert_edges(v).len() {
            return None;
        }
        Some(fan)
    }

    /// Euler characteristic `V - E + F` (2 for a closed sphere-like surface, 1 for a disc).
    #[must_use]
    pub fn euler_characteristic(&self) -> i64 {
        self.vert_count() as i64 - self.edge_count() as i64 + self.face_count() as i64
    }

    /// Edges with exactly one face.
    #[must_use]
    pub fn boundary_edges(&self) -> Vec<EdgeId> {
        self.edges().map(|(id, _)| id).filter(|&e| self.is_boundary_edge(e)).collect()
    }

    /// Edges with no face.
    #[must_use]
    pub fn wire_edges(&self) -> Vec<EdgeId> {
        self.edges().map(|(id, _)| id).filter(|&e| self.edge_faces(e).is_empty()).collect()
    }

    /// Vertices with no edge.
    #[must_use]
    pub fn loose_verts(&self) -> Vec<VertId> {
        self.verts().map(|(id, _)| id).filter(|&v| self.vert_edges(v).is_empty()).collect()
    }

    /// Whether every edge has one or two faces and every vertex with faces has a single consistent fan.
    #[must_use]
    pub fn is_manifold(&self) -> bool {
        self.edges().all(|(e, _)| matches!(self.edge_faces(e).len(), 1 | 2))
            && self.verts().all(|(v, _)| self.vert_fan(v).is_some())
    }

    /// Whether every edge has exactly two faces (a closed, watertight surface) and the surface is manifold.
    #[must_use]
    pub fn is_closed_manifold(&self) -> bool {
        self.edges().all(|(e, _)| self.edge_faces(e).len() == 2) && self.is_manifold()
    }

    /// Whether every two-face edge is walked once in each direction (neighbouring faces agree on winding).
    #[must_use]
    pub fn is_consistently_oriented(&self) -> bool {
        self.edges().all(|(e, edge)| {
            let faces = self.edge_faces(e);
            if faces.len() != 2 {
                return true;
            }
            let [a, b] = edge.verts;
            let forward = faces.iter().filter(|&&f| self.face(f).is_some_and(|x| x.has_directed_edge(a, b))).count();
            forward == 1
        })
    }

    /// Signed volume enclosed by the faces (positive for a closed mesh wound outwards).
    #[must_use]
    pub fn signed_volume(&self) -> f32 {
        let mut vol = 0.0;
        for (f, _) in self.faces() {
            let pts = self.face_positions(f);
            for t in geom::triangulate_polygon(&pts) {
                vol += pts[t[0]].dot(pts[t[1]].cross(pts[t[2]]));
            }
        }
        vol / 6.0
    }

    /// Checks every structural invariant: faces have three or more distinct live vertices and an edge per side, edges
    /// join two distinct live vertices with no duplicates, and the derived adjacency matches the primary data.
    ///
    /// # Errors
    /// [`ModelError::Invariant`] naming the first broken rule.
    pub fn validate(&self) -> ModelResult<()> {
        let bad = |msg: String| Err(ModelError::Invariant(msg));
        let mut expect_edge_faces: BTreeMap<EdgeId, Vec<FaceId>> = BTreeMap::new();
        for (f, face) in self.faces() {
            if face.verts.len() < 3 {
                return bad(format!("face {f:?} has {} corners", face.verts.len()));
            }
            if face.uvs.len() != face.verts.len() {
                return bad(format!("face {f:?} has {} uvs for {} corners", face.uvs.len(), face.verts.len()));
            }
            let set: BTreeSet<VertId> = face.verts.iter().copied().collect();
            if set.len() != face.verts.len() {
                return bad(format!("face {f:?} repeats a vertex"));
            }
            for (a, b) in face.directed_edges() {
                if self.vert(a).is_none() {
                    return bad(format!("face {f:?} uses dead vertex {a:?}"));
                }
                let Some(e) = self.find_edge(a, b) else {
                    return bad(format!("face {f:?} side {a:?}-{b:?} has no edge"));
                };
                expect_edge_faces.entry(e).or_default().push(f);
            }
        }
        let mut seen_keys = BTreeSet::new();
        let mut expect_vert_edges: BTreeMap<VertId, Vec<EdgeId>> = BTreeMap::new();
        for (e, edge) in self.edges() {
            let [a, b] = edge.verts;
            if a >= b {
                return bad(format!("edge {e:?} vertices not ascending or equal"));
            }
            if self.vert(a).is_none() || self.vert(b).is_none() {
                return bad(format!("edge {e:?} uses a dead vertex (dangling)"));
            }
            if !seen_keys.insert((a, b)) {
                return bad(format!("edge {e:?} duplicates another edge"));
            }
            if self.find_edge(a, b) != Some(e) {
                return bad(format!("edge lookup disagrees for {e:?}"));
            }
            expect_vert_edges.entry(a).or_default().push(e);
            expect_vert_edges.entry(b).or_default().push(e);
            let mut want = expect_edge_faces.remove(&e).unwrap_or_default();
            want.sort_unstable();
            if self.edge_faces(e) != want.as_slice() {
                return bad(format!("edge {e:?} radial list {:?} != {want:?}", self.edge_faces(e)));
            }
        }
        if self.edge_lookup.len() != self.edge_count() {
            return bad("edge lookup has stale entries".into());
        }
        for (v, _) in self.verts() {
            let want = expect_vert_edges.remove(&v).unwrap_or_default();
            if self.vert_edges(v) != want.as_slice() {
                return bad(format!("vertex {v:?} disk list mismatch"));
            }
        }
        if let Some((v, _)) = expect_vert_edges.into_iter().next() {
            return bad(format!("edge references missing vertex {v:?}"));
        }
        Ok(())
    }

    /// Deterministic 64-bit hash of the content (ids, positions, edges, flags, faces, UVs). Equal meshes hash equal.
    #[must_use]
    pub fn content_hash(&self) -> u64 {
        let mut h = Fnv(0xcbf2_9ce4_8422_2325);
        for (id, v) in self.verts() {
            h.u32(id.0);
            for c in v.position.to_array() {
                h.u32(c.to_bits());
            }
        }
        h.u32(0xffff_fff1);
        for (id, e) in self.edges() {
            h.u32(id.0);
            h.u32(e.verts[0].0);
            h.u32(e.verts[1].0);
            h.u32(u32::from(e.flags.sharp) | (u32::from(e.flags.seam) << 1));
        }
        h.u32(0xffff_fff2);
        for (id, f) in self.faces() {
            h.u32(id.0);
            h.u32(f.verts.len() as u32);
            for v in &f.verts {
                h.u32(v.0);
            }
            for uv in &f.uvs {
                h.u32(uv.x.to_bits());
                h.u32(uv.y.to_bits());
            }
        }
        h.0
    }

    /// A copy with ids renumbered densely in the current order (tombstones dropped). Not undoable: use it when saving
    /// or when starting a fresh editing session.
    #[must_use]
    pub fn compacted(&self) -> Mesh {
        let mut out = Mesh::new();
        let mut map = BTreeMap::new();
        for (id, v) in self.verts() {
            map.insert(id, out.add_vert(v.position));
        }
        for (_, e) in self.edges() {
            if let (Some(&a), Some(&b)) = (map.get(&e.verts[0]), map.get(&e.verts[1])) {
                if let Ok(new) = out.add_edge(a, b) {
                    out.set_edge_flags(new, e.flags);
                }
            }
        }
        for (_, f) in self.faces() {
            let verts: Option<Vec<VertId>> = f.verts.iter().map(|v| map.get(v).copied()).collect();
            if let Some(verts) = verts {
                // Faces that were valid stay valid under a bijective renumbering.
                let _ = out.add_face_with_uvs(&verts, &f.uvs);
            }
        }
        out
    }

    // ----------------------------------------------------------------------------------------------------------------
    // Journal
    // ----------------------------------------------------------------------------------------------------------------

    /// Runs `op` with change recording on and returns its result with the [`ChangeSet`]. On error every change is
    /// rolled back and the mesh is exactly as before.
    ///
    /// # Errors
    /// Whatever `op` returns.
    pub fn record<R>(&mut self, op: impl FnOnce(&mut Mesh) -> ModelResult<R>) -> ModelResult<(R, ChangeSet)> {
        let outer = self.journal.replace(ChangeSet::default());
        let result = op(self);
        let mut changes = self.journal.take().unwrap_or_default();
        match result {
            Ok(r) => {
                changes.prune();
                self.journal = outer;
                // Fold into an enclosing recording, if any.
                if self.journal.is_some() {
                    self.merge_into_journal(&changes);
                }
                Ok((r, changes))
            }
            Err(e) => {
                // Roll back with recording off, then restore the enclosing recording untouched.
                self.apply_changes(&changes, false);
                self.journal = outer;
                Err(e)
            }
        }
    }

    fn merge_into_journal(&mut self, changes: &ChangeSet) {
        let Some(j) = self.journal.as_mut() else { return };
        for (id, (before, after)) in &changes.verts {
            j.verts.entry(*id).or_insert((*before, None)).1 = *after;
        }
        for (id, (before, after)) in &changes.edges {
            j.edges.entry(*id).or_insert((*before, None)).1 = *after;
        }
        for (id, (before, after)) in &changes.faces {
            j.faces.entry(*id).or_insert_with(|| (before.clone(), None)).1 = after.clone();
        }
    }

    /// Applies a change set forwards (redo) or backwards (undo). Elements are written in an order that keeps the
    /// derived adjacency consistent: faces out, edges out, vertices, edges in, faces in.
    pub fn apply_changes(&mut self, changes: &ChangeSet, forward: bool) {
        let pick = |pair: &(Option<Face>, Option<Face>)| if forward { pair.1.clone() } else { pair.0.clone() };
        for id in changes.faces.keys() {
            self.raw_face(*id, None);
        }
        for id in changes.edges.keys() {
            self.raw_edge(*id, None);
        }
        for (id, (before, after)) in &changes.verts {
            self.raw_vert(*id, if forward { *after } else { *before });
        }
        for (id, (before, after)) in &changes.edges {
            self.raw_edge(*id, if forward { *after } else { *before });
        }
        for (id, pair) in &changes.faces {
            self.raw_face(*id, pick(pair));
        }
        if self.journal.is_some() {
            // Applying a change set inside a recording (rare) is recorded as well.
            let mut inner = changes.clone();
            if !forward {
                for pair in inner.verts.values_mut() {
                    std::mem::swap(&mut pair.0, &mut pair.1);
                }
                for pair in inner.edges.values_mut() {
                    std::mem::swap(&mut pair.0, &mut pair.1);
                }
                for pair in inner.faces.values_mut() {
                    std::mem::swap(&mut pair.0, &mut pair.1);
                }
            }
            self.merge_into_journal(&inner);
        }
    }

    // ----------------------------------------------------------------------------------------------------------------
    // Raw slot writes (maintain adjacency, no journal, no validation)
    // ----------------------------------------------------------------------------------------------------------------

    fn raw_vert(&mut self, id: VertId, value: Option<Vertex>) -> Option<Vertex> {
        if self.vert_edges.len() <= id.index() {
            self.vert_edges.resize_with(id.index() + 1, Vec::new);
        }
        self.verts.set(id.0, value)
    }

    fn raw_edge(&mut self, id: EdgeId, value: Option<Edge>) -> Option<Edge> {
        if self.edge_faces.len() <= id.index() {
            self.edge_faces.resize_with(id.index() + 1, Vec::new);
        }
        let old = self.edges.set(id.0, value);
        if let Some(old) = old {
            if self.edge_lookup.get(&key(old.verts[0], old.verts[1])) == Some(&id) {
                self.edge_lookup.remove(&key(old.verts[0], old.verts[1]));
            }
            for v in old.verts {
                if let Some(list) = self.vert_edges.get_mut(v.index()) {
                    remove_sorted(list, id);
                }
            }
        }
        if let Some(new) = value {
            self.edge_lookup.insert(key(new.verts[0], new.verts[1]), id);
            for v in new.verts {
                if self.vert_edges.len() <= v.index() {
                    self.vert_edges.resize_with(v.index() + 1, Vec::new);
                }
                insert_sorted(&mut self.vert_edges[v.index()], id);
            }
        }
        old
    }

    fn raw_face(&mut self, id: FaceId, value: Option<Face>) -> Option<Face> {
        let sides: Vec<(VertId, VertId)> = value.as_ref().map(|f| f.directed_edges().collect()).unwrap_or_default();
        let old = self.faces.set(id.0, value);
        if let Some(old) = &old {
            for (a, b) in old.directed_edges() {
                if let Some(e) = self.find_edge(a, b) {
                    if let Some(list) = self.edge_faces.get_mut(e.index()) {
                        remove_sorted(list, id);
                    }
                }
            }
        }
        for (a, b) in sides {
            if let Some(e) = self.find_edge(a, b) {
                if self.edge_faces.len() <= e.index() {
                    self.edge_faces.resize_with(e.index() + 1, Vec::new);
                }
                insert_sorted(&mut self.edge_faces[e.index()], id);
            }
        }
        old
    }

    fn rec_vert(&mut self, id: VertId, value: Option<Vertex>) {
        let old = self.raw_vert(id, value);
        if let Some(j) = self.journal.as_mut() {
            j.verts.entry(id).or_insert((old, None)).1 = value;
        }
    }

    fn rec_edge(&mut self, id: EdgeId, value: Option<Edge>) {
        let old = self.raw_edge(id, value);
        if let Some(j) = self.journal.as_mut() {
            j.edges.entry(id).or_insert((old, None)).1 = value;
        }
    }

    fn rec_face(&mut self, id: FaceId, value: Option<Face>) {
        let old = self.raw_face(id, value.clone());
        if let Some(j) = self.journal.as_mut() {
            j.faces.entry(id).or_insert((old, None)).1 = value;
        }
    }

    // ----------------------------------------------------------------------------------------------------------------
    // Journaled mutation primitives
    // ----------------------------------------------------------------------------------------------------------------

    /// Adds a loose vertex.
    pub fn add_vert(&mut self, position: Vec3) -> VertId {
        let id = VertId(self.verts.next_index());
        self.rec_vert(id, Some(Vertex { position }));
        id
    }

    /// Moves a vertex.
    ///
    /// # Errors
    /// [`ModelError::MissingVert`].
    pub fn set_position(&mut self, v: VertId, position: Vec3) -> ModelResult<()> {
        if self.vert(v).is_none() {
            return Err(ModelError::MissingVert(v));
        }
        if !position.is_finite() {
            return Err(ModelError::InvalidParameter(format!("non-finite position {position}")));
        }
        self.rec_vert(v, Some(Vertex { position }));
        Ok(())
    }

    /// Returns the edge between `a` and `b`, adding it when missing.
    ///
    /// # Errors
    /// [`ModelError::InvalidEdge`] when `a == b` or either vertex is missing.
    pub fn add_edge(&mut self, a: VertId, b: VertId) -> ModelResult<EdgeId> {
        if a == b || self.vert(a).is_none() || self.vert(b).is_none() {
            return Err(ModelError::InvalidEdge(a, b));
        }
        if let Some(e) = self.find_edge(a, b) {
            return Ok(e);
        }
        let id = EdgeId(self.edges.next_index());
        let (lo, hi) = key(a, b);
        self.rec_edge(id, Some(Edge { verts: [lo, hi], flags: EdgeFlags::default() }));
        Ok(id)
    }

    /// Sets the sharp and seam marks of an edge (ignored for a missing edge).
    pub fn set_edge_flags(&mut self, e: EdgeId, flags: EdgeFlags) {
        if let Some(edge) = self.edge(e).copied() {
            if edge.flags != flags {
                self.rec_edge(e, Some(Edge { flags, ..edge }));
            }
        }
    }

    fn check_loop(&self, verts: &[VertId]) -> ModelResult<()> {
        if verts.len() < 3 {
            return Err(ModelError::InvalidFace(format!("{} corners", verts.len())));
        }
        let mut sorted = verts.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() != verts.len() {
            return Err(ModelError::InvalidFace("a vertex appears twice".into()));
        }
        if let Some(&v) = verts.iter().find(|&&v| self.vert(v).is_none()) {
            return Err(ModelError::MissingVert(v));
        }
        Ok(())
    }

    /// Adds a face with zero UVs. Missing side edges are created.
    ///
    /// # Errors
    /// [`ModelError::InvalidFace`] / [`ModelError::MissingVert`] for a bad loop.
    pub fn add_face(&mut self, verts: &[VertId]) -> ModelResult<FaceId> {
        self.add_face_with_uvs(verts, &vec![Vec2::ZERO; verts.len()])
    }

    /// Adds a face with one UV per corner.
    ///
    /// # Errors
    /// As [`Mesh::add_face`], plus a UV count mismatch.
    pub fn add_face_with_uvs(&mut self, verts: &[VertId], uvs: &[Vec2]) -> ModelResult<FaceId> {
        self.check_loop(verts)?;
        if uvs.len() != verts.len() {
            return Err(ModelError::InvalidFace(format!("{} uvs for {} corners", uvs.len(), verts.len())));
        }
        for i in 0..verts.len() {
            self.add_edge(verts[i], verts[(i + 1) % verts.len()])?;
        }
        let id = FaceId(self.faces.next_index());
        self.rec_face(id, Some(Face { verts: verts.to_vec(), uvs: uvs.to_vec() }));
        Ok(id)
    }

    /// Replaces a face's corner loop and UVs in place (the id stays). Missing side edges are created; edges the face
    /// no longer uses are left in place (callers remove them if they became loose).
    ///
    /// # Errors
    /// As [`Mesh::add_face_with_uvs`], plus [`ModelError::MissingFace`].
    pub fn set_face(&mut self, f: FaceId, verts: &[VertId], uvs: &[Vec2]) -> ModelResult<()> {
        self.face_ref(f)?;
        self.check_loop(verts)?;
        if uvs.len() != verts.len() {
            return Err(ModelError::InvalidFace(format!("{} uvs for {} corners", uvs.len(), verts.len())));
        }
        for i in 0..verts.len() {
            self.add_edge(verts[i], verts[(i + 1) % verts.len()])?;
        }
        self.rec_face(f, Some(Face { verts: verts.to_vec(), uvs: uvs.to_vec() }));
        Ok(())
    }

    /// Replaces only the UVs of a face.
    ///
    /// # Errors
    /// [`ModelError::MissingFace`] or a UV count mismatch.
    pub fn set_face_uvs(&mut self, f: FaceId, uvs: &[Vec2]) -> ModelResult<()> {
        let face = self.face_ref(f)?.clone();
        if uvs.len() != face.verts.len() {
            return Err(ModelError::InvalidFace(format!("{} uvs for {} corners", uvs.len(), face.verts.len())));
        }
        self.rec_face(f, Some(Face { verts: face.verts, uvs: uvs.to_vec() }));
        Ok(())
    }

    /// Reverses a face's winding (its normal flips); UVs follow their corners.
    ///
    /// # Errors
    /// [`ModelError::MissingFace`].
    pub fn reverse_face(&mut self, f: FaceId) -> ModelResult<()> {
        let mut face = self.face_ref(f)?.clone();
        face.verts.reverse();
        face.uvs.reverse();
        self.rec_face(f, Some(face));
        Ok(())
    }

    /// Removes a face. Its edges and vertices stay.
    ///
    /// # Errors
    /// [`ModelError::MissingFace`].
    pub fn remove_face(&mut self, f: FaceId) -> ModelResult<()> {
        self.face_ref(f)?;
        self.rec_face(f, None);
        Ok(())
    }

    /// Removes an edge and every face that uses it.
    ///
    /// # Errors
    /// [`ModelError::MissingEdge`].
    pub fn remove_edge(&mut self, e: EdgeId) -> ModelResult<()> {
        self.edge_ref(e)?;
        for f in self.edge_faces(e).to_vec() {
            self.rec_face(f, None);
        }
        self.rec_edge(e, None);
        Ok(())
    }

    /// Removes a vertex with every edge and face that uses it.
    ///
    /// # Errors
    /// [`ModelError::MissingVert`].
    pub fn remove_vert(&mut self, v: VertId) -> ModelResult<()> {
        if self.vert(v).is_none() {
            return Err(ModelError::MissingVert(v));
        }
        for e in self.vert_edges(v).to_vec() {
            self.remove_edge(e)?;
        }
        self.rec_vert(v, None);
        Ok(())
    }

    /// Removes the given edges that have no faces left, then the given vertices (and the ends of removed edges) that
    /// have no edges left. Used by operations to clean up what they disconnected.
    pub fn remove_loose(&mut self, edges: impl IntoIterator<Item = EdgeId>, verts: impl IntoIterator<Item = VertId>) {
        let mut vs: BTreeSet<VertId> = verts.into_iter().collect();
        let es: BTreeSet<EdgeId> = edges.into_iter().collect();
        for e in es {
            if let Some(edge) = self.edge(e).copied() {
                if self.edge_faces(e).is_empty() {
                    self.rec_edge(e, None);
                    vs.extend(edge.verts);
                }
            }
        }
        for v in vs {
            if self.vert(v).is_some() && self.vert_edges(v).is_empty() {
                self.rec_vert(v, None);
            }
        }
    }
}

/// FNV-1a over 32-bit words; deterministic across runs and platforms.
struct Fnv(u64);

impl Fnv {
    fn u32(&mut self, x: u32) {
        for b in x.to_le_bytes() {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}
