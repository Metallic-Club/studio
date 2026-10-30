//! Roblox-compatible front end: the flat `ParticleEmitter` property set beginners know.
//!
//! [`ParticleEmitter`] mirrors Roblox's properties (snake_case here; the Luau API and Studio show the
//! Roblox names `Rate`, `Lifetime`, `Speed`, ...) with Roblox defaults, and [`ParticleEmitter::to_emitter`]
//! builds the equivalent module stack. `Emit(n)` and `Clear()` are [`crate::Simulation::emit`] and
//! [`crate::Simulation::clear`]. [`crate::Trail`] and [`crate::Beam`] already use Roblox semantics.
//!
//! Semantics followed (from the Roblox engine reference, see `docs/agents/41-vfx.md`):
//! - `Drag`: speed halves every `1 / Drag` seconds (exponential decay; negative grows).
//! - `Lifetime` 0 spawns nothing; lifetimes are capped at 20 s; `Rate` is capped at 400/s.
//! - `SpreadAngle (x, y)`: random rotation of `±x/2` and `±y/2` degrees about two axes perpendicular to
//!   the emission direction; `(360, 360)` emits every way.
//! - `LightEmission` 0 = normal blending, 1 = additive; `Transparency` 0 = opaque.
//! - `FlipbookMode::OneShot` spreads the frames over the particle's lifetime and ignores the frame rate;
//!   the frame rate is capped at 30 fps.
//! - Shape: `Box` emits along `EmissionDirection`; `Sphere`, `Cylinder` and `Disc` emit along the shape
//!   normal per `ShapeInOut`, sized from the parent part (`part_size`).

use crate::color::Rgb;
use crate::curve::{Curve, Gradient, Range};
use crate::effect::{
    Direction, Emitter, Facing, Flipbook, FlipbookMode, InOut, InitModule, MAX_CAPACITY, Render,
    RenderKind, Shape, SimSpace, SpawnModule, Texture, UpdateModule,
};
use crate::math::Vec3;

/// Roblox's per-emitter rate cap on desktop.
pub const ROBLOX_MAX_RATE: f32 = 400.0;

/// Roblox `Enum.NormalId`, used by `EmissionDirection`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NormalId {
    /// +Y.
    #[default]
    Top,
    /// -Y.
    Bottom,
    /// -X.
    Left,
    /// +X.
    Right,
    /// -Z (the part's look direction).
    Front,
    /// +Z.
    Back,
}

impl NormalId {
    /// Unit vector in the part's local space.
    #[must_use]
    pub fn vector(self) -> Vec3 {
        match self {
            NormalId::Top => Vec3::Y,
            NormalId::Bottom => -Vec3::Y,
            NormalId::Left => -Vec3::X,
            NormalId::Right => Vec3::X,
            NormalId::Front => -Vec3::Z,
            NormalId::Back => Vec3::Z,
        }
    }
}

/// Roblox `Enum.ParticleEmitterShape`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmitterShape {
    /// The parent part's box.
    #[default]
    Box,
    /// A sphere fitted in the part.
    Sphere,
    /// A vertical cylinder fitted in the part.
    Cylinder,
    /// A flat disc fitted in the part.
    Disc,
}

/// Roblox `Enum.ParticleOrientation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    /// Billboard.
    #[default]
    FacingCamera,
    /// Billboard that stays upright.
    FacingCameraWorldUp,
    /// Long axis along the velocity.
    VelocityParallel,
    /// Flat, facing the velocity.
    VelocityPerpendicular,
}

/// Roblox `Enum.ParticleFlipbookLayout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlipbookLayout {
    /// No flipbook.
    #[default]
    None,
    /// 2x2 frames.
    Grid2x2,
    /// 4x4 frames.
    Grid4x4,
    /// 8x8 frames.
    Grid8x8,
    /// `FlipbookSizeX` x `FlipbookSizeY` frames.
    Custom(u32, u32),
}

