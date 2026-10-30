//! Geometry and topology of every operation, each run through `run_checked` (undo and redo must restore exactly).

mod common;

use common::*;
use glam::Vec3;
use metallic_modeling::ops::loops::{edge_loop, edge_ring};
use metallic_modeling::{
    Axis, DeleteKind, EditMesh, History, MeshCommand, ModelError, Op, Primitive, SelectMode, Selection, Symmetry,
    Transform,
};

#[test]
fn cube_primitive_is_a_closed_outward_box() {
    let m = cube();
    closed_sphere_like(&m);
    assert_eq!(counts(&m), (8, 12, 6));
    assert!((m.signed_volume() - 8.0).abs() < 1e-4);
}

#[test]
fn other_primitives_are_valid() {
    for p in [
        Primitive::Cylinder { segments: 12, radius: 1.0, height: 2.0 },
        Primitive::UvSphere { segments: 12, rings: 8, radius: 1.0 },
    ] {
        closed_sphere_like(&p.build(Vec3::ZERO).expect("primitive"));
    }
    let grid = Primitive::Grid { x: 3, z: 2, size: 2.0 }.build(Vec3::ZERO).expect("grid");
    valid(&grid);
    assert_eq!(counts(&grid), (12, 17, 6));
    assert_eq!(grid.euler_characteristic(), 1);
    assert!(grid.faces().all(|(f, _)| grid.face_normal(f).y > 0.99));
}

#[test]
fn extrude_top_face_one_stud() {
    let m = cube();
    let top = face_towards(&m, Vec3::Y);
    let sel = faces_sel(&m, [top]);
    let e = run_checked(m, sel, &Op::ExtrudeFaces { distance: 1.0, individual: false });
    closed_sphere_like(&e.mesh);
    assert_eq!(counts(&e.mesh), (12, 20, 10));
    for p in [Vec3::new(1.0, 2.0, 1.0), Vec3::new(-1.0, 2.0, -1.0), Vec3::new(1.0, 1.0, 1.0)] {
        assert!(has_vert_at(&e.mesh, p), "missing {p}");
    }
    assert!((e.mesh.signed_volume() - 12.0).abs() < 1e-4);
    // The cap keeps its id and stays selected.
    assert_eq!(e.selection.faces.iter().copied().collect::<Vec<_>>(), vec![top]);
    assert!((e.mesh.face_center(top) - Vec3::new(0.0, 2.0, 0.0)).length() < 1e-5);
}

#[test]
fn extrude_region_of_two_faces_builds_walls_only_on_the_boundary() {
    let m = Primitive::Grid { x: 2, z: 1, size: 2.0 }.build(Vec3::ZERO).expect("grid");
    let sel = faces_sel(&m, m.face_ids());
    let e = run_checked(m, sel, &Op::ExtrudeFaces { distance: 0.5, individual: false });
    // 6 old + 6 new verts; 2 caps + 6 walls.
    assert_eq!(counts(&e.mesh), (12, 19, 8));
    assert!(e.mesh.is_consistently_oriented());
}

#[test]
fn extrude_individual_faces_moves_each_along_its_normal() {
    let m = cube();
    let top = face_towards(&m, Vec3::Y);
    let front = face_towards(&m, Vec3::Z);
    let sel = faces_sel(&m, [top, front]);
    let e = run_checked(m, sel, &Op::ExtrudeFaces { distance: 0.5, individual: true });
    valid(&e.mesh);
    assert!(e.mesh.is_consistently_oriented());
    assert!((e.mesh.face_center(top) - Vec3::new(0.0, 1.5, 0.0)).length() < 1e-5);
    assert!((e.mesh.face_center(front) - Vec3::new(0.0, 0.0, 1.5)).length() < 1e-5);
    assert_eq!(e.mesh.face_count(), 6 + 8);
}

#[test]
fn extrude_closed_region_duplicates_the_shell() {
    let m = cube();
    let sel = faces_sel(&m, m.face_ids());
    let e = run_checked(m, sel, &Op::ExtrudeFaces { distance: 0.5, individual: false });
    assert_eq!(counts(&e.mesh), (16, 24, 12));
}

