//! The Settings window (ADR 0009, Settings): a category list on the left, a search box over every setting, and each
//! setting as a row with its name, a line saying what it does, and its control.
//!
//! A window of its own beside Studio's, as Roblox Studio's Settings are (#331): moved, resized and closed like any
//! window, on every screen. Where Studio cannot open one (the tests draw Studio without a windowing system), it floats
//! over Studio's window instead (#238), and is never docked. Either way it opens where it was when it last closed, at
//! the size it had, across runs too ([`StudioSettings::settings_window`], #578); the first time, centred over Studio.
//! Every setting is one [`Entry`] in [`ENTRIES`], so the pages and the search read the same list, and adding a setting
//! is one line there plus its control.

use std::ops::RangeInclusive;

use egui::emath::Numeric;
use egui::{
    Align, Color32, CornerRadius, DragValue, Frame, Id, Label, Layout, Response, RichText, Role, ScrollArea, Sense,
    Stroke, TextEdit, Ui, WidgetInfo, pos2, vec2,
};
use metallic_engine::metallic_render::AntiAliasing;
use metallic_engine::metallic_render::lights::MAX_GPU_LIGHTS;
use metallic_engine::metallic_render::resolution::{MAX_SCALE, MIN_SCALE};
use metallic_engine::metallic_render::settings::{GraphicsSettings, Preset, Quality, Tier};

use super::keybinds::{self, KeybindsPage};
use super::{
    LOD_BIAS, MAX_FRAME_CAP, MAX_VIEW_DISTANCE, MotionPreference, SHADOW_DISTANCE, StudioSettings, TARGET_FRAME_MS,
    WindowPlace, ui_scale,
};
use crate::code;
use crate::keymap::{Command, Keymap};
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{Accent, radius, size, space, text};
use crate::theme::{ButtonKind, ThemeMode, button, current_colors, fonts};

/// A settings category: one page of the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Category {
    /// Studio as a whole.
    #[default]
    General,
    /// The look of the interface.
    Appearance,
    /// The viewport's graphics.
    Rendering,
    /// The script editor (#529): its font, spacing and colours, kept in `code_editor.toml`.
    CodeEditor,
    /// The keymap: its own page ([`KeybindsPage`]), not [`ENTRIES`].
    Keybinds,
}

impl Category {
    /// Every category, in the order the window lists them.
    pub const ALL: [Category; 5] = [
        Category::General,
        Category::Appearance,
        Category::Rendering,
        Category::CodeEditor,
        Category::Keybinds,
    ];

    /// The name people see.
    pub fn label(self) -> &'static str {
        match self {
            Category::General => "General",
            Category::Appearance => "Appearance",
            Category::Rendering => "Rendering",
            Category::CodeEditor => "Code Editor",
            Category::Keybinds => "Keybinds",
        }
    }

    /// The line under the page's title.
    fn lead(self) -> &'static str {
        match self {
            Category::General => "How Studio starts and behaves.",
            Category::Appearance => "Changes apply at once.",
            Category::Rendering => "How the editor's viewport draws your scenes. The game's own look is never changed.",
            Category::CodeEditor => {
                "How scripts look and type. The colours are the current theme's; each theme keeps its own."
            }
            Category::Keybinds => "Click a shortcut to change it, or + to add another. Right-click one to remove it.",
        }
    }
}

/// One setting as the window shows it: where it is listed, what it is called, what it does, and its control.
pub struct Entry {
    /// Its page.
    pub category: Category,
    /// Its name.
    pub name: &'static str,
    /// What it does, in a line.
    pub description: &'static str,
    /// Draws its control, changing the settings.
    control: fn(&mut Ui, &mut StudioSettings),
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry")
            .field("category", &self.category)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Entry {
    /// Whether every word of `query` is in the name or the description, in any letter case.
    pub fn matches(&self, query: &str) -> bool {
        let name = self.name.to_lowercase();
        let description = self.description.to_lowercase();
        query.split_whitespace().all(|word| {
            let word = word.to_lowercase();
            name.contains(&word) || description.contains(&word)
        })
    }
}

/// The settings `query` finds, in the window's order. A blank query finds none.
pub fn search(query: &str) -> Vec<&'static Entry> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    ENTRIES.iter().filter(|entry| entry.matches(query)).collect()
}

/// What the presets give for a frame cap and vsync (the engine's own defaults, as `graphics.toml` lists them).
const PRESET_FRAME_CAP: u32 = 240;
const PRESET_VSYNC: bool = true;

/// A Code Editor entry for the colour of one [`code::settings::Kind`] (#529).
macro_rules! code_colour_entry {
    ($kind:ident, $description:expr) => {
        Entry {
            category: Category::CodeEditor,
            name: code::settings::Kind::$kind.label(),
            description: $description,
            control: |ui, s| code_colour(ui, s, code::settings::Kind::$kind),
        }
    };
}

