//! Studio's icons (#231; ADR 0009, Theme). Every icon Studio draws is an [`Icon`]: a **stable id** (`explorer.part`,
//! `panel.output`, `menu.save`) that never changes, a [`Group`] whose colour tells sections apart, and a default
//! glyph from Lucide's icon font. The ids are not Lucide's names, so which Lucide icon an id shows can change.
//!
//! A file named after an id (`explorer.part.svg` or `explorer.part.png`) in the user's custom icon folder replaces the
//! built-in glyph ([`custom`]); [`paint`] draws whichever the icon has. `docs/studio-icons.md` lists every id
//! ([`docs_markdown`]; a test keeps it current).

pub mod custom;

use egui::{Align2, Color32, Painter, Pos2, Rect, Response, Sense, Ui, Vec2, pos2};

use super::lucide::{self, Glyph};
use super::tokens::Hue;
use super::{Colors, fonts};

/// A group of icons drawn in one colour, so the sections of the interface are easy to tell apart. Each group has one
/// [`Hue`] (the accent family's, or neutral) from the theme's tokens, tuned for Dark, Light and high contrast.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    /// Explorer: the game and its services (storage, script and settings services).
    Service,
    /// Explorer: the world, parts and models.
    World,
    /// Explorer: lights and lighting.
    Light,
    /// Explorer: scripts, module scripts and events.
    Script,
    /// Explorer: cameras.
    Camera,
    /// Explorer: everything else (folders, values, input, interface objects).
    Other,
    /// Panel tabs: the scene's panels (Viewport, Explorer, Properties).
    Scene,
    /// Panel tabs: the project's content (Assets).
    Content,
    /// Panel tabs: the log (Output).
    Log,
    /// Panel tabs and the top bar: Studio itself (Settings).
    Studio,
    /// Menus and the toolbar: files and the project (save, close).
    File,
    /// Menus and the toolbar: editing (undo, copy, delete).
    Edit,
    /// Menus and the toolbar: the view and the layout.
    View,
    /// Menus and the toolbar: inserting objects.
    Insert,
    /// Menus and the toolbar: the transform tools (select, move, rotate, scale).
    Transform,
    /// Menus and the toolbar: running the game.
    Run,
    /// The interface's own marks (search, close, carets, checks, warnings): drawn in the colour their place gives
    /// them, not a group colour.
    Interface,
}

impl Group {
    /// Every group, in the order the docs list them.
    pub const ALL: [Group; 17] = [
        Group::Service,
        Group::World,
        Group::Light,
        Group::Script,
        Group::Camera,
        Group::Other,
        Group::Scene,
        Group::Content,
        Group::Log,
        Group::Studio,
        Group::File,
        Group::Edit,
        Group::View,
        Group::Insert,
        Group::Transform,
        Group::Run,
        Group::Interface,
    ];

    /// Its hue.
    pub const fn hue(self) -> Hue {
        match self {
            Group::Service | Group::Scene | Group::File => Hue::Blue,
            Group::World | Group::Content | Group::Run => Hue::Mint,
            Group::Light | Group::Transform => Hue::Orange,
            Group::Script | Group::Log | Group::View => Hue::Violet,
            Group::Camera | Group::Insert => Hue::Rose,
            Group::Other | Group::Studio | Group::Edit | Group::Interface => Hue::Neutral,
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
            Group::World => "Explorer: world, parts and models",
            Group::Light => "Explorer: lights",
            Group::Script => "Explorer: scripts",
            Group::Camera => "Explorer: cameras",
            Group::Other => "Explorer: other",
            Group::Scene => "Panels: scene",
            Group::Content => "Panels: content",
            Group::Log => "Panels: log",
            Group::Studio => "Studio",
            Group::File => "Menus and toolbar: file",
            Group::Edit => "Menus and toolbar: edit",
            Group::View => "Menus and toolbar: view",
            Group::Insert => "Menus and toolbar: insert",
            Group::Transform => "Menus and toolbar: transform",
            Group::Run => "Menus and toolbar: run",
            Group::Interface => "Interface",
        }
    }
}

