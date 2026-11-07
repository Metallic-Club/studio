//! Studio's icons (#231; ADR 0009, Theme). Every icon Studio draws is an [`Icon`]: a **stable id** (`explorer.part`,
//! `panel.output`, `toolbar.play`) that never changes, a [`Group`] whose colour tells sections apart, and a default
//! glyph from Lucide's icon font. The ids are not Lucide's names, so which Lucide icon an id shows can change.
//!
//! A file named after an id (`explorer.part.svg` or `explorer.part.png`) in the user's custom icon folder replaces the
//! built-in glyph ([`custom`]); [`paint`] draws whichever the icon has. `docs/studio-icons.md` lists every id
//! ([`docs_markdown`]; a test keeps it current).

pub mod custom;

use egui::{Align2, Color32, Painter, Pos2, Rect, Response, Sense, Ui, Vec2, pos2};

use super::tokens::Hue;
use super::{Colors, fonts};

/// A group of icons drawn in one colour, so the sections of the interface are easy to tell apart. Each group has one
/// [`Hue`] (the accent family's, or neutral) from the theme's tokens, tuned for Dark, Light and high contrast.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    /// Explorer: the game and its services (the workspace, lighting, storage, script and settings services).
    Service,
    /// Explorer and Assets: parts, models and scenes (the engine's Parts section).
    World,
    /// Explorer: lights (the engine's Lights section).
    Light,
    /// Explorer and Assets: scripting (the engine's Scripts section, and bindable events and functions).
    Script,
    /// Explorer: cameras (the engine's World section).
    Camera,
    /// Explorer: values (the engine's Values section).
    Value,
    /// Explorer: folders.
    Folder,
    /// Explorer and Assets: everything else (input objects, any other file).
    Other,
    /// Panel tabs: the scene's panels (Viewport, Explorer, Properties).
    Scene,
    /// Panels: the project's content (Assets, images, sounds).
    Content,
    /// Panels: the log (Output).
    Log,
    /// Running the game: the Game tab and the play controls.
    Run,
    /// Studio itself (Settings).
    Studio,
    /// The interface's own marks (search, close, chevrons, checks, warnings): drawn in the colour their place gives
    /// them, not a group colour.
    Interface,
}

impl Group {
    /// Every group, in the order the docs list them.
    pub const ALL: [Group; 14] = [
        Group::Service,
        Group::World,
        Group::Light,
        Group::Script,
        Group::Camera,
        Group::Value,
        Group::Folder,
        Group::Other,
        Group::Scene,
        Group::Content,
        Group::Log,
        Group::Run,
        Group::Studio,
        Group::Interface,
    ];

    /// Its hue.
    pub const fn hue(self) -> Hue {
        match self {
            Group::Service | Group::Scene => Hue::Blue,
            Group::World => Hue::Teal,
            Group::Camera | Group::Content => Hue::Pink,
            Group::Light => Hue::Orange,
            Group::Script | Group::Run => Hue::Green,
            Group::Value | Group::Log => Hue::Violet,
            Group::Folder => Hue::Yellow,
            Group::Other | Group::Studio | Group::Interface => Hue::Neutral,
        }
    }

    /// Its colour in `colors`. Selection and the accent still win where they mark something: callers draw a selected
    /// item's icon in the accent.
    pub fn color(self, colors: &Colors) -> Color32 {
        colors.hue(self.hue())
    }

    /// Where it is used and what it holds, as the docs name it.
    pub const fn label(self) -> &'static str {
        match self {
            Group::Service => "Explorer: services",
            Group::World => "Explorer and Assets: parts, models and scenes",
            Group::Light => "Explorer: lights",
            Group::Script => "Explorer and Assets: scripting",
            Group::Camera => "Explorer: cameras",
            Group::Value => "Explorer: values",
            Group::Folder => "Explorer: folders",
            Group::Other => "Explorer and Assets: other",
            Group::Scene => "Panels: scene",
            Group::Content => "Panels and Assets: content",
            Group::Log => "Panels: log",
            Group::Run => "Running the game",
            Group::Studio => "Studio",
            Group::Interface => "Interface",
        }
    }
}

