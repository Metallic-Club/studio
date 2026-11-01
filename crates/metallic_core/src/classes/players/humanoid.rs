//! Humanoid: the walking, jumping, dying part of a character.
//!
//! Tuning is clamped on write (scripts, scene files, engine code) so a huge value cannot send a
//! character to infinity (#272). Non-finite writes are refused by `World::set` and the old value
//! stays.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Humanoid", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Players")
    .describe("Makes a model a character: health, walking and jumping.")
    .props(&[
        num("Health", || Value::Number(100.0), 0.0, MAX_HUMANOID_HEALTH),
        num(
            "MaxHealth",
            || Value::Number(100.0),
            0.0,
            MAX_HUMANOID_HEALTH,
        ),
        num("WalkSpeed", || Value::Number(16.0), 0.0, MAX_WALK_SPEED),
        num("JumpPower", || Value::Number(50.0), 0.0, MAX_JUMP_POWER),
        num(
            "MaxSlopeAngle",
            || Value::Number(89.0),
            0.0,
            MAX_SLOPE_ANGLE,
        ),
    ]);