/// Declares [`Icon`] and its table: each icon's variant, stable id, group and default Lucide glyph, in one place.
macro_rules! icons {
    ($($(#[doc = $doc:literal])* $name:ident = $id:literal, $group:ident, $glyph:ident;)*) => {
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

            /// Its built-in Lucide glyph.
            pub const fn glyph(self) -> Glyph {
                match self {
                    $(Icon::$name => lucide::$glyph,)*
                }
            }
        }
    };
}

icons! {
    /// The game (the data model).
    ExplorerGame = "explorer.game", Service, GAMEPAD_2;
    /// The workspace.
    ExplorerWorkspace = "explorer.workspace", World, GLOBE;
    /// The lighting service.
    ExplorerLighting = "explorer.lighting", Light, SUN;
    /// Replicated storage.
    ExplorerReplicatedStorage = "explorer.replicated_storage", Service, PACKAGE;
    /// The server script service.
    ExplorerScriptService = "explorer.script_service", Service, SERVER_COG;
    /// Server storage.
    ExplorerServerStorage = "explorer.server_storage", Service, HARD_DRIVE;
    /// A settings or input service (user settings, input, run service).
    ExplorerSettingsService = "explorer.settings_service", Service, SLIDERS_HORIZONTAL;
    /// A camera.
    ExplorerCamera = "explorer.camera", Camera, VIDEO;
    /// A part.
    ExplorerPart = "explorer.part", World, BOX;
    /// A mesh part.
    ExplorerMeshPart = "explorer.mesh_part", World, CUBOID;
    /// A model.
    ExplorerModel = "explorer.model", World, SHAPES;
    /// A directional light.
    ExplorerDirectionalLight = "explorer.directional_light", Light, SUN;
    /// A spotlight.
    ExplorerSpotLight = "explorer.spot_light", Light, FLASHLIGHT;
    /// Any other light.
    ExplorerLight = "explorer.light", Light, LIGHTBULB;
    /// A script.
    ExplorerScript = "explorer.script", Script, FILE_CODE;
    /// A module script.
    ExplorerModuleScript = "explorer.module_script", Script, BRACES;
    /// A bindable event or function.
    ExplorerEvent = "explorer.event", Script, RADIO;
    /// A folder.
    ExplorerFolder = "explorer.folder", Other, FOLDER;
    /// A value object.
    ExplorerValue = "explorer.value", Other, DIAMOND;
    /// An input object.
    ExplorerInput = "explorer.input", Other, MOUSE_POINTER;
    /// Any other instance.
    ExplorerInstance = "explorer.instance", Other, CIRCLE;
    /// The Viewport panel's tab.
    PanelViewport = "panel.viewport", Scene, MONITOR;
    /// The Explorer panel's tab.
    PanelExplorer = "panel.explorer", Scene, LIST_TREE;
    /// The Properties panel's tab.
    PanelProperties = "panel.properties", Scene, LIST;
    /// The Assets panel's tab.
    PanelAssets = "panel.assets", Content, FOLDER_OPEN;
    /// The Output panel's tab.
    PanelOutput = "panel.output", Log, SQUARE_TERMINAL;
    /// The Settings panel's tab.
    PanelSettings = "panel.settings", Studio, SETTINGS;
    /// A mesh in the Assets panel.
    AssetMesh = "asset.mesh", World, CUBOID;
    /// An image in the Assets panel.
    AssetImage = "asset.image", Content, IMAGE;
    /// A sound in the Assets panel.
    AssetAudio = "asset.audio", Content, MUSIC;
    /// A scene in the Assets panel.
    AssetScene = "asset.scene", World, LAYERS;
    /// A script in the Assets panel.
    AssetScript = "asset.script", Script, FILE_CODE;
    /// Any other file in the Assets panel.
    AssetFile = "asset.file", Other, FILE;
    /// File, Save.
    MenuSave = "menu.save", File, SAVE;
    /// File, Close project.
    MenuCloseProject = "menu.close_project", File, FOLDER_X;
    /// View, Reset layout.
    MenuResetLayout = "menu.reset_layout", View, ROTATE_CCW;
    /// The gear that opens Settings.
    MenuSettings = "menu.settings", Studio, SETTINGS;
    /// A search box.
    UiSearch = "ui.search", Interface, SEARCH;
    /// Close, dismiss.
    UiClose = "ui.close", Interface, X;
    /// A warning.
    UiWarning = "ui.warning", Interface, TRIANGLE_ALERT;
    /// An error.
    UiError = "ui.error", Interface, CIRCLE_X;
    /// Information.
    UiInfo = "ui.info", Interface, INFO;
    /// A check mark.
    UiCheck = "ui.check", Interface, CHECK;
    /// A collapsed tree item.
    UiExpand = "ui.expand", Interface, CHEVRON_RIGHT;
    /// An expanded tree item.
    UiCollapse = "ui.collapse", Interface, CHEVRON_DOWN;
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
            painter.image(image.texture, rect, uv, tint);
        }
        None => {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                icon.glyph().code,
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
         **Custom icons:** choose a folder in Settings → Appearance → Icons. A file in it named after an icon's id, \
         `<icon-id>.svg` or `<icon-id>.png` (the SVG wins when both are there), replaces that icon. Icons without a \
         file keep the built-in one, and a file that cannot be read keeps it too, with one warning in Output. \
         Studio reads the folder in the background and notices new and changed files by itself; \
         **Reload icons** reads it again at once. A monochrome image (one colour on transparency, such as Lucide's own \
         SVGs) is drawn in its group's colour; a coloured one is drawn as it is.\n\n\
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
        let name = icon.glyph().name;
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
            assert!(["explorer", "panel", "asset", "menu", "ui"].contains(&area), "{id}");
            assert!(
                !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
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
                let glyph = icon.glyph();
                let has = ui
                    .ctx()
                    .fonts_mut(|f| f.has_glyph(&fonts::named(fonts::ICONS, 16.0), glyph.code));
                assert!(has, "{} ({}, U+{:04X})", icon.id(), glyph.name, glyph.code as u32);
                // The private use area: never a letter of the text fonts.
                assert!(('\u{E000}'..='\u{F8FF}').contains(&glyph.code), "{}", icon.id());
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn groups_take_their_hues_and_selection_is_left_to_the_caller() {
        let colors = Look::default().colors(None);
        assert_eq!(Icon::ExplorerPart.color(&colors), colors.hue(Hue::Mint));
        assert_eq!(Icon::ExplorerSpotLight.color(&colors), colors.hue(Hue::Orange));
        assert_eq!(Icon::ExplorerScript.color(&colors), colors.hue(Hue::Violet));
        assert_eq!(Icon::ExplorerCamera.color(&colors), colors.hue(Hue::Rose));
        assert_eq!(Icon::ExplorerReplicatedStorage.color(&colors), colors.hue(Hue::Blue));
        assert_eq!(Icon::ExplorerFolder.color(&colors), colors.palette.subtle);
        // The Explorer's six groups have six different colours.
        let explorer: HashSet<Color32> = [
            Group::Service,
            Group::World,
            Group::Light,
            Group::Script,
            Group::Camera,
            Group::Other,
        ]
        .iter()
        .map(|group| group.color(&colors))
        .collect();
        assert_eq!(explorer.len(), 6);
        // Every group is used by an icon, except the toolbar's, which #232 fills.
        for group in Group::ALL {
            let used = Icon::ALL.iter().any(|icon| icon.group() == group);
            let toolbar = matches!(group, Group::Edit | Group::Insert | Group::Transform | Group::Run);
            assert_eq!(used, !toolbar, "{group:?}");
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
        for icon in Icon::ALL {
            assert!(written.contains(&format!("`{}`", icon.id())), "{}", icon.id());
        }
    }

    /// Writes `docs/studio-icons.md` from the table.
    #[test]
    #[ignore = "writes docs/studio-icons.md"]
    fn write_the_icon_docs() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/studio-icons.md");
        std::fs::write(path, docs_markdown()).unwrap();
    }
}
