//! Player: one person in the game.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Player", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Players")
    .describe("One person in the game.")
    .props(&[
        PropDesc::new("UserId", ValueType::Number, || Value::Number(0.0)),
        PropDesc::new("RespawnLocation", ValueType::Ref, || Value::Ref(None)),
        PropDesc::new("Character", ValueType::Ref, || Value::Ref(None)),
    ]);
