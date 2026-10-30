//! Conversion between the Instance tree (BasePart properties) and Rapier bodies/colliders.
use metallic_core::{InstanceId, Value, Vec3, World};
use rapier3d::prelude::*;

/// Physical properties of a material.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialProps {
    /// Density (Roblox-like units; mass = density x volume).
    pub density: f32,
    /// Coulomb friction coefficient.
    pub friction: f32,
    /// Bounciness, 0 (none) to 1.
    pub restitution: f32,
}

/// Density, friction and restitution for a `Material` property value.
///
/// | Material      | density | friction | restitution |
/// |---------------|---------|----------|-------------|
/// | Plastic       | 0.70    | 0.40     | 0.00        |
/// | SmoothPlastic | 0.70    | 0.20     | 0.00        |
/// | Metal         | 7.85    | 0.40     | 0.05        |
/// | Wood          | 0.35    | 0.48     | 0.00        |
/// | Concrete      | 2.40    | 0.70     | 0.00        |
/// | Glass         | 2.50    | 0.25     | 0.15        |
/// | Grass         | 0.90    | 0.40     | 0.00        |
/// | Brick         | 1.90    | 0.80     | 0.00        |
/// | Ice           | 0.92    | 0.02     | 0.00        |
/// | Sand          | 1.60    | 0.50     | 0.00        |
///
/// Unknown names fall back to Plastic.
pub fn material_props(name: &str) -> MaterialProps {
    let (density, friction, restitution) = match name {
        "SmoothPlastic" => (0.7, 0.2, 0.0),
        "Metal" => (7.85, 0.4, 0.05),
        "Wood" => (0.35, 0.48, 0.0),
        "Concrete" => (2.4, 0.7, 0.0),
        "Glass" => (2.5, 0.25, 0.15),
        "Grass" => (0.9, 0.4, 0.0),
        "Brick" => (1.9, 0.8, 0.0),
        "Ice" => (0.92, 0.02, 0.0),
        "Sand" => (1.6, 0.5, 0.0),
        _ => (0.7, 0.4, 0.0),
    };
    MaterialProps {
        density,
        friction,
        restitution,
    }
}

/// Everything the physics needs to know about one BasePart.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PartState {
    pub position: Vec3,
    pub orientation: Vec3,
    pub size: Vec3,
    pub anchored: bool,
    pub can_collide: bool,
    pub material: String,
    pub shape: String,
}

/// The part of [`PartState`] that forces the body/collider to be rebuilt when it changes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Signature {
    pub size: Vec3,
    pub anchored: bool,
    pub can_collide: bool,
    pub material: String,
    pub shape: String,
}

impl PartState {
    pub(crate) fn signature(&self) -> Signature {
        Signature {
            size: self.size,
            anchored: self.anchored,
            can_collide: self.can_collide,
            material: self.material.clone(),
            shape: self.shape.clone(),
        }
    }
}

/// Reads the physics-relevant properties of a BasePart; `None` if it is not one.
pub(crate) fn read_part(world: &World, id: InstanceId) -> Option<PartState> {
    if !world.is_a(id, "BasePart") {
        return None;
    }
    let v3 = |p: &str| match world.get(id, p) {
        Ok(Value::Vector3(v)) => Some(v),
        _ => None,
    };
    let b = |p: &str| match world.get(id, p) {
        Ok(Value::Bool(v)) => Some(v),
        _ => None,
    };
    let s = |p: &str| match world.get(id, p) {
        Ok(Value::String(v)) => Some(v),
        _ => None,
    };
    Some(PartState {
        position: v3("Position")?,
        orientation: v3("Orientation")?,
        size: v3("Size")?,
        anchored: b("Anchored")?,
        can_collide: b("CanCollide")?,
        material: s("Material")?,
        shape: s("Shape")?,
    })
}

/// All BasePart instances in the world, depth first.
pub(crate) fn collect_parts(world: &World) -> Vec<InstanceId> {
    let mut out = Vec::new();
    let mut stack: Vec<InstanceId> = world.roots().iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        if world.is_a(id, "BasePart") {
            out.push(id);
        }
        stack.extend(world.children(id).iter().rev().copied());
    }
    out
}

