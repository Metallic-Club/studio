//! Collision shape hints for physics.
//!
//! Each hint is the cheapest shape that matches the visible mesh closely: an exact primitive when one exists
//! (box, ball, cylinder, cone, capsule), a convex hull for other convex solids, or a compound of convex
//! pieces for concave shapes so dynamic (unanchored) parts collide correctly too. Rounded variants carry a
//! `border_radius`: the core shape plus a skin of that radius, which is exactly how the bevelled meshes are
//! built (Rapier's `round_cuboid`, `round_cylinder` and `round_convex_hull` take the same numbers).
use crate::error::MeshGenError;
use crate::polytope;
use crate::shape::{Bevel, Lod, MeshDesc, Shape};
use crate::shapes;
use glam::{Vec2, Vec3};
use std::f32::consts::TAU;

/// A coordinate axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    /// The X axis.
    X,
    /// The Y axis.
    Y,
    /// The Z axis.
    Z,
}

/// Suggested collision shape, in the mesh's own space (centred on the origin, studs).
#[derive(Clone, Debug, PartialEq)]
pub enum ColliderHint {
    /// A box; outer half size = `half_extents + border_radius`.
    Cuboid {
        /// Half size of the core box.
        half_extents: [f32; 3],
        /// Rounding radius added around the core (0 = sharp).
        border_radius: f32,
    },
    /// A sphere.
    Ball {
        /// Radius.
        radius: f32,
    },
    /// A circular cylinder; outer size = core + `border_radius`.
    Cylinder {
        /// Axis of the cylinder.
        axis: Axis,
        /// Half the core length along the axis.
        half_height: f32,
        /// Core radius.
        radius: f32,
        /// Rounding radius added around the core.
        border_radius: f32,
    },
    /// A circular cone with its tip toward +`axis`.
    Cone {
        /// Axis of the cone.
        axis: Axis,
        /// Half the height.
        half_height: f32,
        /// Base radius.
        radius: f32,
    },
    /// A capsule: a segment of length `2 * half_height` along `axis`, grown by `radius`.
    Capsule {
        /// Axis of the segment.
        axis: Axis,
        /// Half the segment length (not counting the round ends).
        half_height: f32,
        /// Radius.
        radius: f32,
    },
    /// The convex hull of `points`, grown by `border_radius`.
    ConvexHull {
        /// Hull points in studs.
        points: Vec<[f32; 3]>,
        /// Rounding radius added around the hull (0 = sharp).
        border_radius: f32,
    },
    /// Several convex pieces for a concave shape.
    Compound(Vec<ColliderPart>),
}

/// One piece of a [`ColliderHint::Compound`].
#[derive(Clone, Debug, PartialEq)]
pub struct ColliderPart {
    /// Offset of the piece from the mesh origin.
    pub center: [f32; 3],
    /// The piece, in its own space around `center`.
    pub shape: ColliderHint,
}

fn round_eq(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-4 * a.abs().max(b.abs()).max(1e-3)
}

/// Unique positions of a coarse, sharp version of the mesh (for hulls of curved convex shapes).
fn coarse_points(desc: &MeshDesc) -> Vec<[f32; 3]> {
    let coarse = MeshDesc {
        lod: Lod::Low,
        bevel: Bevel::NONE,
        ao: false,
        ..*desc
    };
    let key = match coarse.key() {
        Ok(k) => k,
        Err(_) => return Vec::new(),
    };
    let mesh = crate::generate_key(&key);
    let mut pts: Vec<[f32; 3]> = Vec::new();
    for p in mesh.positions {
        if !pts.contains(&p) {
            pts.push(p);
        }
    }
    pts
}

fn hull(points: Vec<Vec3>, center: Vec3) -> ColliderPart {
    ColliderPart {
        center: center.to_array(),
        shape: ColliderHint::ConvexHull {
            points: points.iter().map(|p| (*p - center).to_array()).collect(),
            border_radius: 0.0,
        },
    }
}

