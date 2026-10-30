//! Surfaces of revolution (cylinder, cone, pipe, capsule, hemisphere, torus) with elliptical cross-sections.
//!
//! A shape gives its meridian profile as runs of `(t, ρ)` points: `t` along the axis, `ρ` out from it in the
//! direction `m(θ)`. The cross-section may be an ellipse, so the profile is asked for again at every angle
//! with that angle's ellipse radius; every call must return the same number of points per run. Runs meet at
//! hard edges; inside a run the surface is smooth. Normals come from the surface's own derivatives, so they
//! stay exact on elliptical shapes.
use crate::builder::{Builder, Corner};
use crate::math::{AXES6, Frame, dominant_axis};
use glam::{Vec2, Vec3};
use std::f32::consts::TAU;

/// How a run is UV-mapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunKind {
    /// Flat and perpendicular to the axis: planar projection.
    Cap,
    /// Straight in the meridian (cylinder or cone side): unrolled exactly, 1 UV per stud.
    Straight,
    /// Curved both ways (dome, torus): box projection.
    Curved,
    /// A bevel between two runs: each half continues its neighbour's mapping.
    Round,
}

/// A smooth piece of the meridian profile.
#[derive(Clone, Debug)]
pub(crate) struct Run {
    pub pts: Vec<Vec2>,
    pub kind: RunKind,
}

impl Run {
    pub(crate) fn new(kind: RunKind, pts: Vec<Vec2>) -> Run {
        Run { pts, kind }
    }
}

/// Where the lathe sits: `axis = b × c`; the cross-section ellipse has semi-axes `radii` along `b` and `c`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lathe {
    pub axis: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    pub radii: Vec2,
    pub segments: u32,
    pub half: Vec3,
}

impl Lathe {
    /// A lathe around one of the coordinate axes (0 = X, 1 = Y, 2 = Z), filling the box `half`.
    pub(crate) fn around(axis: usize, half: Vec3, segments: u32) -> Lathe {
        let (a, b, c) = match axis {
            0 => (Vec3::X, Vec3::Y, Vec3::Z),
            1 => (Vec3::Y, Vec3::Z, Vec3::X),
            _ => (Vec3::Z, Vec3::X, Vec3::Y),
        };
        Lathe {
            axis: a,
            b,
            c,
            radii: Vec2::new(b.dot(half), c.dot(half)),
            segments,
            half,
        }
    }

    /// Half the extent along the axis.
    pub(crate) fn half_len(&self) -> f32 {
        self.axis.dot(self.half)
    }

    /// The cross-section ellipse point at parametric angle `theta` (`b·a·cos θ + c·b·sin θ`). Parametric
    /// angles put more samples where an elongated ellipse bends most. Exact at multiples of 90°.
    fn ellipse(&self, theta: f32, quarter: Option<usize>) -> Vec2 {
        let (s, c) = match quarter {
            Some(0) => (0.0, 1.0),
            Some(1) => (1.0, 0.0),
            Some(2) => (0.0, -1.0),
            Some(3) => (-1.0, 0.0),
            _ => theta.sin_cos(),
        };
        Vec2::new(self.radii.x * c, self.radii.y * s)
    }
}

/// Precomputed mapping data for one run.
struct RunUv {
    chart: u32,
    /// Straight runs: arc length around the reference ring, per angle index (0..=N).
    ring: Vec<f32>,
    /// Straight runs: index of the reference point (the end further from the axis).
    reference: usize,
    /// Straight runs: true when the radius changes along the run (a cone), else cylindrical.
    tapered: bool,
    /// +1 when the surface faces away from the axis, -1 when it faces the axis (inside of a pipe).
    facing: f32,
    /// Cap runs: planar frame.
    frame: Option<Frame>,
}