/// Rotation matrix (row-major) for Roblox Euler angles in degrees: R = Ry * Rx * Rz.
fn matrix_from_orientation(o: Vec3) -> [[f64; 3]; 3] {
    let (sx, cx) = o.x.to_radians().sin_cos();
    let (sy, cy) = o.y.to_radians().sin_cos();
    let (sz, cz) = o.z.to_radians().sin_cos();
    [
        [cy * cz + sy * sx * sz, -cy * sz + sy * sx * cz, sy * cx],
        [cx * sz, cx * cz, -sx],
        [-sy * cz + cy * sx * sz, sy * sz + cy * sx * cz, cy * cx],
    ]
}

/// Quaternion `[x, y, z, w]` for Roblox Euler angles in degrees.
pub(crate) fn quat_from_orientation(o: Vec3) -> [f64; 4] {
    let m = matrix_from_orientation(o);
    let tr = m[0][0] + m[1][1] + m[2][2];
    let (x, y, z, w);
    if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
        w = 0.25 * s;
        x = (m[2][1] - m[1][2]) / s;
        y = (m[0][2] - m[2][0]) / s;
        z = (m[1][0] - m[0][1]) / s;
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        w = (m[2][1] - m[1][2]) / s;
        x = 0.25 * s;
        y = (m[0][1] + m[1][0]) / s;
        z = (m[0][2] + m[2][0]) / s;
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        w = (m[0][2] - m[2][0]) / s;
        x = (m[0][1] + m[1][0]) / s;
        y = 0.25 * s;
        z = (m[1][2] + m[2][1]) / s;
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        w = (m[1][0] - m[0][1]) / s;
        x = (m[0][2] + m[2][0]) / s;
        y = (m[1][2] + m[2][1]) / s;
        z = 0.25 * s;
    }
    [x, y, z, w]
}

/// Roblox Euler angles in degrees for a quaternion `[x, y, z, w]`.
pub(crate) fn orientation_from_quat(q: [f64; 4]) -> Vec3 {
    let [x, y, z, w] = q;
    let n = (x * x + y * y + z * z + w * w).sqrt();
    let (x, y, z, w) = (x / n, y / n, z / n, w / n);
    let m = [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ];
    let rx = (-m[1][2]).clamp(-1.0, 1.0).asin();
    let (ry, rz) = if m[1][2].abs() < 0.999_999 {
        (m[0][2].atan2(m[2][2]), m[1][0].atan2(m[1][1]))
    } else {
        ((-m[2][0]).atan2(m[0][0]), 0.0)
    };
    Vec3::new(rx.to_degrees(), ry.to_degrees(), rz.to_degrees())
}

/// Builds the collider for a shape name and part size (studs).
///
/// Block = cuboid. Ball = sphere of the smallest axis. Cylinder = axis along X (length = Size.X,
/// diameter = smaller of Y/Z). Wedge = right triangular prism whose tall edge is at +Z.
pub(crate) fn build_collider(shape: &str, size: Vec3) -> ColliderBuilder {
    let half = |v: f64| ((v / 2.0) as f32).max(1e-3);
    let (hx, hy, hz) = (half(size.x), half(size.y), half(size.z));
    let block = || ColliderBuilder::cuboid(hx, hy, hz);
    match shape {
        "Ball" => ColliderBuilder::ball(hx.min(hy).min(hz)),
        "Cylinder" => ColliderBuilder::cylinder(hx, hy.min(hz)).rotation(Vector::new(
            0.0,
            0.0,
            std::f32::consts::FRAC_PI_2,
        )),
        "Wedge" => {
            let pts = [
                Vector::new(-hx, -hy, -hz),
                Vector::new(hx, -hy, -hz),
                Vector::new(-hx, -hy, hz),
                Vector::new(hx, -hy, hz),
                Vector::new(-hx, hy, hz),
                Vector::new(hx, hy, hz),
            ];
            ColliderBuilder::convex_hull(&pts).unwrap_or_else(block)
        }
        _ => block(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_round_trips_through_quaternion() {
        for o in [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(30.0, 45.0, -20.0),
            Vec3::new(-60.0, 170.0, 80.0),
        ] {
            let back = orientation_from_quat(quat_from_orientation(o));
            assert!((back - o).magnitude() < 1e-6, "{o:?} -> {back:?}");
        }
    }
}