/// Every setting, page by page.
pub static ENTRIES: &[Entry] = &[
    Entry {
        category: Category::General,
        name: "Skip sign-in",
        description: "Go straight to your projects when Studio starts, without the sign-in screen.",
        control: |ui, s| {
            toggle(ui, &mut s.general.skip_sign_in, "Skip sign-in");
        },
    },
    Entry {
        category: Category::Appearance,
        name: "Theme",
        description: "Dark, light, or whichever the system uses.",
        control: |ui, s| {
            let choices = ThemeMode::ALL.map(|mode| (mode, mode_label(mode)));
            segmented(ui, &mut s.appearance.mode, &choices);
        },
    },
    Entry {
        category: Category::Appearance,
        name: "Accent Color",
        description: "Marks focus, the selected item, selected text and progress.",
        control: |ui, s| accent_swatches(ui, &mut s.appearance.accent),
    },
    Entry {
        category: Category::Appearance,
        name: "High contrast",
        description: "Secondary text as strong as body text, and stronger borders.",
        control: |ui, s| {
            toggle(ui, &mut s.appearance.high_contrast, "High contrast");
        },
    },
    Entry {
        category: Category::Appearance,
        name: "Interface scale",
        description: "Makes the whole interface larger or smaller.",
        control: |ui, s| {
            let scale = &mut s.appearance.ui_scale;
            let percent = |scale: f32| format!("{:.0} %", scale * 100.0);
            let shown = egui::ComboBox::from_id_salt("settings.ui_scale")
                .icon(crate::theme::combo_icon)
                .selected_text(percent(*scale))
                .width(COMBO_WIDTH)
                .show_ui(ui, |ui| {
                    for choice in ui_scale::CHOICES {
                        ui.selectable_value(scale, choice, percent(choice));
                    }
                });
            named(&shown.response, "Interface scale");
        },
    },
    Entry {
        category: Category::Appearance,
        name: "Reduced motion",
        description: "Turns off fades and eases. Follows the system's switch unless you choose.",
        control: |ui, s| {
            let choices = MotionPreference::ALL.map(|choice| (choice, choice.label()));
            segmented(ui, &mut s.appearance.motion, &choices);
        },
    },
    Entry {
        category: Category::Appearance,
        name: "Icon folder",
        description: "SVG or PNG files named after icon ids (such as explorer.part.svg) replace Studio's icons. \
                      The ids are listed in docs/studio-icons.md.",
        control: |ui, s| {
            // Right to left: Reload, Browse, then the path filling the rest.
            if button(ui, ButtonKind::Secondary, "Reload")
                .on_hover_text("Read the folder again")
                .clicked()
            {
                icons::custom::request_reload(ui.ctx());
            }
            if button(ui, ButtonKind::Secondary, "Browse\u{2026}")
                .on_hover_text("Choose the folder")
                .clicked()
            {
                // Studio opens the system's folder dialog once the window is drawn (#508).
                request_icon_folder(ui.ctx());
            }
            ui.add(
                TextEdit::singleline(&mut s.appearance.icon_folder)
                    .hint_text("No folder: built-in icons")
                    .desired_width(f32::INFINITY)
                    .margin(vec2(space::S4, space::S3)),
            );
        },
    },
    Entry {
        category: Category::Appearance,
        name: "Panel borders",
        description: "Outlines each group of docked panels, as the Settings window is outlined.",
        control: |ui, s| {
            toggle(ui, &mut s.appearance.panel_borders, "Panel borders");
        },
    },
    Entry {
        category: Category::Rendering,
        name: "Quality preset",
        description: "How much a viewport frame may cost. Auto measures this machine and holds the frame time.",
        control: |ui, s| {
            let preset = &mut s.rendering.preset;
            let shown = egui::ComboBox::from_id_salt("settings.preset")
                .icon(crate::theme::combo_icon)
                .selected_text(preset_label(*preset))
                .width(COMBO_WIDTH)
                .show_ui(ui, |ui| {
                    for &choice in Preset::ALL {
                        ui.selectable_value(preset, choice, preset_label(choice));
                    }
                });
            named(&shown.response, "Quality preset");
        },
    },
    Entry {
        category: Category::Rendering,
        name: "Anti-aliasing",
        description: "Smooths jagged edges. MSAA falls back to what the GPU has.",
        control: |ui, s| {
            let preset = aa_label(preset_quality(&s.rendering).anti_aliasing);
            let choices = AntiAliasing::ALL.map(|aa| (aa, aa_label(aa)));
            override_combo(ui, "Anti-aliasing", &mut s.rendering.anti_aliasing, preset, &choices);
        },
    },
    Entry {
        category: Category::Rendering,
        name: "VSync",
        description: "Waits for the display between frames, so the image never tears.",
        control: |ui, s| {
            let choices = [(true, "On"), (false, "Off")];
            let preset = if PRESET_VSYNC { "On" } else { "Off" };
            override_combo(ui, "VSync", &mut s.rendering.vsync, preset, &choices);
        },
    },
    Entry {
        category: Category::Rendering,
        name: "Frame cap",
        description: "The most frames per second the viewport draws; 0 for no limit.",
        control: |ui, s| {
            override_number(
                ui,
                "Frame cap",
                &mut s.rendering.frame_cap,
                PRESET_FRAME_CAP,
                0..=MAX_FRAME_CAP,
                1.0,
                " fps",
            );
        },
    },
    Entry {
        category: Category::Rendering,
        name: "Render scale",
        description: "The viewport's resolution, as a share of the panel's size.",
        control: |ui, s| {
            let preset = preset_quality(&s.rendering).resolution.scale;
            override_number(
                ui,
                "Render scale",
                &mut s.rendering.render_scale,
                preset,
                MIN_SCALE..=MAX_SCALE,
                0.01,
                " ×",
            );
        },
    },
    Entry {
        category: Category::Rendering,
        name: "Dynamic resolution",
        description: "Lowers the render scale while frames take too long.",
        control: |ui, s| {
            let preset = if preset_quality(&s.rendering).resolution.dynamic {
                "On"
            } else {
                "Off"
            };
            let choices = [(true, "On"), (false, "Off")];
            override_combo(
                ui,
                "settings.dynamic",
                &mut s.rendering.dynamic_resolution,
                preset,
                &choices,
            );
        },
    },
    Entry {
        category: Category::Rendering,
        name: "Target frame time",
        description: "The GPU time per frame dynamic resolution holds.",
        control: |ui, s| {
            let preset = preset_quality(&s.rendering).resolution.target_ms;
            let range = TARGET_FRAME_MS.0..=TARGET_FRAME_MS.1;
            override_number(
                ui,
                "Target frame time",
                &mut s.rendering.target_frame_ms,
                preset,
                range,
                0.1,
                " ms",
            );
        },
    },
    Entry {
        category: Category::Rendering,
        name: "View distance",
        description: "How far the camera sees, in metres; 0 for no limit.",
        control: |ui, s| {
            let distance = preset_quality(&s.rendering).cull.max_distance;
            let preset = if distance.is_finite() { distance } else { 0.0 };
            let range = 0.0..=MAX_VIEW_DISTANCE;
            override_number(
                ui,
                "View distance",
                &mut s.rendering.view_distance,
                preset,
                range,
                10.0,
                " m",
            );
        },
    },
    Entry {
        category: Category::Rendering,
        name: "Shadow quality",
        description: "How many shadow cascades the sun has, and how sharp they are.",
        control: |ui, s| {
            let preset = tier_label(preset_quality(&s.rendering).tier);
            let choices = Tier::ALL.map(|tier| (tier, tier_label(tier)));
            override_combo(
                ui,
                "settings.shadows",
                &mut s.rendering.shadow_quality,
                preset,
                &choices,
            );
        },
    },
    Entry {
        category: Category::Rendering,
        name: "Shadow distance",
        description: "How far the sun's shadows reach, in metres.",
        control: |ui, s| {
            let preset = preset_quality(&s.rendering).shadows.distance;
            let range = SHADOW_DISTANCE.0..=SHADOW_DISTANCE.1;
            override_number(
                ui,
                "Shadow distance",
                &mut s.rendering.shadow_distance,
                preset,
                range,
                1.0,
                " m",
            );
        },
    },
    Entry {
        category: Category::Rendering,
        name: "Light budget",
        description: "How many local lights a frame draws at full strength.",
        control: |ui, s| {
            let preset = preset_quality(&s.rendering).lights.budget;
            let range = 1..=MAX_GPU_LIGHTS as u32;
            override_number(
                ui,
                "Light budget",
                &mut s.rendering.light_budget,
                preset,
                range,
                1.0,
                "",
            );
        },
    },
    Entry {
        category: Category::Rendering,
        name: "Level of detail",
        description: "Higher keeps meshes finer further away.",
        control: |ui, s| {
            let preset = preset_quality(&s.rendering).lod.bias;
            let range = LOD_BIAS.0..=LOD_BIAS.1;
            override_number(
                ui,
                "Level of detail",
                &mut s.rendering.lod_bias,
                preset,
                range,
                0.01,
                " ×",
            );
        },
    },
    Entry {
        category: Category::Rendering,
        name: "Effect quality",
        description: "Bloom's resolution and the edge search of MLAA.",
        control: |ui, s| {
            let preset = tier_label(preset_quality(&s.rendering).tier);
            let choices = Tier::ALL.map(|tier| (tier, tier_label(tier)));
            override_combo(
                ui,
                "settings.effects",
                &mut s.rendering.effect_quality,
                preset,
                &choices,
            );
        },
    },
    // The script editor (#529).
    Entry {
        category: Category::CodeEditor,
        name: "Font",
        description: "The typeface of scripts: JetBrains Mono, Inter, or a font file of yours.",
        control: |ui, s| code_font(ui, &mut s.code.font),
    },
    Entry {
        category: Category::CodeEditor,
        name: "Font size",
        description: "How large the code is.",
        control: |ui, s| code_number(ui, "Font size", &mut s.code.size, code::settings::SIZE, 0.1, " pt"),
    },
    Entry {
        category: Category::CodeEditor,
        name: "Line spacing",
        description: "The space from one line to the next, as a multiple of the font size.",
        control: |ui, s| {
            code_number(
                ui,
                "Line spacing",
                &mut s.code.line_spacing,
                code::settings::LINE_SPACING,
                0.01,
                " \u{00D7}",
            );
        },
    },
    Entry {
        category: Category::CodeEditor,
        name: "Letter spacing",
        description: "Space added between letters.",
        control: |ui, s| {
            code_number(
                ui,
                "Letter spacing",
                &mut s.code.letter_spacing,
                code::settings::LETTER_SPACING,
                0.05,
                " pt",
            );
        },
    },
    Entry {
        category: Category::CodeEditor,
        name: "Tab width",
        description: "How many spaces wide a tab is.",
        control: |ui, s| {
            let choices = code::settings::TAB_WIDTHS.map(|width| (width, ["2", "4", "8"][usize::from(width / 4)]));
            segmented(ui, &mut s.code.tab_width, &choices);
        },
    },
    Entry {
        category: Category::CodeEditor,
        name: "Indent with",
        description: "What Tab puts in: a tab, or that many spaces.",
        control: |ui, s| segmented(ui, &mut s.code.insert_spaces, &[(false, "Tabs"), (true, "Spaces")]),
    },
    Entry {
        category: Category::CodeEditor,
        name: "Show whitespace",
        description: "Spaces show as faint dots and tabs as faint arrows.",
        control: |ui, s| {
            toggle(ui, &mut s.code.show_whitespace, "Show whitespace");
        },
    },
    Entry {
        category: Category::CodeEditor,
        name: "Highlight the current line",
        description: "Lights the line the cursor is on.",
        control: |ui, s| {
            toggle(ui, &mut s.code.highlight_line, "Highlight the current line");
        },
    },
    Entry {
        category: Category::CodeEditor,
        name: "Show the minimap",
        description: "The code in small down the right of the script editor, the part in view shaded; click or drag it to scroll.",
        control: |ui, s| {
            toggle(ui, &mut s.code.minimap, "Show the minimap");
        },
    },
    code_colour_entry!(Text, "The colour of names and anything else."),
    code_colour_entry!(Keyword, "local, function, if, end and Luau's other reserved words."),
    code_colour_entry!(Constant, "The values true, false and nil."),
    code_colour_entry!(
        Builtin,
        "The globals every script has: game, workspace, print, math, Vector3..."
    ),
    code_colour_entry!(Function, "A function's name, where it is made or called."),
    code_colour_entry!(Member, "A property or field, after . or :."),
    code_colour_entry!(String, "Text in quotes."),
    code_colour_entry!(Number, "Numbers."),
    code_colour_entry!(Comment, "Comments, after --."),
    code_colour_entry!(Operator, "Operators and punctuation: + .. == ( ) , and the others."),
    Entry {
        category: Category::CodeEditor,
        name: "Reset colours",
        description: "Puts the current theme's colours back as Studio has them.",
        control: |ui, s| {
            let dark = current_colors(ui.ctx()).dark;
            let defaults = if dark {
                code::settings::Palette::DARK
            } else {
                code::settings::Palette::LIGHT
            };
            if button(ui, ButtonKind::Secondary, "Reset").clicked() {
                *s.code.palette_mut(dark) = defaults;
            }
        },
    },
];

