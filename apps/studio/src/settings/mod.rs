//! Studio's settings (ADR 0009, Settings): one typed [`StudioSettings`] in categories (General, Editing, Appearance,
//! Rendering, Keybinds), stored as `settings.toml` in Studio's config folder ([`store`]) and edited in the Settings
//! window ([`window`], with the Keybinds page in [`keybinds`]).
//!
//! Reading never fails: every field has a default, a value of the wrong kind takes it, and a value out of range is
//! clamped. Keys Studio does not know are kept in the file, so a newer Studio's settings survive a downgrade.

pub mod keybinds;
pub mod store;
pub mod window;

use metallic_engine::metallic_render::lights::MAX_GPU_LIGHTS;
use metallic_engine::metallic_render::resolution::{MAX_SCALE, MIN_SCALE};
use metallic_engine::metallic_render::settings::GraphicsSettings;
use toml::{Table, Value};

use crate::keymap::{Overrides, read_overrides, write_overrides};
use crate::theme::tokens::Accent;
use crate::theme::{Look, ThemeMode};
pub use keybinds::KeybindsPage;
pub use store::{SETTINGS_FILE, SettingsStore, settings_path};
pub use window::{Category, SettingsWindow};

/// The settings file's format version, written as `version` when the file has none.
pub const FORMAT_VERSION: i64 = 1;

/// Everything Studio remembers between runs, by category.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StudioSettings {
    /// Studio as a whole.
    pub general: General,
    /// How the viewport's tools edit (#359).
    pub editing: Editing,
    /// The viewport's camera (#385).
    pub viewport: Viewport,
    /// The look of the interface.
    pub appearance: Appearance,
    /// The viewport's graphics: the engine's own settings type (#84), so the viewport applies them as the player
    /// does. Its `calibration` is always `None` here: Auto's measurement is the player's, not a user setting.
    pub rendering: GraphicsSettings,
    /// The keymap's overrides (#214).
    pub keybinds: Keybinds,
}

/// The General category.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct General {
    /// Skip the sign-in screen and go straight to the projects (the sign-in screen's "Don't ask again").
    pub skip_sign_in: bool,
}

/// The Editing category (#359): the increments the gizmos' drags snap to, and whether they do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Editing {
    /// Whether moves and sizes snap to [`Editing::move_step`].
    pub snap_moves: bool,
    /// The move increment, in studs.
    pub move_step: f32,
    /// Whether turns snap to [`Editing::turn_step`].
    pub snap_turns: bool,
    /// The rotate increment, in degrees.
    pub turn_step: f32,
}

impl Default for Editing {
    fn default() -> Self {
        Self {
            snap_moves: true,
            move_step: 1.0,
            snap_turns: true,
            turn_step: 15.0,
        }
    }
}

/// The move increment's range, in studs.
pub const MOVE_STEP: (f32, f32) = (0.01, 1000.0);
/// The rotate increment's range, in degrees.
pub const TURN_STEP: (f32, f32) = (0.1, 180.0);

/// The viewport's settings (#385): how fast its camera flies. Saved in the `[viewport]` section; the speeds are on
/// Settings, Editing, and the wheel changes them in the viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// How fast the camera flies, in studs a second.
    pub fly_speed: f32,
    /// What Shift multiplies the fly speed by: above one faster, below it slower and more careful.
    pub secondary_speed: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            fly_speed: 40.0,
            secondary_speed: 3.0,
        }
    }
}

/// The fly speed's range, in studs a second.
pub const FLY_SPEED: (f32, f32) = (0.1, 1000.0);
/// The secondary speed's range, a multiple of the fly speed.
pub const SECONDARY_SPEED: (f32, f32) = (0.05, 50.0);

