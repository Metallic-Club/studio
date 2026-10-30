//! Shared helpers for the integration tests.
#![allow(dead_code)]

use std::collections::BTreeSet;

use glam::Vec3;
use metallic_modeling::{EdgeId, EditMesh, FaceId, Mesh, MeshCommand, Op, Primitive, Selection, VertId};

/// A cube of edge 2 centred on the origin.
pub fn cube() -> Mesh {
    Primitive::Cube { size: 2.0 }.build(Vec3::ZERO).expect("cube")
}

/// Panics with the reason when the mesh breaks an invariant.
pub fn valid(mesh: &Mesh) {
    if let Err(e) = mesh.validate() {
        panic!("invalid mesh: {e}");
    }
}

/// Valid, closed, manifold, consistently wound, Euler characteristic 2, positive volume.
pub fn closed_sphere_like(mesh: &Mesh) {
    valid(mesh);
    assert!(mesh.is_closed_manifold(), "not a closed manifold");
    assert!(mesh.is_consistently_oriented(), "winding inconsistent");
    assert_eq!(mesh.euler_characteristic(), 2, "Euler characteristic");
    assert!(mesh.signed_volume() > 0.0, "inside out");
}

pub fn counts(mesh: &Mesh) -> (usize, usize, usize) {
    (mesh.vert_count(), mesh.edge_count(), mesh.face_count())
}

/// The face whose normal is closest to `dir`.
pub fn face_towards(mesh: &Mesh, dir: Vec3) -> FaceId {
    mesh.faces()
        .map(|(f, _)| f)
        .max_by(|&a, &b| mesh.face_normal(a).dot(dir).total_cmp(&mesh.face_normal(b).dot(dir)))
        .expect("faces")
}

/// The edge whose midpoint is closest to `p`.
pub fn edge_near(mesh: &Mesh, p: Vec3) -> EdgeId {
    mesh.edges()
        .map(|(e, _)| e)
        .min_by(|&a, &b| mesh.edge_midpoint(a).distance(p).total_cmp(&mesh.edge_midpoint(b).distance(p)))
        .expect("edges")
}

/// The vertex closest to `p`.
pub fn vert_near(mesh: &Mesh, p: Vec3) -> VertId {
    mesh.verts()
        .map(|(v, _)| v)
        .min_by(|&a, &b| mesh.position_or_zero(a).distance(p).total_cmp(&mesh.position_or_zero(b).distance(p)))
        .expect("verts")
}

/// Runs `op` with `sel`, checks undo restores the exact mesh and selection and redo restores the result, and
/// returns the edited mesh.
pub fn run_checked(mesh: Mesh, sel: Selection, op: &Op) -> EditMesh {
    let mut edit = EditMesh { mesh, selection: sel };
    let before = edit.mesh.content_hash();
    let before_sel = edit.selection.clone();
    let cmd = MeshCommand::run(&mut edit, op).unwrap_or_else(|e| panic!("{} failed: {e}", op.label()));
    valid(&edit.mesh);
    let after = edit.mesh.content_hash();
    let after_mesh = edit.mesh.clone();
    let after_sel = edit.selection.clone();
    cmd.undo(&mut edit);
    valid(&edit.mesh);
    assert_eq!(edit.mesh.content_hash(), before, "undo of {} did not restore the mesh", op.label());
    assert_eq!(edit.selection, before_sel, "undo of {} did not restore the selection", op.label());
    cmd.redo(&mut edit);
    valid(&edit.mesh);
    assert_eq!(edit.mesh.content_hash(), after, "redo of {} differs", op.label());
    assert_eq!(edit.mesh, after_mesh);
    assert_eq!(edit.selection, after_sel);
    edit
}

pub fn faces_sel(mesh: &Mesh, faces: impl IntoIterator<Item = FaceId>) -> Selection {
    Selection::from_faces(mesh, faces)
}

pub fn edges_sel(mesh: &Mesh, edges: impl IntoIterator<Item = EdgeId>) -> Selection {
    Selection::from_edges(mesh, edges)
}

pub fn verts_sel(mesh: &Mesh, verts: impl IntoIterator<Item = VertId>) -> Selection {
    Selection::from_verts(mesh, verts)
}

pub fn approx(a: Vec3, b: Vec3) -> bool {
    a.distance(b) < 1e-4
}

/// Whether some vertex sits at `p`.
pub fn has_vert_at(mesh: &Mesh, p: Vec3) -> bool {
    mesh.verts().any(|(_, v)| approx(v.position, p))
}

pub fn all_faces(mesh: &Mesh) -> BTreeSet<FaceId> {
    mesh.face_ids().into_iter().collect()
}
