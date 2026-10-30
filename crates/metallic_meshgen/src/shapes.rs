//! The shape definitions: each shape is a convex solid, an extruded profile, a lathe, or (the ball) a
//! cube-sphere. This file only describes geometry; the generators do the meshing.
use crate::builder::{Builder, Corner, weld_tolerance};
use crate::extrude::{self, Extrusion, ProfilePoint};
use crate::lathe::{self, Lathe, Run, RunKind, arc};
use crate::math::{AXES6, Frame};
use crate::polytope::{self, Plane};
use crate::shape::{BevelStyle, MeshDesc, Shape};
use crate::tess::{arc_segments, bevel_segments, circle_segments};
use glam::{Vec2, Vec3};
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

/// Builds the triangle soup for `desc` (already quantised).
pub(crate) fn build(desc: &MeshDesc) -> Builder {
    let mut b = Builder::new();
    match desc.shape {
        Shape::Block
        | Shape::RoundedBlock
        | Shape::Wedge
        | Shape::CornerWedge
        | Shape::Pyramid
        | Shape::TriangularPrism
        | Shape::HexPrism => {
            if let Some((_, planes)) = convex_solid(desc) {
                let half = desc.half();
                let tol = polytope_tolerance(half);
                let r = polytope::clamp_radius(&planes, desc.bevel.width, tol);
                let segs = bevel_segments(r, desc.lod);
                polytope::emit(&mut b, &planes, half, r, desc.bevel.style, segs, tol);
            }
        }
        Shape::Ball => ball(&mut b, desc),
        Shape::Cylinder => cylinder(&mut b, desc),
        Shape::Pipe => pipe(&mut b, desc),
        Shape::Cone => cone(&mut b, desc),
        Shape::Hemisphere => hemisphere(&mut b, desc),
        Shape::Capsule => capsule(&mut b, desc),
        Shape::Torus => torus(&mut b, desc),
        Shape::Stairs | Shape::Ramp | Shape::Arch => {
            if let Some((ex, profile, r)) = extrusion(desc) {
                let segs = bevel_segments(r, desc.lod);
                let tol = weld_tolerance(desc.half());
                extrude::emit(
                    &mut b,
                    &ex,
                    &profile,
                    r,
                    desc.bevel.style,
                    segs,
                    desc.ao,
                    tol,
                );
            }
        }
    }
    b
}

pub(crate) fn polytope_tolerance(half: Vec3) -> f32 {
    1e-8 + half.max_element() * 1e-5
}

