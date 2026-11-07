//! The editor's panels (ADR 0009, Docking): each a [`Panel`] in its own module, all listed in [`PANELS`].
//!
//! Adding a panel is one unit type implementing [`Panel`] and one line in [`PANELS`]; its state, if any, is a field
//! of [`crate::editor::EditorState`].

pub mod assets;
pub mod explorer;
pub mod game;
pub mod output;
pub mod properties;
pub mod viewport;

use egui::{Align, Label, Layout, RichText, TextEdit, Ui, vec2};
use metallic_engine::metallic_core::{ClassId, Registry};

use crate::dock::Panel;
use crate::theme::tokens::{space, text};
use crate::theme::{current_colors, fonts};

/// Every panel, in the order the View menu lists them and the default layout's tabs take.
pub static PANELS: &[&dyn Panel] = &[
    &viewport::Viewport,
    &explorer::Explorer,
    &assets::Assets,
    &properties::Properties,
    &output::Output,
    &game::Game,
];

/// The panels' ids.
pub mod ids {
    /// The viewport.
    pub const VIEWPORT: &str = "viewport";
    /// The Explorer.
    pub const EXPLORER: &str = "explorer";
    /// The Assets panel.
    pub const ASSETS: &str = "assets";
    /// The Properties panel.
    pub const PROPERTIES: &str = "properties";
    /// The Output panel.
    pub const OUTPUT: &str = "output";
    /// The running game (Play in Studio).
    pub const GAME: &str = "game";
}

/// Phosphor icons the panels use beyond the theme's own, by their code points in `Phosphor.ttf`.
pub mod icon {
    /// A cube: a part.
    pub const CUBE: char = '\u{E1DA}';
    /// A see-through cube: a mesh part.
    pub const CUBE_TRANSPARENT: char = '\u{EC7C}';
    /// Shapes: a model.
    pub const SHAPES: char = '\u{EC5E}';
    /// A globe: the workspace.
    pub const GLOBE: char = '\u{E28C}';
    /// The sun: lighting, a directional light.
    pub const SUN: char = '\u{E472}';
    /// A light bulb: a light.
    pub const LIGHTBULB: char = '\u{E2DC}';
    /// A flashlight: a spotlight.
    pub const FLASHLIGHT: char = '\u{E246}';
    /// A video camera: a camera.
    pub const VIDEO_CAMERA: char = '\u{E4DA}';
    /// A file of code: a script.
    pub const FILE_CODE: char = '\u{E914}';
    /// Brackets: a module script, script services.
    pub const CODE: char = '\u{E1BC}';
    /// A folder.
    pub const FOLDER: char = '\u{E25A}';
    /// A package: replicated storage.
    pub const PACKAGE: char = '\u{E390}';
    /// Drives: server storage.
    pub const HARD_DRIVES: char = '\u{E2A0}';
    /// A diamond: a value.
    pub const DIAMOND: char = '\u{E1EC}';
    /// A broadcast: a bindable event or function.
    pub const BROADCAST: char = '\u{E0F2}';
    /// A cursor: an input object.
    pub const CURSOR: char = '\u{E1DC}';
    /// Sliders: settings services.
    pub const SLIDERS: char = '\u{E434}';
    /// A game controller: the game.
    pub const GAME_CONTROLLER: char = '\u{E26E}';
    /// A circle: any other class.
    pub const CIRCLE: char = '\u{E18A}';
    /// A caret pointing right: collapsed.
    pub const CARET_RIGHT: char = '\u{E13A}';
    /// A caret pointing down: expanded.
    pub const CARET_DOWN: char = '\u{E136}';
    /// A tree: the Explorer.
    pub const TREE: char = '\u{E67C}';
    /// A list: Properties.
    pub const LIST_BULLETS: char = '\u{E2F2}';
    /// Files: Assets.
    pub const FILES: char = '\u{E710}';
    /// A terminal window: Output.
    pub const TERMINAL: char = '\u{EAE8}';
    /// A monitor: the viewport.
    pub const MONITOR: char = '\u{E32E}';
    /// A circle with a cross: an error.
    pub const X_CIRCLE: char = '\u{E4F8}';
    /// A circle with an i: information.
    pub const INFO: char = '\u{E2CE}';
    /// A check mark.
    pub const CHECK: char = '\u{E182}';
    /// A floppy disk: save.
    pub const FLOPPY_DISK: char = '\u{E248}';
    /// A broom: clear.
    pub const BROOM: char = '\u{EC54}';
    /// A file.
    pub const FILE: char = '\u{E230}';
    /// A picture: an image asset.
    pub const IMAGE: char = '\u{E2CA}';
    /// A note: an audio asset.
    pub const MUSIC_NOTE: char = '\u{E33C}';
    /// A triangle: play.
    pub const PLAY: char = '\u{E3D0}';
    /// Two bars: pause.
    pub const PAUSE: char = '\u{E39E}';
    /// A square: stop.
    pub const STOP: char = '\u{E46C}';
    /// Skip forward (a triangle against a bar): step one frame.
    pub const STEP: char = '\u{E5A6}';
}

