//! Math types, built on `glam` (f32).
//!
//! Conventions shared by every crate: right-handed, Y up, forward is -Z (like Roblox), 1 unit = 1 metre, angles
//! in the scripting API and in properties are degrees.

mod cframe;
mod color;

pub use cframe::CFrame;
pub use color::Color3;

/// A 2D vector (f32).
pub type Vector2 = glam::Vec2;

/// A 3D vector (f32): positions in metres, directions, sizes.
pub type Vector3 = glam::Vec3;
