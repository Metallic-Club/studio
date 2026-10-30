//! The effect data model: an [`Effect`] holds [`Emitter`]s (particles), [`Trail`]s and [`Beam`]s.
//!
//! An emitter is a stack of modules in four stages, run top to bottom like a Niagara emitter stack:
//! [`SpawnModule`] (how many and when), [`InitModule`] (each new particle's starting state),
//! [`UpdateModule`] (forces and over-life changes, every step) and one [`Render`] block (how particles
//! draw). Beginners never see the stack: [`crate::roblox::ParticleEmitter`] builds the same thing from
//! flat Roblox properties.
//!
//! Every module has a `disabled` flag (written only when true) so a creator can switch a module off
//! without losing its settings, and the file diff for that is one line.

use crate::color::Rgb;
use crate::curve::{Curve, Gradient, Range};
use crate::math::Vec3;
use serde::{Deserialize, Serialize};

/// Version written to the `format` field of every effect file.
pub const FORMAT_VERSION: u32 = 1;

/// Largest particle capacity one emitter may ask for (the High-tier simulation budget).
pub const MAX_CAPACITY: u32 = 262_144;

/// Longest particle lifetime in seconds (the Roblox limit). Longer values are clamped.
pub const MAX_LIFETIME: f32 = 20.0;

pub(crate) fn is_false(v: &bool) -> bool {
    !*v
}
pub(crate) fn is_true(v: &bool) -> bool {
    *v
}
pub(crate) fn yes() -> bool {
    true
}
pub(crate) fn one() -> f32 {
    1.0
}
pub(crate) fn is_one(v: &f32) -> bool {
    *v == 1.0
}
pub(crate) fn is_zero(v: &f32) -> bool {
    *v == 0.0
}
fn is_world(s: &SimSpace) -> bool {
    *s == SimSpace::World
}
fn is_camera(f: &Facing) -> bool {
    *f == Facing::Camera
}
fn is_by_distance(s: &SortMode) -> bool {
    *s == SortMode::ByDistance
}
fn is_zero_vec(v: &Vec3) -> bool {
    *v == Vec3::ZERO
}
fn one_u32() -> u32 {
    1
}
fn is_one_u32(v: &u32) -> bool {
    *v == 1
}
fn version() -> u32 {
    FORMAT_VERSION
}

/// The art style an effect was authored for. [`Effect::restyle`] converts between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ArtStyle {
    /// Soft textures, smooth gradients, frame-rate motion.
    #[default]
    Realistic,
    /// Hard-edged shapes, banded colours, erosion instead of fades.
    Toon,
    /// Pixel textures, snapped to a pixel grid, stepped motion at 12 fps.
    Pixel,
    /// Flat polygon shards and hexagons, few colour bands.
    LowPoly,
    /// Everything on the XY plane for 2D games (side view).
    Flat2D,
}

impl ArtStyle {
    /// Every style, in menu order.
    pub const ALL: [ArtStyle; 5] = [
        ArtStyle::Realistic,
        ArtStyle::Toon,
        ArtStyle::Pixel,
        ArtStyle::LowPoly,
        ArtStyle::Flat2D,
    ];

    /// Display name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ArtStyle::Realistic => "Realistic",
            ArtStyle::Toon => "Toon",
            ArtStyle::Pixel => "Pixel",
            ArtStyle::LowPoly => "Low-poly",
            ArtStyle::Flat2D => "2D",
        }
    }
}

/// A complete visual effect: what a creator saves, shares and drops into a place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    /// File format version ([`FORMAT_VERSION`]).
    #[serde(default = "version")]
    pub format: u32,
    /// Display name.
    pub name: String,
    /// One-line description for the library browser.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Search tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Style the effect was authored for.
    #[serde(default)]
    pub style: ArtStyle,
    /// Length of one cycle in seconds. Bursts are timed inside a cycle. 0 means "no fixed length".
    #[serde(default, skip_serializing_if = "is_zero")]
    pub duration: f32,
    /// Restart the cycle forever (true) or play once and finish (false).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub looping: bool,
    /// Particle emitters, drawn in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emitters: Vec<Emitter>,
    /// Trails that follow the effect's origin.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trails: Vec<Trail>,
    /// Beams between two points.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub beams: Vec<Beam>,
    /// How the editor moves the effect while previewing (trails and distance spawning need motion).
    #[serde(default, skip_serializing_if = "PreviewMotion::is_still")]
    pub preview: PreviewMotion,
}

/// Editor preview movement of the effect origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PreviewMotion {
    /// Stays at the origin.
    #[default]
    Still,
    /// Circles the origin, radius 3 studs, one lap per 2.5 s.
    Orbit,
    /// Swings ±80 degrees about the vertical axis, like a sword arm.
    Swing,
    /// Runs back and forth along X, 8 studs.
    Line,
}

