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
use crate::theme::icons::{self, Icon};
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

/// The icon of `class`: its own, or its nearest ancestor's.
pub fn class_icon(registry: &Registry, class: ClassId) -> Icon {
    let Some(info) = registry.class(class) else {
        return Icon::ExplorerInstance;
    };
    for &ancestor in info.ancestors().iter().rev() {
        let Some(name) = registry.class(ancestor).map(|info| info.name()) else {
            continue;
        };
        let icon = match name {
            "Workspace" => Icon::ExplorerWorkspace,
            "Lighting" => Icon::ExplorerLighting,
            "DirectionalLight" => Icon::ExplorerDirectionalLight,
            "ReplicatedStorage" => Icon::ExplorerReplicatedStorage,
            "ServerScriptService" => Icon::ExplorerScriptService,
            "ModuleScript" => Icon::ExplorerModuleScript,
            "ServerStorage" => Icon::ExplorerServerStorage,
            "UserGameSettings" | "UserInputService" | "RunService" => Icon::ExplorerSettingsService,
            "DataModel" => Icon::ExplorerGame,
            "Camera" => Icon::ExplorerCamera,
            "MeshPart" => Icon::ExplorerMeshPart,
            "BasePart" => Icon::ExplorerPart,
            "Model" => Icon::ExplorerModel,
            "SpotLight" => Icon::ExplorerSpotLight,
            "Light" => Icon::ExplorerLight,
            "Folder" => Icon::ExplorerFolder,
            "SourceContainer" => Icon::ExplorerScript,
            "ValueBase" => Icon::ExplorerValue,
            "BindableEvent" | "BindableFunction" => Icon::ExplorerEvent,
            "InputObject" => Icon::ExplorerInput,
            _ => continue,
        };
        return icon;
    }
    Icon::ExplorerInstance
}

/// A search box filling the row, with the magnifier before it. Returns whether the text changed.
pub(crate) fn search_box(ui: &mut Ui, query: &mut String, hint: &str) -> bool {
    let p = current_colors(ui.ctx()).palette;
    ui.horizontal(|ui| {
        icons::show(ui, Icon::UiSearch, text::BODY, p.subtle);
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
                icons::show(ui, Icon::PanelGame, text::BODY, colors.accent);
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
        assert_eq!(icon_of("Part"), Icon::ExplorerPart);
        assert_eq!(icon_of("MeshPart"), Icon::ExplorerMeshPart);
        assert_eq!(icon_of("SpotLight"), Icon::ExplorerSpotLight);
        assert_eq!(icon_of("PointLight"), Icon::ExplorerLight);
        assert_eq!(icon_of("Script"), Icon::ExplorerScript);
        assert_eq!(icon_of("ModuleScript"), Icon::ExplorerModuleScript);
        assert_eq!(icon_of("Workspace"), Icon::ExplorerWorkspace);
        assert_eq!(icon_of("Instance"), Icon::ExplorerInstance);
    }
}
