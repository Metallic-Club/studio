//! Studio's icons (#231; ADR 0009, Theme). Every icon Studio draws is an [`Icon`]: a **stable id** (`explorer.part`,
//! `panel.output`, `toolbar.play`) that never changes, a [`Group`] whose colour tells sections apart, and a default
//! glyph from Lucide's icon font. The ids are not Lucide's names, so which Lucide icon an id shows can change.
//!
//! A file named after an id (`explorer.part.svg` or `explorer.part.png`) in the user's custom icon folder replaces the
//! built-in glyph ([`custom`]); [`paint`] draws whichever the icon has. `docs/studio-icons.md` lists every id
//! ([`docs_markdown`]; a test keeps it current).

pub mod builtin;
pub mod custom;

use egui::{Align2, Color32, Painter, Pos2, Rect, Response, Sense, Ui, Vec2, pos2};

use super::Colors;
use super::tokens::Hue;

/// A group of icons drawn in one colour, so the parts of the interface are easy to tell apart (#290: the owner's
/// groups). Each group has one [`Hue`] from the theme's tokens, tuned for Dark, Light and high contrast.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    /// Explorer: the workspace.
    Workspace,
    /// Explorer: replicated storage.
    Storage,
    /// Explorer: the input and settings services (user input, user settings, run service).
    SettingsService,
    /// Explorer and Assets: server scripts, the server script service and server storage (#321).
    ServerScript,
    /// Explorer: local scripts, which run on the player's machine.
    LocalScript,
    /// Explorer: module scripts.
    ModuleScript,
    /// Explorer: bindable events and functions.
    Event,
    /// Explorer: lighting and lights.
    Light,
    /// Explorer and Assets: the game's objects: parts, models, folders, cameras, meshes and scenes.
    Object,
    /// Explorer: values (Int, String, ...).
    Value,
    /// Explorer and Assets: everything else (input objects, any other file).
    Other,
    /// Panel tabs: the scene's panels (Viewport, Explorer, Properties).
    Scene,
    /// Panels and Assets: the project's content (Assets, images, sounds).
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
    pub const ALL: [Group; 17] = [
        Group::Workspace,
        Group::Storage,
        Group::SettingsService,
        Group::ServerScript,
        Group::LocalScript,
        Group::ModuleScript,
        Group::Event,
        Group::Light,
        Group::Object,
        Group::Value,
        Group::Other,
        Group::Scene,
        Group::Content,
        Group::Log,
        Group::Run,
        Group::Studio,
        Group::Interface,
    ];

    /// Its id in `icons.json`'s colours: its name in snake case. Never renamed once shipped.
    pub const fn id(self) -> &'static str {
        match self {
            Group::Workspace => "workspace",
            Group::Storage => "storage",
            Group::SettingsService => "settings_service",
            Group::ServerScript => "server_script",
            Group::LocalScript => "local_script",
            Group::ModuleScript => "module_script",
            Group::Event => "event",
            Group::Light => "light",
            Group::Object => "object",
            Group::Value => "value",
            Group::Other => "other",
            Group::Scene => "scene",
            Group::Content => "content",
            Group::Log => "log",
            Group::Run => "run",
            Group::Studio => "studio",
            Group::Interface => "interface",
        }
    }

    /// The group with the id `id`, if there is one.
    pub fn from_id(id: &str) -> Option<Group> {
        Group::ALL.iter().copied().find(|group| group.id() == id)
    }

    /// Whether it has a colour of its own: every group but [`Group::Interface`], whose icons take the colour of the
    /// text they sit by.
    pub const fn is_coloured(self) -> bool {
        !matches!(self, Group::Interface)
    }

    /// Its hue.
    pub const fn hue(self) -> Hue {
        match self {
            Group::Workspace | Group::ServerScript | Group::Run => Hue::Green,
            Group::LocalScript | Group::Scene => Hue::Blue,
            Group::ModuleScript | Group::Log => Hue::Purple,
            Group::Event | Group::Content => Hue::Pink,
            Group::Light => Hue::Yellow,
            Group::Storage => Hue::Orange,
            Group::SettingsService => Hue::Cyan,
            Group::Object => Hue::White,
            Group::Value | Group::Other | Group::Studio | Group::Interface => Hue::Grey,
        }
    }

    /// Its colour in `colors`. Selection and the accent still win where they mark something: callers draw a selected
    /// item's icon in the accent.
    pub fn color(self, colors: &Colors) -> Color32 {
        match builtin::group_colours(self) {
            Some(pair) => super::tokens::Hue::color_of(pair, colors.dark, colors.high_contrast),
            None => colors.hue(self.hue()),
        }
    }

    /// Its colours on the dark and the light mode as Studio ships them: its hue's (the palette's subtle grey for
    /// [`Hue::Grey`]).
    pub const fn default_colours(self) -> (Color32, Color32) {
        self.hue().pair_or_subtle()
    }

    /// Where it is used and what it holds, as the docs name it.
    pub const fn label(self) -> &'static str {
        match self {
            Group::Workspace => "Explorer: the workspace",
            Group::Storage => "Explorer: replicated storage",
            Group::SettingsService => "Explorer: input and settings services",
            Group::ServerScript => "Explorer and Assets: server scripts and server storage",
            Group::LocalScript => "Explorer: local scripts",
            Group::ModuleScript => "Explorer: module scripts",
            Group::Event => "Explorer: bindable events and functions",
            Group::Light => "Explorer: lighting and lights",
            Group::Object => "Explorer and Assets: parts, models, folders, cameras",
            Group::Value => "Explorer: values",
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

/// Declares [`Icon`] and its table: each icon's variant, stable id, group and default Lucide icon (by its name in
/// `lucide-static` 1.70.0, whose SVG ships in the icons folder), in one place.
macro_rules! icons {
    ($($(#[doc = $doc:literal])* $name:ident = $id:literal, $group:ident, $lucide:literal;)*) => {
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

            /// The SVG file it uses by default in the icons folder: its Lucide icon's, `<name>.svg` (#291), in the
            /// folder of the part of Studio that draws it (#322).
            pub fn default_file(self) -> String {
                format!("{}/{}.svg", self.folder(), self.lucide())
            }

            /// What it marks, from its doc comment: `icons.json` says so beside each entry.
            pub fn description(self) -> &'static str {
                match self {
                    $(Icon::$name => concat!($($doc),*).trim(),)*
                }
            }
        }
    };
}

icons! {
    /// The game (the data model).
    ExplorerGame = "explorer.game", Object, "gamepad-2";
    /// The workspace.
    ExplorerWorkspace = "explorer.workspace", Workspace, "earth";
    /// The lighting service.
    ExplorerLighting = "explorer.lighting", Light, "sun";
    /// Replicated storage.
    ExplorerReplicatedStorage = "explorer.replicated_storage", Storage, "package";
    /// The server script service.
    ExplorerScriptService = "explorer.script_service", ServerScript, "scroll";
    /// Server storage.
    ExplorerServerStorage = "explorer.server_storage", ServerScript, "hard-drive";
    /// A settings or input service (user settings, input, run service).
    ExplorerSettingsService = "explorer.settings_service", SettingsService, "sliders-horizontal";
    /// A camera.
    ExplorerCamera = "explorer.camera", Object, "camera";
    /// A part.
    ExplorerPart = "explorer.part", Object, "box";
    /// A mesh part.
    ExplorerMeshPart = "explorer.mesh_part", Object, "cuboid";
    /// A model.
    ExplorerModel = "explorer.model", Object, "shapes";
    /// A directional light.
    ExplorerDirectionalLight = "explorer.directional_light", Light, "sun";
    /// A spotlight.
    ExplorerSpotLight = "explorer.spot_light", Light, "spotlight";
    /// Any other light.
    ExplorerLight = "explorer.light", Light, "lightbulb";
    /// A script.
    ExplorerScript = "explorer.script", ServerScript, "scroll";
    /// A local script, which runs on the player's machine: a script's scroll (#289), in its own colour.
    ExplorerLocalScript = "explorer.local_script", LocalScript, "scroll";
    /// A module script.
    ExplorerModuleScript = "explorer.module_script", ModuleScript, "scroll-text";
    /// A bindable event.
    ExplorerEvent = "explorer.event", Event, "radio";
    /// A bindable function.
    ExplorerFunction = "explorer.function", Event, "square-function";
    /// A folder.
    ExplorerFolder = "explorer.folder", Object, "folder";
    /// A value object.
    ExplorerValue = "explorer.value", Value, "diamond";
    /// A true-or-false value.
    ExplorerBoolValue = "explorer.bool_value", Value, "toggle-left";
    /// A whole-number value.
    ExplorerIntValue = "explorer.int_value", Value, "hash";
    /// A number value.
    ExplorerNumberValue = "explorer.number_value", Value, "binary";
    /// A text value.
    ExplorerStringValue = "explorer.string_value", Value, "type";
    /// A value that refers to another object.
    ExplorerObjectValue = "explorer.object_value", Value, "link";
    /// An input object.
    ExplorerInput = "explorer.input", Other, "mouse-pointer";
    /// Any other instance.
    ExplorerInstance = "explorer.instance", Other, "circle";
    /// The Viewport panel.
    PanelViewport = "panel.viewport", Scene, "monitor";
    /// The Explorer panel.
    PanelExplorer = "panel.explorer", Scene, "list-tree";
    /// The Properties panel.
    PanelProperties = "panel.properties", Scene, "list";
    /// The Assets panel.
    PanelAssets = "panel.assets", Content, "files";
    /// The Output panel.
    PanelOutput = "panel.output", Log, "square-terminal";
    /// The Game tab.
    PanelGame = "panel.game", Run, "gamepad-2";
    /// A mesh in the Assets panel.
    AssetMesh = "asset.mesh", Object, "cuboid";
    /// An image in the Assets panel.
    AssetImage = "asset.image", Content, "image";
    /// A sound in the Assets panel.
    AssetAudio = "asset.audio", Content, "music";
    /// A scene in the Assets panel.
    AssetScene = "asset.scene", Object, "list-tree";
    /// A script in the Assets panel.
    AssetScript = "asset.script", ServerScript, "scroll";
    /// Any other file in the Assets panel.
    AssetFile = "asset.file", Other, "file";
    /// Play, and Resume while paused.
    ToolbarPlay = "toolbar.play", Run, "play";
    /// Pause.
    ToolbarPause = "toolbar.pause", Run, "pause";
    /// Step one frame.
    ToolbarStep = "toolbar.step", Run, "skip-forward";
    /// Stop.
    ToolbarStop = "toolbar.stop", Run, "square";
    /// The gear that opens Settings.
    ToolbarSettings = "toolbar.settings", Studio, "settings";
    /// The toolbar's Save tool.
    ToolbarSave = "toolbar.save", Studio, "save";
    /// The toolbar's Close project tool.
    ToolbarClose = "toolbar.close", Studio, "folder-x";
    /// The toolbar's Undo tool.
    ToolbarUndo = "toolbar.undo", Studio, "undo-2";
    /// The toolbar's Redo tool.
    ToolbarRedo = "toolbar.redo", Studio, "redo-2";
    /// The toolbar's Reset layout tool.
    ToolbarResetLayout = "toolbar.reset_layout", Scene, "rotate-ccw";
    /// A plugin's tool that brings no icon of its own.
    ToolbarPlugin = "toolbar.plugin", Studio, "puzzle";
    /// A spacer, in the toolbar's Add window.
    ToolbarSpacer = "toolbar.spacer", Interface, "space";
    /// A separator, in the toolbar's Add window.
    ToolbarSeparator = "toolbar.separator", Interface, "separator-vertical";
    /// A search box.
    UiSearch = "ui.search", Interface, "search";
    /// Close, dismiss.
    UiClose = "ui.close", Interface, "x";
    /// A warning.
    UiWarning = "ui.warning", Interface, "triangle-alert";
    /// An error.
    UiError = "ui.error", Interface, "circle-x";
    /// Information.
    UiInfo = "ui.info", Interface, "info";
    /// A check mark.
    UiCheck = "ui.check", Interface, "check";
    /// A collapsed tree item or section.
    UiExpand = "ui.expand", Interface, "chevron-right";
    /// An expanded tree item or section.
    UiCollapse = "ui.collapse", Interface, "chevron-down";
    /// A colour of your own.
    UiPalette = "ui.palette", Interface, "palette";
    /// Add another.
    UiAdd = "ui.add", Interface, "plus";
    /// Move up in a list.
    UiMoveUp = "ui.move_up", Interface, "chevron-up";
    /// Move down in a list.
    UiMoveDown = "ui.move_down", Interface, "chevron-down";
    /// Delete.
    UiDelete = "ui.delete", Interface, "trash-2";
}

/// The file types a custom icon may be, in the order one is preferred when both are there.
pub const FILE_TYPES: [&str; 2] = ["svg", "png"];

impl Icon {
    /// The icon with the stable id `id`, if there is one.
    pub fn from_id(id: &str) -> Option<Icon> {
        Icon::ALL.iter().copied().find(|icon| icon.id() == id)
    }

    /// The colour it is drawn in in `colors`: its colour group's, as `icons.json` gives both (#322).
    pub fn color(self, colors: &Colors) -> Color32 {
        builtin::colour_group(self).color(colors)
    }

    /// The folder of the icons folder its SVG ships in: the part of Studio that draws it (#322).
    pub fn folder(self) -> &'static str {
        match self.id().split_once('.').map(|(area, _)| area) {
            Some("explorer") => "explorer",
            Some("asset") => "assets",
            Some("panel") => "panels",
            Some("toolbar") => "toolbar",
            _ => "interface",
        }
    }

    /// The names of the files that replace it in a custom icon folder: `<id>.svg` and `<id>.png`.
    pub fn file_names(self) -> [String; 2] {
        FILE_TYPES.map(|kind| format!("{}.{kind}", self.id()))
    }
}