/// The Settings window's viewport (#331).
fn viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("metallic.settings")
}

/// The Metallic mark, as the window's icon.
fn window_icon() -> std::sync::Arc<egui::IconData> {
    static ICON: std::sync::OnceLock<std::sync::Arc<egui::IconData>> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        let mark = crate::emblem::render(64);
        std::sync::Arc::new(egui::IconData {
            rgba: mark.rgba,
            width: mark.width,
            height: mark.height,
        })
    })
    .clone()
}

/// The window's size the first time it opens, in points.
const SIZE: egui::Vec2 = vec2(780.0, 640.0);
/// The smallest it may be made, in points.
const MIN_SIZE: egui::Vec2 = vec2(680.0, 360.0);

/// How the window of its own opens (#578): where and how large, in points, and whether maximised.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Opening {
    /// Its frame's top-left corner; `None` leaves it to the system.
    position: Option<egui::Pos2>,
    /// Its inside's size.
    size: egui::Vec2,
    /// Whether it opens maximised.
    maximized: bool,
}

/// How the window of its own opens: at `remembered` (in physical pixels, at `pixels_per_point`), or else at [`SIZE`]
/// centred over Studio's window at `main` (its frame, in points). A place off every monitor is the shell's to catch.
fn opening(remembered: Option<WindowPlace>, main: Option<egui::Rect>, pixels_per_point: f32) -> Opening {
    let scale = if pixels_per_point > 0.0 { pixels_per_point } else { 1.0 };
    match remembered {
        Some(place) => Opening {
            position: Some(pos2(place.x as f32, place.y as f32) / scale),
            size: (vec2(place.width as f32, place.height as f32) / scale).max(MIN_SIZE),
            maximized: place.maximized,
        },
        None => Opening {
            position: main.map(|main| main.center() - SIZE / 2.0),
            size: SIZE,
            maximized: false,
        },
    }
}

/// Where the window of its own is now, from its viewport's `info` at `pixels_per_point`, in physical pixels; `last`
/// while egui does not know (before it opens, or minimised). Maximised, it keeps the place it had before, to go back to.
fn noted_place(info: &egui::ViewportInfo, pixels_per_point: f32, last: Option<WindowPlace>) -> Option<WindowPlace> {
    if info.minimized == Some(true) {
        return last;
    }
    let (Some(outer), Some(inner)) = (info.outer_rect, info.inner_rect) else {
        return last;
    };
    let place = place_of(outer.min, inner.size(), pixels_per_point);
    if info.maximized == Some(true) {
        return Some(WindowPlace {
            maximized: true,
            ..last.unwrap_or(place)
        });
    }
    Some(place)
}

