//! Face orientation (flip, recalculate outside) and edge marks (sharp, seam).

use std::collections::{BTreeMap, BTreeSet};

use glam::Vec3;

use crate::error::{ModelError, ModelResult};
use crate::geom;
use crate::ids::{EdgeId, FaceId};
use crate::mesh::{EdgeFlags, Mesh};

/// Reverses the winding of `faces` (their normals flip).
///
/// # Errors
/// [`ModelError::NothingSelected`] or [`ModelError::MissingFace`].
pub fn flip_normals(mesh: &mut Mesh, faces: &BTreeSet<FaceId>) -> ModelResult<()> {
    if faces.is_empty() {
        return Err(ModelError::NothingSelected("faces to flip"));
    }
    for &f in faces {
        mesh.reverse_face(f)?;
    }
    Ok(())
}

/// Makes the winding of `faces` consistent across shared (two-face) edges and then points each connected piece
/// outwards (positive enclosed volume measured from the piece's centre). Returns how many faces were flipped.
///
/// # Errors
/// [`ModelError::NothingSelected`] or [`ModelError::MissingFace`].
pub fn recalculate_outside(mesh: &mut Mesh, faces: &BTreeSet<FaceId>) -> ModelResult<usize> {
    if faces.is_empty() {
        return Err(ModelError::NothingSelected("faces to recalculate"));
    }
    let mut flipped = 0;
    let mut seen: BTreeSet<FaceId> = BTreeSet::new();
    for &seed in faces {
        if !seen.insert(seed) {
            continue;
        }
        mesh.face_ref(seed)?;
        // Breadth-first: decide for every face whether it must flip relative to the seed.
        let mut flip: BTreeMap<FaceId, bool> = BTreeMap::from([(seed, false)]);
        let mut queue = std::collections::VecDeque::from([seed]);
        while let Some(f) = queue.pop_front() {
            let face = mesh.face_ref(f)?.clone();
            let f_flip = flip[&f];
            for (a, b) in face.directed_edges() {
                let (a, b) = if f_flip { (b, a) } else { (a, b) };
                let Some(e) = mesh.find_edge(a, b) else {
                    continue;
                };
                let around = mesh.edge_faces(e);
                if around.len() != 2 {
                    continue;
                }
                let g = if around[0] == f { around[1] } else { around[0] };
                if !faces.contains(&g) || flip.contains_key(&g) {
                    continue;
                }
                // g must walk b -> a after its own flip.
                let g_walks_ab = mesh.face(g).is_some_and(|x| x.has_directed_edge(a, b));
                flip.insert(g, g_walks_ab);
                seen.insert(g);
                queue.push_back(g);
            }
        }
        // Orientation of the piece: signed volume around its centre.
        let pts: Vec<Vec3> = flip.keys().flat_map(|&f| mesh.face_positions(f)).collect();
        let c = geom::centroid(&pts);
        let mut vol = 0.0;
        for (&f, &fl) in &flip {
            let mut p = mesh.face_positions(f);
            if fl {
                p.reverse();
            }
            for t in geom::triangulate_polygon(&p) {
                vol += (p[t[0]] - c).dot((p[t[1]] - c).cross(p[t[2]] - c));
            }
        }
        let invert_all = vol < 0.0;
        for (&f, &fl) in &flip {
            if fl != invert_all {
                mesh.reverse_face(f)?;
                flipped += 1;
            }
        }
    }
    Ok(flipped)
}

/// Sets or clears the sharp mark on `edges`.
///
/// # Errors
/// [`ModelError::NothingSelected`] or [`ModelError::MissingEdge`].
pub fn mark_sharp(mesh: &mut Mesh, edges: &BTreeSet<EdgeId>, sharp: bool) -> ModelResult<()> {
    set_flag(mesh, edges, |f| f.sharp = sharp)
}

/// Sets or clears the seam mark on `edges`.
///
/// # Errors
/// [`ModelError::NothingSelected`] or [`ModelError::MissingEdge`].
pub fn mark_seam(mesh: &mut Mesh, edges: &BTreeSet<EdgeId>, seam: bool) -> ModelResult<()> {
    set_flag(mesh, edges, |f| f.seam = seam)
}

fn set_flag(
    mesh: &mut Mesh,
    edges: &BTreeSet<EdgeId>,
    change: impl Fn(&mut EdgeFlags),
) -> ModelResult<()> {
    if edges.is_empty() {
        return Err(ModelError::NothingSelected("edges to mark"));
    }
    for &e in edges {
        let mut flags = mesh.edge_ref(e)?.flags;
        change(&mut flags);
        mesh.set_edge_flags(e, flags);
    }
    Ok(())
}