/// Hull vertices and planes of the convex shapes (`None` for the others).
pub(crate) fn convex_solid(desc: &MeshDesc) -> Option<(Vec<Vec3>, Vec<Plane>)> {
    let h = desc.half();
    let v = |x: f32, y: f32, z: f32| Vec3::new(x * h.x, y * h.y, z * h.z);
    let (verts, faces): (Vec<Vec3>, Vec<Vec<usize>>) = match desc.shape {
        Shape::Block | Shape::RoundedBlock => {
            let verts = (0..8)
                .map(|i| {
                    let s = |bit: usize| if i & bit != 0 { 1.0 } else { -1.0 };
                    v(s(4), s(2), s(1))
                })
                .collect();
            let faces = vec![
                vec![0, 1, 3, 2],
                vec![4, 5, 7, 6],
                vec![0, 1, 5, 4],
                vec![2, 3, 7, 6],
                vec![0, 2, 6, 4],
                vec![1, 3, 7, 5],
            ];
            (verts, faces)
        }
        Shape::Wedge => (
            vec![
                v(-1.0, -1.0, -1.0),
                v(1.0, -1.0, -1.0),
                v(1.0, -1.0, 1.0),
                v(-1.0, -1.0, 1.0),
                v(-1.0, 1.0, 1.0),
                v(1.0, 1.0, 1.0),
            ],
            vec![
                vec![0, 1, 2],
                vec![2, 3, 4],
                vec![0, 3, 4],
                vec![1, 2, 5],
                vec![0, 1, 5],
            ],
        ),
        Shape::CornerWedge => (
            vec![
                v(-1.0, -1.0, -1.0),
                v(1.0, -1.0, -1.0),
                v(1.0, -1.0, 1.0),
                v(-1.0, -1.0, 1.0),
                v(1.0, 1.0, 1.0),
            ],
            vec![
                vec![0, 1, 2],
                vec![1, 2, 4],
                vec![2, 3, 4],
                vec![0, 3, 4],
                vec![0, 1, 4],
            ],
        ),
        Shape::Pyramid => (
            vec![
                v(-1.0, -1.0, -1.0),
                v(1.0, -1.0, -1.0),
                v(1.0, -1.0, 1.0),
                v(-1.0, -1.0, 1.0),
                v(0.0, 1.0, 0.0),
            ],
            vec![
                vec![0, 1, 2],
                vec![0, 1, 4],
                vec![1, 2, 4],
                vec![2, 3, 4],
                vec![3, 0, 4],
            ],
        ),
        Shape::TriangularPrism => (
            vec![
                v(-1.0, -1.0, -1.0),
                v(1.0, -1.0, -1.0),
                v(1.0, -1.0, 1.0),
                v(-1.0, -1.0, 1.0),
                v(-1.0, 1.0, 0.0),
                v(1.0, 1.0, 0.0),
            ],
            vec![
                vec![0, 1, 2],
                vec![0, 1, 5],
                vec![2, 3, 4],
                vec![0, 3, 4],
                vec![1, 2, 5],
            ],
        ),
        Shape::HexPrism => {
            let s60 = (PI / 3.0).sin();
            let mut verts = Vec::with_capacity(12);
            for x in [-1.0f32, 1.0] {
                for k in 0..6 {
                    let a = PI / 3.0 * k as f32;
                    verts.push(Vec3::new(x * h.x, h.y * a.sin() / s60, h.z * a.cos()));
                }
            }
            let mut faces = vec![vec![0, 1, 2], vec![6, 7, 8]];
            for k in 0..6 {
                faces.push(vec![k, (k + 1) % 6, 6 + k]);
            }
            (verts, faces)
        }
        _ => return None,
    };
    let refs: Vec<&[usize]> = faces.iter().map(|f| f.as_slice()).collect();
    let planes = polytope::planes_from_faces(&verts, &refs);
    Some((verts, planes))
}

/// Ellipsoid as a cube-sphere: six warped grids (even spacing, no poles), one UV chart per cube face.
fn ball(b: &mut Builder, desc: &MeshDesc) {
    let h = desc.half();
    let per_face = (circle_segments(h.max_element(), desc.lod) / 4).max(2);
    let n = per_face + per_face % 2;
    for axis in AXES6 {
        let f = Frame::for_normal(axis, Vec3::ZERO);
        let uvf = Frame::for_normal(axis, h);
        let chart = b.chart();
        let point = |i: u32, j: u32| -> Corner {
            let s = (2.0 * i as f32 / n as f32 - 1.0) * FRAC_PI_4;
            let t = (2.0 * j as f32 / n as f32 - 1.0) * FRAC_PI_4;
            let d = (axis + f.u * s.tan() + f.v * t.tan()).normalize_or(axis);
            let p = d * h;
            let nn = (d / h).normalize_or(d);
            Corner::new(p, nn, uvf.uv(p))
        };
        for i in 0..n {
            for j in 0..n {
                b.quad(
                    chart,
                    point(i, j),
                    point(i + 1, j),
                    point(i + 1, j + 1),
                    point(i, j + 1),
                );
            }
        }
    }
}

fn bevel(desc: &MeshDesc) -> (f32, u32) {
    let r = desc.bevel.width.max(0.0);
    let segs = if desc.bevel.style == BevelStyle::Chamfer {
        1
    } else {
        bevel_segments(r, desc.lod)
    };
    (r, segs)
}

