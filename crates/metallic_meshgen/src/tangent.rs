//! MikkTSpace-compatible tangents (written from the published algorithm description).
//!
//! Per triangle: the UV-derived tangent, projected into each corner's normal plane and normalised, weighted
//! by the corner angle measured in that plane. Around each vertex the triangles are split into groups: two
//! triangles are in one group when they share an edge at that vertex and have the same UV orientation (the
//! sign of the UV triangle's area). Each group gets its own tangent; a vertex with several groups is split.
//! The bitangent sign `w` is +1 for orientation-preserving UVs. This matches MikkTSpace with its default
//! settings (no angular splitting); a test compares against the reference implementation.
use crate::mesh::MeshData;
use glam::{Vec2, Vec3};

fn any_perpendicular(n: Vec3) -> Vec3 {
    let u = crate::math::Frame::for_normal(n, Vec3::ZERO).u;
    (u - n * n.dot(u)).normalize_or(Vec3::X)
}

/// Fills `mesh.tangents`, splitting vertices whose triangles form more than one group.
pub(crate) fn generate(mesh: &mut MeshData) {
    let vcount = mesh.positions.len();
    let tri_count = mesh.indices.len() / 3;

    // Per triangle: unprojected UV tangent and orientation (None when the UVs are degenerate).
    let mut face: Vec<Option<(Vec3, bool)>> = Vec::with_capacity(tri_count);
    for t in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [t[0] as usize, t[1] as usize, t[2] as usize];
        let p0 = Vec3::from(mesh.positions[a]);
        let d1 = Vec3::from(mesh.positions[b]) - p0;
        let d2 = Vec3::from(mesh.positions[c]) - p0;
        let t0 = Vec2::from(mesh.uv0[a]);
        let t21 = Vec2::from(mesh.uv0[b]) - t0;
        let t31 = Vec2::from(mesh.uv0[c]) - t0;
        let area = t21.x * t31.y - t21.y * t31.x;
        if area.abs() <= f32::MIN_POSITIVE {
            face.push(None);
            continue;
        }
        let os = d1 * t31.y - d2 * t21.y;
        face.push(Some((if area < 0.0 { -os } else { os }, area > 0.0)));
    }
    let orient = |t: usize| face[t].is_none_or(|f| f.1);

    // Corners around each vertex.
    let mut around: Vec<Vec<(usize, usize)>> = vec![Vec::new(); vcount];
    for t in 0..tri_count {
        for k in 0..3 {
            around[mesh.indices[t * 3 + k] as usize].push((t, k));
        }
    }

    let mut tangents = vec![[0.0f32; 4]; vcount];
    for v in 0..vcount {
        let corners = &around[v];
        let m = corners.len();
        // Group corners: same orientation and connected through shared edges at this vertex.
        let mut group: Vec<usize> = (0..m).collect();
        let mut changed = true;
        while changed {
            changed = false;
            for i in 0..m {
                for j in i + 1..m {
                    let (ti, ki) = corners[i];
                    let (tj, kj) = corners[j];
                    if group[i] == group[j] || orient(ti) != orient(tj) {
                        continue;
                    }
                    let others = |t: usize, k: usize| {
                        [
                            mesh.indices[t * 3 + (k + 1) % 3],
                            mesh.indices[t * 3 + (k + 2) % 3],
                        ]
                    };
                    let (oi, oj) = (others(ti, ki), others(tj, kj));
                    if oi.iter().any(|w| oj.contains(w)) {
                        let g = group[i].min(group[j]);
                        group[i] = g;
                        group[j] = g;
                        changed = true;
                    }
                }
            }
        }

        let n = Vec3::from(mesh.normals[v]);
        let p = Vec3::from(mesh.positions[v]);
        let mut first = true;
        for g in 0..m {
            if !group.contains(&g) {
                continue;
            }
            let mut sum = Vec3::ZERO;
            let mut sign = 1.0;
            for (i, &(t, k)) in corners.iter().enumerate() {
                if group[i] != g {
                    continue;
                }
                sign = if orient(t) { 1.0 } else { -1.0 };
                let Some((os, _)) = face[t] else {
                    continue;
                };
                let e1 = Vec3::from(mesh.positions[mesh.indices[t * 3 + (k + 1) % 3] as usize]) - p;
                let e2 = Vec3::from(mesh.positions[mesh.indices[t * 3 + (k + 2) % 3] as usize]) - p;
                let e1 = (e1 - n * n.dot(e1)).normalize_or_zero();
                let e2 = (e2 - n * n.dot(e2)).normalize_or_zero();
                let angle = e1.dot(e2).clamp(-1.0, 1.0).acos();
                sum += (os - n * n.dot(os)).normalize_or_zero() * angle;
            }
            let tan = sum.normalize_or(any_perpendicular(n));
            let value = [tan.x, tan.y, tan.z, sign];
            if first {
                tangents[v] = value;
                first = false;
                continue;
            }
            // Another group: copy the vertex and point this group's corners at the copy.
            let copy = mesh.positions.len() as u32;
            mesh.positions.push(mesh.positions[v]);
            mesh.normals.push(mesh.normals[v]);
            mesh.uv0.push(mesh.uv0[v]);
            mesh.uv1.push(mesh.uv1[v]);
            mesh.ao.push(mesh.ao[v]);
            tangents.push(value);
            for (i, &(t, k)) in corners.iter().enumerate() {
                if group[i] == g {
                    mesh.indices[t * 3 + k] = copy;
                }
            }
        }
    }
    mesh.tangents = tangents;
}
