//! Merge vertices: at their centre, or every group closer than a distance ("merge by distance").

use std::collections::{BTreeMap, BTreeSet};

use glam::Vec3;

use crate::error::{ModelError, ModelResult};
use crate::ids::{FaceId, VertId};
use crate::mesh::{EdgeFlags, Mesh};
use crate::selection::{SelectMode, Selection};

/// Welds vertices: every key of `map` is replaced by its value everywhere, then removed. Faces that collapse below
/// three corners (or would repeat a corner) are removed; wire edges are re-pointed; edge marks are kept (or-merged).
///
/// # Errors
/// [`ModelError::MissingVert`] for a dead id.
pub fn weld(mesh: &mut Mesh, map: &BTreeMap<VertId, VertId>) -> ModelResult<()> {
    let map: BTreeMap<VertId, VertId> = map
        .iter()
        .filter(|(k, v)| k != v)
        .map(|(&k, &v)| (k, v))
        .collect();
    for (&k, &v) in &map {
        mesh.position(k)?;
        mesh.position(v)?;
    }
    if map.is_empty() {
        return Ok(());
    }
    let to = |v: VertId| map.get(&v).copied().unwrap_or(v);
    // Remember marks of edges that get re-pointed.
    let mut flags: BTreeMap<(VertId, VertId), EdgeFlags> = BTreeMap::new();
    let mut wires: Vec<(VertId, VertId)> = Vec::new();
    let mut faces: BTreeSet<FaceId> = BTreeSet::new();
    for &k in map.keys() {
        for &e in mesh.vert_edges(k) {
            let edge = *mesh.edge_ref(e)?;
            let (a, b) = (to(edge.verts[0]), to(edge.verts[1]));
            if a != b {
                let key = if a < b { (a, b) } else { (b, a) };
                let f = flags.entry(key).or_default();
                f.sharp |= edge.flags.sharp;
                f.seam |= edge.flags.seam;
                if mesh.edge_faces(e).is_empty() {
                    wires.push(key);
                }
            }
            faces.extend(mesh.edge_faces(e).iter().copied());
        }
    }
    for f in faces {
        let face = mesh.face_ref(f)?.clone();
        let mut verts: Vec<VertId> = Vec::new();
        let mut uvs = Vec::new();
        for (i, &v) in face.verts.iter().enumerate() {
            let t = to(v);
            if verts.last() != Some(&t) {
                verts.push(t);
                uvs.push(face.uvs[i]);
            }
        }
        while verts.len() > 1 && verts.first() == verts.last() {
            verts.pop();
            uvs.pop();
        }
        let distinct: BTreeSet<VertId> = verts.iter().copied().collect();
        if verts.len() < 3 || distinct.len() != verts.len() {
            mesh.remove_face(f)?;
        } else {
            mesh.set_face(f, &verts, &uvs)?;
        }
    }
    for (a, b) in wires {
        mesh.add_edge(a, b)?;
    }
    for &k in map.keys() {
        mesh.remove_vert(k)?;
    }
    for ((a, b), fl) in flags {
        if let Some(e) = mesh.find_edge(a, b) {
            let mut cur = mesh.edge_ref(e)?.flags;
            cur.sharp |= fl.sharp;
            cur.seam |= fl.seam;
            mesh.set_edge_flags(e, cur);
        }
    }
    Ok(())
}

/// Merges all `verts` into one vertex at their centre (the lowest id survives). Returns it selected.
///
/// # Errors
/// [`ModelError::NothingSelected`] for fewer than two vertices.
pub fn merge_at_center(mesh: &mut Mesh, verts: &BTreeSet<VertId>) -> ModelResult<Selection> {
    if verts.len() < 2 {
        return Err(ModelError::NothingSelected("two or more vertices to merge"));
    }
    let mut sum = Vec3::ZERO;
    for &v in verts {
        sum += mesh.position(v)?;
    }
    let center = sum / verts.len() as f32;
    let keep = *verts
        .iter()
        .next()
        .ok_or(ModelError::NothingSelected("vertices"))?;
    mesh.set_position(keep, center)?;
    let map = verts.iter().map(|&v| (v, keep)).collect();
    weld(mesh, &map)?;
    Ok(Selection::from_verts(mesh, [keep]).with_mode(SelectMode::Vertex))
}

/// Merges every group of `verts` closer than `distance` (transitively) into its lowest-id vertex, which keeps its
/// position. Returns the survivors selected and how many vertices were removed.
///
/// # Errors
/// [`ModelError::InvalidParameter`] for a negative or non-finite distance.
pub fn merge_by_distance(
    mesh: &mut Mesh,
    verts: &BTreeSet<VertId>,
    distance: f32,
) -> ModelResult<(Selection, usize)> {
    if !(distance.is_finite() && distance >= 0.0) {
        return Err(ModelError::InvalidParameter(format!(
            "merge distance {distance}"
        )));
    }
    let ids: Vec<VertId> = verts
        .iter()
        .copied()
        .filter(|&v| mesh.vert(v).is_some())
        .collect();
    let cell = distance.max(1e-6);
    let quant = |p: Vec3| {
        (
            (p.x / cell).floor() as i64,
            (p.y / cell).floor() as i64,
            (p.z / cell).floor() as i64,
        )
    };
    let mut grid: BTreeMap<(i64, i64, i64), Vec<usize>> = BTreeMap::new();
    for (i, &v) in ids.iter().enumerate() {
        grid.entry(quant(mesh.position(v)?)).or_default().push(i);
    }
    let mut parent: Vec<usize> = (0..ids.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for (i, &v) in ids.iter().enumerate() {
        let p = mesh.position(v)?;
        let (cx, cy, cz) = quant(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let Some(list) = grid.get(&(cx + dx, cy + dy, cz + dz)) else {
                        continue;
                    };
                    for &j in list {
                        if j > i && mesh.position(ids[j])?.distance(p) <= distance {
                            let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                            if ri != rj {
                                // Keep the lower index (lower id) as root.
                                let (lo, hi) = if ri < rj { (ri, rj) } else { (rj, ri) };
                                parent[hi] = lo;
                            }
                        }
                    }
                }
            }
        }
    }
    let mut map = BTreeMap::new();
    for i in 0..ids.len() {
        let r = find(&mut parent, i);
        if r != i {
            map.insert(ids[i], ids[r]);
        }
    }
    let removed = map.len();
    weld(mesh, &map)?;
    let survivors: Vec<VertId> = ids.into_iter().filter(|v| !map.contains_key(v)).collect();
    Ok((Selection::from_verts(mesh, survivors), removed))
}

impl Selection {
    /// The same selection switched to `mode` (flushing is the caller's job when it matters).
    #[must_use]
    pub fn with_mode(mut self, mode: SelectMode) -> Self {
        self.mode = mode;
        self
    }
}
