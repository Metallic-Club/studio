//! Geometry checks for every shape: watertight, outward, no degenerate triangles, bounds, UV density, bevels.
mod common;

use common::*;
use metallic_meshgen::{
    Bevel, BevelStyle, Lod, MeshDesc, MeshGenError, Shading, Shape, ShapeParams, generate,
};

fn bevels() -> [Bevel; 3] {
    [Bevel::NONE, Bevel::round(0.1), Bevel::chamfer(0.1)]
}

#[test]
fn every_shape_lod_and_bevel_is_a_valid_closed_mesh() {
    for shape in Shape::ALL {
        for lod in Lod::ALL {
            for bevel in bevels() {
                let desc = MeshDesc::new(shape, shape.default_size())
                    .with_lod(lod)
                    .with_bevel(bevel);
                let m = generate(&desc).unwrap();
                assert_valid(&m, &desc, &format!("{shape:?} {lod:?} {bevel:?}"));
            }
        }
    }
}

#[test]
fn shading_modes_are_valid() {
    for shape in Shape::ALL {
        for shading in [Shading::Flat, Shading::Crease { angle_deg: 40.0 }] {
            let desc = MeshDesc::new(shape, shape.default_size()).with_shading(shading);
            let m = generate(&desc).unwrap();
            assert_valid(&m, &desc, &format!("{shape:?} {shading:?}"));
        }
    }
}

#[test]
fn bounds_match_size_without_bevel() {
    for shape in Shape::ALL {
        let size = [3.0, 1.5, 2.5];
        let desc = MeshDesc::new(shape, size).with_bevel(Bevel::NONE);
        let m = generate(&desc).unwrap();
        let s = m.bounds.size();
        for k in 0..3 {
            assert!(
                (s[k] - size[k]).abs() < 1e-4,
                "{shape:?}: bounds {:?} vs size {size:?}",
                m.bounds
            );
            assert!(
                (m.bounds.min[k] + m.bounds.max[k]).abs() < 1e-4 || shape == Shape::CornerWedge
            );
        }
    }
}

#[test]
fn rounded_box_like_shapes_still_fill_their_box() {
    for shape in [
        Shape::Block,
        Shape::RoundedBlock,
        Shape::Cylinder,
        Shape::Pipe,
        Shape::Stairs,
        Shape::Arch,
    ] {
        let size = [3.0, 2.0, 2.5];
        let desc = MeshDesc::new(shape, size).with_bevel(Bevel::round(0.2));
        let m = generate(&desc).unwrap();
        let s = m.bounds.size();
        for k in 0..3 {
            assert!((s[k] - size[k]).abs() < 1e-3, "{shape:?}: {:?}", m.bounds);
        }
    }
}

#[test]
fn closed_volume_matches_the_solid() {
    let cases = [
        (Shape::Block, [4.0, 1.0, 2.0], 8.0),
        (Shape::Wedge, [4.0, 1.0, 2.0], 4.0),
        (Shape::Pyramid, [3.0, 3.0, 3.0], 9.0),
        (Shape::CornerWedge, [3.0, 3.0, 3.0], 9.0),
        (Shape::TriangularPrism, [4.0, 2.0, 4.0], 16.0),
        (
            Shape::Ball,
            [2.0, 2.0, 2.0],
            4.0 / 3.0 * std::f32::consts::PI,
        ),
        (Shape::Cylinder, [2.0, 2.0, 2.0], 2.0 * std::f32::consts::PI),
        (Shape::Cone, [2.0, 3.0, 2.0], std::f32::consts::PI),
        (
            Shape::Hemisphere,
            [2.0, 1.0, 2.0],
            2.0 / 3.0 * std::f32::consts::PI,
        ),
        (
            Shape::Stairs,
            [2.0, 4.0, 4.0],
            2.0 * 4.0 * 4.0 * (4.0 + 1.0) / 8.0,
        ),
    ];
    for (shape, size, expected) in cases {
        let desc = MeshDesc::new(shape, size).with_bevel(Bevel::NONE);
        let v = signed_volume(&generate(&desc).unwrap());
        assert!(
            (v - expected).abs() / expected < 0.01,
            "{shape:?}: volume {v} expected {expected}"
        );
    }
}

#[test]
fn bevel_width_is_the_same_at_every_size() {
    for size in [1.0f32, 4.0, 40.0] {
        let desc = MeshDesc::new(Shape::Block, [size, size, size]).with_bevel(Bevel::round(0.1));
        let m = generate(&desc).unwrap();
        // The flat top face (normal exactly +Y) ends exactly one bevel width before the edge.
        let edge = m
            .positions
            .iter()
            .zip(&m.normals)
            .filter(|(_, n)| **n == [0.0, 1.0, 0.0])
            .map(|(p, _)| p[0].abs())
            .fold(0.0f32, f32::max);
        assert!(
            (size * 0.5 - edge - 0.1).abs() < 1e-4,
            "size {size}: flat face ends at {edge}"
        );
    }
}