#[test]
fn extrude_boundary_edge_keeps_winding() {
    let m = Primitive::Plane { size: 2.0 }.build(Vec3::ZERO).expect("plane");
    let e0 = edge_near(&m, Vec3::new(1.0, 0.0, 0.0));
    let sel = edges_sel(&m, [e0]);
    let e = run_checked(m, sel, &Op::ExtrudeEdges { offset: Vec3::new(1.0, 0.0, 0.0) });
    assert_eq!(counts(&e.mesh), (6, 7, 2));
    assert!(e.mesh.is_consistently_oriented());
    assert!(has_vert_at(&e.mesh, Vec3::new(2.0, 0.0, 1.0)));
    assert_eq!(e.selection.mode, SelectMode::Edge);
    assert_eq!(e.selection.edges.len(), 1);
}

#[test]
fn extrude_vertex_makes_a_wire_edge() {
    let m = cube();
    let v = vert_near(&m, Vec3::ONE);
    let sel = verts_sel(&m, [v]);
    let e = run_checked(m, sel, &Op::ExtrudeVerts { offset: Vec3::ONE });
    assert_eq!(counts(&e.mesh), (9, 13, 6));
    assert_eq!(e.mesh.wire_edges().len(), 1);
    assert!(has_vert_at(&e.mesh, Vec3::splat(2.0)));
}

#[test]
fn inset_top_face_known_coordinates() {
    let m = cube();
    let top = face_towards(&m, Vec3::Y);
    let sel = faces_sel(&m, [top]);
    let e = run_checked(m, sel, &Op::Inset { thickness: 0.25, depth: 0.0, individual: false });
    closed_sphere_like(&e.mesh);
    assert_eq!(counts(&e.mesh), (12, 20, 10));
    for (x, z) in [(0.75, 0.75), (-0.75, 0.75), (0.75, -0.75), (-0.75, -0.75)] {
        assert!(has_vert_at(&e.mesh, Vec3::new(x, 1.0, z)), "inner corner {x} {z}");
    }
    assert!((e.mesh.face_area(top) - 2.25).abs() < 1e-4);
    // Volume unchanged (flat inset).
    assert!((e.mesh.signed_volume() - 8.0).abs() < 1e-4);
}

#[test]
fn inset_with_depth_pushes_inwards() {
    let m = cube();
    let top = face_towards(&m, Vec3::Y);
    let sel = faces_sel(&m, [top]);
    let e = run_checked(m, sel, &Op::Inset { thickness: 0.5, depth: -0.5, individual: false });
    closed_sphere_like(&e.mesh);
    assert!(has_vert_at(&e.mesh, Vec3::new(0.5, 0.5, 0.5)));
    // A frustum pocket (2x2 opening, 1x1 floor, 0.5 deep) was removed: h/3 * (4 + 1 + 2).
    assert!((e.mesh.signed_volume() - (8.0 - 7.0 / 6.0)).abs() < 1e-4, "{}", e.mesh.signed_volume());
}

#[test]
fn inset_region_vs_individual() {
    let m = Primitive::Grid { x: 2, z: 1, size: 2.0 }.build(Vec3::ZERO).expect("grid");
    let sel = faces_sel(&m, m.face_ids());
    let region = run_checked(m.clone(), sel.clone(), &Op::Inset { thickness: 0.1, depth: 0.0, individual: false });
    let indiv = run_checked(m, sel, &Op::Inset { thickness: 0.1, depth: 0.0, individual: true });
    // Region: one ring of 6 quads around the pair. Individual: two rings of 4.
    assert_eq!(region.mesh.face_count(), 2 + 6);
    assert_eq!(indiv.mesh.face_count(), 2 + 8);
    assert!(region.mesh.is_consistently_oriented() && indiv.mesh.is_consistently_oriented());
}

#[test]
fn bevel_one_cube_edge_chamfer() {
    let m = cube();
    let edge = edge_near(&m, Vec3::new(0.0, 1.0, 1.0));
    let sel = edges_sel(&m, [edge]);
    let e = run_checked(m, sel, &Op::Bevel { width: 0.2, segments: 1 });
    closed_sphere_like(&e.mesh);
    assert_eq!(counts(&e.mesh), (10, 15, 7));
    for x in [-1.0, 1.0] {
        assert!(has_vert_at(&e.mesh, Vec3::new(x, 1.0, 0.8)), "top rail at x={x}");
        assert!(has_vert_at(&e.mesh, Vec3::new(x, 0.8, 1.0)), "front rail at x={x}");
    }
    assert!(!has_vert_at(&e.mesh, Vec3::new(1.0, 1.0, 1.0)));
    // The chamfer cut away a prism of 0.2 * 0.2 / 2 * 2 = 0.04.
    assert!((e.mesh.signed_volume() - (8.0 - 0.04)).abs() < 1e-4, "{}", e.mesh.signed_volume());
    assert_eq!(e.selection.faces.len(), 1);
}