/// A place at `corner` of `size`, both in points at `pixels_per_point`, in physical pixels.
fn place_of(corner: egui::Pos2, size: egui::Vec2, pixels_per_point: f32) -> WindowPlace {
    let (corner, size) = (corner * pixels_per_point, size * pixels_per_point);
    WindowPlace {
        x: corner.x.round() as i32,
        y: corner.y.round() as i32,
        width: size.x.round() as u32,
        height: size.y.round() as u32,
        maximized: false,
    }
}

/// Where the floating window (#238) first opens in a run: its centre and outer size in points, from `remembered` (its
/// rect in Studio's window, in physical pixels at `pixels_per_point`), or else [`SIZE`] centred in `content`. egui
/// keeps it on the window, and remembers it after that.
fn floating_place(
    remembered: Option<WindowPlace>,
    content: egui::Rect,
    pixels_per_point: f32,
) -> (egui::Pos2, egui::Vec2) {
    let scale = if pixels_per_point > 0.0 { pixels_per_point } else { 1.0 };
    match remembered {
        Some(place) => {
            let rect = egui::Rect::from_min_size(
                pos2(place.x as f32, place.y as f32) / scale,
                vec2(place.width as f32, place.height as f32) / scale,
            );
            (rect.center(), rect.size())
        }
        None => (content.center(), SIZE),
    }
}

/// The width of a setting's control column.
pub(super) const CONTROL_WIDTH: f32 = 260.0;
/// The width of a drop-down in it.
const COMBO_WIDTH: f32 = 200.0;
/// The width of the category list.
const CATEGORY_WIDTH: f32 = 168.0;

/// The Settings window's own state: whether it is open, the page shown, the search, and the Keybinds page's.
#[derive(Debug, Default)]
pub struct SettingsWindow {
    open: bool,
    category: Category,
    query: String,
    keybinds: KeybindsPage,
    /// Where the window was last drawn, while open.
    rect: Option<egui::Rect>,
    /// Whether the last click went into the window (and not to what is under it).
    focused: bool,
    /// Whether it is shown in a window of its own (#331), which has its own input: Studio's then never reads it.
    own_window: bool,
    /// Where it is while open, in physical pixels (#578): kept in the settings once it closes.
    place: Option<WindowPlace>,
    /// How its window of its own was opened, kept while it is open: egui moves a window whose builder changes.
    opened_as: Option<egui::ViewportBuilder>,
}

impl SettingsWindow {
    /// Whether it is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens or closes it. Opened, it has the focus, as a click in it gives.
    pub fn set_open(&mut self, open: bool) {
        self.open = open;
        self.focused = open;
        if !open {
            self.rect = None;
            self.opened_as = None;
        }
    }

    /// Where it floats over Studio's window (#238), as last drawn: `None` closed, or in a window of its own.
    pub fn rect(&self) -> Option<egui::Rect> {
        self.rect
    }

    /// Keeps where the window is in `settings` (#578), to open there next time: it does so itself when it closes, and
    /// Studio as Studio closes with it open.
    pub fn keep_place(&self, settings: &mut StudioSettings) {
        if let Some(place) = self.place {
            settings.settings_window = Some(place);
        }
    }

    /// Once it has closed: keeps where it was in `settings`, and forgets it, so a place from before never overwrites
    /// the settings later.
    fn forget_place(&mut self, settings: &mut StudioSettings) {
        self.keep_place(settings);
        self.place = None;
    }

    /// Whether it is open and the user's last click was in it: what is under it then gets no shortcuts.
    pub fn has_focus(&self) -> bool {
        self.open && self.focused
    }

    /// Whether it is open under `at`, as last drawn.
    pub fn contains(&self, at: egui::Pos2) -> bool {
        self.open && self.rect.is_some_and(|rect| rect.contains(at))
    }

    /// The page shown when the search is empty.
    pub fn category(&self) -> Category {
        self.category
    }

    /// Shows `category`'s page, clearing the search.
    pub fn show_category(&mut self, category: Category) {
        self.category = category;
        self.query.clear();
    }