/// The icon of `class`: its own, or its nearest ancestor's.
pub fn class_icon(registry: &Registry, class: ClassId) -> char {
    let Some(info) = registry.class(class) else {
        return icon::CIRCLE;
    };
    for &ancestor in info.ancestors().iter().rev() {
        let Some(name) = registry.class(ancestor).map(|info| info.name()) else {
            continue;
        };
        let icon = match name {
            "Workspace" => icon::GLOBE,
            "Lighting" | "DirectionalLight" => icon::SUN,
            "ReplicatedStorage" => icon::PACKAGE,
            "ServerScriptService" | "ModuleScript" => icon::CODE,
            "ServerStorage" => icon::HARD_DRIVES,
            "UserGameSettings" | "UserInputService" | "RunService" => icon::SLIDERS,
            "DataModel" => icon::GAME_CONTROLLER,
            "Camera" => icon::VIDEO_CAMERA,
            "MeshPart" => icon::CUBE_TRANSPARENT,
            "BasePart" => icon::CUBE,
            "Model" => icon::SHAPES,
            "SpotLight" => icon::FLASHLIGHT,
            "Light" => icon::LIGHTBULB,
            "Folder" => icon::FOLDER,
            "SourceContainer" => icon::FILE_CODE,
            "ValueBase" => icon::DIAMOND,
            "BindableEvent" | "BindableFunction" => icon::BROADCAST,
            "InputObject" => icon::CURSOR,
            _ => continue,
        };
        return icon;
    }
    icon::CIRCLE
}

/// A search box filling the row, with the magnifier before it. Returns whether the text changed.
pub(crate) fn search_box(ui: &mut Ui, query: &mut String, hint: &str) -> bool {
    let p = current_colors(ui.ctx()).palette;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(fonts::icon::SEARCH.to_string())
                .font(fonts::named(fonts::ICONS, text::BODY))
                .color(p.subtle),
        );
        ui.add(
            TextEdit::singleline(query)
                .hint_text(hint)
                .desired_width(f32::INFINITY)
                .margin(vec2(space::S3, space::S2)),
        )
        .changed()
    })
    .inner
}

/// A quiet line in the middle of an empty panel.
pub(crate) fn empty_note(ui: &mut Ui, line: &str) {
    let p = current_colors(ui.ctx()).palette;
    ui.add_space(space::S6);
    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.add(Label::new(RichText::new(line).size(text::BODY).color(p.muted)).wrap());
    });
}

/// Whether every word of `query` is in `text`, in any letter case. A blank query matches everything.
pub(crate) fn matches_words(text: &str, query: &str) -> bool {
    let text = text.to_lowercase();
    query.split_whitespace().all(|word| text.contains(&word.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_ids_are_unique_and_match_the_constants() {
        let mut seen = std::collections::HashSet::new();
        for panel in PANELS {
            assert!(seen.insert(panel.id()), "{} twice", panel.id());
        }
        for id in [
            ids::VIEWPORT,
            ids::EXPLORER,
            ids::ASSETS,
            ids::PROPERTIES,
            ids::OUTPUT,
            ids::GAME,
        ] {
            assert!(seen.contains(id), "{id}");
        }
    }

    #[test]
    fn classes_get_their_own_icon_or_an_ancestors() {
        let registry = metallic_engine::metallic_classes::shared_registry().unwrap();
        let icon_of = |name: &str| class_icon(&registry, registry.class_id(name).unwrap());
        assert_eq!(icon_of("Part"), icon::CUBE);
        assert_eq!(icon_of("MeshPart"), icon::CUBE_TRANSPARENT);
        assert_eq!(icon_of("SpotLight"), icon::FLASHLIGHT);
        assert_eq!(icon_of("PointLight"), icon::LIGHTBULB);
        assert_eq!(icon_of("Script"), icon::FILE_CODE);
        assert_eq!(icon_of("Workspace"), icon::GLOBE);
        assert_eq!(icon_of("Instance"), icon::CIRCLE);
    }

    #[test]
    fn the_icons_are_in_phosphor() {
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts::definitions());
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for icon in [
                icon::CUBE,
                icon::CUBE_TRANSPARENT,
                icon::SHAPES,
                icon::GLOBE,
                icon::SUN,
                icon::LIGHTBULB,
                icon::FLASHLIGHT,
                icon::VIDEO_CAMERA,
                icon::FILE_CODE,
                icon::CODE,
                icon::FOLDER,
                icon::PACKAGE,
                icon::HARD_DRIVES,
                icon::DIAMOND,
                icon::BROADCAST,
                icon::CURSOR,
                icon::SLIDERS,
                icon::GAME_CONTROLLER,
                icon::CIRCLE,
                icon::CARET_RIGHT,
                icon::CARET_DOWN,
                icon::TREE,
                icon::LIST_BULLETS,
                icon::FILES,
                icon::TERMINAL,
                icon::MONITOR,
                icon::X_CIRCLE,
                icon::INFO,
                icon::CHECK,
                icon::FLOPPY_DISK,
                icon::BROOM,
                icon::FILE,
                icon::IMAGE,
                icon::MUSIC_NOTE,
                icon::PLAY,
                icon::PAUSE,
                icon::STOP,
                icon::STEP,
            ] {
                let has = ui
                    .ctx()
                    .fonts_mut(|f| f.has_glyph(&fonts::named(fonts::ICONS, 16.0), icon));
                assert!(has, "U+{:04X}", icon as u32);
            }
        });
        output.textures_delta.clear();
    }
}
