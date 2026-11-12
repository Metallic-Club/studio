//! The editor's panels (ADR 0009, Docking): each a [`Panel`] in its own module, all listed in [`PANELS`].
//!
//! Adding a panel is one unit type implementing [`Panel`] and one line in [`PANELS`]; its state, if any, is a field
//! of [`crate::editor::EditorState`].

pub mod assets;
pub mod color_picker;
pub mod explorer;
pub mod game;
pub mod insert;
pub mod output;
pub mod properties;
pub mod script;
pub mod search;
pub mod values;
pub mod viewport;

use egui::{Align, Label, Layout, RichText, TextEdit, Ui, vec2};
use metallic_engine::metallic_core::{ClassId, Registry};
use metallic_engine::metallic_render::ViewportTarget;

use crate::dock::Panel;
use crate::theme::current_colors;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{space, text};

/// Every panel, in the order the View menu lists them and the default layout's tabs take.
pub static PANELS: &[&dyn Panel] = &[
    &viewport::Viewport,
    &explorer::Explorer,
    &properties::Properties,
    // Output before Assets: the first tab at the bottom (#323).
    &output::Output,
    &assets::Assets,
    &game::Game,
    // A tab per script, opened from the Explorer (#528).
    &script::ScriptEditor,
    // Text across every script (#555), on the left.
    &search::Search,
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
    /// The search across every script (#555).
    pub const SEARCH: &str = "search";
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
            "LocalScript" => Icon::ExplorerLocalScript,
            "SourceContainer" => Icon::ExplorerScript,
            "BoolValue" => Icon::ExplorerBoolValue,
            "IntValue" => Icon::ExplorerIntValue,
            "NumberValue" => Icon::ExplorerNumberValue,
            "StringValue" => Icon::ExplorerStringValue,
            "ObjectValue" => Icon::ExplorerObjectValue,
            "ValueBase" => Icon::ExplorerValue,
            "BindableEvent" => Icon::ExplorerEvent,
            "BindableFunction" => Icon::ExplorerFunction,
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

/// Draws a rendered frame (the Viewport's or the Game tab's) `texture` filling `rect` (#330). The engine draws each
/// frame at the panel's size into the top-left corner of a target kept larger than the panel, `texture_pixels` in size
/// as last drawn; the size it has this frame is worked out here as the engine does
/// ([`ViewportTarget::room_for`], up to `limit`), so the corner shown is exactly the one drawn now. A resize is followed
/// in the same frame: never stretched, and never an old frame.
pub(crate) fn draw_render(
    ui: &Ui,
    texture: egui::TextureId,
    texture_pixels: Option<(u32, u32)>,
    limit: u32,
    rect: egui::Rect,
) -> egui::Rect {
    let ppp = ui.ctx().pixels_per_point();
    let wanted = (rect.size() * ppp).round();
    let wanted = (wanted.x.max(1.0) as u32, wanted.y.max(1.0) as u32);
    let corner = match texture_pixels {
        Some(current) => {
            let room = ViewportTarget::room_for(wanted, current, limit);
            let drawn = (wanted.0.min(room.0), wanted.1.min(room.1));
            egui::pos2(drawn.0 as f32 / room.0 as f32, drawn.1 as f32 / room.1 as f32)
        }
        None => egui::pos2(1.0, 1.0),
    };
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), corner);
    ui.painter()
        .with_clip_rect(rect)
        .image(texture, rect, uv, egui::Color32::WHITE);
    rect
}

/// Where a list row's background goes (#301): the row `rect` stretched past the panel's inset to its sides, and a
/// painter clipped to that width, so stripes and the selected fill leave no gap at the edges.
pub(crate) fn row_band(ui: &Ui, rect: egui::Rect) -> (egui::Painter, egui::Rect) {
    let margin = crate::dock::PANE_MARGIN;
    let clip = ui.clip_rect();
    let wider = egui::Rect::from_x_y_ranges((clip.left() - margin)..=(clip.right() + margin), clip.y_range());
    // A painter on the same layer, so the clip can widen past the panel's inset: a child painter only narrows it.
    let painter = ui.ctx().layer_painter(ui.layer_id()).with_clip_rect(wider);
    (painter, rect.expand2(vec2(margin, 0.0)))
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
    fn classes_take_the_owner_s_colour_groups() {
        use crate::theme::icons::Group;

        let registry = metallic_engine::metallic_classes::shared_registry().unwrap();
        let group = |name: &str| class_icon(&registry, registry.class_id(name).unwrap()).group();
        // #290.
        for (class, expected) in [
            ("Workspace", Group::Workspace),
            ("Script", Group::ServerScript),
            ("ServerScriptService", Group::ServerScript),
            ("LocalScript", Group::LocalScript),
            ("ModuleScript", Group::ModuleScript),
            ("Lighting", Group::Light),
            ("PointLight", Group::Light),
            ("SpotLight", Group::Light),
            ("ReplicatedStorage", Group::Storage),
            // As the server script service (#321).
            ("ServerStorage", Group::ServerScript),
            ("UserInputService", Group::SettingsService),
            ("Camera", Group::Object),
            ("Folder", Group::Object),
            ("Model", Group::Object),
            ("Part", Group::Object),
            ("MeshPart", Group::Object),
            ("IntValue", Group::Value),
            ("StringValue", Group::Value),
            ("BindableEvent", Group::Event),
        ] {
            assert_eq!(group(class), expected, "{class}");
        }
    }

    #[test]
    fn classes_show_the_lucide_icons_the_owner_chose() {
        let registry = metallic_engine::metallic_classes::shared_registry().unwrap();
        let lucide = |name: &str| class_icon(&registry, registry.class_id(name).unwrap()).lucide();
        // #289.
        assert_eq!(lucide("Camera"), "camera");
        assert_eq!(lucide("SpotLight"), "spotlight");
        assert_eq!(lucide("Workspace"), "earth");
        assert_eq!(lucide("ModuleScript"), "scroll-text");
        for scripted in ["Script", "LocalScript", "ServerScriptService"] {
            assert_eq!(lucide(scripted), "scroll", "{scripted}");
        }
    }

    #[test]
    fn every_creatable_class_has_its_own_icon() {
        let registry = metallic_engine::metallic_classes::shared_registry().unwrap();
        for info in registry
            .classes()
            .iter()
            .filter(|info| info.creatable() || info.service_order().is_some())
        {
            let class = registry.class_id(info.name()).unwrap();
            assert_ne!(
                class_icon(&registry, class),
                Icon::ExplorerInstance,
                "{} has no icon",
                info.name()
            );
        }
        // Look-alikes have their own (#278).
        let icon_of = |name: &str| class_icon(&registry, registry.class_id(name).unwrap());
        assert_ne!(icon_of("BindableFunction"), icon_of("BindableEvent"));
        let values: std::collections::HashSet<Icon> =
            ["BoolValue", "IntValue", "NumberValue", "StringValue", "ObjectValue"]
                .into_iter()
                .map(icon_of)
                .collect();
        assert_eq!(values.len(), 5);
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