    /// The search.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Sets the search.
    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
    }

    /// The Keybinds page's state.
    pub fn keybinds(&mut self) -> &mut KeybindsPage {
        &mut self.keybinds
    }

    /// Whether a keybind is being recorded: Studio does not run the keymap meanwhile.
    pub fn is_capturing(&self) -> bool {
        self.keybinds.is_capturing()
    }

    /// Reads the frame's input for a keybind being recorded (#266). Studio calls it every frame before anything is
    /// drawn, in place of the keymap, so the keys reach nothing else. In a window of its own (#331) the keys arrive
    /// there, and the window reads them itself.
    pub fn listen(&mut self, ctx: &egui::Context, keymap: &mut Keymap) {
        if !self.own_window {
            self.keybinds.listen(ctx, keymap);
        }
    }

    /// Draws the window, if open, changing `settings` and the `keymap` as the user does: in a window of its own where
    /// Studio can open one (#331), else floating over `ctx`.
    pub fn show(&mut self, ctx: &egui::Context, settings: &mut StudioSettings, keymap: &mut Keymap) {
        self.own_window = self.open && !ctx.embed_viewports();
        if !self.open {
            // However it was closed, where it was is kept for next time (#578).
            self.forget_place(settings);
            return;
        }
        if self.own_window {
            self.show_own_window(ctx, settings, keymap);
            return;
        }
        let mut open = true;
        let pixels_per_point = ctx.pixels_per_point();
        let (centre, size) = floating_place(settings.settings_window, ctx.content_rect(), pixels_per_point);
        let shown = egui::Window::new("Settings")
            .id(Id::unique("metallic.settings"))
            // Where it last was (#578), or centred on the window; after that, where it is moved to is egui's to
            // remember. The pivot stays the centre: egui would read the place it keeps by a new one.
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(centre)
            // Its own header, with Lucide's close button (#278), in place of egui's title bar.
            .title_bar(false)
            .collapsible(false)
            .resizable(true)
            .default_size(size)
            .min_width(MIN_SIZE.x)
            .min_height(MIN_SIZE.y)
            .show(ctx, |ui| {
                crate::theme::window_header(ui, "Settings", &mut open);
                self.contents(ui, settings, keymap);
            });
        self.rect = shown.map(|shown| shown.response.rect);
        if let Some(rect) = self.rect {
            self.place = Some(place_of(rect.min, rect.size(), pixels_per_point));
        }
        let press = ctx.input(|input| {
            input
                .pointer
                .any_pressed()
                .then(|| input.pointer.interact_pos())
                .flatten()
        });
        if let Some(at) = press {
            self.focused = self.rect.is_some_and(|rect| rect.contains(at));
        }
        if !open {
            self.set_open(false);
            self.forget_place(settings);
        }
    }

    /// The window of its own (#331): its title bar the system's, where it was when it last closed (#578) or else
    /// centred over Studio's window, and closed by its close button. A keybind being recorded reads its keys here,
    /// before the page is drawn.
    fn show_own_window(&mut self, ctx: &egui::Context, settings: &mut StudioSettings, keymap: &mut Keymap) {
        // What is under the pointer in Studio's window is never this window.
        self.rect = None;
        self.focused = false;
        let builder = match &self.opened_as {
            Some(builder) => builder.clone(),
            None => {
                // A place noted before it was closed and opened again within a frame.
                self.keep_place(settings);
                let main = ctx.input(|input| input.viewport().outer_rect);
                let opening = opening(settings.settings_window, main, ctx.pixels_per_point());
                let mut builder = egui::ViewportBuilder::default()
                    .with_title("Settings")
                    .with_inner_size(opening.size)
                    .with_min_inner_size(MIN_SIZE)
                    .with_maximized(opening.maximized)
                    .with_icon(window_icon());
                if let Some(at) = opening.position {
                    builder = builder.with_position(at);
                }
                self.opened_as = Some(builder.clone());
                builder
            }
        };
        let mut close = false;
        ctx.show_viewport_immediate(viewport_id(), builder, |ui, _| {
            self.place = ui.input(|input| noted_place(input.viewport(), input.pixels_per_point(), self.place));
            if self.keybinds.is_capturing() {
                self.keybinds.listen(ui.ctx(), keymap);
            }
            let fill = ui.visuals().window_fill;
            let rect = ui.max_rect();
            ui.painter().rect_filled(rect, 0.0, fill);
            let inner = rect.shrink(space::S6);
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                self.contents(ui, settings, keymap);
            });
            close = ui.input(|input| input.viewport().close_requested());
        });
        if close {
            self.set_open(false);
            self.forget_place(settings);
        }
    }

    /// The pages, filling `ui`: the search box, the category list and the page or the search's results.
    pub fn contents(&mut self, ui: &mut Ui, settings: &mut StudioSettings, keymap: &mut Keymap) {
        let p = current_colors(ui.ctx()).palette;
        ui.horizontal(|ui| {
            icons::show(ui, Icon::UiSearch, text::BODY_LG, p.subtle);
            ui.add(
                TextEdit::singleline(&mut self.query)
                    .hint_text("Search settings")
                    .desired_width(f32::INFINITY)
                    .margin(vec2(space::S4, space::S3)),
            );
        });
        ui.add_space(space::S4);
        let searching = !self.query.trim().is_empty();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(CATEGORY_WIDTH);
                for category in Category::ALL {
                    let selected = !searching && category == self.category;
                    if category_item(ui, category.label(), selected, false).clicked() {
                        self.show_category(category);
                    }
                }
            });
            ui.separator();
            // Sideways too, its bar only when the page is wider than the window (#454): a large interface scale on a
            // small screen leaves no room for the controls' column, the scale's own among them.
            ScrollArea::both()
                .id_salt("settings.page")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // The scroll area takes the row's left-to-right layout; the page reads top to bottom.
                    ui.vertical(|ui| {
                        if searching {
                            self.results(ui, settings, keymap);
                        } else if self.category == Category::Keybinds {
                            self.keybinds.show(ui, keymap);
                        } else {
                            page(ui, self.category, settings);
                        }
                    });
                });
        });
    }

    /// Every setting and keybind the search finds, by category.
    fn results(&mut self, ui: &mut Ui, settings: &mut StudioSettings, keymap: &mut Keymap) {
        let p = current_colors(ui.ctx()).palette;
        let found = search(&self.query);
        let commands: Vec<Command> = keymap
            .registry()
            .commands()
            .iter()
            .filter(|command| keybinds::matches(command, &self.query, keymap))
            .copied()
            .collect();
        if found.is_empty() && commands.is_empty() {
            ui.label(
                RichText::new(format!("No settings match \u{201C}{}\u{201D}.", self.query.trim()))
                    .size(text::BODY_LG)
                    .color(p.muted),
            );
            return;
        }
        for category in Category::ALL {
            let mut entries = found.iter().filter(|entry| entry.category == category).peekable();
            if entries.peek().is_none() {
                continue;
            }
            section_label(ui, category.label());
            for entry in entries {
                row(ui, entry, settings);
            }
            ui.add_space(space::S6);
        }
        // Keybinds last, as in the category list.
        if !commands.is_empty() {
            section_label(ui, Category::Keybinds.label());
            for (index, command) in commands.iter().enumerate() {
                let rule = super::keybinds::rule_below(index, commands.len());
                self.keybinds.command_row(ui, command, keymap, rule);
            }
        }
    }
}

/// A category's page: its title, its lead line, and its settings.
fn page(ui: &mut Ui, category: Category, settings: &mut StudioSettings) {
    let p = current_colors(ui.ctx()).palette;
    ui.label(
        RichText::new(category.label())
            .font(fonts::named(fonts::HEADING, text::TITLE))
            .color(p.text),
    );
    ui.label(RichText::new(category.lead()).size(text::BODY).color(p.muted));
    ui.add_space(space::S5);
    for entry in ENTRIES.iter().filter(|entry| entry.category == category) {
        row(ui, entry, settings);
    }
}

/// The script editor's typeface (#529): a drop-down of the bundled two and a font file; choosing a file asks Studio
/// for the system's file dialog ([`take_font_request`]).
fn code_font(ui: &mut Ui, font: &mut code::settings::CodeFont) {
    use code::settings::CodeFont;
    let mut file = false;
    let shown = egui::ComboBox::from_id_salt("settings.code.font")
        .icon(crate::theme::combo_icon)
        .selected_text(font.label())
        .width(COMBO_WIDTH)
        .show_ui(ui, |ui| {
            ui.selectable_value(font, CodeFont::JetBrainsMono, "JetBrains Mono");
            ui.selectable_value(font, CodeFont::Inter, "Inter");
            if ui
                .selectable_label(matches!(font, CodeFont::File(_)), "A font file\u{2026}")
                .clicked()
            {
                file = true;
            }
        });
    named(&shown.response, "Font");
    if file {
        ui.ctx().data_mut(|data| data.insert_temp(font_request(), true));
    }
}

/// Where the request for the font file's dialog waits for Studio ([`take_font_request`]).
fn font_request() -> egui::Id {
    egui::Id::unique("metallic.settings.code.font.browse")
}

/// Whether "A font file..." was chosen since the last call (#529).
pub(crate) fn take_font_request(ctx: &egui::Context) -> bool {
    ctx.data_mut(|data| data.remove_temp::<bool>(font_request()))
        .unwrap_or(false)
}

/// A number of the script editor's (#529), dragged or typed, in `range`.
fn code_number(ui: &mut Ui, name: &str, value: &mut f32, range: RangeInclusive<f32>, speed: f64, suffix: &str) {
    let response = ui.add(
        DragValue::new(value)
            .range(range)
            .speed(speed)
            .max_decimals(2)
            .suffix(suffix.to_owned()),
    );
    response.widget_info(|| WidgetInfo::labeled(Role::SpinButton, response.enabled(), name));
}

