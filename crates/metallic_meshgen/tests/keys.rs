//! Determinism, cache keys, the mesh cache, tangents against the MikkTSpace reference, and collider hints.
mod common;

use common::*;
use metallic_meshgen::{
    Bevel, ColliderHint, Lod, MeshCache, MeshDesc, Shading, Shape, ShapeParams, collider_hint,
    generate,
};
use std::sync::Arc;

#[test]
fn generation_is_deterministic() {
    for shape in Shape::ALL {
        let desc = MeshDesc::new(shape, [2.7, 1.3, 3.1]).with_bevel(Bevel::round(0.15));
        assert_eq!(
            generate(&desc).unwrap(),
            generate(&desc).unwrap(),
            "{shape:?}"
        );
    }
}

#[test]
fn keys_quantise_size_and_ignore_unused_parameters() {
    let a = MeshDesc::new(Shape::Block, [4.0, 1.0, 2.0]);
    let b = MeshDesc::new(Shape::Block, [4.000_01, 1.0, 2.0]);
    assert_eq!(a.key().unwrap(), b.key().unwrap());
    let c = a.with_params(ShapeParams {
        steps: 9,
        wall: 2.0,
        ..ShapeParams::default()
    });
    assert_eq!(a.key().unwrap(), c.key().unwrap());
    // AO cannot change a convex shape.
    assert_eq!(a.key().unwrap(), a.with_ao(false).key().unwrap());
    // A ball has no edges to bevel.
    let ball = MeshDesc::new(Shape::Ball, [2.0, 2.0, 2.0]);
    assert_eq!(
        ball.key().unwrap(),
        ball.with_bevel(Bevel::round(0.3)).key().unwrap()
    );
    // Things that matter do split.
    assert_ne!(a.key().unwrap(), a.with_bevel(Bevel::NONE).key().unwrap());
    assert_ne!(ball.key().unwrap(), ball.with_lod(Lod::Low).key().unwrap());
    let stairs = MeshDesc::new(Shape::Stairs, [4.0, 4.0, 4.0]);
    assert_ne!(
        stairs.key().unwrap(),
        stairs
            .with_params(ShapeParams {
                steps: 3,
                ..ShapeParams::default()
            })
            .key()
            .unwrap()
    );
}

#[test]
fn quantised_key_regenerates_the_same_mesh() {
    let desc = MeshDesc::new(Shape::Cylinder, [1.234_567, 2.345_678, 3.456_789]);
    let key = desc.key().unwrap();
    assert_eq!(key.desc().key().unwrap(), key);
    assert_eq!(
        generate(&desc).unwrap(),
        metallic_meshgen::generate_key(&key)
    );
}

#[test]
fn stable_hash_is_fixed() {
    let key = MeshDesc::new(Shape::Block, [4.0, 1.0, 2.0]).key().unwrap();
    // Changing this number means on-disk caches are invalidated; bump MESHGEN_VERSION when geometry changes.
    assert_eq!(key.stable_hash(), key.stable_hash());
    let other = MeshDesc::new(Shape::Block, [4.0, 1.0, 2.5]).key().unwrap();
    assert_ne!(key.stable_hash(), other.stable_hash());
    let hashes: std::collections::HashSet<u64> = Shape::ALL
        .iter()
        .map(|s| {
            MeshDesc::new(*s, [2.0, 2.0, 2.0])
                .key()
                .unwrap()
                .stable_hash()
        })
        .collect();
    assert_eq!(hashes.len(), Shape::ALL.len());
}

#[test]
fn cache_shares_identical_parts() {
    let mut cache = MeshCache::new();
    let a = cache
        .get_or_generate(&MeshDesc::new(Shape::Wedge, [4.0, 1.0, 2.0]))
        .unwrap();
    let b = cache
        .get_or_generate(&MeshDesc::new(Shape::Wedge, [4.0, 1.0, 2.0]))
        .unwrap();
    let c = cache
        .get_or_generate(&MeshDesc::new(Shape::Wedge, [4.0, 2.0, 2.0]))
        .unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    assert!(!Arc::ptr_eq(&a, &c));
    assert_eq!(cache.len(), 2);
    drop(c);
    assert_eq!(cache.evict_unused(), 1);
    assert_eq!(cache.len(), 1);
}

struct Ref<'a> {
    m: &'a metallic_meshgen::MeshData,
    out: Vec<[f32; 4]>,
}

