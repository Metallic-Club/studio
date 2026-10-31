//! Small vector helpers shared by the generators.
use glam::{Vec2, Vec3};

/// Spherical interpolation between unit vectors. `t = 0` returns `a` and `t = 1` returns `b` exactly, so
/// patches that share an arc end on identical points.
pub(crate) fn slerp(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let theta = a.dot(b).clamp(-1.0, 1.0).acos();
    if theta < 1e-5 {
        return a.lerp(b, t).normalize_or(a);
    }
    let s = theta.sin();
    (a * (((1.0 - t) * theta).sin() / s) + b * ((t * theta).sin() / s)).normalize_or(a)
}

/// A planar UV projection in studs: `uv = (p·u + off.x, p·v + off.y)`.
///
/// `u × v` equals the face normal, so every chart is right-handed and tangents always have `w = +1`.
/// Vertical faces keep `v` pointing up (textures stand upright); horizontal faces use `u = +X`.
/// The offset puts `uv = 0` on the box corner, so an axis-aligned face spans `0..size` exactly.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    pub u: Vec3,
    pub v: Vec3,
    off: Vec2,
}

impl Frame {
    pub(crate) fn for_normal(n: Vec3, half: Vec3) -> Frame {
        let (u, v) = if n.y.abs() > 0.9999 {
            let u = (Vec3::X - n * n.x).normalize_or(Vec3::X);
            (u, n.cross(u).normalize_or(Vec3::Z))
        } else {
            let v = (Vec3::Y - n * n.y).normalize_or(Vec3::Y);
            (v.cross(n).normalize_or(Vec3::X), v)
        };
        Frame {
            u,
            v,
            off: Vec2::new(u.abs().dot(half), v.abs().dot(half)),
        }
    }

    pub(crate) fn uv(&self, p: Vec3) -> Vec2 {
        Vec2::new(p.dot(self.u) + self.off.x, p.dot(self.v) + self.off.y)
    }
}

/// The six axis directions used for box projection, in a fixed order.
pub(crate) const AXES6: [Vec3; 6] = [
    Vec3::X,
    Vec3::NEG_X,
    Vec3::Y,
    Vec3::NEG_Y,
    Vec3::Z,
    Vec3::NEG_Z,
];

/// Index into [`AXES6`] of the axis closest to `n`.
pub(crate) fn dominant_axis(n: Vec3) -> usize {
    let mut best = 0;
    let mut best_dot = f32::MIN;
    for (i, a) in AXES6.iter().enumerate() {
        let d = a.dot(n);
        if d > best_dot {
            best_dot = d;
            best = i;
        }
    }
    best
}

/// Outward normal of a 2D edge running along `e` on a counter-clockwise polygon.
pub(crate) fn edge_normal_2d(e: Vec2) -> Vec2 {
    Vec2::new(e.y, -e.x).normalize_or(Vec2::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slerp_endpoints_are_exact() {
        let a = Vec3::new(0.3, 0.4, 0.866).normalize();
        let b = Vec3::Y;
        assert_eq!(slerp(a, b, 0.0), a);
        assert_eq!(slerp(a, b, 1.0), b);
        assert!((slerp(a, b, 0.5).length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn frames_are_right_handed() {
        for n in AXES6
            .iter()
            .copied()
            .chain([Vec3::new(0.0, 1.0, -1.0).normalize()])
        {
            let f = Frame::for_normal(n, Vec3::ONE);
            assert!((f.u.cross(f.v) - n).length() < 1e-5, "{n}");
        }
    }
}