/// Declares [`Icon`] and its table: each icon's variant, stable id, group, Lucide name and code point in
/// `Lucide.ttf` (`lucide-static` 1.70.0's `font/codepoints.json`), in one place.
macro_rules! icons {
    ($($(#[doc = $doc:literal])* $name:ident = $id:literal, $group:ident, $lucide:literal, $code:literal;)*) => {
        /// Every icon Studio draws, by its stable id.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Icon {
            $($(#[doc = $doc])* $name,)*
        }

        impl Icon {
            /// Every icon, in the order the docs list them.
            pub const ALL: &'static [Icon] = &[$(Icon::$name,)*];

            /// Its stable id: what its file in a custom icon folder is called. Never renamed once shipped.
            pub const fn id(self) -> &'static str {
                match self {
                    $(Icon::$name => $id,)*
                }
            }

            /// Its group, whose colour it is drawn in.
            pub const fn group(self) -> Group {
                match self {
                    $(Icon::$name => Group::$group,)*
                }
            }

            /// The Lucide icon it shows by default, by its name on lucide.dev.
            pub const fn lucide(self) -> &'static str {
                match self {
                    $(Icon::$name => $lucide,)*
                }
            }

            /// Its built-in glyph: the Lucide icon's code point in Studio's icon font.
            pub const fn glyph(self) -> char {
                match self {
                    $(Icon::$name => $code,)*
                }
            }
        }
    };
}

icons! {
    /// The game (the data model).
    ExplorerGame = "explorer.game", Service, "gamepad-2", '\u{E0DF}';
    /// The workspace.
    ExplorerWorkspace = "explorer.workspace", Service, "globe", '\u{E0E8}';
    /// The lighting service.
    ExplorerLighting = "explorer.lighting", Service, "sun", '\u{E178}';
    /// Replicated storage.
    ExplorerReplicatedStorage = "explorer.replicated_storage", Service, "package", '\u{E129}';
    /// The server script service.
    ExplorerScriptService = "explorer.script_service", Service, "braces", '\u{E36A}';
    /// Server storage.
    ExplorerServerStorage = "explorer.server_storage", Service, "hard-drive", '\u{E0ED}';
    /// A settings or input service (user settings, input, run service).
    ExplorerSettingsService = "explorer.settings_service", Service, "sliders-horizontal", '\u{E29A}';
    /// A camera.
    ExplorerCamera = "explorer.camera", Camera, "video", '\u{E1A5}';
    /// A part.
    ExplorerPart = "explorer.part", World, "box", '\u{E061}';
    /// A mesh part.
    ExplorerMeshPart = "explorer.mesh_part", World, "cuboid", '\u{E524}';
    /// A model.
    ExplorerModel = "explorer.model", World, "shapes", '\u{E4B3}';
    /// A directional light.
    ExplorerDirectionalLight = "explorer.directional_light", Light, "sun", '\u{E178}';
    /// A spotlight.
    ExplorerSpotLight = "explorer.spot_light", Light, "flashlight", '\u{E0D3}';
    /// Any other light.
    ExplorerLight = "explorer.light", Light, "lightbulb", '\u{E1C2}';
    /// A script.
    ExplorerScript = "explorer.script", Script, "file-code", '\u{E0C3}';
    /// A local script, which runs on the player's machine.
    ExplorerLocalScript = "explorer.local_script", Script, "file-user", '\u{E62D}';
    /// A module script.
    ExplorerModuleScript = "explorer.module_script", Script, "braces", '\u{E36A}';
    /// A bindable event.
    ExplorerEvent = "explorer.event", Script, "radio", '\u{E142}';
    /// A bindable function.
    ExplorerFunction = "explorer.function", Script, "square-function", '\u{E22D}';
    /// A folder.
    ExplorerFolder = "explorer.folder", Folder, "folder", '\u{E0D7}';
    /// A value object.
    ExplorerValue = "explorer.value", Value, "diamond", '\u{E2D2}';
    /// A true-or-false value.
    ExplorerBoolValue = "explorer.bool_value", Value, "toggle-left", '\u{E18B}';
    /// A whole-number value.
    ExplorerIntValue = "explorer.int_value", Value, "hash", '\u{E0EF}';
    /// A number value.
    ExplorerNumberValue = "explorer.number_value", Value, "binary", '\u{E1F2}';
    /// A text value.
    ExplorerStringValue = "explorer.string_value", Value, "type", '\u{E198}';
    /// A value that refers to another object.
    ExplorerObjectValue = "explorer.object_value", Value, "link", '\u{E102}';
    /// An input object.
    ExplorerInput = "explorer.input", Other, "mouse-pointer", '\u{E11F}';
    /// Any other instance.
    ExplorerInstance = "explorer.instance", Other, "circle", '\u{E076}';
    /// The Viewport panel.
    PanelViewport = "panel.viewport", Scene, "monitor", '\u{E11D}';
    /// The Explorer panel.
    PanelExplorer = "panel.explorer", Scene, "list-tree", '\u{E408}';
    /// The Properties panel.
    PanelProperties = "panel.properties", Scene, "list", '\u{E106}';
    /// The Assets panel.
    PanelAssets = "panel.assets", Content, "files", '\u{E0CF}';
    /// The Output panel.
    PanelOutput = "panel.output", Log, "square-terminal", '\u{E20A}';
    /// The Game tab.
    PanelGame = "panel.game", Run, "gamepad-2", '\u{E0DF}';
    /// A mesh in the Assets panel.
    AssetMesh = "asset.mesh", World, "cuboid", '\u{E524}';
    /// An image in the Assets panel.
    AssetImage = "asset.image", Content, "image", '\u{E0F6}';
    /// A sound in the Assets panel.
    AssetAudio = "asset.audio", Content, "music", '\u{E122}';
    /// A scene in the Assets panel.
    AssetScene = "asset.scene", World, "list-tree", '\u{E408}';
    /// A script in the Assets panel.
    AssetScript = "asset.script", Script, "file-code", '\u{E0C3}';
    /// Any other file in the Assets panel.
    AssetFile = "asset.file", Other, "file", '\u{E0C0}';
    /// Play, and Resume while paused.
    ToolbarPlay = "toolbar.play", Run, "play", '\u{E13C}';
    /// Pause.
    ToolbarPause = "toolbar.pause", Run, "pause", '\u{E12E}';
    /// Step one frame.
    ToolbarStep = "toolbar.step", Run, "skip-forward", '\u{E160}';
    /// Stop.
    ToolbarStop = "toolbar.stop", Run, "square", '\u{E167}';
    /// The gear that opens Settings.
    ToolbarSettings = "toolbar.settings", Studio, "settings", '\u{E154}';
    /// A search box.
    UiSearch = "ui.search", Interface, "search", '\u{E151}';
    /// Close, dismiss.
    UiClose = "ui.close", Interface, "x", '\u{E1B2}';
    /// A warning.
    UiWarning = "ui.warning", Interface, "triangle-alert", '\u{E193}';
    /// An error.
    UiError = "ui.error", Interface, "circle-x", '\u{E084}';
    /// Information.
    UiInfo = "ui.info", Interface, "info", '\u{E0F9}';
    /// A check mark.
    UiCheck = "ui.check", Interface, "check", '\u{E06C}';
    /// A collapsed tree item or section.
    UiExpand = "ui.expand", Interface, "chevron-right", '\u{E06F}';
    /// An expanded tree item or section.
    UiCollapse = "ui.collapse", Interface, "chevron-down", '\u{E06D}';
    /// A colour of your own.
    UiPalette = "ui.palette", Interface, "palette", '\u{E1DD}';
    /// Add another.
    UiAdd = "ui.add", Interface, "plus", '\u{E13D}';
}

/// The file types a custom icon may be, in the order one is preferred when both are there.
pub const FILE_TYPES: [&str; 2] = ["svg", "png"];

impl Icon {
    /// The icon with the stable id `id`, if there is one.
    pub fn from_id(id: &str) -> Option<Icon> {
        Icon::ALL.iter().copied().find(|icon| icon.id() == id)
    }

    /// Its group's colour in `colors`.
    pub fn color(self, colors: &Colors) -> Color32 {
        self.group().color(colors)
    }

    /// The names of the files that replace it in a custom icon folder: `<id>.svg` and `<id>.png`.
    pub fn file_names(self) -> [String; 2] {
        FILE_TYPES.map(|kind| format!("{}.{kind}", self.id()))
    }
}

/// Paints `icon` filling the square `rect` in `color`: the custom folder's image if it has one (tinted with `color`
/// only when it is monochrome), the built-in glyph otherwise.
pub fn paint(painter: &Painter, icon: Icon, rect: Rect, color: Color32) {
    match custom::get(painter.ctx(), icon) {
        Some(image) => {
            let tint = if image.monochrome {
                color
            } else {
                // A full-colour image keeps its colours, and only the colour's opacity (a disabled control's fade).
                Color32::from_white_alpha(color.a())
            };
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(image.texture.id(), rect, uv, tint);
        }
        None => {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                icon.glyph(),
                fonts::named(fonts::ICONS, rect.height()),
                color,
            );
        }
    }
}