/// Cylinder along X; bevel rounds both rims.
fn cylinder(b: &mut Builder, desc: &MeshDesc) {
    let h = desc.half();
    let la = Lathe::around(0, h, circle_segments(h.y.max(h.z), desc.lod));
    let (r, segs) = bevel(desc);
    let r = r.min(0.98 * h.x).min(0.98 * h.y.min(h.z));
    let hl = h.x;
    lathe::emit(b, &la, false, |rad| {
        if r > 0.0 {
            vec![
                Run::new(
                    RunKind::Cap,
                    vec![Vec2::new(-hl, 0.0), Vec2::new(-hl, rad - r)],
                ),
                Run::new(
                    RunKind::Round,
                    arc(Vec2::new(-hl + r, rad - r), r, PI, FRAC_PI_2, segs),
                ),
                Run::new(
                    RunKind::Straight,
                    vec![Vec2::new(-hl + r, rad), Vec2::new(hl - r, rad)],
                ),
                Run::new(
                    RunKind::Round,
                    arc(Vec2::new(hl - r, rad - r), r, FRAC_PI_2, 0.0, segs),
                ),
                Run::new(
                    RunKind::Cap,
                    vec![Vec2::new(hl, rad - r), Vec2::new(hl, 0.0)],
                ),
            ]
        } else {
            vec![
                Run::new(RunKind::Cap, vec![Vec2::new(-hl, 0.0), Vec2::new(-hl, rad)]),
                Run::new(
                    RunKind::Straight,
                    vec![Vec2::new(-hl, rad), Vec2::new(hl, rad)],
                ),
                Run::new(RunKind::Cap, vec![Vec2::new(hl, rad), Vec2::new(hl, 0.0)]),
            ]
        }
    });
}

/// Pipe wall thickness after clamping (the hole keeps at least 10% of the radius).
pub(crate) fn pipe_wall(desc: &MeshDesc) -> f32 {
    let h = desc.half();
    desc.params
        .wall
        .clamp(h.y.min(h.z) * 0.02, 0.9 * h.y.min(h.z))
}

/// Tube along X; bevel rounds the four rims.
fn pipe(b: &mut Builder, desc: &MeshDesc) {
    let h = desc.half();
    let la = Lathe::around(0, h, circle_segments(h.y.max(h.z), desc.lod));
    let wall = pipe_wall(desc);
    let (r, segs) = bevel(desc);
    let r = r.min(0.49 * wall).min(0.98 * h.x);
    let hl = h.x;
    lathe::emit(b, &la, true, |rad| {
        let ri = rad - wall;
        if r > 0.0 {
            vec![
                Run::new(
                    RunKind::Cap,
                    vec![Vec2::new(-hl, ri + r), Vec2::new(-hl, rad - r)],
                ),
                Run::new(
                    RunKind::Round,
                    arc(Vec2::new(-hl + r, rad - r), r, PI, FRAC_PI_2, segs),
                ),
                Run::new(
                    RunKind::Straight,
                    vec![Vec2::new(-hl + r, rad), Vec2::new(hl - r, rad)],
                ),
                Run::new(
                    RunKind::Round,
                    arc(Vec2::new(hl - r, rad - r), r, FRAC_PI_2, 0.0, segs),
                ),
                Run::new(
                    RunKind::Cap,
                    vec![Vec2::new(hl, rad - r), Vec2::new(hl, ri + r)],
                ),
                Run::new(
                    RunKind::Round,
                    arc(Vec2::new(hl - r, ri + r), r, 0.0, -FRAC_PI_2, segs),
                ),
                Run::new(
                    RunKind::Straight,
                    vec![Vec2::new(hl - r, ri), Vec2::new(-hl + r, ri)],
                ),
                Run::new(
                    RunKind::Round,
                    arc(Vec2::new(-hl + r, ri + r), r, -FRAC_PI_2, -PI, segs),
                ),
            ]
        } else {
            vec![
                Run::new(RunKind::Cap, vec![Vec2::new(-hl, ri), Vec2::new(-hl, rad)]),
                Run::new(
                    RunKind::Straight,
                    vec![Vec2::new(-hl, rad), Vec2::new(hl, rad)],
                ),
                Run::new(RunKind::Cap, vec![Vec2::new(hl, rad), Vec2::new(hl, ri)]),
                Run::new(
                    RunKind::Straight,
                    vec![Vec2::new(hl, ri), Vec2::new(-hl, ri)],
                ),
            ]
        }
    });
}