#[test]
fn bevel_is_clamped_on_thin_parts() {
    let desc = MeshDesc::new(Shape::Block, [4.0, 0.2, 2.0]).with_bevel(Bevel::round(1.0));
    let m = generate(&desc).unwrap();
    assert_valid(&m, &desc, "thin plate");
    assert!((m.bounds.size()[1] - 0.2).abs() < 1e-4);
}

#[test]
fn uv0_is_one_unit_per_stud_and_size_independent() {
    for shape in Shape::ALL {
        let base = shape.default_size();
        let small = generate(&MeshDesc::new(shape, base)).unwrap();
        let big = generate(&MeshDesc::new(shape, base.map(|v| v * 3.0))).unwrap();
        let (ds, db) = (median_uv_density(&small), median_uv_density(&big));
        assert!((0.55..=1.05).contains(&ds), "{shape:?}: density {ds}");
        assert!(
            (ds - db).abs() < 0.08,
            "{shape:?}: density changes with size: {ds} vs {db}"
        );
        // No triangle is badly stretched or squashed.
        for t in 0..big.triangle_count() {
            let (a, _) = tri_geom(&big, t);
            if a > 1e-6 {
                let r = uv_area(&big, t).abs() / a;
                assert!(
                    (0.2..=4.0).contains(&r),
                    "{shape:?}: triangle {t} uv/area {r}"
                );
            }
        }
    }
}

#[test]
fn flat_faces_map_exactly_one_uv_per_stud() {
    for shape in [Shape::Block, Shape::Wedge, Shape::Pyramid, Shape::Stairs] {
        let m = generate(&MeshDesc::new(shape, [5.0, 3.0, 4.0]).with_bevel(Bevel::NONE)).unwrap();
        for t in 0..m.triangle_count() {
            let (a, _) = tri_geom(&m, t);
            let r = uv_area(&m, t) / a;
            assert!(
                (r - 1.0).abs() < 1e-3,
                "{shape:?}: triangle {t} uv/area {r}"
            );
        }
    }
}

#[test]
fn block_uv1_covers_each_face_once() {
    let m =
        generate(&MeshDesc::new(Shape::Block, [4.0, 1.0, 2.0]).with_bevel(Bevel::NONE)).unwrap();
    assert_eq!(m.vertex_count(), 24);
    assert_eq!(m.triangle_count(), 12);
    for uv in &m.uv1 {
        assert!(uv.iter().all(|v| *v == 0.0 || *v == 1.0), "{uv:?}");
    }
}

#[test]
fn convex_shapes_have_no_occlusion_and_stairs_darken_their_creases() {
    let ball = generate(&MeshDesc::new(Shape::Ball, [2.0, 2.0, 2.0])).unwrap();
    assert!(ball.ao.iter().all(|a| *a == 1.0));
    let stairs = generate(&MeshDesc::new(Shape::Stairs, [4.0, 4.0, 4.0])).unwrap();
    let min = stairs.ao.iter().copied().fold(1.0f32, f32::min);
    assert!(min < 0.8, "crease ao {min}");
    // The top tread far from any riser is open.
    let open = stairs
        .positions
        .iter()
        .zip(&stairs.ao)
        .filter(|(p, _)| p[1] > 1.999 && p[2] > 1.5)
        .map(|(_, a)| *a)
        .fold(0.0f32, f32::max);
    assert!(open > 0.9, "open tread ao {open}");
    let off = generate(&MeshDesc::new(Shape::Stairs, [4.0, 4.0, 4.0]).with_ao(false)).unwrap();
    assert!(off.ao.iter().all(|a| *a == 1.0));
}

#[test]
fn flat_shading_gives_every_triangle_its_own_normal() {
    let m = generate(
        &MeshDesc::new(Shape::Ball, [2.0, 2.0, 2.0])
            .with_shading(Shading::Flat)
            .with_lod(Lod::Low),
    )
    .unwrap();
    for t in 0..m.triangle_count() {
        let (_, g) = tri_geom(&m, t);
        for v in tri(&m, t) {
            assert!(dot(g, m.normals[v]) > 0.9999);
        }
    }
}

#[test]
fn crease_shading_keeps_box_edges_hard_and_smooths_low_poly_curves() {
    let block = generate(
        &MeshDesc::new(Shape::Block, [2.0, 2.0, 2.0])
            .with_bevel(Bevel::NONE)
            .with_shading(Shading::Crease { angle_deg: 30.0 }),
    )
    .unwrap();
    assert_eq!(block.vertex_count(), 24);
    let cyl = generate(
        &MeshDesc::new(Shape::Cylinder, [2.0, 2.0, 2.0])
            .with_lod(Lod::Low)
            .with_bevel(Bevel::NONE)
            .with_shading(Shading::Crease { angle_deg: 60.0 }),
    )
    .unwrap();
    // Side normals are radial (smoothed), caps are flat.
    let smooth = cyl
        .normals
        .iter()
        .filter(|n| n[0] == 0.0 && n[1].abs() > 0.1 && n[2].abs() > 0.1)
        .count();
    assert!(smooth > 0);
}