/// Paints `icon` `size` points square, placed at `pos` by `align`. Returns where it went.
pub fn paint_at(painter: &Painter, pos: Pos2, align: Align2, icon: Icon, size: f32, color: Color32) -> Rect {
    let rect = align.anchor_size(pos, Vec2::splat(size));
    paint(painter, icon, rect, color);
    rect
}

/// `icon` as a widget in `ui`'s layout, `size` points square.
pub fn show(ui: &mut Ui, icon: Icon, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint(ui.painter(), icon, rect, color);
    }
    response
}

/// `docs/studio-icons.md`: every icon id, its group and colour, its default Lucide icon, and the files that replace it.
pub fn docs_markdown() -> String {
    let mut out = String::from(
        "# Studio icons\n\n\
         <!-- Generated from apps/studio/src/theme/icons/mod.rs by \
         `cargo test -p metallic_studio write_the_icon_docs -- --ignored`. Do not edit by hand. -->\n\n\
         Every icon in Metallic Studio has a stable id. By default it is drawn from [Lucide](https://lucide.dev/icons/)'s \
         icon font (ISC licence), in its group's colour.\n\n\
         **Custom icons:** set a folder in Settings → Appearance → Icon folder. A file in it named after an icon's id, \
         `<icon-id>.svg` or `<icon-id>.png` (the SVG wins when both are there), replaces that icon. Icons without a \
         file keep the built-in one, and a file that cannot be read keeps it too, with one warning in Output. \
         Studio reads the folder in the background when it is set and when you press **Reload**. A monochrome image \
         (one colour on transparency, such as Lucide's own SVGs) is drawn in its group's colour; a coloured one is \
         drawn as it is.\n\n\
         ## Groups\n\n\
         | Group | Colour |\n|---|---|\n",
    );
    for group in Group::ALL {
        let hue = match group {
            Group::Interface => "its place's (neutral by default)".to_owned(),
            _ => format!("{:?}", group.hue()).to_lowercase(),
        };
        out.push_str(&format!("| {} | {hue} |\n", group.label()));
    }
    out.push_str("\n## Icons\n\n| Icon id | Group | Lucide icon | Custom files |\n|---|---|---|---|\n");
    for icon in Icon::ALL {
        let [svg, png] = icon.file_names();
        let name = icon.lucide();
        out.push_str(&format!(
            "| `{}` | {} | [`{name}`](https://lucide.dev/icons/{name}) | `{svg}`, `{png}` |\n",
            icon.id(),
            icon.group().label(),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::Path;

    use super::*;
    use crate::theme::Look;

    #[test]
    fn ids_are_unique_and_well_formed() {
        let mut seen = HashSet::new();
        for icon in Icon::ALL {
            let id = icon.id();
            assert!(seen.insert(id), "{id} twice");
            assert_eq!(Icon::from_id(id), Some(*icon));
            // `<area>.<name>`, lower case: safe as a file name on every system.
            let (area, name) = id.split_once('.').unwrap();
            assert!(["explorer", "panel", "asset", "toolbar", "ui"].contains(&area), "{id}");
            assert!(
                !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{id}"
            );
        }
        assert_eq!(Icon::from_id("explorer.teleporter"), None);
        assert_eq!(
            Icon::ExplorerPart.file_names(),
            ["explorer.part.svg".to_owned(), "explorer.part.png".to_owned()]
        );
    }

    #[test]
    fn every_icon_is_in_lucide() {
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts::definitions());
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for icon in Icon::ALL {
                let has = ui
                    .ctx()
                    .fonts_mut(|f| f.has_glyph(&fonts::named(fonts::ICONS, 16.0), icon.glyph()));
                assert!(has, "{} ({}, U+{:04X})", icon.id(), icon.lucide(), icon.glyph() as u32);
                // The private use area: never a letter of the text fonts.
                assert!(('\u{E000}'..='\u{F8FF}').contains(&icon.glyph()), "{}", icon.id());
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn groups_take_their_hues_and_selection_is_left_to_the_caller() {
        let colors = Look::default().colors(None);
        assert_eq!(Icon::ExplorerPart.color(&colors), colors.hue(Hue::Teal));
        assert_eq!(Icon::ExplorerSpotLight.color(&colors), colors.hue(Hue::Orange));
        assert_eq!(Icon::ExplorerScript.color(&colors), colors.hue(Hue::Green));
        assert_eq!(
            Icon::ExplorerEvent.color(&colors),
            colors.hue(Hue::Green),
            "scripting is one section"
        );
        assert_eq!(Icon::ExplorerCamera.color(&colors), colors.hue(Hue::Pink));
        assert_eq!(Icon::ExplorerValue.color(&colors), colors.hue(Hue::Violet));
        assert_eq!(Icon::ExplorerFolder.color(&colors), colors.hue(Hue::Yellow));
        assert_eq!(Icon::ExplorerReplicatedStorage.color(&colors), colors.hue(Hue::Blue));
        assert_eq!(
            Icon::ExplorerLighting.color(&colors),
            colors.hue(Hue::Blue),
            "a service"
        );
        assert_eq!(Icon::ExplorerInput.color(&colors), colors.palette.subtle);
        // The Explorer's eight sections have eight different colours (#277).
        let explorer: HashSet<Color32> = [
            Group::Service,
            Group::World,
            Group::Light,
            Group::Script,
            Group::Camera,
            Group::Value,
            Group::Folder,
            Group::Other,
        ]
        .iter()
        .map(|group| group.color(&colors))
        .collect();
        assert_eq!(explorer.len(), 8);
        // Every group is used by an icon.
        for group in Group::ALL {
            assert!(Icon::ALL.iter().any(|icon| icon.group() == group), "{group:?}");
        }
    }

    #[test]
    fn the_icon_docs_are_current() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/studio-icons.md");
        let written = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
        assert!(
            written == docs_markdown(),
            "docs/studio-icons.md is stale: run `cargo test -p metallic_studio write_the_icon_docs -- --ignored`"
        );
    }

    /// Writes `docs/studio-icons.md` from the table.
    #[test]
    #[ignore = "writes docs/studio-icons.md"]
    fn write_the_icon_docs() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/studio-icons.md");
        std::fs::write(path, docs_markdown()).unwrap();
    }
}