/// The Appearance category: what [`Look`] the interface has, and how large it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    /// Dark, light or the system's.
    pub mode: ThemeMode,
    /// The accent colour.
    pub accent: Accent,
    /// Secondary text as strong as body text, and stronger borders.
    pub high_contrast: bool,
    /// The interface's scale, [`ui_scale::MIN`] to [`ui_scale::MAX`] (egui's zoom factor).
    pub ui_scale: f32,
    /// Whether fades and eases play.
    pub motion: MotionPreference,
    /// Whether each group of docked panels is outlined, as the Settings window is (#262).
    pub panel_borders: bool,
    /// A folder of custom icons, empty for none: a file named after an icon's id replaces it (#231).
    pub icon_folder: String,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            mode: ThemeMode::default(),
            accent: Accent::default(),
            high_contrast: false,
            ui_scale: 1.0,
            motion: MotionPreference::default(),
            panel_borders: true,
            icon_folder: String::new(),
        }
    }
}

impl Appearance {
    /// The look these settings give, when the system does or does not ask for reduced motion.
    pub fn look(&self, system_reduces_motion: bool) -> Look {
        Look {
            mode: self.mode,
            accent: self.accent,
            high_contrast: self.high_contrast,
            reduce_motion: self.motion.reduces(system_reduces_motion),
        }
    }
}

/// The interface's scale.
pub mod ui_scale {
    /// The smallest.
    pub const MIN: f32 = 0.75;
    /// The largest.
    pub const MAX: f32 = 2.0;
    /// The scales Settings offers.
    pub const CHOICES: [f32; 8] = [0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];
}

/// Whether fades and eases play: as the system says, or always reduced, or always full.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MotionPreference {
    /// Follow the system's switch, where Studio can read it.
    #[default]
    System,
    /// No fades or eases.
    Reduced,
    /// Fades and eases, whatever the system says.
    Full,
}

impl MotionPreference {
    /// Every choice, in the order Settings lists them.
    pub const ALL: [MotionPreference; 3] = [
        MotionPreference::System,
        MotionPreference::Reduced,
        MotionPreference::Full,
    ];

    /// The id the settings store.
    pub fn as_str(self) -> &'static str {
        match self {
            MotionPreference::System => "system",
            MotionPreference::Reduced => "reduced",
            MotionPreference::Full => "full",
        }
    }

    /// The name people see.
    pub fn label(self) -> &'static str {
        match self {
            MotionPreference::System => "System",
            MotionPreference::Reduced => "Reduced",
            MotionPreference::Full => "Full",
        }
    }

    /// Reads an id in any letter case; `None` for an unknown one.
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|choice| choice.as_str().eq_ignore_ascii_case(id.trim()))
    }

    /// Whether motion is reduced, when the system does or does not ask for it.
    pub fn reduces(self, system_reduces_motion: bool) -> bool {
        match self {
            MotionPreference::System => system_reduces_motion,
            MotionPreference::Reduced => true,
            MotionPreference::Full => false,
        }
    }
}

/// The Keybinds category: the keymap's overrides ([`crate::keymap`]), `[keybinds]` in the file. Only bindings that
/// differ from the defaults are stored (`"file.save" = ["Primary+S"]`, `[]` for unbound); ids this Studio does not
/// know are kept, so a newer Studio's bindings survive a downgrade.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keybinds {
    /// Command id to binding texts.
    pub overrides: Overrides,
}

/// The `[rendering]` keys Studio owns: [`GraphicsSettings`]' file keys, less the calibration (Auto's measurement,
/// which is the player's and not a user setting).
pub const RENDERING_KEYS: [&str; 15] = [
    "preset",
    "render_scale",
    "dynamic_resolution",
    "target_frame_ms",
    "min_render_scale",
    "max_render_scale",
    "vsync",
    "frame_cap",
    "view_distance",
    "shadow_quality",
    "shadow_distance",
    "light_budget",
    "lod_bias",
    "anti_aliasing",
    "effect_quality",
];