/// A convex prism piece: the 2D quad `q` (in the `u`,`v` plane) pushed ±`half_w` along `w`.
fn prism(q: [Vec2; 4], u: Vec3, v: Vec3, w: Vec3, half_w: f32) -> ColliderPart {
    let mut pts = Vec::with_capacity(8);
    for s in [-1.0f32, 1.0] {
        for p in q {
            pts.push(u * p.x + v * p.y + w * (s * half_w));
        }
    }
    let center = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
    hull(pts, center)
}

/// The collision hint for `desc` (validated and quantised the same way as the mesh).
pub fn collider_hint(desc: &MeshDesc) -> Result<ColliderHint, MeshGenError> {
    let desc = desc.key()?.desc();
    let h = desc.half();
    Ok(match desc.shape {
        Shape::Block | Shape::RoundedBlock => {
            let r = desc.bevel.width.min(0.95 * h.min_element());
            ColliderHint::Cuboid {
                half_extents: (h - Vec3::splat(r)).to_array(),
                border_radius: r,
            }
        }
        Shape::Wedge
        | Shape::CornerWedge
        | Shape::Pyramid
        | Shape::TriangularPrism
        | Shape::HexPrism => match shapes::convex_solid(&desc) {
            Some((_, planes)) => {
                let tol = shapes::polytope_tolerance(h);
                let r = polytope::clamp_radius(&planes, desc.bevel.width, tol);
                ColliderHint::ConvexHull {
                    points: polytope::inner_vertices(&planes, r, tol)
                        .iter()
                        .map(|p| p.to_array())
                        .collect(),
                    border_radius: r,
                }
            }
            None => ColliderHint::Cuboid {
                half_extents: h.to_array(),
                border_radius: 0.0,
            },
        },
        Shape::Ball => {
            if round_eq(h.x, h.y) && round_eq(h.y, h.z) {
                ColliderHint::Ball { radius: h.x }
            } else {
                ColliderHint::ConvexHull {
                    points: coarse_points(&desc),
                    border_radius: 0.0,
                }
            }
        }
        Shape::Cylinder => {
            if round_eq(h.y, h.z) {
                let r = desc.bevel.width.min(0.98 * h.x).min(0.98 * h.y);
                ColliderHint::Cylinder {
                    axis: Axis::X,
                    half_height: h.x - r,
                    radius: h.y - r,
                    border_radius: r,
                }
            } else {
                ColliderHint::ConvexHull {
                    points: coarse_points(&desc),
                    border_radius: 0.0,
                }
            }
        }
        Shape::Cone => {
            if round_eq(h.x, h.z) {
                ColliderHint::Cone {
                    axis: Axis::Y,
                    half_height: h.y,
                    radius: h.x,
                }
            } else {
                ColliderHint::ConvexHull {
                    points: coarse_points(&desc),
                    border_radius: 0.0,
                }
            }
        }
        Shape::Capsule => {
            if round_eq(h.y, h.z) && h.x >= h.y {
                ColliderHint::Capsule {
                    axis: Axis::X,
                    half_height: h.x - h.y,
                    radius: h.y,
                }
            } else {
                ColliderHint::ConvexHull {
                    points: coarse_points(&desc),
                    border_radius: 0.0,
                }
            }
        }
        Shape::Hemisphere => ColliderHint::ConvexHull {
            points: coarse_points(&desc),
            border_radius: 0.0,
        },
        Shape::Stairs => {
            let n = shapes::stair_steps(&desc);
            let (dz, dy) = (desc.size[2] / n as f32, desc.size[1] / n as f32);
            ColliderHint::Compound(
                (0..n)
                    .map(|i| {
                        let height = (i + 1) as f32 * dy;
                        ColliderPart {
                            center: [0.0, -h.y + 0.5 * height, -h.z + (i as f32 + 0.5) * dz],
                            shape: ColliderHint::Cuboid {
                                half_extents: [h.x, 0.5 * height, 0.5 * dz],
                                border_radius: 0.0,
                            },
                        }
                    })
                    .collect(),
            )
        }
        Shape::Ramp => {
            let coarse = MeshDesc {
                lod: Lod::Low,
                ..desc
            };
            let curve = shapes::ramp_curve(&coarse, 0.0);
            let mut parts: Vec<ColliderPart> = curve
                .windows(2)
                .map(|w| {
                    let (a, b) = (w[1], w[0]); // front to back
                    prism(
                        [Vec2::new(a.x, -h.y), Vec2::new(b.x, -h.y), b, a],
                        Vec3::Z,
                        Vec3::Y,
                        Vec3::X,
                        h.x,
                    )
                })
                .collect();
            let deck = curve[0].x;
            if h.z - deck > 1e-4 {
                parts.push(ColliderPart {
                    center: [0.0, 0.0, 0.5 * (deck + h.z)],
                    shape: ColliderHint::Cuboid {
                        half_extents: [h.x, h.y, 0.5 * (h.z - deck)],
                        border_radius: 0.0,
                    },
                });
            }
            ColliderHint::Compound(parts)
        }
        Shape::Arch => {
            let coarse = MeshDesc {
                lod: Lod::Low,
                ..desc
            };
            let opening = shapes::arch_opening(&coarse);
            let a = opening[0].x.abs();
            let leg = 0.5 * (h.x - a);
            let mut parts = vec![
                ColliderPart {
                    center: [-h.x + leg, 0.0, 0.0],
                    shape: ColliderHint::Cuboid {
                        half_extents: [leg, h.y, h.z],
                        border_radius: 0.0,
                    },
                },
                ColliderPart {
                    center: [h.x - leg, 0.0, 0.0],
                    shape: ColliderHint::Cuboid {
                        half_extents: [leg, h.y, h.z],
                        border_radius: 0.0,
                    },
                },
            ];
            for w in opening.windows(2) {
                let (p, q) = (w[0], w[1]);
                if (q.x - p.x).abs() < 1e-5 {
                    continue; // a vertical jamb: covered by the leg boxes
                }
                parts.push(prism(
                    [p, q, Vec2::new(q.x, h.y), Vec2::new(p.x, h.y)],
                    Vec3::X,
                    Vec3::Y,
                    Vec3::Z,
                    h.z,
                ));
            }
            ColliderHint::Compound(parts)
        }
        Shape::Pipe => {
            let wall = shapes::pipe_wall(&desc);
            let n = 16;
            let ring = |i: usize, inset: f32| {
                let t = TAU * i as f32 / n as f32;
                Vec3::new(0.0, (h.y - inset) * t.cos(), (h.z - inset) * t.sin())
            };
            ColliderHint::Compound(
                (0..n)
                    .map(|i| {
                        let mut pts = Vec::with_capacity(8);
                        for x in [-h.x, h.x] {
                            for p in [
                                ring(i, 0.0),
                                ring(i + 1, 0.0),
                                ring(i, wall),
                                ring(i + 1, wall),
                            ] {
                                pts.push(p + Vec3::X * x);
                            }
                        }
                        let c = pts.iter().copied().sum::<Vec3>() / 8.0;
                        hull(pts, c)
                    })
                    .collect(),
            )
        }
        Shape::Torus => {
            let k = shapes::torus_tube(&desc) / h.y.min(h.z);
            let (n, m) = (16, 8);
            let point = |i: usize, j: usize| {
                let t = TAU * i as f32 / n as f32;
                let s = TAU * j as f32 / m as f32;
                let ring = Vec3::new(0.0, h.y * t.cos(), h.z * t.sin());
                Vec3::X * (h.x * s.cos()) + ring * (1.0 - k - k * s.sin())
            };
            ColliderHint::Compound(
                (0..n)
                    .map(|i| {
                        let pts: Vec<Vec3> = (0..m)
                            .flat_map(|j| [point(i, j), point(i + 1, j)])
                            .collect();
                        let c = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
                        hull(pts, c)
                    })
                    .collect(),
            )
        }
    })
}