/// Cone along +Y; bevel rounds the base rim (the tip stays sharp).
fn cone(b: &mut Builder, desc: &MeshDesc) {
    let h = desc.half();
    let la = Lathe::around(1, h, circle_segments(h.x.max(h.z), desc.lod));
    let (r, segs) = bevel(desc);
    let hl = h.y;
    // Fillet tangent length per unit radius at base radius `rad`: tan(turn / 2) where turn is the angle the
    // outline turns through at the rim. Written as (L + rad) / 2h (L = slant length) to stay exact for very
    // flat cones, where the acos form loses all precision.
    let tan_half = |rad: f32| (Vec2::new(2.0 * hl, rad).length() + rad) / (2.0 * hl);
    let mut r = r;
    for rad in [h.x, h.z] {
        let slant = Vec2::new(2.0 * hl, rad).length();
        r = r
            .min(0.9 * rad / tan_half(rad))
            .min(0.45 * slant / tan_half(rad));
    }
    lathe::emit(b, &la, false, |rad| {
        if r > 0.0 {
            let lt = r * tan_half(rad);
            let centre = Vec2::new(-hl + r, rad - lt);
            let a1 = (2.0 * hl).atan2(rad); // direction of the slant's outward normal
            let round = arc(centre, r, PI, a1, segs);
            let t2 = round[round.len() - 1];
            vec![
                Run::new(
                    RunKind::Cap,
                    vec![Vec2::new(-hl, 0.0), Vec2::new(-hl, rad - lt)],
                ),
                Run::new(RunKind::Round, round),
                Run::new(RunKind::Straight, vec![t2, Vec2::new(hl, 0.0)]),
            ]
        } else {
            vec![
                Run::new(RunKind::Cap, vec![Vec2::new(-hl, 0.0), Vec2::new(-hl, rad)]),
                Run::new(
                    RunKind::Straight,
                    vec![Vec2::new(-hl, rad), Vec2::new(hl, 0.0)],
                ),
            ]
        }
    });
}

/// Half ellipsoid, dome up. With a bevel it is the exact rounded solid: a smaller half ellipsoid grown by a
/// ball, so the base rim is a true fillet and the dome stays tangent to it.
fn hemisphere(b: &mut Builder, desc: &MeshDesc) {
    let h = desc.half();
    let la = Lathe::around(1, h, circle_segments(h.x.max(h.z), desc.lod));
    let (r, segs) = bevel(desc);
    let height = 2.0 * h.y;
    let r = r.min(0.9 * h.x.min(h.z)).min(0.45 * height);
    let m = arc_segments(h.x.max(h.z).max(height), FRAC_PI_2, desc.lod);
    let base = -h.y;
    lathe::emit(b, &la, false, |rad| {
        let (rc, hc, t0) = (rad - r, height - 2.0 * r, base + r);
        let dome: Vec<Vec2> = (0..=m)
            .map(|i| {
                let s = FRAC_PI_2 * i as f32 / m as f32;
                if i == m {
                    return Vec2::new(-base, 0.0);
                }
                let core = Vec2::new(t0 + hc * s.sin(), rc * s.cos());
                let n =
                    Vec2::new(s.sin() / hc.max(1e-9), s.cos() / rc.max(1e-9)).normalize_or(Vec2::Y);
                core + n * r
            })
            .collect();
        let mut runs = vec![Run::new(
            RunKind::Cap,
            vec![Vec2::new(base, 0.0), Vec2::new(base, rc)],
        )];
        if r > 0.0 {
            runs.push(Run::new(
                RunKind::Round,
                arc(Vec2::new(t0, rc), r, PI, FRAC_PI_2, segs),
            ));
        }
        runs.push(Run::new(RunKind::Curved, dome));
        runs
    });
}