#[test]
fn bevel_one_cube_edge_rounded_profile() {
    let m = cube();
    let edge = edge_near(&m, Vec3::new(0.0, 1.0, 1.0));
    let sel = edges_sel(&m, [edge]);
    let e = run_checked(m, sel, &Op::Bevel { width: 0.2, segments: 3 });
    closed_sphere_like(&e.mesh);
    assert_eq!(counts(&e.mesh), (14, 21, 9));
    // Every new point lies on the quarter circle of radius 0.2 around the line (x, 0.8, 0.8).
    let rounded: Vec<Vec3> =
        e.mesh.verts().map(|(_, v)| v.position).filter(|p| p.y > 0.79 && p.z > 0.79).collect();
    assert_eq!(rounded.len(), 8);
    for p in rounded {
        let r = Vec3::new(0.0, p.y - 0.8, p.z - 0.8).length();
        assert!((r - 0.2).abs() < 1e-4, "{p} off the arc ({r})");
    }
    // The side faces took the profile points: the beveled corner became 4 corners (2 rails + 2 profile points).
    let side = face_towards(&e.mesh, Vec3::X);
    assert_eq!(e.mesh.face(side).map(|f| f.verts.len()), Some(7));
}

#[test]
fn bevel_all_cube_edges_makes_corner_triangles() {
    let m = cube();
    let sel = edges_sel(&m, m.edge_ids());
    let e = run_checked(m, sel, &Op::Bevel { width: 0.2, segments: 1 });
    closed_sphere_like(&e.mesh);
    assert_eq!(counts(&e.mesh), (24, 48, 26));
    let triangles = e.mesh.faces().filter(|(_, f)| f.verts.len() == 3).count();
    assert_eq!(triangles, 8);
    assert!(has_vert_at(&e.mesh, Vec3::new(1.0, 0.8, 0.8)));
}

/// A beveled edge ending at a vertex with more than three faces (here: the top was inset first, so the top corner
/// also has the inset's diagonal edge). The closing faces share the profile and nothing is left at the old corner.
#[test]
fn bevel_edge_ending_at_a_valence_four_corner_leaves_no_overhang() {
    for segments in [1, 2, 3] {
        let m = cube();
        let top = face_towards(&m, Vec3::Y);
        let sel = faces_sel(&m, [top]);
        let inset = run_checked(m, sel, &Op::Inset { thickness: 0.3, depth: 0.0, individual: false });
        let vertical = edge_near(&inset.mesh, Vec3::new(1.0, 0.0, 1.0));
        let sel = edges_sel(&inset.mesh, [vertical]);
        let e = run_checked(inset.mesh, sel, &Op::Bevel { width: 0.25, segments });
        closed_sphere_like(&e.mesh);
        assert!(!has_vert_at(&e.mesh, Vec3::new(1.0, 1.0, 1.0)), "segments {segments}: top corner left behind");
        assert!(!has_vert_at(&e.mesh, Vec3::new(1.0, -1.0, 1.0)), "segments {segments}: bottom corner left behind");
        // Every vertex stays inside the rounded/chamfered outline seen from above.
        for (_, v) in e.mesh.verts() {
            let p = v.position;
            assert!(p.x + p.z <= 1.75 + 1e-4 || segments > 1, "segments {segments}: {p} outside the chamfer");
            if segments > 1 {
                let r = Vec3::new(p.x - 0.75, 0.0, p.z - 0.75);
                let outside = p.x > 0.75 + 1e-4 && p.z > 0.75 + 1e-4 && r.length() > 0.25 + 1e-4;
                assert!(!outside, "segments {segments}: {p} outside the rounded corner");
            }
        }
        // Only the corner prism left the volume: 0.25^2 / 2 * 2 for the chamfer, less for a rounded profile.
        let vol = e.mesh.signed_volume();
        if segments == 1 {
            assert!((vol - (8.0 - 0.0625)).abs() < 1e-4, "{vol}");
        } else {
            assert!(vol < 8.0 && vol > 8.0 - 0.0625, "segments {segments}: {vol}");
        }
    }
}