/// The colour of `kind` in the theme showing (#529): a swatch that opens Properties' colour picker (#416), its hex
/// beside it.
fn code_colour(ui: &mut Ui, settings: &mut StudioSettings, kind: code::settings::Kind) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let palette = settings.code.palette_mut(colors.dark);
    let mut rgb = palette.get(kind);
    let (rect, response) = ui.allocate_exact_size(vec2(36.0, 22.0), Sense::click());
    let label = format!("{} colour", kind.label());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &label));
    if ui.is_rect_visible(rect) {
        let [r, g, b] = rgb;
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::ITEM), Color32::from_rgb(r, g, b));
        let ink = if response.hovered() { p.text } else { p.border };
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(radius::ITEM),
            Stroke::new(size::BORDER, ink),
            egui::StrokeKind::Inside,
        );
    }
    let id = ui.make_persistent_id(("settings.code.colour", kind.label()));
    egui::Popup::menu(&response)
        .id(id)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            if crate::panels::color_picker::picker(ui, id.with("picker"), &mut rgb) {
                palette.set(kind, rgb);
            }
        });
    ui.label(
        RichText::new(code::settings::to_hex(rgb))
            .font(egui::FontId::monospace(text::SMALL))
            .color(p.muted),
    );
}

/// Where the request for the icon folder's dialog waits for Studio ([`take_icon_folder_request`]).
fn icon_folder_request() -> egui::Id {
    egui::Id::unique("metallic.settings.icon_folder.browse")
}

/// Asks Studio to open the system's folder dialog for the icon folder (#508): Settings' controls cannot open it.
fn request_icon_folder(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(icon_folder_request(), true));
}

/// Whether the icon folder's Browse was clicked since the last call.
pub(crate) fn take_icon_folder_request(ctx: &egui::Context) -> bool {
    ctx.data_mut(|data| data.remove_temp::<bool>(icon_folder_request()))
        .unwrap_or(false)
}

/// An uppercase section label, as the website's.
pub(crate) fn section_label(ui: &mut Ui, label: &str) {
    let p = current_colors(ui.ctx()).palette;
    ui.label(
        RichText::new(label.to_uppercase())
            .font(fonts::named(fonts::STRONG, text::LABEL))
            .extra_letter_spacing(text::LABEL_TRACKING)
            .color(p.subtle),
    );
    ui.add_space(space::S2);
}

/// One setting: its name and description on the left, its control on the right, a divider under it.
fn row(ui: &mut Ui, entry: &Entry, settings: &mut StudioSettings) {
    let p = current_colors(ui.ctx()).palette;
    ui.push_id(entry.name, |ui| {
        ui.horizontal(|ui| {
            let words = (ui.available_width() - CONTROL_WIDTH - space::S6).max(160.0);
            // The name, and its description under it (#521, which undid #501's tooltips).
            ui.allocate_ui_with_layout(vec2(words, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(words);
                ui.add_space(space::S3);
                ui.label(
                    RichText::new(entry.name)
                        .font(fonts::named(fonts::STRONG, text::BODY))
                        .color(p.text),
                );
                ui.add(Label::new(RichText::new(entry.description).size(text::SMALL).color(p.muted)).wrap());
                ui.add_space(space::S3);
            });
            // A column of fixed width, so a row never asks for more than the window has: a resizable window grows
            // to what its content takes, and content sized from the window's width would grow it every frame.
            ui.allocate_ui_with_layout(
                vec2(CONTROL_WIDTH, size::CONTROL),
                Layout::right_to_left(Align::Center),
                |ui| {
                    ui.set_width(CONTROL_WIDTH);
                    (entry.control)(ui, settings);
                },
            );
        });
        let rect = ui.min_rect();
        ui.painter().hline(
            rect.x_range(),
            ui.cursor().top() - ui.spacing().item_spacing.y / 2.0,
            Stroke::new(size::BORDER, p.border),
        );
    });
}

/// A category in the list: muted text, or for the selected one a surface and bold text, with the accent's 3 × 19 bar on
/// its left when `accent_bar` (the projects page's rail; not the Settings window's list, #601).
pub(crate) fn category_item(ui: &mut Ui, label: &str, selected: bool, accent_bar: bool) -> Response {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), size::CONTROL), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Tab, true, selected, label));
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            p.surface_hover
        } else if response.hovered() {
            p.surface
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, CornerRadius::same(radius::ITEM), fill);
        if selected && accent_bar {
            let bar = egui::Rect::from_center_size(
                pos2(rect.left() + size::ACCENT_BAR / 2.0, rect.center().y),
                vec2(size::ACCENT_BAR, size::SELECTED_BAR),
            );
            ui.painter().rect_filled(bar, CornerRadius::same(2), colors.accent);
        }
        let font = if selected {
            fonts::named(fonts::STRONG, text::BODY)
        } else {
            egui::FontId::proportional(text::BODY)
        };
        ui.painter().text(
            pos2(rect.left() + space::S5, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            font,
            if selected { p.text } else { p.muted },
        );
    }
    response
}

/// A switch: the primary button's neutral fill when on (never the accent), a bordered input when off.
fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let p = current_colors(ui.ctx()).palette;
    let (rect, mut response) = ui.allocate_exact_size(vec2(40.0, 22.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| WidgetInfo::selected(Role::CheckBox, ui.is_enabled(), *on, label));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let round = CornerRadius::same((rect.height() / 2.0) as u8);
        let (track, edge, knob) = if *on {
            (p.action, p.action, p.action_text)
        } else if response.hovered() {
            (p.input, p.subtle, p.muted)
        } else {
            (p.input, p.border, p.muted)
        };
        ui.painter().rect(
            rect,
            round,
            track,
            Stroke::new(size::BORDER, edge),
            egui::StrokeKind::Inside,
        );
        let radius = rect.height() / 2.0 - 4.0;
        let x = egui::lerp((rect.left() + radius + 4.0)..=(rect.right() - radius - 4.0), t);
        ui.painter().circle_filled(pos2(x, rect.center().y), radius, knob);
    }
    response
}

/// One of a few choices side by side, as one control: the chosen one raised on a surface.
fn segmented<T: Copy + PartialEq>(ui: &mut Ui, value: &mut T, choices: &[(T, &str)]) {
    let p = current_colors(ui.ctx()).palette;
    Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(size::BORDER, p.border))
        .corner_radius(CornerRadius::same(radius::INPUT))
        .inner_margin(2.0)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            // Laid out right to left by the row; the choices read left to right.
            for &(choice, label) in choices.iter().rev() {
                let chosen = *value == choice;
                let text = RichText::new(label)
                    .font(fonts::named(fonts::STRONG, text::BODY))
                    .color(if chosen { p.text } else { p.muted });
                let response = ui.add(
                    egui::Button::new(text)
                        .fill(if chosen { p.surface_hover } else { Color32::TRANSPARENT })
                        .stroke(Stroke::NONE)
                        .corner_radius(CornerRadius::same(radius::ITEM - 1))
                        .min_size(vec2(0.0, size::CONTROL - 6.0)),
                );
                if response.clicked() {
                    *value = choice;
                }
            }
        });
}