/// Cylinder along X with ellipsoidal ends.
fn capsule(b: &mut Builder, desc: &MeshDesc) {
    let h = desc.half();
    let la = Lathe::around(0, h, circle_segments(h.y.max(h.z), desc.lod));
    let hl = h.x;
    let cap = h.y.min(h.z).min(hl);
    let m = arc_segments(cap.max(h.y.max(h.z)), FRAC_PI_2, desc.lod);
    let straight = 2.0 * (hl - cap) > 1e-4 * hl;
    lathe::emit(b, &la, false, |rad| {
        let left: Vec<Vec2> = (0..=m)
            .map(|i| {
                let s = FRAC_PI_2 * i as f32 / m as f32;
                if i == 0 {
                    return Vec2::new(-hl, 0.0);
                }
                Vec2::new(-hl + cap - cap * s.cos(), rad * s.sin())
            })
            .collect();
        let right: Vec<Vec2> = left.iter().rev().map(|p| Vec2::new(-p.x, p.y)).collect();
        let mut runs = vec![Run::new(RunKind::Curved, left)];
        if straight {
            runs.push(Run::new(
                RunKind::Straight,
                vec![Vec2::new(-hl + cap, rad), Vec2::new(hl - cap, rad)],
            ));
        }
        runs.push(Run::new(RunKind::Curved, right));
        runs
    });
}

/// Ring around X. The tube is `size.x` thick along the axis and, across, a fixed fraction of the ring radius
/// (`tube / min radius`, at most 0.45 so the hole never closes). On an elliptical ring that is an evenly
/// stretched circular torus, which can never fold into itself however extreme the size.
pub(crate) fn torus_tube(desc: &MeshDesc) -> f32 {
    let h = desc.half();
    h.x.min(0.45 * h.y.min(h.z))
}

fn torus(b: &mut Builder, desc: &MeshDesc) {
    let h = desc.half();
    let la = Lathe::around(0, h, circle_segments(h.y.max(h.z), desc.lod));
    let a = h.x;
    let tube = torus_tube(desc);
    let k = tube / h.y.min(h.z);
    let m = circle_segments(a.max(tube), desc.lod) as usize;
    lathe::emit(b, &la, true, |rad| {
        let mut pts: Vec<Vec2> = (0..m)
            .map(|i| {
                let s = TAU * i as f32 / m as f32;
                Vec2::new(a * s.cos(), rad * (1.0 - k - k * s.sin()))
            })
            .collect();
        pts.push(pts[0]);
        vec![Run::new(RunKind::Curved, pts)]
    });
}

/// Stairs step count after applying the automatic rule.
pub(crate) fn stair_steps(desc: &MeshDesc) -> u32 {
    if desc.params.steps == 0 {
        (desc.size[1].round() as u32).clamp(2, 64)
    } else {
        desc.params.steps.clamp(1, 64)
    }
}

/// Arch thickness after clamping.
pub(crate) fn arch_thickness(desc: &MeshDesc) -> f32 {
    let [sx, sy, _] = desc.size;
    desc.params
        .thickness
        .clamp(0.02 * sx.min(sy), (0.45 * sx).min(0.9 * sy))
}

