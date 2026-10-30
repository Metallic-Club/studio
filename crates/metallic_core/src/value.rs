//! Property value types.
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
        }
    }
}

impl Value {
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
        }
    }
}