#[test]
fn bevel_rejects_open_edges_and_leaves_mesh_untouched() {
    let m = Primitive::Plane { size: 2.0 }.build(Vec3::ZERO).expect("plane");
    let hash = m.content_hash();
    let edge = edge_near(&m, Vec3::new(1.0, 0.0, 0.0));
    let mut edit = EditMesh { selection: edges_sel(&m, [edge]), mesh: m };
    let err = MeshCommand::run(&mut edit, &Op::Bevel { width: 0.1, segments: 1 });
    assert!(matches!(err, Err(ModelError::NonManifold(_))));
    assert_eq!(edit.mesh.content_hash(), hash);
}

#[test]
fn bevel_too_wide_fails_cleanly() {
    let m = cube();
    let hash = m.content_hash();
    let edge = edge_near(&m, Vec3::new(0.0, 1.0, 1.0));
    let mut edit = EditMesh { selection: edges_sel(&m, [edge]), mesh: m };
    assert!(MeshCommand::run(&mut edit, &Op::Bevel { width: 5.0, segments: 2 }).is_err());
    assert_eq!(edit.mesh.content_hash(), hash);
    valid(&edit.mesh);
}

#[test]
fn loop_cut_cube_counts_and_positions() {
    let m = cube();
    let edge = edge_near(&m, Vec3::new(1.0, 1.0, 0.0));
    let e = run_checked(m, Selection::new(SelectMode::Edge), &Op::LoopCut { edge, cuts: 1 });
    closed_sphere_like(&e.mesh);
    assert_eq!(counts(&e.mesh), (12, 20, 10));
    for (x, y) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
        assert!(has_vert_at(&e.mesh, Vec3::new(x, y, 0.0)));
    }
    assert_eq!(e.selection.edges.len(), 4);
    assert!(e.selection.edges.iter().all(|&x| e.mesh.edge_midpoint(x).z.abs() < 1e-5));
}

#[test]
fn loop_cut_three_cuts() {
    let m = cube();
    let edge = edge_near(&m, Vec3::new(1.0, 1.0, 0.0));
    let e = run_checked(m, Selection::default(), &Op::LoopCut { edge, cuts: 3 });
    closed_sphere_like(&e.mesh);
    assert_eq!(counts(&e.mesh), (20, 36, 18));
    assert!(has_vert_at(&e.mesh, Vec3::new(1.0, 1.0, 0.5)));
    assert!(has_vert_at(&e.mesh, Vec3::new(-1.0, -1.0, -0.5)));
}

#[test]
fn loop_cut_around_a_cylinder() {
    // A loop cut across the vertical side edges goes all the way round.
    let m = Primitive::Cylinder { segments: 6, radius: 1.0, height: 2.0 }.build(Vec3::ZERO).expect("cyl");
    let vertical = m
        .edges()
        .map(|(e, _)| e)
        .find(|&e| {
            let p = m.edge_midpoint(e);
            p.y.abs() < 1e-5
        })
        .expect("side edge");
    let e = run_checked(m, Selection::default(), &Op::LoopCut { edge: vertical, cuts: 1 });
    closed_sphere_like(&e.mesh);
    // A horizontal ring around the middle: +6 verts, +6 faces, +12 edges.
    assert_eq!(counts(&e.mesh), (18, 30, 14));
}

