//! Triangulation, normals, UVs and export.

mod common;

use std::collections::BTreeSet;

use common::*;
use glam::{Vec2, Vec3};
use metallic_modeling::export::{to_glb, to_gltf, to_obj};
use metallic_modeling::uv::{UnwrapOptions, box_project, unwrap};
use metallic_modeling::{Mesh, NormalMode, Op, Primitive, Selection, triangulate};

#[test]
fn triangulate_cube_and_ngon() {
    let m = cube();
    let t = triangulate(&m, NormalMode::default());
    assert_eq!(t.data.triangle_count(), 12);
    assert_eq!(t.data.positions.len(), 24);
    assert_eq!(t.triangle_faces.len(), 12);
    assert_eq!(t.data.bounds.min, [-1.0; 3]);
    // Hexagon -> 4 triangles.
    let pts: Vec<Vec3> = (0..6)
        .map(|k| {
            let a = std::f32::consts::TAU * k as f32 / 6.0;
            Vec3::new(a.cos(), 0.0, -a.sin())
        })
        .collect();
    let hex = Mesh::from_polygons(&pts, &[(0..6).collect()]).expect("hex");
    let t = triangulate(&hex, NormalMode::Flat);
    assert_eq!(t.data.triangle_count(), 4);
    assert!(
        t.data
            .normals
            .iter()
            .all(|n| (Vec3::from_array(*n) - Vec3::Y).length() < 1e-5)
    );
}

#[test]
fn normals_follow_smoothing_and_sharp_edges() {
    let corner = |t: &metallic_modeling::Triangulated| -> Vec<Vec3> {
        t.data
            .positions
            .iter()
            .zip(&t.data.normals)
            .filter(|(p, _)| Vec3::from_array(**p).distance(Vec3::ONE) < 1e-5)
            .map(|(_, n)| Vec3::from_array(*n))
            .collect()
    };
    let m = cube();
    // Auto smooth (30°) keeps the cube faceted.
    for n in corner(&triangulate(&m, NormalMode::default())) {
        assert!(n.abs().max_element() > 0.999, "{n}");
    }
    // Fully smooth: the corner normal is the diagonal.
    for n in corner(&triangulate(&m, NormalMode::Smooth)) {
        assert!((n - Vec3::ONE.normalize()).length() < 1e-4, "{n}");
    }
    // Smooth, but every edge sharp: faceted again.
    let mut sharp = m.clone();
    let all: BTreeSet<_> = sharp.edge_ids().into_iter().collect();
    metallic_modeling::ops::normals::mark_sharp(&mut sharp, &all, true).expect("sharp");
    for n in corner(&triangulate(&sharp, NormalMode::Smooth)) {
        assert!(n.abs().max_element() > 0.999, "{n}");
    }
    // A sphere shades smoothly with auto smooth: one normal per vertex position away from the UV split.
    let s = Primitive::UvSphere {
        segments: 16,
        rings: 8,
        radius: 1.0,
    }
    .build(Vec3::ZERO)
    .expect("sphere");
    let t = triangulate(&s, NormalMode::default());
    for (p, n) in t.data.positions.iter().zip(&t.data.normals) {
        let p = Vec3::from_array(*p);
        assert!(
            p.normalize().dot(Vec3::from_array(*n)) > 0.97,
            "normal at {p} not radial"
        );
    }
}

fn uv_edge_ratios(m: &Mesh) -> Vec<f32> {
    let mut out = Vec::new();
    for (f, face) in m.faces() {
        let pts = m.face_positions(f);
        let n = pts.len();
        for i in 0..n {
            let d3 = pts[i].distance(pts[(i + 1) % n]);
            let d2 = face.uvs[i].distance(face.uvs[(i + 1) % n]);
            out.push(d2 / d3);
        }
    }
    out
}

fn assert_isometric(m: &Mesh) {
    let r = uv_edge_ratios(m);
    let mean = r.iter().sum::<f32>() / r.len() as f32;
    for x in &r {
        assert!((x / mean - 1.0).abs() < 2e-3, "uv distortion {x} vs {mean}");
    }
}