/// A Roblox `ParticleEmitter`'s properties, with Roblox defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct ParticleEmitter {
    /// `Enabled`.
    pub enabled: bool,
    /// `Rate`, particles per second.
    pub rate: f32,
    /// `Lifetime` in seconds.
    pub lifetime: Range,
    /// `Speed` in studs per second.
    pub speed: Range,
    /// `SpreadAngle` in degrees.
    pub spread_angle: (f32, f32),
    /// `EmissionDirection`.
    pub emission_direction: NormalId,
    /// `Shape`.
    pub shape: EmitterShape,
    /// `ShapeInOut`.
    pub shape_in_out: InOut,
    /// `ShapeStyle` is `Surface` (true) or `Volume` (false).
    pub shape_surface: bool,
    /// `ShapePartial`.
    pub shape_partial: f32,
    /// Size of the parent part; the shape is fitted to it.
    pub part_size: Vec3,
    /// `Size` over life.
    pub size: Curve,
    /// `Squash` over life.
    pub squash: Curve,
    /// `Transparency` over life.
    pub transparency: Curve,
    /// `Color` over life.
    pub color: Gradient,
    /// `LightEmission`.
    pub light_emission: f32,
    /// `LightInfluence` (0 is the `Instance.new` default).
    pub light_influence: f32,
    /// `Brightness`.
    pub brightness: f32,
    /// `ZOffset`.
    pub z_offset: f32,
    /// `Orientation`.
    pub orientation: Orientation,
    /// `Rotation` in degrees.
    pub rotation: Range,
    /// `RotSpeed` in degrees per second.
    pub rot_speed: Range,
    /// `Drag`.
    pub drag: f32,
    /// `Acceleration`.
    pub acceleration: Vec3,
    /// `VelocityInheritance`.
    pub velocity_inheritance: f32,
    /// `LockedToPart`.
    pub locked_to_part: bool,
    /// `TimeScale`.
    pub time_scale: f32,
    /// `Texture` (built-in or asset).
    pub texture: Texture,
    /// `FlipbookLayout`.
    pub flipbook_layout: FlipbookLayout,
    /// `FlipbookMode`.
    pub flipbook_mode: FlipbookMode,
    /// `FlipbookFramerate`.
    pub flipbook_framerate: Range,
    /// `FlipbookStartRandom`.
    pub flipbook_start_random: bool,
    /// `FlipbookBlendFrames`.
    pub flipbook_blend_frames: bool,
}

impl Default for ParticleEmitter {
    fn default() -> Self {
        Self {
            enabled: true,
            rate: 20.0,
            lifetime: Range::new(5.0, 10.0),
            speed: Range::constant(5.0),
            spread_angle: (0.0, 0.0),
            emission_direction: NormalId::Top,
            shape: EmitterShape::Box,
            shape_in_out: InOut::Outward,
            shape_surface: false,
            shape_partial: 1.0,
            part_size: Vec3::new(4.0, 1.0, 2.0),
            size: Curve::constant(1.0),
            squash: Curve::constant(0.0),
            transparency: Curve::constant(0.0),
            color: Gradient::constant(Rgb::WHITE),
            light_emission: 0.0,
            light_influence: 0.0,
            brightness: 1.0,
            z_offset: 0.0,
            orientation: Orientation::FacingCamera,
            rotation: Range::constant(0.0),
            rot_speed: Range::constant(0.0),
            drag: 0.0,
            acceleration: Vec3::ZERO,
            velocity_inheritance: 0.0,
            locked_to_part: false,
            time_scale: 1.0,
            texture: Texture::Twinkle,
            flipbook_layout: FlipbookLayout::None,
            flipbook_mode: FlipbookMode::Loop,
            flipbook_framerate: Range::constant(1.0),
            flipbook_start_random: false,
            flipbook_blend_frames: true,
        }
    }
}

