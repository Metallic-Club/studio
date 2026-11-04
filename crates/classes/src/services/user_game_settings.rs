//! `UserGameSettings`: the player's graphics settings, which scripts may read but never change.
//!
//! # Who writes it
//!
//! The engine's renderer (`metallic_engine`'s `SceneRenderer`), through [`set_user_game_settings`], whenever the
//! graphics settings in effect change: at the first frame it draws, when the player changes them, and when the Auto
//! preset's calibration chooses its tier. The properties are what the renderer applies, not what the settings file
//! says: the preset, the quality level it draws at (for Auto, the calibrated one) and each value after the player's
//! overrides. A dedicated server draws nothing, so its service keeps the defaults.
//!
//! # Why read-only
//!
//! The settings are the player's (ADR 0008 §8: each one is overridable by the player and *readable* by scripts). A
//! game can adapt to them, for example by spawning fewer effects at a low quality level, but cannot raise or lower
//! them: every property is `READ_ONLY` for scripts, and none is saved with the game.
//!
//! The render scale here is the setting; with dynamic resolution on, the scale of each frame moves between the
//! settings' limits and is not written every frame.

use metallic_core::{ClassDesc, DataModel, EnumDesc, EnumItem, PropFlags, PropMeta, Value};

use crate::GameError;

/// `GraphicsPreset`'s items, in order.
pub static ENUMS: &[EnumDesc] = &[
    EnumDesc::new("GraphicsPreset", &["Low", "Medium", "High", "Ultra", "Auto"]).describe(
        "A graphics preset: how much a frame may cost, never what the game looks like. Auto measures the machine \
         and holds the frame time with dynamic resolution.",
    ),
];

/// The group the properties are listed under in the API dump.
const GROUP: &str = "Graphics";

/// Set by the engine, read by scripts, never saved.
const SETTING: PropFlags = PropFlags::READ_ONLY
    .union(PropFlags::NO_SAVE)
    .union(PropFlags::RUNTIME);

static PROPS: &[PropMeta] = &[
    PropMeta::enumeration("GraphicsPreset", "GraphicsPreset", "Auto")
        .flags(SETTING)
        .group(GROUP)
        .order(0)
        .desc("The graphics preset the player chose."),
    PropMeta::int("QualityLevel", 2)
        .flags(SETTING)
        .group(GROUP)
        .order(1)
        .desc(
            "The quality level drawn at, from 1 (Low) to 4 (Ultra). With the Auto preset, the level its calibration \
             chose for this machine.",
        ),
    PropMeta::number("RenderScale", 1.0)
        .flags(SETTING)
        .group(GROUP)
        .order(2)
        .desc(
            "The share of the window's width and height the scene is drawn at before it is scaled to the window \
             (0.5 to 2). With dynamic resolution, the highest scale it may use.",
        ),
    PropMeta::boolean("DynamicResolution", false)
        .flags(SETTING)
        .group(GROUP)
        .order(3)
        .desc("Whether the render scale changes every frame to hold the GPU frame time at TargetFrameTime."),
    PropMeta::number("TargetFrameTime", 15.0)
        .flags(SETTING)
        .group(GROUP)
        .order(4)
        .desc("The GPU frame time dynamic resolution holds, in milliseconds."),
    PropMeta::number("ViewDistance", 0.0)
        .flags(SETTING)
        .group(GROUP)
        .order(5)
        .desc("How far the camera sees, in metres; 0 when it sees without limit."),
    PropMeta::int("ShadowQualityLevel", 2)
        .flags(SETTING)
        .group(GROUP)
        .order(6)
        .desc("The quality of the sun's shadows, from 1 (Low) to 4 (Ultra): how many shadow maps, how sharp."),
    PropMeta::number("ShadowDistance", 192.0)
        .flags(SETTING)
        .group(GROUP)
        .order(7)
        .desc("How far from the camera the sun's shadows reach, in metres."),
    PropMeta::int("LocalLightBudget", 128)
        .flags(SETTING)
        .group(GROUP)
        .order(8)
        .desc(
            "How many point and spot lights a frame draws at full strength, the most important ones; the others \
             fade out.",
        ),
    PropMeta::number("LodBias", 1.0)
        .flags(SETTING)
        .group(GROUP)
        .order(9)
        .desc("How finely distant round parts are drawn: 1 keeps their outlines within half a pixel; higher is finer."),
    PropMeta::boolean("VSync", true)
        .flags(SETTING)
        .group(GROUP)
        .order(10)
        .desc("Whether frames wait for the display's refresh."),
    PropMeta::int("FrameRateCap", 240)
        .flags(SETTING)
        .group(GROUP)
        .order(11)
        .desc("The most frames per second; 0 when uncapped."),
];

/// The graphics settings in effect, as [`set_user_game_settings`] writes them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UserGameSettings {
    /// The preset's item name (`Low`, `Medium`, `High`, `Ultra`, `Auto`).
    pub preset: &'static str,
    /// The quality level drawn at, 1 to 4.
    pub quality_level: u8,
    /// The render scale (with dynamic resolution, its highest).
    pub render_scale: f64,
    /// Whether dynamic resolution is on.
    pub dynamic_resolution: bool,
    /// The GPU frame time dynamic resolution holds, in milliseconds.
    pub target_frame_time: f64,
    /// The view distance in metres; 0 for none.
    pub view_distance: f64,
    /// The sun's shadow quality, 1 to 4.
    pub shadow_quality_level: u8,
    /// How far the sun's shadows reach, in metres.
    pub shadow_distance: f64,
    /// The local lights a frame draws.
    pub local_light_budget: u32,
    /// The levels of detail's bias.
    pub lod_bias: f64,
    /// Whether frames wait for vsync.
    pub vsync: bool,
    /// The frame cap; 0 for none.
    pub frame_rate_cap: u32,
}

