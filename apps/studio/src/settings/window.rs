//! The Settings window (ADR 0009, Settings): a category list on the left, a search box over every setting, and each
//! setting as a row with its name, a line saying what it does, and its control.
//!
//! A floating window over every screen, the editor's included (#238): moved, resized and closed like any window,
//! and never docked. Every setting is one [`Entry`] in [`ENTRIES`], so the pages and the search read the same list,
//! and adding a setting is one line there plus its control.

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
    ui_scale,
};
use crate::keymap::{Command, Keymap};
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
    /// The keymap: its own page ([`KeybindsPage`]), not [`ENTRIES`].
    Keybinds,
}

impl Category {
    /// Every category, in the order the window lists them.
    pub const ALL: [Category; 4] = [
        Category::General,
        Category::Appearance,
        Category::Rendering,
        Category::Keybinds,
    ];

    /// The name people see.
    pub fn label(self) -> &'static str {
        match self {
            Category::General => "General",
            Category::Appearance => "Appearance",
            Category::Rendering => "Rendering",
            Category::Keybinds => "Keybinds",
        }
    }

    /// The line under the page's title.
    fn lead(self) -> &'static str {
        match self {
            Category::General => "How Studio starts and behaves.",
            Category::Appearance => "Changes apply at once.",
            Category::Rendering => "How the editor's viewport draws your scenes. The game's own look is never changed.",
            Category::Keybinds => "Click a shortcut to change it. Right-click to add or remove one.",
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
        name: "Accent colour",
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
            egui::ComboBox::from_id_salt("settings.ui_scale")
                .selected_text(percent(*scale))
                .width(COMBO_WIDTH)
                .show_ui(ui, |ui| {
                    for choice in ui_scale::CHOICES {
                        ui.selectable_value(scale, choice, percent(choice));
                    }
                });
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
        category: Category::Rendering,
        name: "Quality preset",
        description: "How much a viewport frame may cost. Auto measures this machine and holds the frame time.",
        control: |ui, s| {
            let preset = &mut s.rendering.preset;
            egui::ComboBox::from_id_salt("settings.preset")
                .selected_text(preset_label(*preset))
                .width(COMBO_WIDTH)
                .show_ui(ui, |ui| {
                    for &choice in Preset::ALL {
                        ui.selectable_value(preset, choice, preset_label(choice));
                    }
                });
        },
    },
    Entry {
        category: Category::Rendering,
        name: "Anti-aliasing",
        description: "Smooths jagged edges. MSAA falls back to what the GPU has.",
        control: |ui, s| {
            let preset = aa_label(preset_quality(&s.rendering).anti_aliasing);
            let choices = AntiAliasing::ALL.map(|aa| (aa, aa_label(aa)));
            override_combo(ui, "settings.aa", &mut s.rendering.anti_aliasing, preset, &choices);
        },
    },
    Entry {
        category: Category::Rendering,
        name: "VSync",
        description: "Waits for the display between frames, so the image never tears.",
        control: |ui, s| {
            let choices = [(true, "On"), (false, "Off")];
            let preset = if PRESET_VSYNC { "On" } else { "Off" };
            override_combo(ui, "settings.vsync", &mut s.rendering.vsync, preset, &choices);
        },
    },
    Entry {
        category: Category::Rendering,
        name: "Frame cap",
        description: "The most frames per second the viewport draws; 0 for no limit.",
        control: |ui, s| {
            override_number(
                ui,
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
            override_number(ui, &mut s.rendering.target_frame_ms, preset, range, 0.1, " ms");
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
            override_number(ui, &mut s.rendering.view_distance, preset, range, 10.0, " m");
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
            override_number(ui, &mut s.rendering.shadow_distance, preset, range, 1.0, " m");
        },
    },
    Entry {
        category: Category::Rendering,
        name: "Light budget",
        description: "How many local lights a frame draws at full strength.",
        control: |ui, s| {
            let preset = preset_quality(&s.rendering).lights.budget;
            let range = 1..=MAX_GPU_LIGHTS as u32;
            override_number(ui, &mut s.rendering.light_budget, preset, range, 1.0, "");
        },
    },
    Entry {
        category: Category::Rendering,
        name: "Level of detail",
        description: "Higher keeps meshes finer further away.",
        control: |ui, s| {
            let preset = preset_quality(&s.rendering).lod.bias;
            let range = LOD_BIAS.0..=LOD_BIAS.1;
            override_number(ui, &mut s.rendering.lod_bias, preset, range, 0.01, " ×");
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
];

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
        }
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

    /// Whether the keybind capture dialog is open: Studio does not run the keymap meanwhile.
    pub fn is_capturing(&self) -> bool {
        self.keybinds.is_capturing()
    }

    /// The keybind capture dialog, while a capture is going on, over `ctx`. Studio calls it every frame, whether the
    /// pages are in the window or docked.
    pub fn dialog(&mut self, ctx: &egui::Context, keymap: &mut Keymap) {
        self.keybinds.dialog(ctx, keymap);
    }

    /// Draws the window, if open, over `ctx`, changing `settings` and the `keymap` as the user does.
    pub fn show(&mut self, ctx: &egui::Context, settings: &mut StudioSettings, keymap: &mut Keymap) {
        if !self.open {
            return;
        }
        let mut open = true;
        let shown = egui::Window::new("Settings")
            .id(Id::unique("metallic.settings"))
            // Opens centred on the window; where it is moved to is egui's to remember.
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([780.0, 640.0])
            .min_width(680.0)
            .min_height(360.0)
            .show(ctx, |ui| self.contents(ui, settings, keymap));
        self.rect = shown.map(|shown| shown.response.rect);
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
        }
    }

    /// The pages, filling `ui`: the search box, the category list and the page or the search's results.
    pub fn contents(&mut self, ui: &mut Ui, settings: &mut StudioSettings, keymap: &mut Keymap) {
        let p = current_colors(ui.ctx()).palette;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(fonts::icon::SEARCH.to_string())
                    .font(fonts::named(fonts::ICONS, text::BODY_LG))
                    .color(p.subtle),
            );
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
                    if category_item(ui, category.label(), selected).clicked() {
                        self.show_category(category);
                    }
                }
            });
            ui.separator();
            ScrollArea::vertical()
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
            for command in &commands {
                if let Some((id, slot)) = keybinds::command_row(ui, command, keymap) {
                    self.keybinds.capture(id, slot);
                }
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

/// A category in the list: muted text, or for the selected one a surface with the accent's 3 × 19 bar on its left.
/// The projects page's rail uses it too.
pub(crate) fn category_item(ui: &mut Ui, label: &str, selected: bool) -> Response {
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
        if selected {
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

/// A drop-down over `choices`.
fn combo<T: Copy + PartialEq>(ui: &mut Ui, id: &str, value: &mut T, choices: &[(T, String)]) {
    let selected = choices
        .iter()
        .find(|(choice, _)| choice == value)
        .map_or_else(String::new, |(_, label)| label.clone());
    egui::ComboBox::from_id_salt(id)
        .selected_text(selected)
        .width(COMBO_WIDTH)
        .show_ui(ui, |ui| {
            for (choice, label) in choices {
                ui.selectable_value(value, *choice, label);
            }
        });
}

/// A drop-down for an override: the preset's value (named) or one of `choices`.
fn override_combo<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: &str,
    value: &mut Option<T>,
    preset: &str,
    choices: &[(T, &str)],
) {
    let mut all = vec![(None, format!("Preset ({preset})"))];
    all.extend(choices.iter().map(|&(choice, label)| (Some(choice), label.to_owned())));
    combo(ui, id, value, &all);
}

/// A number for an override: the preset's value until it is edited, with "Reset" to go back to it.
fn override_number<N: Numeric>(
    ui: &mut Ui,
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
    }

    #[test]
    fn every_category_but_keybinds_has_settings() {
        for category in Category::ALL {
            let count = ENTRIES.iter().filter(|entry| entry.category == category).count();
            assert_eq!(count == 0, category == Category::Keybinds, "{category:?}");
        }
        let mut seen = std::collections::HashSet::new();
        for entry in ENTRIES {
            assert!(seen.insert(entry.name), "{} twice", entry.name);
            assert!(entry.description.ends_with('.'), "{}", entry.name);
        }
    }
}
