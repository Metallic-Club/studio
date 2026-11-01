//! Camera: the camera `workspace.CurrentCamera` returns.
//!
//! While `CameraType` is `Custom` the game's own camera publishes its pose here every frame; with
//! `Scriptable` (or `Fixed`) the script's `CFrame` and `FieldOfView` are the camera.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Camera", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Base")
    .describe("The view a client renders the world from.")
    // Each client has its own camera: nothing here is sent to a player.
    .props(&[
        PropDesc::new("CFrame", ValueType::CFrame, || {
            Value::CFrame(Box::new(CFrame::IDENTITY))
        })
        .server_only(),
        num("FieldOfView", || Value::Number(70.0), 1.0, 120.0).server_only(),
        choice(
            "CameraType",
            || Value::String("Custom".into()),
            CAMERA_TYPES,
        )
        .server_only(),
    ]);