/// The most frames per second a frame cap may ask for.
pub const MAX_FRAME_CAP: u32 = 1000;
/// The farthest view distance, in metres.
pub const MAX_VIEW_DISTANCE: f32 = 100_000.0;
/// The nearest and farthest shadow distances, in metres.
pub const SHADOW_DISTANCE: (f32, f32) = (16.0, 10_000.0);
/// The shortest and longest GPU frame time dynamic resolution may hold, in milliseconds.
pub const TARGET_FRAME_MS: (f32, f32) = (1.0, 100.0);
/// The levels of detail's bias range.
pub const LOD_BIAS: (f32, f32) = (0.25, 8.0);

impl StudioSettings {
    /// The settings in `document` (a parsed `settings.toml`): defaults for what is missing or of the wrong kind,
    /// clamped where out of range. Each value not used is logged.
    pub fn from_document(document: &Table) -> Self {
        let mut settings = Self::default();
        let section = |name: &str| document.get(name).and_then(Value::as_table);
        if let Some(general) = section("general") {
            read(
                general,
                "skip_sign_in",
                Value::as_bool,
                &mut settings.general.skip_sign_in,
            );
        }
        if let Some(editing) = section("editing") {
            let e = &mut settings.editing;
            read(editing, "snap_moves", Value::as_bool, &mut e.snap_moves);
            read(editing, "move_step", number, &mut e.move_step);
            read(editing, "snap_turns", Value::as_bool, &mut e.snap_turns);
            read(editing, "turn_step", number, &mut e.turn_step);
        }
        if let Some(viewport) = section("viewport") {
            let v = &mut settings.viewport;
            read(viewport, "fly_speed", number, &mut v.fly_speed);
            read(viewport, "secondary_speed", number, &mut v.secondary_speed);
        }
        if let Some(appearance) = section("appearance") {
            let a = &mut settings.appearance;
            read(
                appearance,
                "mode",
                |v| v.as_str().and_then(ThemeMode::parse),
                &mut a.mode,
            );
            read(
                appearance,
                "accent",
                |v| v.as_str().and_then(Accent::parse),
                &mut a.accent,
            );
            read(appearance, "high_contrast", Value::as_bool, &mut a.high_contrast);
            read(appearance, "ui_scale", number, &mut a.ui_scale);
            read(
                appearance,
                "reduce_motion",
                |v| v.as_str().and_then(MotionPreference::parse),
                &mut a.motion,
            );
            read(appearance, "panel_borders", Value::as_bool, &mut a.panel_borders);
            read(
                appearance,
                "icon_folder",
                |v| v.as_str().map(str::to_owned),
                &mut a.icon_folder,
            );
        }
        if let Some(rendering) = section("rendering") {
            settings.rendering = rendering_from(rendering);
        }
        if let Some(keybinds) = section("keybinds") {
            settings.keybinds.overrides = read_overrides(keybinds);
        }
        settings.clamp();
        settings
    }

    /// `base` (the file as it was read) with these settings written over it: every key Studio does not know stays.
    pub fn to_document(&self, base: &Table) -> Table {
        let mut document = base.clone();
        document.entry("version").or_insert(Value::Integer(FORMAT_VERSION));
        with_section(&mut document, "general", |general| {
            general.insert("skip_sign_in".into(), Value::Boolean(self.general.skip_sign_in));
        });
        let e = &self.editing;
        with_section(&mut document, "editing", |editing| {
            // In thousandths, so 0.2 is written as 0.2 and not as the f32's 0.20000000298023224.
            let round = |step: f32| Value::Float((f64::from(step) * 1000.0).round() / 1000.0);
            editing.insert("snap_moves".into(), Value::Boolean(e.snap_moves));
            editing.insert("move_step".into(), round(e.move_step));
            editing.insert("snap_turns".into(), Value::Boolean(e.snap_turns));
            editing.insert("turn_step".into(), round(e.turn_step));
        });
        let v = &self.viewport;
        with_section(&mut document, "viewport", |viewport| {
            let round = |speed: f32| Value::Float((f64::from(speed) * 100.0).round() / 100.0);
            viewport.insert("fly_speed".into(), round(v.fly_speed));
            viewport.insert("secondary_speed".into(), round(v.secondary_speed));
        });
        let a = &self.appearance;
        with_section(&mut document, "appearance", |appearance| {
            appearance.insert("mode".into(), Value::String(a.mode.as_str().into()));
            appearance.insert("accent".into(), Value::String(a.accent.as_str().into()));
            appearance.insert("high_contrast".into(), Value::Boolean(a.high_contrast));
            // In hundredths, so 1.1 is written as 1.1 and not as the f32's 1.100000023841858.
            let scale = (f64::from(a.ui_scale) * 100.0).round() / 100.0;
            appearance.insert("ui_scale".into(), Value::Float(scale));
            appearance.insert("reduce_motion".into(), Value::String(a.motion.as_str().into()));
            appearance.insert("panel_borders".into(), Value::Boolean(a.panel_borders));
            appearance.insert("icon_folder".into(), Value::String(a.icon_folder.clone()));
        });
        with_section(&mut document, "rendering", |rendering| {
            for key in RENDERING_KEYS {
                rendering.remove(key);
            }
            rendering.extend(rendering_to(&self.rendering));
        });
        with_section(&mut document, "keybinds", |keybinds| {
            write_overrides(&self.keybinds.overrides, keybinds);
        });
        document
    }