impl PreviewMotion {
    /// Every motion, in menu order.
    pub const ALL: [PreviewMotion; 4] = [
        PreviewMotion::Still,
        PreviewMotion::Orbit,
        PreviewMotion::Swing,
        PreviewMotion::Line,
    ];

    fn is_still(&self) -> bool {
        *self == PreviewMotion::Still
    }

    /// Display name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            PreviewMotion::Still => "Still",
            PreviewMotion::Orbit => "Orbit",
            PreviewMotion::Swing => "Swing",
            PreviewMotion::Line => "Line",
        }
    }

    /// Effect transform at preview time `t` seconds.
    #[must_use]
    pub fn transform_at(self, t: f32) -> crate::math::Transform {
        use crate::math::Transform;
        use std::f32::consts::{PI, TAU};
        match self {
            PreviewMotion::Still => Transform::IDENTITY,
            PreviewMotion::Orbit => {
                let a = t / 2.5 * TAU;
                Transform::from_position_yaw(
                    Vec3::new(3.0 * a.cos(), 1.5 + 0.4 * (2.0 * a).sin(), 3.0 * a.sin()),
                    -a,
                )
            }
            PreviewMotion::Swing => {
                // Fast strike, slow return: a sword arc.
                let phase = (t / 1.4).fract();
                let s = if phase < 0.3 {
                    let u = phase / 0.3;
                    1.0 - 2.0 * (u * u * (3.0 - 2.0 * u))
                } else {
                    let u = (phase - 0.3) / 0.7;
                    -1.0 + 2.0 * (u * u * (3.0 - 2.0 * u))
                };
                Transform::from_position_yaw(
                    Vec3::new(0.0, 0.0, 0.0),
                    s * 80f32.to_radians() + PI * 0.5,
                )
            }
            PreviewMotion::Line => {
                let x = 4.0 * (t * 0.8).sin();
                Transform::from_position(Vec3::new(x, 0.0, 0.0))
            }
        }
    }
}

impl Default for Effect {
    fn default() -> Self {
        Self {
            format: FORMAT_VERSION,
            name: "New effect".into(),
            description: String::new(),
            tags: Vec::new(),
            style: ArtStyle::Realistic,
            duration: 0.0,
            looping: true,
            emitters: vec![Emitter::default()],
            trails: Vec::new(),
            beams: Vec::new(),
            preview: PreviewMotion::Still,
        }
    }
}

/// Where particles live once spawned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SimSpace {
    /// Particles stay where they were born when the effect moves (smoke behind a rocket).
    #[default]
    World,
    /// Particles move with the effect (Roblox `LockedToPart`).
    Local,
}

/// One particle emitter: a module stack plus a renderer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Emitter {
    /// Display name, unique inside the effect.
    pub name: String,
    /// Off emitters neither spawn nor draw.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// Most particles alive at once. Spawns beyond it are dropped.
    pub capacity: u32,
    /// World or local simulation.
    #[serde(default, skip_serializing_if = "is_world")]
    pub space: SimSpace,
    /// Speed multiplier for this emitter's clock (Roblox `TimeScale`), `0..=1` typical.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub time_scale: f32,
    /// Seconds after the effect starts before this emitter runs.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub delay: f32,
    /// Step the simulation at this fixed rate (for example 12 for pixel art "on twos"); 0 = every frame.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fixed_fps: f32,
    /// Spawn stage.
    #[serde(default)]
    pub spawn: Vec<SpawnModule>,
    /// Initialise stage, run once per new particle.
    #[serde(default)]
    pub init: Vec<InitModule>,
    /// Update stage, run every step for every particle.
    #[serde(default)]
    pub update: Vec<UpdateModule>,
    /// How particles draw.
    #[serde(default)]
    pub render: Render,
}

impl Default for Emitter {
    fn default() -> Self {
        Self {
            name: "Emitter".into(),
            enabled: true,
            capacity: 256,
            space: SimSpace::World,
            time_scale: 1.0,
            delay: 0.0,
            fixed_fps: 0.0,
            spawn: vec![SpawnModule::Rate {
                per_second: 20.0,
                disabled: false,
            }],
            init: vec![
                InitModule::Lifetime {
                    seconds: Range::new(1.0, 2.0),
                    disabled: false,
                },
                InitModule::Velocity {
                    speed: Range::new(2.0, 4.0),
                    direction: Direction::Axis(Vec3::Y),
                    spread: (30.0, 30.0),
                    disabled: false,
                },
            ],
            update: vec![UpdateModule::TransparencyOverLife {
                curve: Curve::linear(&[(0.0, 0.0), (1.0, 1.0)]),
                disabled: false,
            }],
            render: Render::default(),
        }
    }
}