#[test]
fn edge_loop_and_ring_selection() {
    let m = Primitive::UvSphere { segments: 8, rings: 6, radius: 1.0 }.build(Vec3::ZERO).expect("sphere");
    // An equator edge (horizontal, y = 0).
    let eq = m
        .edges()
        .map(|(e, _)| e)
        .find(|&e| {
            let edge = m.edge(e).expect("edge");
            edge.verts.iter().all(|&v| m.position_or_zero(v).y.abs() < 1e-5)
        })
        .expect("equator edge");
    assert_eq!(edge_loop(&m, eq).len(), 8);
    let ring = edge_ring(&m, eq);
    // The ring crosses quads up and down until it reaches the pole triangles: rings - 2 + 1 edges.
    assert_eq!(ring.edges.len(), 5);
    assert!(!ring.closed);
    let c = cube();
    let r = edge_ring(&c, edge_near(&c, Vec3::new(1.0, 1.0, 0.0)));
    assert!(r.closed);
    assert_eq!(r.edges.len(), 4);
    // Boundary loop of a grid.
    let g = Primitive::Grid { x: 3, z: 3, size: 3.0 }.build(Vec3::ZERO).expect("grid");
    let b = edge_near(&g, Vec3::new(1.5, 0.0, 0.0));
    assert_eq!(edge_loop(&g, b).len(), 12);
}

#[test]
fn bridge_two_planes_into_a_box() {
    let mut m = Primitive::Plane { size: 2.0 }.build(Vec3::ZERO).expect("plane");
    let bottom = m.face_ids();
    metallic_modeling::ops::normals::flip_normals(&mut m, &bottom.into_iter().collect()).expect("flip");
    Primitive::Plane { size: 2.0 }.add_to(&mut m, Vec3::new(0.0, 2.0, 0.0)).expect("top");
    let sel = edges_sel(&m, m.edge_ids());
    let e = run_checked(m, sel, &Op::BridgeEdgeLoops);
    closed_sphere_like(&e.mesh);
    assert_eq!(counts(&e.mesh), (8, 12, 6));
    assert!((e.mesh.signed_volume() - 8.0).abs() < 1e-4);
}

#[test]
fn bridge_rejects_unequal_loops() {
    let mut m = Primitive::Plane { size: 2.0 }.build(Vec3::ZERO).expect("plane");
    Primitive::Grid { x: 2, z: 1, size: 2.0 }.add_to(&mut m, Vec3::new(0.0, 2.0, 0.0)).expect("grid");
    let boundary = m.boundary_edges();
    let mut edit = EditMesh { selection: edges_sel(&m, boundary), mesh: m };
    assert!(matches!(MeshCommand::run(&mut edit, &Op::BridgeEdgeLoops), Err(ModelError::Unsupported(_))));
}

#[test]
fn merge_by_distance_welds_coincident_vertices() {
    let pts = [
        (0.0, 0.0),
        (0.0, 1.0),
        (1.0, 1.0),
        (1.0, 0.0),
        (1.0, 0.0),
        (1.0, 1.0),
        (2.0, 1.0),
        (2.0, 0.0),
    ]
    .map(|(x, z)| Vec3::new(x, 0.0, z));
    let m = metallic_modeling::Mesh::from_polygons(&pts, &[vec![0, 1, 2, 3], vec![4, 5, 6, 7]]).expect("mesh");
    assert_eq!(counts(&m), (8, 8, 2));
    let sel = verts_sel(&m, m.vert_ids());
    let e = run_checked(m, sel, &Op::MergeByDistance { distance: 1e-3 });
    assert_eq!(counts(&e.mesh), (6, 7, 2));
    assert!(e.mesh.is_manifold() && e.mesh.is_consistently_oriented());
    assert_eq!(e.mesh.boundary_edges().len(), 6);
}

#[test]
fn merge_at_center_collapses_top_into_a_pyramid() {
    let m = cube();
    let top: Vec<_> = m.verts().filter(|(_, v)| v.position.y > 0.5).map(|(id, _)| id).collect();
    let sel = verts_sel(&m, top);
    let e = run_checked(m, sel, &Op::MergeAtCenter);
    closed_sphere_like(&e.mesh);
    assert_eq!(counts(&e.mesh), (5, 8, 5));
    assert!(has_vert_at(&e.mesh, Vec3::new(0.0, 1.0, 0.0)));
}

