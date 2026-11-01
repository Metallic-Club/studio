//! Property value types.
use crate::cframe::CFrame;
use crate::instance::InstanceId;
use serde::{Deserialize, Serialize};
use std::ops::{Add, Mul, Sub};

/// A 3D vector (studs).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec3 {
    /// X component.
    pub x: f64,
    /// Y component.
    pub y: f64,
    /// Z component.
    pub z: f64,
}

impl Vec3 {
    /// Creates a vector from its components.
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
    /// Length of the vector.
    pub fn magnitude(self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }
    /// The vector scaled to length 1; the zero vector stays zero.
    pub fn unit(self) -> Self {
        let m = self.magnitude();
        if m == 0.0 {
            self
        } else {
            Vec3::new(self.x / m, self.y / m, self.z / m)
        }
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl Mul<f64> for Vec3 {
    type Output = Vec3;
    fn mul(self, k: f64) -> Vec3 {
        Vec3::new(self.x * k, self.y * k, self.z * k)
    }
}

/// An RGB colour with each channel in 0..=255 (as in Roblox's `Color3.fromRGB`).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Color3 {
    /// Red, 0 to 255.
    pub r: f64,
    /// Green, 0 to 255.
    pub g: f64,
    /// Blue, 0 to 255.
    pub b: f64,
}

impl Color3 {
    /// Creates a colour from 0..=255 channels.
    pub const fn new(r: f64, g: f64, b: f64) -> Self {
        Self { r, g, b }
    }
}

/// A 2D size or position made of a fraction of the parent plus a pixel offset per axis.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct UDim2 {
    /// Horizontal fraction of the parent (0..=1).
    pub x_scale: f64,
    /// Horizontal pixel offset.
    pub x_offset: f64,
    /// Vertical fraction of the parent (0..=1).
    pub y_scale: f64,
    /// Vertical pixel offset.
    pub y_offset: f64,
}

impl UDim2 {
    /// Creates a UDim2 from scale and offset per axis.
    pub const fn new(x_scale: f64, x_offset: f64, y_scale: f64, y_offset: f64) -> Self {
        Self {
            x_scale,
            x_offset,
            y_scale,
            y_offset,
        }
    }
}

/// A dynamically typed property value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    /// No value.
    Nil,
    /// A boolean.
    Bool(bool),
    /// A number.
    Number(f64),
    /// A text string.
    String(String),
    /// A 3D vector.
    Vector3(Vec3),
    /// A colour.
    Color3(Color3),
    /// A GUI size or position.
    UDim2(UDim2),
    /// A reference to another instance (or none).
    Ref(Option<InstanceId>),
    /// A position and rotation. Boxed so the enum stays as small as before (CFrame is 96 bytes).
    CFrame(Box<CFrame>),
}

/// The type of a property, used to type-check [`Value`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    /// [`Value::Nil`]
    Nil,
    /// [`Value::Bool`]
    Bool,
    /// [`Value::Number`]
    Number,
    /// [`Value::String`]
    String,
    /// [`Value::Vector3`]
    Vector3,
    /// [`Value::Color3`]
    Color3,
    /// [`Value::UDim2`]
    UDim2,
    /// [`Value::Ref`]
    Ref,
    /// [`Value::CFrame`]
    CFrame,
}

impl ValueType {
    /// Human readable type name used in error messages.
    pub fn name(self) -> &'static str {
        match self {
            ValueType::Nil => "Nil",
            ValueType::Bool => "Bool",
            ValueType::Number => "Number",
            ValueType::String => "String",
            ValueType::Vector3 => "Vector3",
            ValueType::Color3 => "Color3",
            ValueType::UDim2 => "UDim2",
            ValueType::Ref => "Ref",
            ValueType::CFrame => "CFrame",
        }
    }
}

impl Value {
    /// Whether every numeric component is finite; nonnumeric variants have no such restriction.
    pub(crate) fn has_finite_components(&self) -> bool {
        match self {
            Value::Number(number) => number.is_finite(),
            Value::Vector3(vector) => {
                vector.x.is_finite() && vector.y.is_finite() && vector.z.is_finite()
            }
            Value::Color3(color) => {
                color.r.is_finite() && color.g.is_finite() && color.b.is_finite()
            }
            Value::UDim2(value) => {
                value.x_scale.is_finite()
                    && value.x_offset.is_finite()
                    && value.y_scale.is_finite()
                    && value.y_offset.is_finite()
            }
            Value::CFrame(cf) => cf.is_finite(),
            Value::Nil | Value::Bool(_) | Value::String(_) | Value::Ref(_) => true,
        }
    }

    /// The type of this value.
    pub fn value_type(&self) -> ValueType {
        match self {
            Value::Nil => ValueType::Nil,
            Value::Bool(_) => ValueType::Bool,
            Value::Number(_) => ValueType::Number,
            Value::String(_) => ValueType::String,
            Value::Vector3(_) => ValueType::Vector3,
            Value::Color3(_) => ValueType::Color3,
            Value::UDim2(_) => ValueType::UDim2,
            Value::Ref(_) => ValueType::Ref,
            Value::CFrame(_) => ValueType::CFrame,
        }
    }
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

/// Unit quaternion `[x, y, z, w]` for a part's `Orientation` (Roblox Euler angles in degrees,
/// applied Y then X then Z). Physics and the wire encoding share it; [`quat_to_orientation`]
/// is the inverse.
pub fn orientation_to_quat(o: Vec3) -> [f64; 4] {
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

/// Roblox Euler angles in degrees for a quaternion `[x, y, z, w]` (normalized first); the
/// inverse of [`orientation_to_quat`] as a rotation. Near the X = ±90° singularity Z is 0.
pub fn quat_to_orientation(q: [f64; 4]) -> Vec3 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_round_trip() {
        // 100 seeded orientations (xorshift64): the round trip is the same rotation within 1e-9.
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut angle = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64 * 360.0 - 180.0
        };
        for _ in 0..100 {
            let o = Vec3::new(angle() / 2.0, angle(), angle());
            let back = quat_to_orientation(orientation_to_quat(o));
            let (a, b) = (matrix_from_orientation(o), matrix_from_orientation(back));
            for r in 0..3 {
                for c in 0..3 {
                    assert!((a[r][c] - b[r][c]).abs() < 1e-9, "{o:?} -> {back:?}");
                }
            }
            let [x, y, z, w] = orientation_to_quat(o);
            assert!(((x * x + y * y + z * z + w * w).sqrt() - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn cframe_does_not_grow_value() {
        // Property reads and writes copy Values; the boxed CFrame keeps them at 40 bytes.
        assert_eq!(std::mem::size_of::<Value>(), 40);
    }
}