/// Spawn stage modules: when particles are born.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SpawnModule {
    /// Continuous spawning (Roblox `Rate`).
    Rate {
        /// Particles per second.
        per_second: f32,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// A burst of particles at a time inside the cycle, optionally repeated.
    Burst {
        /// Seconds into the cycle.
        time: f32,
        /// Particles per burst.
        count: u32,
        /// How many bursts (1 = once).
        #[serde(default = "one_u32", skip_serializing_if = "is_one_u32")]
        cycles: u32,
        /// Seconds between repeated bursts.
        #[serde(default, skip_serializing_if = "is_zero")]
        interval: f32,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// Spawning by distance travelled (footsteps, tyre dust).
    Distance {
        /// Particles per stud the effect moves.
        per_stud: f32,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
}

/// Where along the shape's normal particles move when the velocity direction comes from the shape
/// (Roblox `ShapeInOut`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum InOut {
    /// Away from the shape.
    #[default]
    Outward,
    /// Towards the shape's centre.
    Inward,
    /// Randomly either.
    Both,
}

/// An emission shape, in the effect's local space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    /// A single point at the origin.
    Point,
    /// A sphere. `partial` 1 = full sphere, 0.5 = upper dome (Roblox `ShapePartial`).
    Sphere {
        /// Radius in studs.
        radius: f32,
        /// Portion of the sphere from the top, `0..=1`.
        #[serde(default = "one", skip_serializing_if = "is_one")]
        partial: f32,
    },
    /// An axis-aligned box centred on the origin.
    Box {
        /// Full size in studs.
        size: Vec3,
    },
    /// A disc in the XZ plane. `inner` is the hole radius as a fraction (1 = a thin ring).
    Disc {
        /// Radius in studs.
        radius: f32,
        /// Inner radius fraction, `0..=1`.
        #[serde(default, skip_serializing_if = "is_zero")]
        inner: f32,
    },
    /// A vertical cylinder; `top` scales the top radius (0 = cone point up).
    Cylinder {
        /// Base radius in studs.
        radius: f32,
        /// Height in studs.
        height: f32,
        /// Top radius fraction.
        #[serde(default = "one", skip_serializing_if = "is_one")]
        top: f32,
    },
    /// A disc whose normals tilt outward by `angle` degrees at the rim: emits a cone of directions.
    Cone {
        /// Base radius in studs.
        radius: f32,
        /// Opening half-angle in degrees.
        angle: f32,
    },
    /// A straight edge between two points.
    Edge {
        /// Start point.
        a: Vec3,
        /// End point.
        b: Vec3,
    },
    /// Surface of a mesh asset. Stub: until mesh assets exist this emits from a unit sphere surface.
    MeshSurface {
        /// Asset path of the mesh.
        mesh: String,
    },
}

/// Which way a new particle moves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Direction {
    /// Along a local axis (Roblox `EmissionDirection`).
    Axis(Vec3),
    /// Along the shape's normal at the spawn point.
    FromShape(InOut),
}