/// Emits the lathe. `profile(r)` returns the runs for an ellipse radius `r`; `closed` joins the last run back
/// to the first (pipe, torus).
pub(crate) fn emit(b: &mut Builder, la: &Lathe, closed: bool, profile: impl Fn(f32) -> Vec<Run>) {
    let n = la.segments.max(3) as usize;
    let ellipse: Vec<Vec2> = (0..n)
        .map(|i| {
            let quarter = (i * 4 % n == 0).then_some(i * 4 / n);
            la.ellipse(TAU * i as f32 / n as f32, quarter)
        })
        .collect();
    let dirs: Vec<Vec3> = ellipse
        .iter()
        .map(|e| (la.b * e.x + la.c * e.y).normalize_or(la.b))
        .collect();
    let profiles: Vec<Vec<Run>> = ellipse.iter().map(|e| profile(e.length())).collect();
    let runs = profiles[0].len();
    let pos = |i: usize, q: usize, k: usize| -> Vec3 {
        let i = i % n;
        let p = profiles[i][q].pts[k];
        la.axis * p.x + dirs[i] * p.y
    };
    let meridian_normal = |i: usize, q: usize, k: usize| -> (Vec3, Vec3) {
        let pts = &profiles[i % n][q].pts;
        let last = pts.len() - 1;
        let d = if k == 0 {
            pts[1] - pts[0]
        } else if k == last {
            pts[last] - pts[last - 1]
        } else {
            pts[k + 1] - pts[k - 1]
        };
        let n2 = Vec2::new(-d.y, d.x).normalize_or(Vec2::Y);
        let dm = la.axis * d.x + dirs[i % n] * d.y;
        (la.axis * n2.x + dirs[i % n] * n2.y, dm)
    };
    let normal = |i: usize, q: usize, k: usize| -> Vec3 {
        let (n0, dm) = meridian_normal(i, q, k);
        if profiles[0][q].kind == RunKind::Cap {
            return n0;
        }
        let dt = pos(i + 1, q, k) - pos(i + n - 1, q, k);
        let c = dt.cross(dm);
        if c.length_squared() <= 1e-12 * dt.length_squared().max(1e-12) * dm.length_squared() {
            return n0;
        }
        let c = c.normalize_or(n0);
        if c.dot(n0) < 0.0 { -c } else { c }
    };

    let half_len = la.half_len();
    let uvs: Vec<RunUv> = (0..runs)
        .map(|q| {
            let run = &profiles[0][q];
            let last = run.pts.len() - 1;
            let mid = last / 2;
            let (n0, _) = meridian_normal(0, q, mid);
            let facing = if n0.dot(dirs[0]) < -1e-4 { -1.0 } else { 1.0 };
            let reference = if run.pts[last].y > run.pts[0].y {
                last
            } else {
                0
            };
            let mut ring = vec![0.0f32; n + 1];
            if run.kind == RunKind::Straight {
                for i in 0..n {
                    ring[i + 1] = ring[i] + pos(i, q, reference).distance(pos(i + 1, q, reference));
                }
            }
            let tapered = (run.pts[last].y - run.pts[0].y).abs()
                > 1e-4 * run.pts[last].distance(run.pts[0]).max(1e-6);
            RunUv {
                chart: match run.kind {
                    RunKind::Cap | RunKind::Straight => b.chart(),
                    _ => u32::MAX,
                },
                ring,
                reference,
                tapered,
                facing,
                frame: (run.kind == RunKind::Cap).then(|| Frame::for_normal(n0, la.half)),
            }
        })
        .collect();
    let box_charts: Vec<u32> = (0..6).map(|_| b.chart()).collect();
    let box_frames: Vec<Frame> = AXES6
        .iter()
        .map(|a| Frame::for_normal(*a, la.half))
        .collect();

    // UV of a point at angle index `i` with meridian coordinates `m` under run `q`'s mapping.
    let straight_uv = |q: usize, i: usize, m: Vec2| -> Vec2 {
        let u = &uvs[q];
        let s = u.ring[i] * u.facing;
        if !u.tapered {
            return Vec2::new(s, m.x + half_len);
        }
        let pts = &profiles[i % n][q].pts;
        let (a, z) = (pts[0], pts[pts.len() - 1]);
        let t_apex = a.x - a.y * (z.x - a.x) / (z.y - a.y);
        let sin_a = (z.y - a.y).abs() / a.distance(z).max(1e-9);
        let r_ref = pts[u.reference].y.max(1e-9);
        let psi = s / r_ref * sin_a;
        let dist = Vec2::new(m.x - t_apex, m.y).length();
        if t_apex > 0.5 * (a.x + z.x) {
            Vec2::new(dist * psi.sin(), -dist * psi.cos())
        } else {
            Vec2::new(dist * psi.sin(), dist * psi.cos())
        }
    };

    for q in 0..runs {
        let kind = profiles[0][q].kind;
        let len = profiles[0][q].pts.len();
        for k in 0..len - 1 {
            // Round runs borrow the mapping of the neighbour on their side.
            let owner = if kind == RunKind::Round {
                let prev = if q == 0 && closed {
                    runs - 1
                } else {
                    q.saturating_sub(1)
                };
                let next = if q + 1 == runs && closed {
                    0
                } else {
                    (q + 1).min(runs - 1)
                };
                if (k as f32 + 0.5) < (len - 1) as f32 * 0.5 {
                    prev
                } else {
                    next
                }
            } else {
                q
            };
            let owner_kind = profiles[0][owner].kind;
            for i in 0..n {
                let cell = [(i, k), (i + 1, k), (i + 1, k + 1), (i, k + 1)];
                let corners = cell.map(|(ii, kk)| {
                    let p = pos(ii, q, kk);
                    let nn = normal(ii, q, kk);
                    (p, nn, profiles[ii % n][q].pts[kk], ii)
                });
                for tri in [[0, 1, 2], [0, 2, 3]] {
                    let c = tri.map(|x| corners[x]);
                    let (chart, uv): (u32, [Vec2; 3]) = match owner_kind {
                        RunKind::Cap => {
                            let f = uvs[owner]
                                .frame
                                .unwrap_or_else(|| Frame::for_normal(la.axis, la.half));
                            (uvs[owner].chart, c.map(|x| f.uv(x.0)))
                        }
                        RunKind::Straight => {
                            (uvs[owner].chart, c.map(|x| straight_uv(owner, x.3, x.2)))
                        }
                        RunKind::Curved | RunKind::Round => {
                            let ax = dominant_axis(c[0].1 + c[1].1 + c[2].1);
                            (box_charts[ax], c.map(|x| box_frames[ax].uv(x.0)))
                        }
                    };
                    b.tri(
                        chart,
                        Corner::new(c[0].0, c[0].1, uv[0]),
                        Corner::new(c[1].0, c[1].1, uv[1]),
                        Corner::new(c[2].0, c[2].1, uv[2]),
                    );
                }
            }
        }
    }
}

/// Points of a circular arc in the meridian plane: centre `c`, radius `r`, from angle `a0` to `a1`.
pub(crate) fn arc(c: Vec2, r: f32, a0: f32, a1: f32, segs: u32) -> Vec<Vec2> {
    let segs = segs.max(1);
    (0..=segs)
        .map(|s| {
            let a = a0 + (a1 - a0) * s as f32 / segs as f32;
            c + Vec2::new(a.cos(), a.sin()) * r
        })
        .collect()
}
