//! The Metallic Engine renderer: draws the Parts of a `metallic_core::World` with wgpu.
//!
//! [`Renderer::sync`] reads BasePart transforms, sizes, colours, materials and transparency
//! from a world; [`Renderer::render`] draws them into any texture view. Shading is physically
//! based with procedural per-material detail, one sun with cascaded shadow maps, a gradient
//! sky for ambient light, distance fog, optional 4x MSAA and a sorted blended pass for
//! transparent parts. Skinned characters ([`metallic_asset::SkinnedModel`]) are drawn with GPU
//! linear-blend skinning in the same passes, with level of detail by screen size; see
//! [`Renderer::set_skinned`] and [`skinned`]. The device is created by [`RenderContext`] or
//! borrowed from a host such as the Studio editor.
#![warn(missing_docs)]

pub mod camera;
pub mod materials;
pub mod meshes;
pub mod scene;
pub mod shadows;
pub mod skinned;
pub mod sky;

mod context;
mod renderer;

pub use camera::Camera;
pub use context::{RenderContext, RenderError};
pub use glam;
pub use metallic_asset;
pub use renderer::{Quality, RenderSettings, Renderer};
pub use skinned::{SkinStyle, SkinnedInstance};