/// Initialise stage modules: each particle's starting state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InitModule {
    /// Spawn position from a shape.
    Shape {
        /// The shape.
        shape: Shape,
        /// Spawn on the surface only (Roblox `ShapeStyle = Surface`).
        #[serde(default, skip_serializing_if = "is_false")]
        surface: bool,
        /// Shape rotation in degrees about X, then Y, then Z.
        #[serde(default, skip_serializing_if = "is_zero_vec")]
        rotation: Vec3,
        /// Shape centre, local space.
        #[serde(default, skip_serializing_if = "is_zero_vec")]
        offset: Vec3,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// Lifetime in seconds (Roblox `Lifetime`). A lifetime of 0 means nothing spawns.
    Lifetime {
        /// Random range.
        seconds: Range,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// Starting velocity (Roblox `Speed`, `EmissionDirection`, `SpreadAngle`).
    Velocity {
        /// Speed in studs per second; negative values reverse the direction.
        speed: Range,
        /// Base direction.
        direction: Direction,
        /// Random spread in degrees about two perpendicular axes (`±value/2`); `(360, 360)` = any way.
        spread: (f32, f32),
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// Starting size in studs (multiplies `SizeOverLife`).
    Size {
        /// Random range.
        size: Range,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// Starting colour: one of `choices`, or (with `mix`) anywhere along them as a gradient.
    Color {
        /// Candidate colours.
        choices: Vec<Rgb>,
        /// Blend between neighbouring choices instead of picking one.
        #[serde(default, skip_serializing_if = "is_false")]
        mix: bool,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// Starting rotation and spin in degrees (Roblox `Rotation`, `RotSpeed`).
    Rotation {
        /// Starting angle range, degrees.
        angle: Range,
        /// Spin range, degrees per second.
        spin: Range,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// Adds a fraction of the effect's velocity (Roblox `VelocityInheritance`).
    InheritVelocity {
        /// Fraction, `0..=1`.
        amount: f32,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
}

/// What a particle does when it hits a collision plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HitAction {
    /// Bounce off.
    #[default]
    Bounce,
    /// Die on contact.
    Kill,
    /// Stop and stay on the plane.
    Stick,
}

/// Update stage modules: forces and over-life changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UpdateModule {
    /// Constant acceleration in studs/s² (Roblox `Acceleration`; gravity, buoyancy, wind).
    Acceleration {
        /// Acceleration vector.
        accel: Vec3,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// Exponential slow-down (Roblox `Drag`): speed halves every `1 / drag` seconds.
    Drag {
        /// Drag rate; negative values speed particles up.
        drag: f32,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// Divergence-free swirling (curl noise): smoke, magic, leaves.
    CurlNoise {
        /// Force in studs/s².
        strength: f32,
        /// Size of one swirl in studs.
        scale: f32,
        /// How fast the field drifts, studs per second.
        #[serde(default)]
        scroll: Vec3,
        /// Detail octaves, 1 to 4.
        #[serde(default = "one_u32", skip_serializing_if = "is_one_u32")]
        octaves: u32,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// Spin around an axis through `center` (local space), with optional pull towards the axis.
    Vortex {
        /// Axis direction.
        axis: Vec3,
        /// A point on the axis, local space.
        #[serde(default)]
        center: Vec3,
        /// Tangential acceleration, studs/s².
        swirl: f32,
        /// Acceleration towards the axis (negative pushes out).
        #[serde(default, skip_serializing_if = "is_zero")]
        pull: f32,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// Pull towards a point (local space). Strength falls off past `radius`.
    Attractor {
        /// Target point.
        position: Vec3,
        /// Acceleration at the target, studs/s².
        strength: f32,
        /// Falloff radius in studs.
        radius: f32,
        /// Particles closer than this die (0 = never).
        #[serde(default, skip_serializing_if = "is_zero")]
        kill_radius: f32,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// Caps the speed.
    SpeedLimit {
        /// Maximum speed in studs per second.
        max: f32,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// An infinite plane particles cannot pass (ground, water surface).
    CollisionPlane {
        /// A point on the plane, local space.
        point: Vec3,
        /// Plane normal (the side particles stay on).
        normal: Vec3,
        /// Kept fraction of the normal speed on a bounce.
        bounce: f32,
        /// Lost fraction of the sliding speed on a bounce.
        friction: f32,
        /// What happens on contact.
        #[serde(default)]
        on_hit: HitAction,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// Size multiplier over life (Roblox `Size`).
    SizeOverLife {
        /// Curve over normalised age.
        curve: Curve,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// Colour multiplier over life (Roblox `Color`).
    ColorOverLife {
        /// Gradient over normalised age.
        gradient: Gradient,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// Transparency over life, 0 = opaque, 1 = invisible (Roblox `Transparency`).
    TransparencyOverLife {
        /// Curve over normalised age.
        curve: Curve,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// Stretch over life: above 0 thinner and taller, below 0 wider and shorter (Roblox `Squash`).
    SquashOverLife {
        /// Curve over normalised age.
        curve: Curve,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
    /// Alpha erosion over life: the toon replacement for fading. 0 = whole shape, 1 = eaten away.
    ErosionOverLife {
        /// Curve over normalised age.
        curve: Curve,
        /// Module switched off.
        #[serde(default, skip_serializing_if = "is_false")]
        disabled: bool,
    },
}

/// Built-in particle textures. Generated procedurally by [`crate::textures`], so effects work with no
/// asset downloads; `Asset` names an image the renderer loads (the preview shows a soft dot).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Texture {
    /// Soft radial falloff.
    #[default]
    SoftDot,
    /// Hard-edged disc.
    Dot,
    /// Bright core with a long soft tail along +Y (use with velocity orientation or stretching).
    Spark,
    /// Four-point twinkle.
    Twinkle,
    /// Thin ring.
    Ring,
    /// Soft noisy puff.
    Smoke,
    /// Hard-edged cartoon cloud.
    ToonPuff,
    /// 4x4 flipbook of a billowing puff.
    SmokeSheet,
    /// 4x4 flipbook of a flame tongue.
    FlameSheet,
    /// Thin vertical streak (rain, speed lines).
    Streak,
    /// Leaf silhouette.
    Leaf,
    /// Paper rectangle.
    Confetti,
    /// Water drop.
    Droplet,
    /// Crescent swoosh for toon smears and slashes.
    Smear,
    /// Flat triangle shard (low-poly).
    Shard,
    /// Flat hexagon (low-poly puffs).
    Hexagon,
    /// 8x8 pixel-art dot.
    PixelDot,
    /// 8x8 pixel-art plus-shaped spark.
    PixelSpark,
    /// 16x16 pixel-art puff.
    PixelPuff,
    /// Bubble: ring with a highlight.
    Bubble,
    /// Soft horizontal band for beams and trails (bright centre, soft edges across V).
    Band,
    /// Electric band with a noisy core (lightning beams).
    Electric,
    /// An image asset path.
    Asset(String),
}

impl Texture {
    /// All built-in textures, in picker order.
    pub const BUILTIN: [Texture; 22] = [
        Texture::SoftDot,
        Texture::Dot,
        Texture::Spark,
        Texture::Twinkle,
        Texture::Ring,
        Texture::Smoke,
        Texture::ToonPuff,
        Texture::SmokeSheet,
        Texture::FlameSheet,
        Texture::Streak,
        Texture::Leaf,
        Texture::Confetti,
        Texture::Droplet,
        Texture::Smear,
        Texture::Shard,
        Texture::Hexagon,
        Texture::PixelDot,
        Texture::PixelSpark,
        Texture::PixelPuff,
        Texture::Bubble,
        Texture::Band,
        Texture::Electric,
    ];

    /// Display name.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Texture::SoftDot => "Soft dot",
            Texture::Dot => "Dot",
            Texture::Spark => "Spark",
            Texture::Twinkle => "Twinkle",
            Texture::Ring => "Ring",
            Texture::Smoke => "Smoke",
            Texture::ToonPuff => "Toon puff",
            Texture::SmokeSheet => "Smoke sheet 4x4",
            Texture::FlameSheet => "Flame sheet 4x4",
            Texture::Streak => "Streak",
            Texture::Leaf => "Leaf",
            Texture::Confetti => "Confetti",
            Texture::Droplet => "Droplet",
            Texture::Smear => "Smear",
            Texture::Shard => "Shard",
            Texture::Hexagon => "Hexagon",
            Texture::PixelDot => "Pixel dot",
            Texture::PixelSpark => "Pixel spark",
            Texture::PixelPuff => "Pixel puff",
            Texture::Bubble => "Bubble",
            Texture::Band => "Band",
            Texture::Electric => "Electric",
            Texture::Asset(path) => path,
        }
    }

    /// Pixel-art textures are sampled with nearest filtering.
    #[must_use]
    pub fn is_pixel_art(&self) -> bool {
        matches!(
            self,
            Texture::PixelDot | Texture::PixelSpark | Texture::PixelPuff
        )
    }

    /// Flipbook grid baked into the texture, if any: `(columns, rows)`.
    #[must_use]
    pub fn sheet(&self) -> Option<(u32, u32)> {
        match self {
            Texture::SmokeSheet | Texture::FlameSheet => Some((4, 4)),
            _ => None,
        }
    }
}

/// Particle facing (Roblox `Orientation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Facing {
    /// Always faces the camera (Roblox `FacingCamera`).
    #[default]
    Camera,
    /// Faces the camera but stays upright (Roblox `FacingCameraWorldUp`).
    CameraWorldUp,
    /// Long axis along the velocity, facing the camera (Roblox `VelocityParallel`).
    VelocityParallel,
    /// Flat, facing along the velocity (Roblox `VelocityPerpendicular`).
    VelocityPerpendicular,
    /// Flat on the ground plane (shockwaves, ripples, decals).
    Ground,
}

/// How a renderer turns particles into geometry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RenderKind {
    /// One quad per particle.
    Billboard,
    /// A quad stretched along the velocity: sparks, rain, speed lines.
    Stretched {
        /// Extra length per stud/s of speed.
        speed_scale: f32,
        /// Smallest length multiple of the size.
        #[serde(default = "one", skip_serializing_if = "is_one")]
        min_length: f32,
    },
    /// One strip through all live particles in spawn order (ribbons, ribbons per burst).
    Ribbon {
        /// Texture tiling along the strip.
        #[serde(default)]
        texture_mode: TextureMode,
        /// Studs per texture repeat (Wrap/Static) or repeats (Stretch).
        #[serde(default = "one", skip_serializing_if = "is_one")]
        texture_length: f32,
    },
    /// A mesh per particle. Stub: the preview draws a billboard until mesh assets exist.
    Mesh {
        /// Asset path of the mesh.
        mesh: String,
    },
}

/// Flipbook playback (Roblox `FlipbookMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FlipbookMode {
    /// Play frames repeatedly.
    #[default]
    Loop,
    /// Play once across the particle's life; `fps` is ignored.
    OneShot,
    /// First to last, then back.
    PingPong,
    /// Random frames, crossfaded.
    Random,
}

/// Flipbook settings (Roblox `FlipbookLayout`, `FlipbookMode`, `FlipbookFramerate`,
/// `FlipbookStartRandom`, `FlipbookBlendFrames`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Flipbook {
    /// Columns in the texture grid.
    pub columns: u32,
    /// Rows in the texture grid.
    pub rows: u32,
    /// Playback mode.
    #[serde(default)]
    pub mode: FlipbookMode,
    /// Frames per second, random per particle (max 30 like Roblox for Loop/PingPong/Random).
    #[serde(default = "default_fps")]
    pub fps: Range,
    /// Start on a random frame.
    #[serde(default, skip_serializing_if = "is_false")]
    pub start_random: bool,
    /// Crossfade between frames.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub blend: bool,
}

fn default_fps() -> Range {
    Range::constant(12.0)
}

impl Flipbook {
    /// Frame count.
    #[must_use]
    pub fn frames(&self) -> u32 {
        self.columns.max(1) * self.rows.max(1)
    }
}

/// Texture tiling for ribbons, trails and beams (Roblox `TextureMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TextureMode {
    /// Repeat `texture_length` times over the whole length.
    #[default]
    Stretch,
    /// One repeat per `texture_length` studs, fixed to the head.
    Wrap,
    /// One repeat per `texture_length` studs, fixed in the world (footprints).
    Static,
}

/// Depth sorting before drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SortMode {
    /// Back to front by camera distance (needed for alpha blending).
    #[default]
    ByDistance,
    /// Oldest particles on top.
    OldestOnTop,
    /// Newest particles on top.
    NewestOnTop,
}

/// Look and draw settings of an emitter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Render {
    /// Geometry kind.
    pub kind: RenderKind,
    /// Quad orientation.
    #[serde(default, skip_serializing_if = "is_camera")]
    pub facing: Facing,
    /// Texture.
    #[serde(default)]
    pub texture: Texture,
    /// Flipbook settings for sheet textures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flipbook: Option<Flipbook>,
    /// 0 = normal alpha blending, 1 = additive (Roblox `LightEmission`).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub light_emission: f32,
    /// Colour multiplier; above 1 feeds bloom in HDR renderers (Roblox `Brightness`).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub brightness: f32,
    /// How much scene lighting shades particles, `0..=1` (Roblox `LightInfluence`). Preview ignores it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub light_influence: f32,
    /// Studs towards the camera (Roblox `ZOffset`).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub z_offset: f32,
    /// Depth fade distance against geometry in studs (soft particles); 0 = off. GPU renderer only.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub soft_fade: f32,
    /// Draw order.
    #[serde(default, skip_serializing_if = "is_by_distance")]
    pub sort: SortMode,
    /// Opacity levels for toon and pixel looks: 0 = smooth, 1 = cut-out, n = n bands.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub alpha_steps: u32,
    /// Snap quad corners to a grid of this many screen pixels (pixel art); 0 = off.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pixel_snap: f32,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

impl Default for Render {
    fn default() -> Self {
        Self {
            kind: RenderKind::Billboard,
            facing: Facing::Camera,
            texture: Texture::SoftDot,
            flipbook: None,
            light_emission: 0.0,
            brightness: 1.0,
            light_influence: 0.0,
            z_offset: 0.0,
            soft_fade: 0.0,
            sort: SortMode::ByDistance,
            alpha_steps: 0,
            pixel_snap: 0.0,
        }
    }
}

/// A trail drawn behind the effect's origin as it moves (Roblox `Trail`). Attachments are offsets in
/// the effect's local space; the trail's width is the distance between them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trail {
    /// Display name.
    pub name: String,
    /// Off trails stop adding segments; existing ones expire.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// First edge point, local space (Roblox `Attachment0`).
    pub attachment0: Vec3,
    /// Second edge point, local space (Roblox `Attachment1`).
    pub attachment1: Vec3,
    /// Seconds each segment lasts, `0.01..=20`.
    pub lifetime: f32,
    /// Studs the origin must move before a new segment starts.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub min_length: f32,
    /// Longest trail in studs; 0 = unlimited.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub max_length: f32,
    /// Width multiplier from head (0) to tail (1).
    #[serde(default)]
    pub width_scale: Curve,
    /// Colour from head (0) to tail (1).
    #[serde(default)]
    pub color: Gradient,
    /// Transparency from new (0) to expiring (1).
    #[serde(default)]
    pub transparency: Curve,
    /// Turn the strip to face the camera.
    #[serde(default, skip_serializing_if = "is_false")]
    pub face_camera: bool,
    /// Texture.
    #[serde(default = "band")]
    pub texture: Texture,
    /// Texture tiling.
    #[serde(default)]
    pub texture_mode: TextureMode,
    /// Texture length (see [`TextureMode`]).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub texture_length: f32,
    /// 0 = alpha blend, 1 = additive.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub light_emission: f32,
    /// Colour multiplier.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub brightness: f32,
}

fn band() -> Texture {
    Texture::Band
}

impl Default for Trail {
    fn default() -> Self {
        Self {
            name: "Trail".into(),
            enabled: true,
            attachment0: Vec3::new(0.0, 0.5, 0.0),
            attachment1: Vec3::new(0.0, -0.5, 0.0),
            lifetime: 0.6,
            min_length: 0.1,
            max_length: 0.0,
            width_scale: Curve::linear(&[(0.0, 1.0), (1.0, 0.0)]),
            color: Gradient::constant(Rgb::WHITE),
            transparency: Curve::linear(&[(0.0, 0.0), (1.0, 1.0)]),
            face_camera: true,
            texture: Texture::Band,
            texture_mode: TextureMode::Stretch,
            texture_length: 1.0,
            light_emission: 0.0,
            brightness: 1.0,
        }
    }
}

/// A textured strip along a cubic Bezier between two points (Roblox `Beam`).
///
/// Control points follow Roblox: `P1 = attachment0 + direction0 * curve_size0`,
/// `P2 = attachment1 - direction1 * curve_size1`, where the directions stand in for the attachments'
/// `RightVector`s.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Beam {
    /// Display name.
    pub name: String,
    /// Off beams do not draw.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// Start point, local space.
    pub attachment0: Vec3,
    /// End point, local space.
    pub attachment1: Vec3,
    /// Start tangent direction (Attachment0's right vector).
    #[serde(default = "x_axis")]
    pub direction0: Vec3,
    /// End tangent direction (Attachment1's right vector).
    #[serde(default = "x_axis")]
    pub direction1: Vec3,
    /// Distance from `attachment0` to the first control point.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub curve_size0: f32,
    /// Distance from `attachment1` to the second control point.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub curve_size1: f32,
    /// Straight pieces approximating the curve (Roblox default 10).
    pub segments: u32,
    /// Width at the start in studs.
    pub width0: f32,
    /// Width at the end in studs.
    pub width1: f32,
    /// Turn the strip to face the camera.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub face_camera: bool,
    /// Colour along the beam.
    #[serde(default)]
    pub color: Gradient,
    /// Transparency along the beam.
    #[serde(default)]
    pub transparency: Curve,
    /// Texture.
    #[serde(default = "band")]
    pub texture: Texture,
    /// Texture tiling.
    #[serde(default)]
    pub texture_mode: TextureMode,
    /// Texture length (see [`TextureMode`]).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub texture_length: f32,
    /// Texture scroll in cycles per second.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub texture_speed: f32,
    /// 0 = alpha blend, 1 = additive.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub light_emission: f32,
    /// Colour multiplier.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub brightness: f32,
    /// Studs towards the camera.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub z_offset: f32,
    /// Random sideways offset of inner points in studs (lightning). A Metallic addition.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub jitter: f32,
    /// How often the jitter pattern changes per second.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub jitter_fps: f32,
}

fn x_axis() -> Vec3 {
    Vec3::X
}

impl Default for Beam {
    fn default() -> Self {
        Self {
            name: "Beam".into(),
            enabled: true,
            attachment0: Vec3::new(-3.0, 1.0, 0.0),
            attachment1: Vec3::new(3.0, 1.0, 0.0),
            direction0: Vec3::X,
            direction1: Vec3::X,
            curve_size0: 0.0,
            curve_size1: 0.0,
            segments: 10,
            width0: 1.0,
            width1: 1.0,
            face_camera: true,
            color: Gradient::constant(Rgb::WHITE),
            transparency: Curve::constant(0.0),
            texture: Texture::Band,
            texture_mode: TextureMode::Stretch,
            texture_length: 1.0,
            texture_speed: 1.0,
            light_emission: 0.0,
            brightness: 1.0,
            z_offset: 0.0,
            jitter: 0.0,
            jitter_fps: 0.0,
        }
    }
}

impl SpawnModule {
    /// Display name for the module stack.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            SpawnModule::Rate { .. } => "Spawn rate",
            SpawnModule::Burst { .. } => "Burst",
            SpawnModule::Distance { .. } => "Spawn by distance",
        }
    }

    /// True when switched off.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        match self {
            SpawnModule::Rate { disabled, .. }
            | SpawnModule::Burst { disabled, .. }
            | SpawnModule::Distance { disabled, .. } => *disabled,
        }
    }

    /// Mutable access to the off switch.
    pub fn disabled_mut(&mut self) -> &mut bool {
        match self {
            SpawnModule::Rate { disabled, .. }
            | SpawnModule::Burst { disabled, .. }
            | SpawnModule::Distance { disabled, .. } => disabled,
        }
    }
}

