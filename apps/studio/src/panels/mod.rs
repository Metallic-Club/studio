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

/// Lucide icons (https://lucide.dev/icons, #257) the panels use beyond the theme's own, by their code points in
/// `Lucide.ttf` (`lucide-static` 1.70.0's `font/codepoints.json`). Each names the Lucide icon it is.
pub mod icon {
    /// `box`: a part.
    pub const CUBE: char = '\u{E061}';
    /// `cuboid`: a mesh part.
    pub const CUBE_TRANSPARENT: char = '\u{E524}';
    /// `shapes`: a model.
    pub const SHAPES: char = '\u{E4B3}';
    /// `globe`: the workspace.
    pub const GLOBE: char = '\u{E0E8}';
    /// `sun`: lighting, a directional light.
    pub const SUN: char = '\u{E178}';
    /// `lightbulb`: a light.
    pub const LIGHTBULB: char = '\u{E1C2}';
    /// `flashlight`: a spotlight.
    pub const FLASHLIGHT: char = '\u{E0D3}';
    /// `video`: a camera.
    pub const VIDEO_CAMERA: char = '\u{E1A5}';
    /// `file-code`: a script.
    pub const FILE_CODE: char = '\u{E0C3}';
    /// `braces`: a module script, script services.
    pub const CODE: char = '\u{E36A}';
    /// `folder`.
    pub const FOLDER: char = '\u{E0D7}';
    /// `package`: replicated storage.
    pub const PACKAGE: char = '\u{E129}';
    /// `hard-drive`: server storage.
    pub const HARD_DRIVES: char = '\u{E0ED}';
    /// `diamond`: a value.
    pub const DIAMOND: char = '\u{E2D2}';
    /// `radio`: a bindable event or function.
    pub const BROADCAST: char = '\u{E142}';
    /// `mouse-pointer`: an input object.
    pub const CURSOR: char = '\u{E11F}';
    /// `sliders-horizontal`: settings services.
    pub const SLIDERS: char = '\u{E29A}';
    /// `gamepad-2`: the game.
    pub const GAME_CONTROLLER: char = '\u{E0DF}';
    /// `circle`: any other class.
    pub const CIRCLE: char = '\u{E076}';
    /// `chevron-right`: collapsed.
    pub const CARET_RIGHT: char = '\u{E06F}';
    /// `chevron-down`: expanded.
    pub const CARET_DOWN: char = '\u{E06D}';
    /// `list-tree`: the Explorer.
    pub const TREE: char = '\u{E408}';
    /// `list`: Properties.
    pub const LIST_BULLETS: char = '\u{E106}';
    /// `files`: Assets.
    pub const FILES: char = '\u{E0CF}';
    /// `square-terminal`: Output.
    pub const TERMINAL: char = '\u{E20A}';
    /// `monitor`: the viewport.
    pub const MONITOR: char = '\u{E11D}';
    /// `circle-x`: an error.
    pub const X_CIRCLE: char = '\u{E084}';
    /// `info`: information.
    pub const INFO: char = '\u{E0F9}';
    /// `check`.
    pub const CHECK: char = '\u{E06C}';
    /// `save`.
    pub const FLOPPY_DISK: char = '\u{E14D}';
    /// `eraser`: clear.
    pub const BROOM: char = '\u{E28F}';
    /// `file`.
    pub const FILE: char = '\u{E0C0}';
    /// `image`: an image asset.
    pub const IMAGE: char = '\u{E0F6}';
    /// `music`: an audio asset.
    pub const MUSIC_NOTE: char = '\u{E122}';
    /// `play`.
    pub const PLAY: char = '\u{E13C}';
    /// `pause`.
    pub const PAUSE: char = '\u{E12E}';
    /// `square`: stop.
    pub const STOP: char = '\u{E167}';
    /// `skip-forward`: step one frame.
    pub const STEP: char = '\u{E160}';
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

/// The strip at the top of the Explorer and Properties while they show the running game (live editing, ADR 0010
/// §5): the accent's soft tint, the game controller, and what Stop does.
pub(crate) fn playing_banner(ui: &mut Ui) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    egui::Frame::new()
        .fill(colors.accent.gamma_multiply(0.14))
        .stroke(egui::Stroke::new(
            crate::theme::tokens::size::BORDER,
            colors.accent.gamma_multiply(0.5),
        ))
        .corner_radius(egui::CornerRadius::same(crate::theme::tokens::radius::ITEM))
        .inner_margin(egui::Margin::symmetric(space::S3 as i8, space::S2 as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icon::GAME_CONTROLLER.to_string())
                        .font(fonts::named(fonts::ICONS, text::BODY))
                        .color(colors.accent),
                );
                ui.add(
                    Label::new(
                        RichText::new("Running game")
                            .font(fonts::named(fonts::STRONG, text::SMALL))
                            .color(p.text),
                    )
                    .truncate(),
                )
                .on_hover_text("These are the running game's. Stop puts the scene back as it was before Play.");
            });
        });
    ui.add_space(space::S3);
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
    fn the_icons_are_in_lucide() {
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