fn assert_in_unit_square_and_front_facing(m: &Mesh) {
    for (_, face) in m.faces() {
        for uv in &face.uvs {
            assert!(
                uv.x >= -1e-5 && uv.y >= -1e-5 && uv.x <= 1.0 + 1e-5 && uv.y <= 1.0 + 1e-5,
                "{uv}"
            );
        }
        assert!(
            metallic_modeling::geom::signed_area_2d(&face.uvs) > 0.0,
            "flipped UV face"
        );
    }
}

#[test]
fn lscm_flattens_a_flat_grid_without_distortion() {
    let mut m = Primitive::Grid {
        x: 4,
        z: 3,
        size: 2.0,
    }
    .build(Vec3::ZERO)
    .expect("grid");
    let faces: BTreeSet<_> = m.face_ids().into_iter().collect();
    let report = unwrap(&mut m, &faces, UnwrapOptions::default()).expect("unwrap");
    assert_eq!(report.charts, 1);
    assert_eq!(report.fallback_charts, 0);
    assert_isometric(&m);
    assert_in_unit_square_and_front_facing(&m);
}

#[test]
fn lscm_unfolds_a_cube_cut_along_a_cross_net() {
    let mut m = cube();
    // Cut 7 edges so the faces form a cross: front joins top, bottom, left, right; back joins top.
    let cuts = [
        Vec3::new(-1.0, 1.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0),
        Vec3::new(-1.0, -1.0, 0.0),
        Vec3::new(1.0, -1.0, 0.0),
        Vec3::new(0.0, -1.0, -1.0),
        Vec3::new(-1.0, 0.0, -1.0),
        Vec3::new(1.0, 0.0, -1.0),
    ];
    let seams: BTreeSet<_> = cuts.iter().map(|&p| edge_near(&m, p)).collect();
    metallic_modeling::ops::normals::mark_seam(&mut m, &seams, true).expect("seams");
    let faces = all_faces(&m);
    let report = unwrap(&mut m, &faces, UnwrapOptions::default()).expect("unwrap");
    assert_eq!((report.charts, report.fallback_charts), (1, 0));
    assert_isometric(&m);
    assert_in_unit_square_and_front_facing(&m);
    // Total UV area equals 6 equal squares of side s.
    let areas: Vec<f32> = m
        .faces()
        .map(|(_, f)| metallic_modeling::geom::signed_area_2d(&f.uvs))
        .collect();
    for a in &areas {
        assert!((a / areas[0] - 1.0).abs() < 1e-3);
    }
}

#[test]
fn unwrap_closed_shell_without_seams_falls_back_to_box() {
    let mut m = cube();
    let faces = all_faces(&m);
    let report = unwrap(&mut m, &faces, UnwrapOptions::default()).expect("unwrap");
    assert_eq!(report.fallback_charts, 1);
    assert_eq!(report.charts, 6);
    assert_in_unit_square_and_front_facing(&m);
}

#[test]
fn unwrap_two_charts_are_packed_apart() {
    let mut m = Primitive::Grid {
        x: 2,
        z: 1,
        size: 2.0,
    }
    .build(Vec3::ZERO)
    .expect("grid");
    let mid = edge_near(&m, Vec3::ZERO);
    metallic_modeling::ops::normals::mark_seam(&mut m, &BTreeSet::from([mid]), true).expect("seam");
    let faces = all_faces(&m);
    let report = unwrap(&mut m, &faces, UnwrapOptions::default()).expect("unwrap");
    assert_eq!(report.charts, 2);
    let boxes: Vec<(Vec2, Vec2)> = m
        .faces()
        .map(|(_, f)| {
            f.uvs
                .iter()
                .fold((Vec2::splat(9.0), Vec2::splat(-9.0)), |(l, h), &p| {
                    (l.min(p), h.max(p))
                })
        })
        .collect();
    let overlap = boxes[0].0.x < boxes[1].1.x - 1e-4
        && boxes[1].0.x < boxes[0].1.x - 1e-4
        && boxes[0].0.y < boxes[1].1.y - 1e-4
        && boxes[1].0.y < boxes[0].1.y - 1e-4;
    assert!(!overlap, "charts overlap: {boxes:?}");
}