impl InitModule {
    /// Display name for the module stack.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            InitModule::Shape { .. } => "Shape",
            InitModule::Lifetime { .. } => "Lifetime",
            InitModule::Velocity { .. } => "Velocity",
            InitModule::Size { .. } => "Size",
            InitModule::Color { .. } => "Color",
            InitModule::Rotation { .. } => "Rotation",
            InitModule::InheritVelocity { .. } => "Inherit velocity",
        }
    }

    /// True when switched off.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        match self {
            InitModule::Shape { disabled, .. }
            | InitModule::Lifetime { disabled, .. }
            | InitModule::Velocity { disabled, .. }
            | InitModule::Size { disabled, .. }
            | InitModule::Color { disabled, .. }
            | InitModule::Rotation { disabled, .. }
            | InitModule::InheritVelocity { disabled, .. } => *disabled,
        }
    }

    /// Mutable access to the off switch.
    pub fn disabled_mut(&mut self) -> &mut bool {
        match self {
            InitModule::Shape { disabled, .. }
            | InitModule::Lifetime { disabled, .. }
            | InitModule::Velocity { disabled, .. }
            | InitModule::Size { disabled, .. }
            | InitModule::Color { disabled, .. }
            | InitModule::Rotation { disabled, .. }
            | InitModule::InheritVelocity { disabled, .. } => disabled,
        }
    }
}