#[test]
fn lod_changes_curved_shapes_only() {
    let count = |shape, lod| {
        generate(&MeshDesc::new(shape, [4.0, 4.0, 4.0]).with_lod(lod))
            .unwrap()
            .triangle_count()
    };
    assert!(count(Shape::Ball, Lod::Low) < count(Shape::Ball, Lod::Medium));
    assert!(count(Shape::Ball, Lod::Medium) < count(Shape::Ball, Lod::High));
    assert!(count(Shape::Torus, Lod::Low) < count(Shape::Torus, Lod::High));
    let flat = |lod| {
        MeshDesc::new(Shape::Wedge, [4.0, 1.0, 2.0])
            .with_bevel(Bevel::chamfer(0.1))
            .with_lod(lod)
    };
    assert_eq!(
        flat(Lod::Low).key().unwrap(),
        flat(Lod::High).key().unwrap()
    );
}

#[test]
fn bigger_curves_get_more_segments() {
    let small = generate(&MeshDesc::new(Shape::Cylinder, [1.0, 1.0, 1.0])).unwrap();
    let large = generate(&MeshDesc::new(Shape::Cylinder, [1.0, 40.0, 40.0])).unwrap();
    assert!(large.triangle_count() > small.triangle_count());
}

#[test]
fn random_sizes_bevels_and_params_stay_valid() {
    // MESHGEN_ROUNDS raises the count for a longer local soak (the default keeps CI fast).
    let rounds = std::env::var("MESHGEN_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(12);
    let seed = std::env::var("MESHGEN_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0x5eed_35);
    let mut rng = Rng::new(seed);
    for round in 0..rounds {
        for shape in Shape::ALL {
            let size = [
                rng.log_range(0.05, 60.0),
                rng.log_range(0.05, 60.0),
                rng.log_range(0.05, 60.0),
            ];
            let style = if rng.range(0.0, 1.0) < 0.5 {
                BevelStyle::Round
            } else {
                BevelStyle::Chamfer
            };
            let bevel = Bevel {
                width: rng.range(0.0, 0.6),
                style,
            };
            let lod = Lod::ALL[(rng.next_u64() % 3) as usize];
            let params = ShapeParams {
                steps: (rng.next_u64() % 12) as u32,
                wall: rng.range(0.0, 3.0),
                thickness: rng.range(0.0, 4.0),
                corner_radius: rng.range(0.0, 2.0),
            };
            let desc = MeshDesc::new(shape, size)
                .with_bevel(bevel)
                .with_lod(lod)
                .with_params(params);
            let m = generate(&desc).unwrap();
            assert_valid(
                &m,
                &desc,
                &format!("round {round} {shape:?} {size:?} {bevel:?} {lod:?} {params:?}"),
            );
        }
    }
}

#[test]
fn tiny_and_huge_parts_work() {
    for shape in Shape::ALL {
        for size in [
            [0.001, 0.001, 0.001],
            [0.01, 2.0, 0.5],
            [2048.0, 1.0, 2048.0],
        ] {
            let desc = MeshDesc::new(shape, size).with_lod(Lod::Low);
            let m = generate(&desc).unwrap();
            assert_valid(&m, &desc, &format!("{shape:?} {size:?}"));
        }
    }
}

#[test]
fn invalid_requests_are_typed_errors() {
    assert!(matches!(
        generate(&MeshDesc::new(Shape::Block, [0.0, 1.0, 1.0])),
        Err(MeshGenError::InvalidSize(_))
    ));
    assert!(matches!(
        generate(&MeshDesc::new(Shape::Block, [f32::NAN, 1.0, 1.0])),
        Err(MeshGenError::InvalidSize(_))
    ));
    assert!(matches!(
        generate(&MeshDesc::new(Shape::Block, [1e6, 1.0, 1.0])),
        Err(MeshGenError::SizeTooLarge { .. })
    ));
    assert!(matches!(
        generate(&MeshDesc::new(Shape::Block, [1.0, 1.0, 1.0]).with_bevel(Bevel::round(-1.0))),
        Err(MeshGenError::InvalidParameter { .. })
    ));
    assert!(matches!(
        "Blob".parse::<Shape>(),
        Err(MeshGenError::UnknownShape(_))
    ));
}

#[test]
fn names_round_trip_and_core_names_are_known() {
    for shape in Shape::ALL {
        assert_eq!(Shape::from_name(shape.name()), Some(shape));
    }
    for core in ["Block", "Ball", "Cylinder", "Wedge"] {
        assert!(Shape::from_name(core).is_some());
    }
    assert_eq!(Shape::from_name("Sphere"), Some(Shape::Ball));
}

#[test]
fn obj_export_has_every_vertex_and_face() {
    let m = generate(&MeshDesc::new(Shape::Wedge, [4.0, 1.0, 2.0])).unwrap();
    let obj = m.to_obj("wedge");
    assert_eq!(
        obj.lines().filter(|l| l.starts_with("v ")).count(),
        m.vertex_count()
    );
    assert_eq!(
        obj.lines().filter(|l| l.starts_with("f ")).count(),
        m.triangle_count()
    );
    assert_eq!(m.interleaved().len(), m.vertex_count());
}