    /// Brings every value into its range.
    pub fn clamp(&mut self) {
        let e = &mut self.editing;
        let within = |value: f32, (min, max): (f32, f32), default: f32| {
            if value.is_finite() {
                value.clamp(min, max)
            } else {
                default
            }
        };
        e.move_step = within(e.move_step, MOVE_STEP, 1.0);
        e.turn_step = within(e.turn_step, TURN_STEP, 15.0);
        let v = &mut self.viewport;
        v.fly_speed = within(v.fly_speed, FLY_SPEED, 40.0);
        v.secondary_speed = within(v.secondary_speed, SECONDARY_SPEED, 3.0);
        let a = &mut self.appearance;
        a.ui_scale = if a.ui_scale.is_finite() {
            a.ui_scale.clamp(ui_scale::MIN, ui_scale::MAX)
        } else {
            1.0
        };
        clamp_rendering(&mut self.rendering);
    }
}

/// Sets `out` to `table[key]` read by `get`, or logs why the value was not used and leaves `out` as it is.
fn read<T>(table: &Table, key: &str, get: impl FnOnce(&Value) -> Option<T>, out: &mut T) {
    if let Some(value) = table.get(key) {
        match get(value) {
            Some(read) => *out = read,
            None => log::warn!("settings: `{key} = {value}` is not a valid value; using the default"),
        }
    }
}

/// A number, integer or not.
fn number(value: &Value) -> Option<f32> {
    match value {
        Value::Float(n) => Some(*n as f32),
        Value::Integer(n) => Some(*n as f32),
        _ => None,
    }
}

/// Edits the table called `name` in `document` with `edit`: an empty one if there is none, or if the value there is
/// not a table.
fn with_section(document: &mut Table, name: &str, edit: impl FnOnce(&mut Table)) {
    let mut section = match document.remove(name) {
        Some(Value::Table(table)) => table,
        _ => Table::new(),
    };
    edit(&mut section);
    document.insert(name.into(), Value::Table(section));
}

/// The graphics settings in a `[rendering]` table, read with the engine's own parser
/// ([`GraphicsSettings::from_text`]), one key at a time so a bad value loses only itself.
fn rendering_from(table: &Table) -> GraphicsSettings {
    let mut text = String::new();
    for key in RENDERING_KEYS {
        let Some(value) = table.get(key) else {
            continue;
        };
        let line = format!("{key} = {value}\n");
        match GraphicsSettings::from_text(&line) {
            Ok(_) => text.push_str(&line),
            Err(error) => log::warn!(
                "settings: [rendering] {}: {}; using the default",
                line.trim(),
                error.reason
            ),
        }
    }
    // Every line parsed alone, so they parse together.
    GraphicsSettings::from_text(&text).unwrap_or_default()
}