/// The accents as round swatches; the chosen one ringed in the text colour.
fn accent_swatches(ui: &mut Ui, accent: &mut Accent) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    ui.spacing_mut().item_spacing.x = space::S3;
    // Right to left: the custom colour first, so it reads last.
    custom_accent(ui, accent);
    for choice in Accent::ALL.into_iter().rev() {
        let (rect, response) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
        let chosen = *accent == choice;
        response.widget_info(|| WidgetInfo::selected(Role::RadioButton, true, chosen, choice.label()));
        if response.clicked() {
            *accent = choice;
        }
        let response = response.on_hover_text(choice.label());
        if ui.is_rect_visible(rect) {
            let centre = rect.center();
            let swatch = choice.on(colors.dark);
            ui.painter().circle_filled(centre, 9.0, swatch);
            let ring = if chosen {
                Some(p.text)
            } else if response.hovered() {
                Some(p.subtle)
            } else {
                None
            };
            if let Some(ring) = ring {
                ui.painter().circle_stroke(centre, 12.0, Stroke::new(2.0, ring));
            }
        }
    }
}

/// The custom accent colour (#263, #509): a round button the swatches' size, Lucide's palette on it until a colour is
/// picked and that colour after; it opens Properties' colour picker (#416), and the colour picked is the accent.
fn custom_accent(ui: &mut Ui, accent: &mut Accent) -> Response {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let custom = match *accent {
        Accent::Custom(rgb) => Some(rgb),
        _ => None,
    };
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, custom.is_some(), "Custom colour"));
    if ui.is_rect_visible(rect) {
        let centre = rect.center();
        match custom {
            Some([r, g, b]) => {
                ui.painter()
                    .circle_filled(centre, 9.0, egui::Color32::from_rgb(r, g, b));
                ui.painter().circle_stroke(centre, 12.0, Stroke::new(2.0, p.text));
            }
            None => {
                ui.painter().circle(
                    centre,
                    11.0,
                    p.input,
                    Stroke::new(size::BORDER, if response.hovered() { p.text } else { p.subtle }),
                );
                icons::paint_at(
                    ui.painter(),
                    centre,
                    egui::Align2::CENTER_CENTER,
                    Icon::UiPalette,
                    text::BODY,
                    p.text,
                );
            }
        }
    }
    let id = ui.make_persistent_id("settings.accent.picker");
    egui::Popup::menu(&response)
        .id(id)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            // From the accent there is: a preset's own colour the first time.
            let [r, g, b, _] = accent.on(true).to_array();
            let mut rgb = [r, g, b];
            if crate::panels::color_picker::picker(ui, id.with("picker"), &mut rgb) {
                *accent = Accent::Custom(rgb);
            }
        });
    response.on_hover_text("A colour of your own")
}

/// Gives a drop-down `name` for screen readers (#464): the setting it changes, which its row shows beside it.
fn named(response: &Response, name: &str) {
    response.widget_info(|| WidgetInfo::labeled(Role::ComboBox, response.enabled(), name));
}

/// A drop-down over `choices` for the setting `name`.
fn combo<T: Copy + PartialEq>(ui: &mut Ui, name: &str, value: &mut T, choices: &[(T, String)]) {
    let selected = choices
        .iter()
        .find(|(choice, _)| choice == value)
        .map_or_else(String::new, |(_, label)| label.clone());
    let shown = egui::ComboBox::from_id_salt(("settings.combo", name))
        .icon(crate::theme::combo_icon)
        .selected_text(selected)
        .width(COMBO_WIDTH)
        .show_ui(ui, |ui| {
            for (choice, label) in choices {
                ui.selectable_value(value, *choice, label);
            }
        });
    named(&shown.response, name);
}

/// A drop-down for an override: the preset's value (named) or one of `choices`.
fn override_combo<T: Copy + PartialEq>(
    ui: &mut Ui,
    name: &str,
    value: &mut Option<T>,
    preset: &str,
    choices: &[(T, &str)],
) {
    let mut all = vec![(None, format!("Preset ({preset})"))];
    all.extend(choices.iter().map(|&(choice, label)| (Some(choice), label.to_owned())));
    combo(ui, name, value, &all);
}

/// A number for the override `name`: the preset's value until it is edited, with "Reset" to go back to it.
fn override_number<N: Numeric>(
    ui: &mut Ui,
    name: &str,
    value: &mut Option<N>,
    preset: N,
    range: RangeInclusive<N>,
    speed: f64,
    suffix: &str,
) {
    let p = current_colors(ui.ctx()).palette;
    // Right to left: the trailing part first.
    if value.is_some() {
        if button(ui, ButtonKind::Quiet, "Reset").clicked() {
            *value = None;
        }
    } else {
        ui.label(RichText::new("Preset").size(text::SMALL).color(p.subtle));
    }
    let mut shown = value.unwrap_or(preset);
    let response = ui.add(
        DragValue::new(&mut shown)
            .range(range)
            .speed(speed)
            .suffix(suffix.to_owned()),
    );
    // Named for screen readers (#464), as its row is.
    response.widget_info(|| WidgetInfo::labeled(Role::SpinButton, response.enabled(), name));
    if response.changed() {
        *value = Some(shown);
    }
}

/// What the preset alone gives (Auto at its uncalibrated tier): the values an unset override shows.
fn preset_quality(graphics: &GraphicsSettings) -> Quality {
    GraphicsSettings::new(graphics.preset).quality()
}

fn mode_label(mode: ThemeMode) -> &'static str {
    match mode {
        ThemeMode::Dark => "Dark",
        ThemeMode::Light => "Light",
        ThemeMode::System => "System",
    }
}

fn preset_label(preset: Preset) -> &'static str {
    match preset {
        Preset::Low => "Low",
        Preset::Medium => "Medium",
        Preset::High => "High",
        Preset::Ultra => "Ultra",
        Preset::Auto => "Auto",
    }
}

fn tier_label(tier: Tier) -> &'static str {
    preset_label(tier.preset())
}