#[test]
fn dissolve_edge_faces_and_vertex() {
    let m = cube();
    let edge = edge_near(&m, Vec3::new(0.0, 1.0, 1.0));
    let sel = edges_sel(&m, [edge]);
    let e = run_checked(m, sel, &Op::DissolveEdges);
    closed_sphere_like(&e.mesh);
    assert_eq!(counts(&e.mesh), (8, 11, 5));

    let g = Primitive::Grid { x: 2, z: 2, size: 2.0 }.build(Vec3::ZERO).expect("grid");
    let sel = faces_sel(&g, g.face_ids());
    let e = run_checked(g.clone(), sel, &Op::DissolveFaces);
    assert_eq!(counts(&e.mesh), (8, 8, 1));
    assert!(e.mesh.faces().all(|(f, _)| e.mesh.face_normal(f).y > 0.99));

    let center = vert_near(&g, Vec3::ZERO);
    let sel = verts_sel(&g, [center]);
    let e = run_checked(g, sel, &Op::DissolveVerts);
    assert_eq!(counts(&e.mesh), (8, 8, 1));
}

#[test]
fn delete_variants() {
    let m = cube();
    let top = face_towards(&m, Vec3::Y);
    let e = run_checked(m.clone(), faces_sel(&m, [top]), &Op::Delete(DeleteKind::Faces));
    assert_eq!(counts(&e.mesh), (8, 12, 5));
    assert_eq!(e.mesh.boundary_edges().len(), 4);
    let v = vert_near(&m, Vec3::ONE);
    let e = run_checked(m.clone(), verts_sel(&m, [v]), &Op::Delete(DeleteKind::Verts));
    assert_eq!(counts(&e.mesh), (7, 9, 3));
    let edge = edge_near(&m, Vec3::new(0.0, 1.0, 1.0));
    let e = run_checked(m.clone(), edges_sel(&m, [edge]), &Op::Delete(DeleteKind::Edges));
    assert_eq!(counts(&e.mesh), (8, 11, 4));
    // Deleting the whole cube's faces leaves nothing.
    let e = run_checked(m.clone(), faces_sel(&m, m.face_ids()), &Op::Delete(DeleteKind::Faces));
    assert_eq!(counts(&e.mesh), (0, 0, 0));
}

#[test]
fn fill_hole_closes_the_cube() {
    let mut m = cube();
    let top = face_towards(&m, Vec3::Y);
    m.remove_face(top).expect("remove");
    let sel = edges_sel(&m, m.boundary_edges());
    let e = run_checked(m, sel, &Op::Fill);
    closed_sphere_like(&e.mesh);
    // Fill from three loose vertices.
    let pts = [Vec3::ZERO, Vec3::X, Vec3::Z];
    let mut loose = metallic_modeling::Mesh::new();
    let ids: Vec<_> = pts.iter().map(|&p| loose.add_vert(p)).collect();
    let e = run_checked(loose.clone(), verts_sel(&loose, ids), &Op::Fill);
    assert_eq!(e.mesh.face_count(), 1);
}

#[test]
fn grid_fill_cylinder_cap() {
    let mut m = Primitive::Cylinder { segments: 8, radius: 1.0, height: 2.0 }.build(Vec3::ZERO).expect("cyl");
    let top = face_towards(&m, Vec3::Y);
    m.remove_face(top).expect("remove");
    let sel = edges_sel(&m, m.boundary_edges());
    let e = run_checked(m, sel, &Op::GridFill { span: Some(2), offset: 0 });
    closed_sphere_like(&e.mesh);
    assert_eq!(counts(&e.mesh), (17, 28, 13));
    assert!(has_vert_at(&e.mesh, Vec3::new(0.0, 1.0, 0.0)));
}

#[test]
fn mirror_half_box_into_whole() {
    let mut m = Primitive::Cube { size: 2.0 }.build(Vec3::new(1.0, 0.0, 0.0)).expect("cube");
    let inner = face_towards(&m, -Vec3::X);
    m.remove_face(inner).expect("remove");
    let sel = faces_sel(&m, m.face_ids());
    let e = run_checked(m, sel, &Op::Mirror { axis: Axis::X, merge_distance: 1e-3 });
    closed_sphere_like(&e.mesh);
    assert_eq!(counts(&e.mesh), (12, 20, 10));
    let (lo, hi) = e.mesh.bounds().expect("bounds");
    assert!(approx(lo, Vec3::new(-2.0, -1.0, -1.0)) && approx(hi, Vec3::new(2.0, 1.0, 1.0)));
}