/// The `[rendering]` keys of `graphics`: the preset and each override set, written by the engine
/// ([`GraphicsSettings::to_text`]).
fn rendering_to(graphics: &GraphicsSettings) -> Table {
    let graphics = GraphicsSettings {
        calibration: None,
        ..graphics.clone()
    };
    match graphics.to_text().parse::<Table>() {
        Ok(table) => table,
        Err(error) => {
            // The engine writes TOML; this is a bug, and only the preset is kept.
            log::error!("settings: the graphics settings did not read back as TOML: {error}");
            Table::from_iter([("preset".to_owned(), Value::String(graphics.preset.name().into()))])
        }
    }
}

/// Brings the graphics overrides into the ranges the renderer takes, and drops the calibration.
fn clamp_rendering(g: &mut GraphicsSettings) {
    let finite = |value: &mut Option<f32>| {
        if value.is_some_and(|v| !v.is_finite()) {
            *value = None;
        }
    };
    for scale in [&mut g.render_scale, &mut g.min_render_scale, &mut g.max_render_scale] {
        finite(scale);
        *scale = scale.map(|s| s.clamp(MIN_SCALE, MAX_SCALE));
    }
    finite(&mut g.target_frame_ms);
    g.target_frame_ms = g.target_frame_ms.map(|t| t.clamp(TARGET_FRAME_MS.0, TARGET_FRAME_MS.1));
    g.frame_cap = g.frame_cap.map(|cap| cap.min(MAX_FRAME_CAP));
    finite(&mut g.view_distance);
    g.view_distance = g.view_distance.map(|d| d.clamp(0.0, MAX_VIEW_DISTANCE));
    finite(&mut g.shadow_distance);
    g.shadow_distance = g.shadow_distance.map(|d| d.clamp(SHADOW_DISTANCE.0, SHADOW_DISTANCE.1));
    g.light_budget = g.light_budget.map(|n| n.clamp(1, MAX_GPU_LIGHTS as u32));
    finite(&mut g.lod_bias);
    g.lod_bias = g.lod_bias.map(|b| b.clamp(LOD_BIAS.0, LOD_BIAS.1));
    g.calibration = None;
}

#[cfg(test)]
mod tests {
    use metallic_engine::metallic_render::AntiAliasing;
    use metallic_engine::metallic_render::settings::{Preset, Tier};

    use super::*;

    fn parse(text: &str) -> StudioSettings {
        StudioSettings::from_document(&text.parse().unwrap())
    }

    fn changed() -> StudioSettings {
        let mut settings = StudioSettings::default();
        settings.general.skip_sign_in = true;
        settings.appearance = Appearance {
            mode: ThemeMode::System,
            accent: Accent::Violet,
            high_contrast: true,
            ui_scale: 1.25,
            motion: MotionPreference::Reduced,
            panel_borders: false,
            icon_folder: "C:/Icons".to_owned(),
        };
        settings.rendering = GraphicsSettings {
            preset: Preset::High,
            anti_aliasing: Some(AntiAliasing::Msaa(4)),
            vsync: Some(false),
            frame_cap: Some(144),
            render_scale: Some(0.75),
            shadow_quality: Some(Tier::Ultra),
            ..GraphicsSettings::default()
        };
        settings
    }

    #[test]
    fn an_empty_file_is_the_defaults() {
        let settings = parse("");
        assert_eq!(settings, StudioSettings::default());
        assert_eq!(settings.appearance.look(false), Look::default());
        assert!(
            settings.appearance.look(true).reduce_motion,
            "follows the system by default"
        );
        assert_eq!(settings.rendering.preset, Preset::Auto);
    }