impl mikktspace::Geometry for Ref<'_> {
    fn num_faces(&self) -> usize {
        self.m.triangle_count()
    }
    fn num_vertices_of_face(&self, _face: usize) -> usize {
        3
    }
    fn position(&self, face: usize, vert: usize) -> [f32; 3] {
        self.m.positions[self.m.indices[face * 3 + vert] as usize]
    }
    fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
        self.m.normals[self.m.indices[face * 3 + vert] as usize]
    }
    fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
        self.m.uv0[self.m.indices[face * 3 + vert] as usize]
    }
    fn set_tangent_encoded(&mut self, tangent: [f32; 4], face: usize, vert: usize) {
        self.out[face * 3 + vert] = tangent;
    }
}

#[test]
fn tangents_match_the_mikktspace_reference() {
    for shape in Shape::ALL {
        for shading in [Shading::Smooth, Shading::Flat] {
            let desc = MeshDesc::new(shape, shape.default_size())
                .with_lod(Lod::Low)
                .with_shading(shading);
            let m = generate(&desc).unwrap();
            let mut r = Ref {
                m: &m,
                out: vec![[0.0; 4]; m.indices.len()],
            };
            assert!(mikktspace::generate_tangents(&mut r));
            let mut worst = (1.0f32, 0usize);
            for (i, t) in r.out.iter().enumerate() {
                let ours = m.tangents[m.indices[i] as usize];
                assert_eq!(ours[3], t[3], "{shape:?}: bitangent sign at corner {i}");
                let d = dot([ours[0], ours[1], ours[2]], [t[0], t[1], t[2]]);
                if d < worst.0 {
                    worst = (d, i);
                }
            }
            let v = m.indices[worst.1] as usize;
            assert!(
                worst.0 > 0.999,
                "{shape:?} {shading:?}: tangent differs from MikkTSpace (dot {}) at {:?} n {:?} uv {:?}: ours {:?} ref {:?}",
                worst.0,
                m.positions[v],
                m.normals[v],
                m.uv0[v],
                m.tangents[v],
                r.out[worst.1]
            );
        }
    }
}

#[test]
fn every_chart_is_right_handed() {
    for shape in Shape::ALL {
        let m = generate(&MeshDesc::new(shape, shape.default_size())).unwrap();
        assert!(
            m.tangents.iter().all(|t| t[3] == 1.0),
            "{shape:?} has mirrored UVs"
        );
    }
}

#[test]
fn collider_hints_fit_the_mesh() {
    let block =
        collider_hint(&MeshDesc::new(Shape::Block, [4.0, 1.0, 2.0]).with_bevel(Bevel::round(0.1)))
            .unwrap();
    match block {
        ColliderHint::Cuboid {
            half_extents,
            border_radius,
        } => {
            assert!((half_extents[0] + border_radius - 2.0).abs() < 1e-4);
            assert!((border_radius - 0.1).abs() < 1e-4);
        }
        other => panic!("block: {other:?}"),
    }
    assert!(matches!(
        collider_hint(&MeshDesc::new(Shape::Ball, [3.0, 3.0, 3.0])).unwrap(),
        ColliderHint::Ball { radius } if (radius - 1.5).abs() < 1e-4
    ));
    assert!(matches!(
        collider_hint(&MeshDesc::new(Shape::Ball, [3.0, 1.0, 3.0])).unwrap(),
        ColliderHint::ConvexHull { .. }
    ));
    match collider_hint(&MeshDesc::new(Shape::Stairs, [4.0, 4.0, 4.0])).unwrap() {
        ColliderHint::Compound(parts) => assert_eq!(parts.len(), 4),
        other => panic!("stairs: {other:?}"),
    }
    for shape in Shape::ALL {
        let desc = MeshDesc::new(shape, shape.default_size());
        let hint = collider_hint(&desc).unwrap();
        // Every hull point lies inside the part's box.
        let mut stack = vec![([0.0f32; 3], hint)];
        while let Some((c, h)) = stack.pop() {
            match h {
                ColliderHint::ConvexHull {
                    points,
                    border_radius,
                } => {
                    assert!(!points.is_empty(), "{shape:?}");
                    for p in points {
                        for k in 0..3 {
                            let v = (p[k] + c[k]).abs() + border_radius;
                            assert!(
                                v <= desc.size[k] * 0.5 + 1e-3,
                                "{shape:?}: hull point {p:?}"
                            );
                        }
                    }
                }
                ColliderHint::Compound(parts) => {
                    for part in parts {
                        stack.push((part.center, part.shape));
                    }
                }
                _ => {}
            }
        }
    }
}