#[test]
fn symmetric_transform_moves_mirror_partners() {
    let m = cube();
    let right: Vec<_> = m.verts().filter(|(_, v)| v.position.x > 0.5).map(|(id, _)| id).collect();
    let sel = verts_sel(&m, right);
    let xf = Transform::translate(Vec3::new(0.5, 0.0, 0.0)).with_symmetry(Symmetry::default());
    let e = run_checked(m, sel, &Op::Transform(xf));
    assert!(has_vert_at(&e.mesh, Vec3::new(1.5, 1.0, 1.0)));
    assert!(has_vert_at(&e.mesh, Vec3::new(-1.5, 1.0, 1.0)));
    let (lo, hi) = e.mesh.bounds().expect("bounds");
    assert!((lo.x + 1.5).abs() < 1e-5 && (hi.x - 1.5).abs() < 1e-5);
}

#[test]
fn rotate_and_scale_about_pivot() {
    let m = cube();
    let sel = verts_sel(&m, m.vert_ids());
    let rot = Transform::rotate(glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_4), Vec3::ZERO);
    let e = run_checked(m.clone(), sel.clone(), &Op::Transform(rot));
    assert!(has_vert_at(&e.mesh, Vec3::new(0.0, 1.0, std::f32::consts::SQRT_2)));
    let e = run_checked(m, sel, &Op::Transform(Transform::scale(Vec3::splat(2.0), Vec3::new(1.0, 1.0, 1.0))));
    assert!(has_vert_at(&e.mesh, Vec3::new(-3.0, -3.0, -3.0)));
    assert!(has_vert_at(&e.mesh, Vec3::ONE));
}

#[test]
fn subdivide_top_face() {
    let m = cube();
    let top = face_towards(&m, Vec3::Y);
    let sel = faces_sel(&m, [top]);
    let e = run_checked(m, sel, &Op::Subdivide { levels: 1 });
    closed_sphere_like(&e.mesh);
    assert_eq!(counts(&e.mesh), (13, 20, 9));
    assert!(has_vert_at(&e.mesh, Vec3::new(0.0, 1.0, 0.0)));
}

fn radius_spread(m: &metallic_modeling::Mesh) -> f32 {
    let r: Vec<f32> = m.verts().map(|(_, v)| v.position.length()).collect();
    let max = r.iter().copied().fold(0.0, f32::max);
    let min = r.iter().copied().fold(f32::MAX, f32::min);
    max / min
}

#[test]
fn catmull_clark_cube_converges_towards_a_sphere() {
    let m = cube();
    let l1 = run_checked(m.clone(), Selection::default(), &Op::CatmullClark { levels: 1 });
    closed_sphere_like(&l1.mesh);
    assert_eq!(counts(&l1.mesh), (26, 48, 24));
    // Classic result: the cube corner (1,1,1) moves to (5/9, 5/9, 5/9).
    assert!(has_vert_at(&l1.mesh, Vec3::splat(5.0 / 9.0)), "corner rule");
    // Face centres stay, edge points are (mid + avg of face centres) / 2 = (0.75, 0.75, 0).
    assert!(has_vert_at(&l1.mesh, Vec3::new(0.0, 1.0, 0.0)));
    assert!(has_vert_at(&l1.mesh, Vec3::new(0.75, 0.75, 0.0)));
    let l2 = run_checked(m.clone(), Selection::default(), &Op::CatmullClark { levels: 2 });
    assert_eq!(counts(&l2.mesh), (98, 192, 96));
    let l3 = run_checked(m.clone(), Selection::default(), &Op::CatmullClark { levels: 3 });
    closed_sphere_like(&l3.mesh);
    // (Level 0 has only the 8 corners, all at the same radius, so the spread starts at level 1.)
    let (s1, s2, s3) = (radius_spread(&l1.mesh), radius_spread(&l2.mesh), radius_spread(&l3.mesh));
    assert!(s1 > s2 && s2 > s3, "{s1} {s2} {s3}");
    assert!(s3 < 1.04, "level 3 max/min radius {s3}");
}

#[test]
fn catmull_clark_respects_sharp_edges() {
    let mut m = cube();
    let all: std::collections::BTreeSet<_> = m.edge_ids().into_iter().collect();
    metallic_modeling::ops::normals::mark_sharp(&mut m, &all, true).expect("sharp");
    let e = run_checked(m, Selection::default(), &Op::CatmullClark { levels: 2 });
    closed_sphere_like(&e.mesh);
    // Fully creased: the cube keeps its shape.
    assert!(has_vert_at(&e.mesh, Vec3::ONE));
    assert!(e.mesh.verts().all(|(_, v)| v.position.abs().max_element() > 0.9999));
    assert!(e.mesh.edges().filter(|(_, x)| x.flags.sharp).count() == 48);
}