/// Writes `settings` into the game's `UserGameSettings`, creating the service if the game lacks it. Only the
/// properties that changed fire `Changed`.
pub fn set_user_game_settings(dm: &mut DataModel, settings: &UserGameSettings) -> Result<(), GameError> {
    let class = dm
        .registry()
        .class_id(CLASS.name)
        .ok_or(GameError::MissingClass(CLASS.name))?;
    let service = match dm.service(class) {
        Some(service) => service,
        None => dm.create_service(class)?,
    };
    let values = [
        ("GraphicsPreset", Value::Enum(EnumItem::new("GraphicsPreset", settings.preset))),
        ("QualityLevel", Value::Int(i64::from(settings.quality_level))),
        ("RenderScale", Value::Number(settings.render_scale)),
        ("DynamicResolution", Value::Bool(settings.dynamic_resolution)),
        ("TargetFrameTime", Value::Number(settings.target_frame_time)),
        ("ViewDistance", Value::Number(settings.view_distance)),
        ("ShadowQualityLevel", Value::Int(i64::from(settings.shadow_quality_level))),
        ("ShadowDistance", Value::Number(settings.shadow_distance)),
        ("LocalLightBudget", Value::Int(i64::from(settings.local_light_budget))),
        ("LodBias", Value::Number(settings.lod_bias)),
        ("VSync", Value::Bool(settings.vsync)),
        ("FrameRateCap", Value::Int(i64::from(settings.frame_rate_cap))),
    ];
    for (name, value) in values {
        if dm.get_by_name(service, name).ok().as_ref() != Some(&value) {
            dm.set_by_name(service, name, value)?;
        }
    }
    Ok(())
}

/// The player's graphics settings, readable by scripts.
pub static CLASS: ClassDesc = ClassDesc::new("UserGameSettings", Some("Instance"))
    .describe(
        "The player's graphics settings: the preset, the quality level it draws at and the values in effect. \
         Scripts can read them, to adapt a game to the player's machine, but never change them: they are the \
         player's. Get it with game:GetService(\"UserGameSettings\").",
    )
    .service(7)
    .hidden_in_explorer()
    .props(PROPS);

#[cfg(test)]
mod tests {
    use metallic_core::{EnumItem, PropFlags, Value};

    use super::{UserGameSettings, set_user_game_settings};
    use crate::testing::game;

    fn service(dm: &metallic_core::DataModel) -> metallic_core::Id {
        let class = dm.registry().class_id("UserGameSettings").unwrap();
        dm.service(class).unwrap()
    }

    #[test]
    fn the_engine_writes_what_scripts_read() {
        let mut dm = game();
        let id = service(&dm);
        assert_eq!(
            dm.get_by_name(id, "GraphicsPreset"),
            Ok(Value::Enum(EnumItem::new("GraphicsPreset", "Auto")))
        );
        let settings = UserGameSettings {
            preset: "High",
            quality_level: 3,
            render_scale: 0.75,
            dynamic_resolution: true,
            target_frame_time: 8.0,
            view_distance: 2048.0,
            shadow_quality_level: 3,
            shadow_distance: 256.0,
            local_light_budget: 192,
            lod_bias: 2.0,
            vsync: false,
            frame_rate_cap: 0,
        };
        set_user_game_settings(&mut dm, &settings).unwrap();
        assert_eq!(
            dm.get_by_name(id, "GraphicsPreset"),
            Ok(Value::Enum(EnumItem::new("GraphicsPreset", "High")))
        );
        assert_eq!(dm.get_by_name(id, "QualityLevel"), Ok(Value::Int(3)));
        assert_eq!(dm.get_by_name(id, "RenderScale"), Ok(Value::Number(0.75)));
        assert_eq!(dm.get_by_name(id, "DynamicResolution"), Ok(Value::Bool(true)));
        assert_eq!(dm.get_by_name(id, "LocalLightBudget"), Ok(Value::Int(192)));
        assert_eq!(dm.get_by_name(id, "VSync"), Ok(Value::Bool(false)));
        assert_eq!(dm.get_by_name(id, "FrameRateCap"), Ok(Value::Int(0)));
    }

    #[test]
    fn scripts_cannot_write_and_games_do_not_save_the_settings() {
        let dm = game();
        let id = service(&dm);
        let class = dm.class_of(id).unwrap();
        let registry = dm.registry();
        let desc = registry.class(class).unwrap().desc();
        assert!(desc.hidden_in_explorer);
        for prop in desc.props {
            let meta = registry.prop(class, registry.prop_id(class, prop.name).unwrap()).unwrap();
            assert!(meta.meta.flags.contains(PropFlags::READ_ONLY), "{}", prop.name);
            assert!(meta.meta.flags.contains(PropFlags::NO_SAVE), "{}", prop.name);
        }
    }
}