impl UpdateModule {
    /// Display name for the module stack.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            UpdateModule::Acceleration { .. } => "Acceleration",
            UpdateModule::Drag { .. } => "Drag",
            UpdateModule::CurlNoise { .. } => "Curl noise",
            UpdateModule::Vortex { .. } => "Vortex",
            UpdateModule::Attractor { .. } => "Attractor",
            UpdateModule::SpeedLimit { .. } => "Speed limit",
            UpdateModule::CollisionPlane { .. } => "Collision plane",
            UpdateModule::SizeOverLife { .. } => "Size over life",
            UpdateModule::ColorOverLife { .. } => "Color over life",
            UpdateModule::TransparencyOverLife { .. } => "Transparency over life",
            UpdateModule::SquashOverLife { .. } => "Squash over life",
            UpdateModule::ErosionOverLife { .. } => "Erosion over life",
        }
    }

    /// True when switched off.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        match self {
            UpdateModule::Acceleration { disabled, .. }
            | UpdateModule::Drag { disabled, .. }
            | UpdateModule::CurlNoise { disabled, .. }
            | UpdateModule::Vortex { disabled, .. }
            | UpdateModule::Attractor { disabled, .. }
            | UpdateModule::SpeedLimit { disabled, .. }
            | UpdateModule::CollisionPlane { disabled, .. }
            | UpdateModule::SizeOverLife { disabled, .. }
            | UpdateModule::ColorOverLife { disabled, .. }
            | UpdateModule::TransparencyOverLife { disabled, .. }
            | UpdateModule::SquashOverLife { disabled, .. }
            | UpdateModule::ErosionOverLife { disabled, .. } => *disabled,
        }
    }

    /// Mutable access to the off switch.
    pub fn disabled_mut(&mut self) -> &mut bool {
        match self {
            UpdateModule::Acceleration { disabled, .. }
            | UpdateModule::Drag { disabled, .. }
            | UpdateModule::CurlNoise { disabled, .. }
            | UpdateModule::Vortex { disabled, .. }
            | UpdateModule::Attractor { disabled, .. }
            | UpdateModule::SpeedLimit { disabled, .. }
            | UpdateModule::CollisionPlane { disabled, .. }
            | UpdateModule::SizeOverLife { disabled, .. }
            | UpdateModule::ColorOverLife { disabled, .. }
            | UpdateModule::TransparencyOverLife { disabled, .. }
            | UpdateModule::SquashOverLife { disabled, .. }
            | UpdateModule::ErosionOverLife { disabled, .. } => disabled,
        }
    }

    /// True for modules evaluated from the particle's age when drawing, not as forces.
    #[must_use]
    pub fn is_over_life(&self) -> bool {
        matches!(
            self,
            UpdateModule::SizeOverLife { .. }
                | UpdateModule::ColorOverLife { .. }
                | UpdateModule::TransparencyOverLife { .. }
                | UpdateModule::SquashOverLife { .. }
                | UpdateModule::ErosionOverLife { .. }
        )
    }
}