    #[test]
    fn settings_round_trip_through_a_document() {
        let settings = changed();
        let document = settings.to_document(&Table::new());
        let text = document.to_string();
        assert!(text.contains("version = 1"), "{text}");
        assert!(text.contains("accent = \"violet\""), "{text}");
        assert!(text.contains("anti_aliasing = \"msaa4\""), "{text}");
        assert!(!text.contains("lod_bias"), "only overrides that are set: {text}");
        assert_eq!(parse(&text), settings);
        let defaults = StudioSettings::default().to_document(&Table::new()).to_string();
        assert_eq!(parse(&defaults), StudioSettings::default());
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let settings = parse(
            "[appearance]\nui_scale = 9\n\
             [rendering]\nrender_scale = 7.5\nframe_cap = 99999\nlight_budget = 0\nlod_bias = -3.0\n\
             shadow_distance = 1.0\nview_distance = -5.0\ntarget_frame_ms = 0.0\n",
        );
        assert_eq!(settings.appearance.ui_scale, ui_scale::MAX);
        let g = &settings.rendering;
        assert_eq!(g.render_scale, Some(MAX_SCALE));
        assert_eq!(g.frame_cap, Some(MAX_FRAME_CAP));
        assert_eq!(g.light_budget, Some(1));
        assert_eq!(g.lod_bias, Some(LOD_BIAS.0));
        assert_eq!(g.shadow_distance, Some(SHADOW_DISTANCE.0));
        assert_eq!(g.view_distance, Some(0.0));
        assert_eq!(g.target_frame_ms, Some(TARGET_FRAME_MS.0));
        assert_eq!(parse("[appearance]\nui_scale = 0.1").appearance.ui_scale, ui_scale::MIN);
        assert_eq!(parse("[appearance]\nui_scale = nan").appearance.ui_scale, 1.0);
    }

    #[test]
    fn values_of_the_wrong_kind_take_their_defaults() {
        let settings = parse(
            "general = 5\n\
             [appearance]\nmode = \"sepia\"\naccent = 3\nhigh_contrast = true\nreduce_motion = \"FULL\"\n\
             [rendering]\npreset = \"ultra\"\nvsync = \"maybe\"\nanti_aliasing = \"mlaa\"\n",
        );
        assert_eq!(settings.general, General::default());
        assert_eq!(settings.appearance.mode, ThemeMode::Dark);
        assert_eq!(settings.appearance.accent, Accent::Orange);
        assert!(
            settings.appearance.high_contrast,
            "the good values next to bad ones are kept"
        );
        assert_eq!(settings.appearance.motion, MotionPreference::Full);
        assert_eq!(settings.rendering.preset, Preset::Ultra);
        assert_eq!(settings.rendering.vsync, None);
        assert_eq!(settings.rendering.anti_aliasing, Some(AntiAliasing::Mlaa));
    }

    #[test]
    fn unknown_keys_are_kept() {
        let base: Table = "version = 7\nfuture = \"yes\"\n\
             [appearance]\nmode = \"light\"\nsparkles = 3\n\
             [rendering]\nvsync = true\nray_tracing = \"on\"\n\
             [keybinds]\n\"file.save\" = [\"Primary+S\"]\n"
            .parse()
            .unwrap();
        let settings = StudioSettings::from_document(&base);
        assert_eq!(settings.appearance.mode, ThemeMode::Light);
        let mut changed = settings.clone();
        changed.rendering.vsync = None;
        let out = changed.to_document(&base);
        assert_eq!(out["version"].as_integer(), Some(7), "a newer file's version is kept");
        assert_eq!(out["future"].as_str(), Some("yes"));
        assert_eq!(out["appearance"]["sparkles"].as_integer(), Some(3));
        assert_eq!(out["rendering"]["ray_tracing"].as_str(), Some("on"));
        assert!(
            out["rendering"].get("vsync").is_none(),
            "an override cleared is removed"
        );
        assert_eq!(out["keybinds"]["file.save"][0].as_str(), Some("Primary+S"));
    }