/// Profile, placement and clamped bevel radius of the extruded shapes.
pub(crate) fn extrusion(desc: &MeshDesc) -> Option<(Extrusion, Vec<ProfilePoint>, f32)> {
    let h = desc.half();
    let r_req = desc.bevel.width.max(0.0);
    let sharp = ProfilePoint::sharp;
    let (ex, profile) = match desc.shape {
        Shape::Stairs => {
            let n = stair_steps(desc);
            let (dz, dy) = (desc.size[2] / n as f32, desc.size[1] / n as f32);
            let mut p = vec![sharp(-h.z, -h.y), sharp(h.z, -h.y), sharp(h.z, h.y)];
            for i in (0..n).rev() {
                let z = -h.z + i as f32 * dz;
                p.push(sharp(z, -h.y + (i + 1) as f32 * dy));
                if i > 0 {
                    p.push(sharp(z, -h.y + i as f32 * dy));
                }
            }
            (side_extrusion(h), p)
        }
        Shape::Ramp => {
            let lip = if r_req > 0.0 {
                (2.4 * r_req).min(0.25 * desc.size[1])
            } else {
                0.0
            };
            let curve = ramp_curve(desc, lip);
            let deck_corner = curve[0];
            let mut p = vec![
                sharp(-h.z, -h.y),
                sharp(h.z, -h.y),
                sharp(h.z, h.y),
                sharp(deck_corner.x, deck_corner.y),
            ];
            p.extend(
                curve[1..curve.len() - 1]
                    .iter()
                    .map(|q| ProfilePoint::smooth(*q)),
            );
            if lip > 0.0 {
                p.push(sharp(-h.z, -h.y + lip));
            }
            (side_extrusion(h), p)
        }
        Shape::Arch => {
            let opening = arch_opening(desc);
            let last = opening.len() - 1;
            let mut p = vec![sharp(-h.x, -h.y), sharp(opening[0].x, opening[0].y)];
            p.extend(opening[1..last].iter().map(|q| ProfilePoint::smooth(*q)));
            p.extend([
                sharp(opening[last].x, opening[last].y),
                sharp(h.x, -h.y),
                sharp(h.x, h.y),
                sharp(-h.x, h.y),
            ]);
            let ex = Extrusion {
                u: Vec3::X,
                v: Vec3::Y,
                w: Vec3::Z,
                half_w: h.z,
                half: h,
            };
            (ex, p)
        }
        _ => return None,
    };
    let r = extrude::clamp_radius(&profile, r_req, ex.half_w);
    Some((ex, profile, r))
}

/// The ramp's curved surface in the (z, y) plane, from the deck corner (top) down to the front, `lip` studs
/// above the floor. The deck is a flat strip at the top back (0.2 of the depth, at most 0.5 stud).
pub(crate) fn ramp_curve(desc: &MeshDesc, lip: f32) -> Vec<Vec2> {
    let h = desc.half();
    let deck = (0.2 * desc.size[2]).min(0.5);
    let (rz, ry) = (desc.size[2] - deck, desc.size[1] - lip);
    let m = arc_segments(rz.max(ry), FRAC_PI_2, desc.lod).max(2);
    (0..=m)
        .map(|s| {
            let t = FRAC_PI_2 * (1.0 - s as f32 / m as f32);
            Vec2::new(-h.z + rz * t.sin(), h.y - ry * t.cos())
        })
        .collect()
}

/// The arch opening in the (x, y) plane, from the left foot up and over to the right foot. A semicircle on
/// straight jambs when the opening is taller than half its width, else a flattened half ellipse on short
/// jambs.
pub(crate) fn arch_opening(desc: &MeshDesc) -> Vec<Vec2> {
    let h = desc.half();
    let t = arch_thickness(desc);
    let a = h.x - t;
    let open = desc.size[1] - t;
    let (jamb, rise) = if open >= a {
        (open - a, a)
    } else {
        (0.25 * open, 0.75 * open)
    };
    let m = arc_segments(a.max(rise), PI, desc.lod).max(4);
    let mut p = vec![Vec2::new(-a, -h.y)];
    if jamb > 0.0 {
        p.push(Vec2::new(-a, -h.y + jamb));
    }
    for s in 1..m {
        let phi = PI * (1.0 - s as f32 / m as f32);
        p.push(Vec2::new(a * phi.cos(), -h.y + jamb + rise * phi.sin()));
    }
    if jamb > 0.0 {
        p.push(Vec2::new(a, -h.y + jamb));
    }
    p.push(Vec2::new(a, -h.y));
    p
}

/// Profile in the (z, y) plane, extruded along X (`u = Z`, `v = Y`, `w = Z × Y = -X`).
fn side_extrusion(h: Vec3) -> Extrusion {
    Extrusion {
        u: Vec3::Z,
        v: Vec3::Y,
        w: Vec3::NEG_X,
        half_w: h.x,
        half: h,
    }
}