#[test]
fn box_projection_uses_dominant_axis() {
    let mut m = cube();
    let faces = all_faces(&m);
    box_project(&mut m, &faces, 0.5).expect("box");
    let top = face_towards(&m, Vec3::Y);
    let face = m.face(top).expect("top");
    for (v, uv) in face.verts.iter().zip(&face.uvs) {
        let p = m.position_or_zero(*v);
        assert!((uv.x - p.x * 0.5).abs() < 1e-6 && (uv.y + p.z * 0.5).abs() < 1e-6);
    }
    assert!(
        m.faces()
            .all(|(_, f)| metallic_modeling::geom::signed_area_2d(&f.uvs) > 0.0)
    );
}

#[test]
fn uv_ops_are_undoable_commands() {
    let m = cube();
    let sel = faces_sel(&m, m.face_ids());
    run_checked(m.clone(), sel.clone(), &Op::UvBox { scale: 2.0 });
    run_checked(
        m.clone(),
        sel.clone(),
        &Op::UvPlanar {
            normal: Vec3::Z,
            scale: 1.0,
        },
    );
    run_checked(m, sel, &Op::UvUnwrap(UnwrapOptions::default()));
}

#[test]
fn glb_export_validates_with_the_gltf_crate() {
    let m = Primitive::UvSphere {
        segments: 12,
        rings: 6,
        radius: 1.0,
    }
    .build(Vec3::ZERO)
    .expect("sphere");
    let t = triangulate(&m, NormalMode::default());
    let glb = to_glb(&t.data, "Sphere").expect("glb");
    assert_eq!(&glb[0..4], b"glTF");
    assert_eq!(glb.len() % 4, 0);
    let gltf = gltf::Gltf::from_slice(&glb).expect("valid glb");
    let blob = gltf.blob.as_ref().expect("bin chunk");
    let mesh = gltf.document.meshes().next().expect("mesh");
    assert_eq!(mesh.name(), Some("Sphere"));
    let prim = mesh.primitives().next().expect("primitive");
    let pos = prim.get(&gltf::Semantic::Positions).expect("positions");
    assert_eq!(pos.count(), t.data.positions.len());
    assert_eq!(
        prim.get(&gltf::Semantic::Normals).map(|a| a.count()),
        Some(t.data.normals.len())
    );
    assert_eq!(
        prim.get(&gltf::Semantic::TexCoords(0)).map(|a| a.count()),
        Some(t.data.uv0.len())
    );
    assert_eq!(
        prim.indices().map(|a| a.count()),
        Some(t.data.indices.len())
    );
    let views: usize = gltf.document.views().map(|v| v.length()).sum();
    assert!(blob.len() >= views);
    // Bounds recorded on POSITION.
    let min = pos.min().expect("min");
    assert!((min[1].as_f64().unwrap_or(0.0) + 1.0).abs() < 1e-5);
}

#[test]
fn gltf_text_export_embeds_the_buffer() {
    let t = triangulate(&cube(), NormalMode::default());
    let text = to_gltf(&t.data, "Cube").expect("gltf");
    assert!(text.contains("data:application/octet-stream;base64,"));
    let gltf = gltf::Gltf::from_slice(text.as_bytes()).expect("valid gltf");
    assert_eq!(gltf.document.accessors().count(), 4);
    assert!(gltf.blob.is_none());
}

#[test]
fn export_rejects_empty_data() {
    let t = triangulate(&Mesh::new(), NormalMode::default());
    assert!(to_glb(&t.data, "Empty").is_err());
}

#[test]
fn obj_keeps_ngons() {
    let m = Primitive::Cylinder {
        segments: 6,
        radius: 1.0,
        height: 1.0,
    }
    .build(Vec3::ZERO)
    .expect("cyl");
    let obj = to_obj(&m, "Cyl");
    assert_eq!(obj.lines().filter(|l| l.starts_with("v ")).count(), 12);
    let faces: Vec<&str> = obj.lines().filter(|l| l.starts_with("f ")).collect();
    assert_eq!(faces.len(), 8);
    assert_eq!(
        faces
            .iter()
            .filter(|l| l.split_whitespace().count() == 7)
            .count(),
        2
    );
    let _ = Selection::default();
}