fn aa_label(aa: AntiAliasing) -> &'static str {
    match aa {
        AntiAliasing::Off => "Off",
        AntiAliasing::Mlaa => "MLAA",
        AntiAliasing::Msaa(2) => "MSAA 2×",
        AntiAliasing::Msaa(4) => "MSAA 4×",
        AntiAliasing::Msaa(_) => "MSAA 8×",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #601: the Settings window's selected category is marked by its surface and bold text, with no accent bar; the
    /// projects page's rail keeps its bar.
    #[test]
    fn the_selected_category_has_no_accent_bar_in_settings() {
        let ctx = egui::Context::default();
        let bars = |accent_bar: bool| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                category_item(ui, "General", true, accent_bar);
            });
            output.textures_delta.clear();
            let accent = current_colors(&ctx).accent;
            output
                .shapes
                .iter()
                .filter(|clipped| matches!(&clipped.shape, egui::Shape::Rect(rect) if rect.fill == accent))
                .count()
        };
        assert_eq!(bars(false), 0, "Settings");
        assert_eq!(bars(true), 1, "the projects page");
    }

    fn names(found: &[&Entry]) -> Vec<&'static str> {
        found.iter().map(|entry| entry.name).collect()
    }

    #[test]
    fn the_search_finds_settings_by_name_and_description() {
        assert!(search("").is_empty());
        assert!(search("   ").is_empty());
        assert_eq!(names(&search("contrast")), ["High contrast"]);
        assert_eq!(names(&search("CONTRAST")), ["High contrast"], "any letter case");
        // From the description: "Turns off fades and eases."
        assert_eq!(names(&search("fades")), ["Reduced motion"]);
        // Every word must match, in any order.
        assert_eq!(names(&search("shadow metres")), ["Shadow distance"]);
        assert!(names(&search("shadow")).contains(&"Shadow quality"));
        assert!(search("teleport").is_empty());
        // Spelt as the owner asked (#318).
        assert_eq!(names(&search("accent")), ["Accent Color"]);
        assert_eq!(names(&search("color")), ["Accent Color"]);
    }

    #[test]
    fn a_narrow_window_scrolls_its_page_sideways_to_the_controls() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;

        use crate::keymap::Overrides;

        struct Open {
            window: SettingsWindow,
            settings: StudioSettings,
            keymap: Keymap,
        }
        let mut window = SettingsWindow::default();
        window.show_category(Category::Appearance);
        let open = Open {
            window,
            settings: StudioSettings::default(),
            keymap: Keymap::studio(&Overrides::new()),
        };
        let width = 480.0;
        let mut harness = Harness::builder()
            .with_size(vec2(width, 400.0))
            .with_context_setup(crate::theme::fonts::install)
            .build_ui_state(
                |ui, open: &mut Open| open.window.contents(ui, &mut open.settings, &mut open.keymap),
                open,
            );
        harness.run();
        // The Appearance page's first drop-down, at the right of its row.
        let control = |harness: &Harness<'_, Open>| harness.get_all_by_role(Role::ComboBox).next().unwrap().rect();
        assert!(
            control(&harness).right() > width,
            "past the window: {:?}",
            control(&harness)
        );
        // Scrolled sideways over the page, it comes into view (#454).
        harness.hover_at(egui::pos2(width - 40.0, 200.0));
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(-600.0, 0.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run();
        assert!(control(&harness).right() <= width, "{:?}", control(&harness));
    }

    #[test]
    fn every_page_s_controls_have_accessible_names() {
        use egui_kittest::Harness;

        use crate::keymap::Overrides;

        // kittest's accessibility check fails a page with an unnamed control (#464: the drop-downs were).
        for category in Category::ALL {
            let mut window = SettingsWindow::default();
            window.show_category(category);
            let mut harness = Harness::builder()
                .with_size(vec2(900.0, 2000.0))
                .with_context_setup(crate::theme::fonts::install)
                .build_ui_state(
                    |ui, (window, settings, keymap): &mut (SettingsWindow, StudioSettings, Keymap)| {
                        window.contents(ui, settings, keymap);
                    },
                    (window, StudioSettings::default(), Keymap::studio(&Overrides::new())),
                );
            harness.run();
        }
    }

    /// #578: the window of its own opens where it was, at the size it had, or centred over Studio the first time.
    #[test]
    fn the_window_of_its_own_opens_where_it_was() {
        let main = egui::Rect::from_min_size(pos2(100.0, 50.0), vec2(1280.0, 800.0));
        let first = opening(None, Some(main), 1.0);
        assert_eq!(first.position, Some(main.center() - SIZE / 2.0));
        assert_eq!((first.size, first.maximized), (SIZE, false));
        assert_eq!(
            opening(None, None, 1.0).position,
            None,
            "the system's place, without Studio's"
        );
        // Kept in physical pixels, opened in points.
        let place = WindowPlace {
            x: -1500,
            y: 300,
            width: 1000,
            height: 750,
            maximized: true,
        };
        let again = opening(Some(place), Some(main), 1.25);
        assert_eq!(again.position, Some(pos2(-1200.0, 240.0)));
        assert_eq!((again.size, again.maximized), (vec2(800.0, 600.0), true));
        // Never smaller than it may be made, nor scaled by nothing.
        let small = WindowPlace {
            width: 400,
            height: 400,
            ..place
        };
        assert_eq!(opening(Some(small), None, 1.0).size, vec2(MIN_SIZE.x, 400.0));
        assert_eq!(opening(Some(place), None, 0.0).position, Some(pos2(-1500.0, 300.0)));
    }

    /// #578: where the window of its own is, as its viewport says, in physical pixels.
    #[test]
    fn the_window_of_its_own_s_place_is_noted_from_its_viewport() {
        let info = |outer: egui::Rect, maximized: bool, minimized: bool| egui::ViewportInfo {
            // The frame's corner, and the inside below its title bar.
            outer_rect: Some(outer),
            inner_rect: Some(egui::Rect::from_min_max(
                outer.min + vec2(8.0, 30.0),
                outer.max - vec2(8.0, 8.0),
            )),
            maximized: Some(maximized),
            minimized: Some(minimized),
            ..egui::ViewportInfo::default()
        };
        let normal = egui::Rect::from_min_size(pos2(-400.0, 80.0), vec2(816.0, 668.0));
        let noted = noted_place(&info(normal, false, false), 1.5, None);
        let place = WindowPlace {
            x: -600,
            y: 120,
            width: 1200,
            height: 945,
            maximized: false,
        };
        assert_eq!(noted, Some(place));
        // Not open yet, or minimised: as it was.
        assert_eq!(noted_place(&egui::ViewportInfo::default(), 1.5, noted), noted);
        assert_eq!(noted_place(&info(normal, false, true), 1.5, noted), noted);
        // Maximised: the place to go back to, marked maximised.
        let screen = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
        let maximized = noted_place(&info(screen, true, false), 1.5, noted);
        assert_eq!(
            maximized,
            Some(WindowPlace {
                maximized: true,
                ..place
            })
        );
    }

    /// #578: floating, it opens where it was in Studio's window, or centred in it.
    #[test]
    fn the_floating_window_opens_where_it_was() {
        let content = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 1000.0));
        assert_eq!(floating_place(None, content, 1.0), (pos2(800.0, 500.0), SIZE));
        let place = place_of(pos2(40.0, 60.0), vec2(700.0, 500.0), 2.0);
        assert_eq!((place.x, place.y, place.width, place.height), (80, 120, 1400, 1000));
        assert_eq!(
            floating_place(Some(place), content, 2.0),
            (pos2(390.0, 310.0), vec2(700.0, 500.0))
        );
    }

    #[test]
    fn every_category_but_keybinds_has_settings() {
        for category in Category::ALL {
            let count = ENTRIES.iter().filter(|entry| entry.category == category).count();
            assert_eq!(count == 0, category == Category::Keybinds, "{category:?}");
        }
        // No Editing page (#410): the increments are in the toolbar, the camera speeds on the wheel.
        assert!(Category::ALL.iter().all(|category| category.label() != "Editing"));
        let mut seen = std::collections::HashSet::new();
        for entry in ENTRIES {
            assert!(seen.insert(entry.name), "{} twice", entry.name);
            assert!(entry.description.ends_with('.'), "{}", entry.name);
        }
    }
}