    #[test]
    fn the_editing_increments_round_trip_and_stay_in_range() {
        let document: Table = toml::from_str(
            "[editing]
move_step = 0.5
snap_turns = false
turn_step = 999
",
        )
        .unwrap();
        let settings = StudioSettings::from_document(&document);
        assert_eq!(
            settings.editing,
            Editing {
                snap_moves: true,
                move_step: 0.5,
                snap_turns: false,
                turn_step: TURN_STEP.1,
            }
        );
        let written = settings.to_document(&Table::new());
        assert_eq!(StudioSettings::from_document(&written).editing, settings.editing);
        assert_eq!(StudioSettings::default().editing.move_step, 1.0);
    }

    #[test]
    fn the_camera_speeds_round_trip_and_stay_in_range() {
        let document: Table = toml::from_str("[viewport]\nfly_speed = 12.5\nsecondary_speed = 0.01\n").unwrap();
        let settings = StudioSettings::from_document(&document);
        assert_eq!(
            settings.viewport,
            Viewport {
                fly_speed: 12.5,
                secondary_speed: SECONDARY_SPEED.0,
            }
        );
        let written = settings.to_document(&Table::new());
        assert_eq!(StudioSettings::from_document(&written).viewport, settings.viewport);
        assert_eq!(StudioSettings::default().viewport.fly_speed, 40.0);
    }

    #[test]
    fn keybinds_round_trip_and_keep_unknown_ids() {
        let base: Table = "[keybinds]\n\"future.teleport\" = [\"Primary+J\"]\n\"settings.open\" = [\"F10\"]\nodd = 5\n"
            .parse()
            .unwrap();
        let mut settings = StudioSettings::from_document(&base);
        assert_eq!(settings.keybinds.overrides["settings.open"], ["F10"]);
        assert_eq!(settings.keybinds.overrides["future.teleport"], ["Primary+J"]);
        // Settings back to its default, Zoom unbound.
        settings.keybinds.overrides.remove("settings.open");
        settings.keybinds.overrides.insert("viewport.zoom".into(), Vec::new());
        let out = settings.to_document(&base);
        let keybinds = out["keybinds"].as_table().unwrap();
        assert!(keybinds.get("settings.open").is_none(), "only overrides are stored");
        assert_eq!(keybinds["viewport.zoom"].as_array().map(Vec::len), Some(0));
        assert_eq!(keybinds["future.teleport"][0].as_str(), Some("Primary+J"));
        assert_eq!(
            keybinds["odd"].as_integer(),
            Some(5),
            "a value that is not bindings stays"
        );
        assert_eq!(StudioSettings::from_document(&out), settings);
    }

    #[test]
    fn the_rendering_keys_are_the_engines() {
        // Every override the engine writes (as a comment when unset) is one Studio owns, and no other.
        let text = GraphicsSettings::default().to_text();
        let mut keys: Vec<&str> = text
            .lines()
            .filter_map(|line| line.trim_start_matches("# ").split_once(" = ").map(|(key, _)| key))
            .filter(|key| key.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
            .collect();
        keys.sort_unstable();
        let mut ours = RENDERING_KEYS.to_vec();
        ours.sort_unstable();
        assert_eq!(keys, ours);
    }

    #[test]
    fn motion_follows_the_system_unless_chosen() {
        assert!(MotionPreference::System.reduces(true));
        assert!(!MotionPreference::System.reduces(false));
        assert!(MotionPreference::Reduced.reduces(false));
        assert!(!MotionPreference::Full.reduces(true));
        for choice in MotionPreference::ALL {
            assert_eq!(MotionPreference::parse(choice.as_str()), Some(choice));
        }
    }

    #[test]
    fn the_old_tab_position_key_is_kept_and_changes_nothing() {
        // #230's setting, removed in #236: a file that has it reads as the defaults and keeps the key.
        let base: Table = "[appearance]\ntab_position = \"bottom\"\n".parse().unwrap();
        let settings = StudioSettings::from_document(&base);
        assert_eq!(settings, StudioSettings::default());
        let out = settings.to_document(&base);
        assert_eq!(out["appearance"]["tab_position"].as_str(), Some("bottom"));
    }
}