/// Paints `icon` filling the square `rect` in `color`: the custom folder's image if it has one (tinted with `color`
/// only when it is monochrome), the built-in glyph otherwise.
pub fn paint(painter: &Painter, icon: Icon, rect: Rect, color: Color32) {
    paint_with(painter, icon, rect, color, false);
}

/// Paints `icon` filling the square `rect` in `color` with its shapes filled in (#302: the play controls). A custom
/// image is drawn as it is.
pub fn paint_filled(painter: &Painter, icon: Icon, rect: Rect, color: Color32) {
    paint_with(painter, icon, rect, color, true);
}

fn paint_with(painter: &Painter, icon: Icon, rect: Rect, color: Color32, filled: bool) {
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
            // The folder's SVG, drawn at exactly the pixels it covers, on the pixel grid, so it stays sharp (#291).
            let ppp = painter.pixels_per_point();
            let side = (rect.height().min(rect.width()) * ppp).round().max(1.0);
            let min = ((rect.center().to_vec2() * ppp - Vec2::splat(side / 2.0)).round() / ppp).to_pos2();
            let snapped = Rect::from_min_size(min, Vec2::splat(side / ppp));
            if let Some(texture) = builtin::texture(painter.ctx(), icon, side as u32, filled) {
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                painter.image(texture, snapped, uv, color);
            }
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
         Every icon in Metallic Studio has a stable id. By default it is a [Lucide](https://lucide.dev/icons/) SVG \
         (ISC licence) in Studio's icons folder, drawn in its group's colour.\n\n\
         **The icons folder** is `icons` in Studio's own folder: `%APPDATA%\\Metallic\\Studio\\icons` on Windows. \
         Studio ships its icons beside its executable and copies any file missing there on start, so your edits \
         are kept. `icons.json` sits at the top; the SVGs are in a folder per part of Studio that draws them \
         (`explorer`, `assets`, `panels`, `toolbar`, `interface`), and Lucide's licence is in `licenses`. \
         `icons.json` says which SVG file each icon id uses and which colour group it is drawn in, with a line on \
         what it marks, and under `colors` each group's colour on the dark and the light mode (`#rrggbb`). To \
         change an icon for good, put an SVG in the folder and point its entry's `file` at it; to change a colour, \
         edit its group's colours, or point an icon's `color` at another group. A file that is missing or cannot \
         be read falls back to the default, with one warning in Output. Studio reads `icons.json` when it \
         starts.\n\n\
         **Custom icons:** set a folder in Settings → Appearance → Icon folder. A file in it named after an icon's id, \
         `<icon-id>.svg` or `<icon-id>.png` (the SVG wins when both are there), replaces that icon. Icons without a \
         file keep the built-in one, and a file that cannot be read keeps it too, with one warning in Output. \
         Studio reads the folder in the background when it is set and when you press **Reload**. A monochrome image \
         (one colour on transparency, such as Lucide's own SVGs) is drawn in its group's colour; a coloured one is \
         drawn as it is.\n\n\
         ## Groups\n\n\
         | Group | Id in `icons.json` | Colour | Dark | Light |\n|---|---|---|---|---|\n",
    );
    for group in Group::ALL {
        let (hue, dark, light) = if group.is_coloured() {
            let (dark, light) = group.default_colours();
            let hex = |c: Color32| format!("`#{:02x}{:02x}{:02x}`", c.r(), c.g(), c.b());
            (format!("{:?}", group.hue()).to_lowercase(), hex(dark), hex(light))
        } else {
            (
                "its place's (neutral by default)".to_owned(),
                String::new(),
                String::new(),
            )
        };
        out.push_str(&format!(
            "| {} | `{}` | {hue} | {dark} | {light} |\n",
            group.label(),
            group.id()
        ));
    }
    out.push_str(
        "\n## Icons\n\n| Icon id | What it marks | Group | Default file | Custom files |\n|---|---|---|---|---|\n",
    );
    for icon in Icon::ALL {
        let [svg, png] = icon.file_names();
        let name = icon.lucide();
        out.push_str(&format!(
            "| `{}` | {} | {} | [`{}`](https://lucide.dev/icons/{name}) | `{svg}`, `{png}` |\n",
            icon.id(),
            icon.description(),
            icon.group().label(),
            icon.default_file(),
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
    fn groups_take_their_hues_and_selection_is_left_to_the_caller() {
        let colors = Look::default().colors(None);
        // The owner's groups (#290).
        for (icon, hue) in [
            (Icon::ExplorerWorkspace, Hue::Green),
            (Icon::ExplorerScript, Hue::Green),
            (Icon::ExplorerScriptService, Hue::Green),
            (Icon::ExplorerLocalScript, Hue::Blue),
            (Icon::ExplorerModuleScript, Hue::Purple),
            (Icon::ExplorerLighting, Hue::Yellow),
            (Icon::ExplorerSpotLight, Hue::Yellow),
            (Icon::ExplorerLight, Hue::Yellow),
            (Icon::ExplorerReplicatedStorage, Hue::Orange),
            // Server storage as the server script service (#321).
            (Icon::ExplorerServerStorage, Hue::Green),
            (Icon::ExplorerSettingsService, Hue::Cyan),
            (Icon::ExplorerCamera, Hue::White),
            (Icon::ExplorerFolder, Hue::White),
            (Icon::ExplorerModel, Hue::White),
            (Icon::ExplorerPart, Hue::White),
            (Icon::ExplorerIntValue, Hue::Grey),
            (Icon::ExplorerStringValue, Hue::Grey),
        ] {
            assert_eq!(icon.color(&colors), colors.hue(hue), "{icon:?}");
        }
        assert_eq!(Icon::ExplorerValue.color(&colors), colors.palette.subtle);
        // Every hue is used, and they differ from one another.
        let all: HashSet<Color32> = Hue::ALL.iter().map(|&hue| colors.hue(hue)).collect();
        assert_eq!(all.len(), Hue::ALL.len());
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