impl ParticleEmitter {
    /// Builds the equivalent emitter module stack.
    #[must_use]
    pub fn to_emitter(&self, name: &str) -> Emitter {
        let s = self.part_size;
        let min_xz = s.x.min(s.z).abs();
        let shape = match self.shape {
            EmitterShape::Box => Shape::Box { size: s },
            EmitterShape::Sphere => Shape::Sphere {
                radius: s.x.min(s.y).min(s.z).abs() * 0.5,
                partial: self.shape_partial,
            },
            EmitterShape::Cylinder => Shape::Cylinder {
                radius: min_xz * 0.5,
                height: s.y.abs(),
                top: self.shape_partial,
            },
            EmitterShape::Disc => Shape::Disc {
                radius: min_xz * 0.5,
                inner: self.shape_partial,
            },
        };
        let direction = match self.shape {
            EmitterShape::Box => Direction::Axis(self.emission_direction.vector()),
            _ => Direction::FromShape(self.shape_in_out),
        };
        let rate = self.rate.clamp(0.0, ROBLOX_MAX_RATE);
        let max_life = self.lifetime.min.max(self.lifetime.max).clamp(0.0, 20.0);
        let capacity = ((rate * max_life * 1.25).ceil() as u32 + 64)
            .max(512)
            .min(MAX_CAPACITY);
        let flipbook = match self.flipbook_layout {
            FlipbookLayout::None => None,
            layout => {
                let (columns, rows) = match layout {
                    FlipbookLayout::Grid2x2 => (2, 2),
                    FlipbookLayout::Grid4x4 => (4, 4),
                    FlipbookLayout::Grid8x8 => (8, 8),
                    FlipbookLayout::Custom(x, y) => (x.clamp(1, 64), y.clamp(1, 64)),
                    FlipbookLayout::None => (1, 1),
                };
                Some(Flipbook {
                    columns,
                    rows,
                    mode: self.flipbook_mode,
                    fps: Range::new(
                        self.flipbook_framerate.min.clamp(0.0, 30.0),
                        self.flipbook_framerate.max.clamp(0.0, 30.0),
                    ),
                    start_random: self.flipbook_start_random,
                    blend: self.flipbook_blend_frames,
                })
            }
        };
        let facing = match self.orientation {
            Orientation::FacingCamera => Facing::Camera,
            Orientation::FacingCameraWorldUp => Facing::CameraWorldUp,
            Orientation::VelocityParallel => Facing::VelocityParallel,
            Orientation::VelocityPerpendicular => Facing::VelocityPerpendicular,
        };
        Emitter {
            name: name.to_string(),
            enabled: self.enabled,
            capacity,
            space: if self.locked_to_part {
                SimSpace::Local
            } else {
                SimSpace::World
            },
            time_scale: self.time_scale.clamp(0.0, 1.0),
            delay: 0.0,
            fixed_fps: 0.0,
            spawn: vec![SpawnModule::Rate {
                per_second: rate,
                disabled: false,
            }],
            init: vec![
                InitModule::Shape {
                    shape,
                    surface: self.shape_surface,
                    rotation: Vec3::ZERO,
                    offset: Vec3::ZERO,
                    disabled: false,
                },
                InitModule::Lifetime {
                    seconds: self.lifetime,
                    disabled: false,
                },
                InitModule::Velocity {
                    speed: self.speed,
                    direction,
                    spread: self.spread_angle,
                    disabled: false,
                },
                InitModule::Rotation {
                    angle: self.rotation,
                    spin: self.rot_speed,
                    disabled: false,
                },
                InitModule::InheritVelocity {
                    amount: self.velocity_inheritance.clamp(0.0, 1.0),
                    disabled: false,
                },
            ],
            update: vec![
                UpdateModule::Acceleration {
                    accel: self.acceleration,
                    disabled: false,
                },
                UpdateModule::Drag {
                    drag: self.drag,
                    disabled: false,
                },
                UpdateModule::SizeOverLife {
                    curve: self.size.clone(),
                    disabled: false,
                },
                UpdateModule::SquashOverLife {
                    curve: self.squash.clone(),
                    disabled: false,
                },
                UpdateModule::TransparencyOverLife {
                    curve: self.transparency.clone(),
                    disabled: false,
                },
                UpdateModule::ColorOverLife {
                    gradient: self.color.clone(),
                    disabled: false,
                },
            ],
            render: Render {
                kind: RenderKind::Billboard,
                facing,
                texture: self.texture.clone(),
                flipbook,
                light_emission: self.light_emission.clamp(0.0, 1.0),
                brightness: self.brightness.max(0.0),
                light_influence: self.light_influence.clamp(0.0, 1.0),
                z_offset: self.z_offset,
                ..Render::default()
            },
        }
    }
}
