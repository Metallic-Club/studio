//! Metallic VFX: particle effects, trails and beams for the Metallic Engine.
//!
//! - **Data model** ([`effect`]): an [`Effect`] holds [`Emitter`]s, [`Trail`]s and [`Beam`]s. An emitter
//!   is a Niagara-style module stack: spawn → init → update → render. Every module can be switched off.
//! - **Beginner front end** ([`roblox`]): [`roblox::ParticleEmitter`] has the flat Roblox properties
//!   (`Rate`, `Lifetime`, `Speed`, `SpreadAngle`, `Drag`, `LightEmission`, ...) and builds an emitter
//!   with the same behaviour. [`Trail`] and [`Beam`] follow Roblox semantics directly.
//! - **Simulator** ([`sim`]): a deterministic, seeded CPU reference. Same effect, seed and step sequence
//!   give bit-identical results everywhere. Particles are stored structure-of-arrays in `vec4` streams
//!   ready for a compute-shader port ([`gpu`] documents the WGSL side).
//! - **Drawing** ([`draw`]): a renderer-neutral, depth-sorted list of textured quads with premultiplied
//!   colour (one blend state draws alpha and additive particles).
//! - **Styles** ([`style`]): [`Effect::restyle`] converts any effect to Realistic, Toon, Pixel, Low-poly
//!   or 2D, so one preset library serves every art style.
//! - **Presets** ([`presets`]): 24 ready effects; **format** ([`format`]): diff-friendly RON files.
//!
//! The crate has no GPU or UI dependency; `metallic_vfx_editor` is the egui editor on top of it.
#![warn(missing_docs)]

pub mod color;
pub mod curve;
pub mod draw;
pub mod effect;
pub mod format;
pub mod gpu;
pub mod math;
pub mod noise;
pub mod presets;
pub mod rng;
pub mod roblox;
pub mod sim;
pub mod style;
pub mod textures;

pub use color::Rgb;
pub use curve::{BlendSpace, ColorKey, Curve, Gradient, Interp, Key, Range};
pub use draw::{Camera, DrawList, Quad, build_draw_list};
pub use effect::{
    ArtStyle, Beam, Direction, Effect, Emitter, Facing, Flipbook, FlipbookMode, HitAction, InOut,
    InitModule, Render, RenderKind, Shape, SimSpace, SortMode, SpawnModule, Texture, TextureMode,
    Trail, UpdateModule,
};
pub use math::{Transform, Vec3};
pub use sim::{Aabb, ParticlePool, Simulation};

/// Errors returned by the VFX crate.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum VfxError {
    /// The effect file is not valid RON for an effect.
    #[error("cannot parse effect: {0}")]
    Parse(String),
    /// Writing the effect failed.
    #[error("cannot write effect: {0}")]
    Format(String),
    /// The file was written by a newer format version.
    #[error("effect format version {0} is newer than this build supports")]
    UnsupportedVersion(u32),
    /// A value is out of range.
    #[error("invalid value at {path}: {reason}")]
    Invalid {
        /// Where in the effect.
        path: String,
        /// What is wrong.
        reason: String,
    },
    /// An emitter index is out of range.
    #[error("no emitter with index {0}")]
    NoSuchEmitter(usize),
    /// No preset has this name.
    #[error("no preset named `{0}`")]
    NoSuchPreset(String),
}
