//! 2D rigid-body physics and a platformer character controller for the Metallic Engine:
//! Rapier 2D behind our own interface, shaped like `metallic_physics` (3D).
//!
//! The crate takes plain data ([`Scene2d`] of [`BodyDesc`]s and [`JointDesc`]s keyed by
//! caller-chosen [`BodyId`]s) so it does not depend on how 2D maps onto the Instance tree, and
//! no backend type crosses the boundary, so the backend can be replaced (for example by
//! Box2D v3 once its Rust bindings mature; see `docs/agents/126-physics2d.md`).
//!
//! A [`PhysicsWorld2d`] mirrors a scene: [`PhysicsWorld2d::sync_from`] creates, rebuilds and
//! removes bodies and joints to match it; [`PhysicsWorld2d::step`] applies moved poses,
//! simulates fixed ticks and writes dynamic poses back; [`PhysicsWorld2d::raycast`] answers
//! queries; [`PhysicsWorld2d::drain_touched`] and [`PhysicsWorld2d::drain_sensor_events`]
//! report contacts. Characters ([`CharacterConfig`], [`CharacterInput`]) live in the world
//! and are moved inside its fixed tick.
//!
//! Units are studs and seconds, y points up, angles are radians counter-clockwise. Gravity
//! defaults to 196.2 studs/s^2 like 3D. Time advances in fixed 1/60 s ticks with 4 solver
//! substeps each (see [`WorldConfig`]); a non-finite or negative `dt` is rejected with
//! [`Physics2dError::InvalidDt`] and changes nothing.
//!
//! Mapping: [`BodyKind::Static`] bodies whose pose is changed in the scene are moved as
//! kinematic movers for that step, so bodies and characters standing on them are carried
//! (like anchored Parts in 3D). Sensors report enter/exit and never block. One-way platforms
//! ([`ColliderDesc::one_way`]) are solid only for things arriving from their open side, for
//! simulated bodies and characters alike. Results are deterministic for the same inputs in
//! the same build; the `cross-platform-determinism` feature makes them bit-identical across
//! machines at some speed cost.
#![warn(missing_docs)]

mod character;
mod desc;
mod error;
mod events;
mod math;
mod shapes;
mod world;

pub use character::{
    CharacterConfig, CharacterEvent, CharacterEventKind, CharacterInput, CharacterState,
    JumpPhysics, WallSide,
};
pub use desc::{
    BodyDesc, BodyId, BodyKind, ColliderDesc, CollisionGroups, JointDesc, JointId, JointKind,
    JointMotor, MAX_POLYGON_POINTS, Material2d, Scene2d, Shape2d, Spring,
};
pub use error::Physics2dError;
pub use events::{SensorEvent, SensorEventKind};
pub use math::{Pose2, Vec2};
pub use shapes::DebugGeometry;
pub use world::{
    BodyState, DebugBodyKind, DebugShape2d, PhysicsWorld2d, RayFilter, RayHit2d, WorldConfig,
};

/// Default gravity in studs per second squared (downward), the same as 3D.
pub const GRAVITY: f64 = 196.2;