#[test]
fn flip_then_recalculate_outside() {
    let m = cube();
    let some: Vec<_> = m.face_ids().into_iter().take(3).collect();
    let flipped = run_checked(m.clone(), faces_sel(&m, some), &Op::FlipNormals);
    assert!(!flipped.mesh.is_consistently_oriented());
    let all = flipped.mesh.face_ids();
    let fixed = run_checked(flipped.mesh.clone(), faces_sel(&flipped.mesh, all), &Op::RecalculateOutside);
    closed_sphere_like(&fixed.mesh);
    // Inside-out whole cube is turned back too.
    let inv = run_checked(m.clone(), faces_sel(&m, m.face_ids()), &Op::FlipNormals);
    assert!(inv.mesh.signed_volume() < 0.0);
    let back = run_checked(inv.mesh.clone(), Selection::default(), &Op::RecalculateOutside);
    closed_sphere_like(&back.mesh);
}

#[test]
fn mark_sharp_and_seam_are_undoable() {
    let m = cube();
    let edge = edge_near(&m, Vec3::new(0.0, 1.0, 1.0));
    let e = run_checked(m.clone(), edges_sel(&m, [edge]), &Op::MarkSharp(true));
    assert!(e.mesh.edge(edge).is_some_and(|x| x.flags.sharp));
    let e = run_checked(e.mesh.clone(), edges_sel(&e.mesh, [edge]), &Op::MarkSeam(true));
    assert!(e.mesh.edge(edge).is_some_and(|x| x.flags.seam && x.flags.sharp));
    let e = run_checked(e.mesh.clone(), edges_sel(&e.mesh, [edge]), &Op::MarkSharp(false));
    assert!(e.mesh.edge(edge).is_some_and(|x| x.flags.seam && !x.flags.sharp));
}

#[test]
fn add_primitive_and_history_round_trip() {
    let mut edit = EditMesh::default();
    let mut h = History::new();
    h.apply(&mut edit, &Op::AddPrimitive { primitive: Primitive::Cube { size: 2.0 }, center: Vec3::ZERO }).expect("add");
    let top = face_towards(&edit.mesh, Vec3::Y);
    let select = Op::Select(Selection::from_faces(&edit.mesh, [top]));
    h.apply(&mut edit, &select).expect("select");
    h.apply(&mut edit, &Op::ExtrudeFaces { distance: 1.0, individual: false }).expect("extrude");
    h.apply(&mut edit, &Op::Inset { thickness: 0.2, depth: 0.0, individual: false }).expect("inset");
    let end = edit.mesh.content_hash();
    assert_eq!(h.labels(), vec!["Add shape", "Select", "Extrude", "Inset"]);
    while h.undo(&mut edit).is_some() {}
    assert_eq!(counts(&edit.mesh), (0, 0, 0));
    while h.redo(&mut edit).is_some() {}
    assert_eq!(edit.mesh.content_hash(), end);
    closed_sphere_like(&edit.mesh);
    // A failed op changes nothing and records nothing.
    let mut sel = edit.selection.clone();
    sel.clear();
    h.apply(&mut edit, &Op::Select(sel)).expect("deselect");
    let n = h.labels().len();
    assert!(h.apply(&mut edit, &Op::Inset { thickness: 0.1, depth: 0.0, individual: false }).is_err());
    assert_eq!(h.labels().len(), n);
    assert_eq!(edit.mesh.content_hash(), end);
}

#[test]
fn compacted_drops_tombstones_and_keeps_shape() {
    let m = cube();
    let top = face_towards(&m, Vec3::Y);
    let e = run_checked(m, faces_sel(&cube(), [top]), &Op::Inset { thickness: 0.3, depth: -0.2, individual: false });
    let c = e.mesh.compacted();
    valid(&c);
    assert_eq!(counts(&c), counts(&e.mesh));
    assert_eq!(c.slot_counts(), counts(&c));
    assert!((c.signed_volume() - e.mesh.signed_volume()).abs() < 1e-5);
}
