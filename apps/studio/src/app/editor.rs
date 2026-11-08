//! The Editor screen's own pieces: the progress overlay while a project opens, the top bar with its menus, and the
//! About window.
//!
//! The menu bar is Roblox Studio's (#239): **File, Edit, View, Plugins, Test, Window, Help**, listed as data in
//! [`MENUS`]. Items that work act through [`MenuAction`]s the app carries out, the same as the keymap's commands, and
//! show every binding of their command from the keymap. Items for features still to come are shown disabled
//! ([`Entry::Planned`]); enabling one is changing its line in [`MENUS`].

use egui::{
    Align, Align2, Color32, CornerRadius, Frame, Id, Layout, Rect, Response, RichText, Role, Sense, Stroke, Ui,
    UiBuilder, WidgetInfo, pos2, vec2,
};

use crate::dock::Dock;
use crate::editor::{EditorState, STEPS};
use crate::keymap::{Keymap, ids};
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, shadow, size, space, text};
use crate::theme::{ButtonKind, button, current_colors, fonts};

/// The top bar's height.
pub(crate) const BAR_HEIGHT: f32 = 40.0;
/// The narrowest a menu is (#276: otherwise as wide as its longest item and shortcut need).
const MENU_MIN_WIDTH: f32 = 150.0;
/// The space between an item's label and its shortcut.
const SHORTCUT_GAP: f32 = space::S8;
/// A menu item's height.
const ITEM_HEIGHT: f32 = 28.0;
/// A menu title's height in the bar.
const TITLE_HEIGHT: f32 = 28.0;
/// The progress card's width.
const CARD_WIDTH: f32 = 420.0;
/// The About window's width.
const ABOUT_WIDTH: f32 = 360.0;

/// What a menu item asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuAction {
    /// File, Save.
    Save,
    /// File, Close project.
    Close,
    /// View: open or close a panel.
    Toggle(&'static str),
    /// Window, Reset layout.
    ResetLayout,
    /// File or Window, Settings, and the gear: open or close the Settings window.
    Settings,
    /// Help, Keyboard shortcuts: the Settings window on its Keybinds page.
    KeyboardShortcuts,
    /// Help, About Metallic Studio.
    About,
    /// Test, Play: start a game session in the Game tab.
    Play,
    /// Test, Pause: pause or resume the game.
    Pause,
    /// Test, Step: one frame of the paused game.
    Step,
    /// Test, Stop: end the game session.
    Stop,
}

/// Where a game session is, for what the menus and the play buttons allow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PlayState {
    /// A session runs (paused or not).
    pub playing: bool,
    /// The session's game is paused.
    pub paused: bool,
}

impl PlayState {
    /// The state of `editor`'s session.
    pub(crate) fn of(editor: &EditorState) -> Self {
        Self {
            playing: editor.is_playing(),
            paused: editor.is_paused(),
        }
    }

    /// Whether `action` can run now: Play without a session; Pause and Stop with one; Step while paused; Save only
    /// without one (ADR 0010 §3). Everything else always.
    pub(crate) fn allows(self, action: MenuAction) -> bool {
        match action {
            MenuAction::Play | MenuAction::Save => !self.playing,
            MenuAction::Pause | MenuAction::Stop => self.playing,
            MenuAction::Step => self.paused,
            _ => true,
        }
    }
}

/// One line of a menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    /// An item that works: `label`, doing `action`, with the bindings of keymap command `command` beside it.
    Item {
        /// Its name.
        label: &'static str,
        /// What it does.
        action: MenuAction,
        /// The keymap command it is, whose bindings it shows.
        command: Option<&'static str>,
    },
    /// An item for a feature still to come, shown disabled.
    Planned(&'static str),
    /// A check item for each of the dock's panels, opening or closing it.
    Panels,
    /// A line between groups of items.
    Separator,
}

/// A menu in the bar: its title and its lines.
#[derive(Debug)]
pub(crate) struct Menu {
    /// The name in the bar.
    pub title: &'static str,
    /// Its lines, top to bottom.
    pub entries: &'static [Entry],
}

/// An item that works.
const fn item(label: &'static str, action: MenuAction, command: Option<&'static str>) -> Entry {
    Entry::Item { label, action, command }
}

/// The menu bar, left to right, as in Roblox Studio.
pub(crate) static MENUS: &[Menu] = &[
    Menu {
        title: "File",
        entries: &[
            Entry::Planned("New project"),
            Entry::Planned("Open project"),
            Entry::Separator,
            item("Save", MenuAction::Save, Some(ids::FILE_SAVE)),
            Entry::Planned("Save as"),
            Entry::Separator,
            item("Settings", MenuAction::Settings, Some(ids::SETTINGS_OPEN)),
            Entry::Separator,
            item("Close project", MenuAction::Close, Some(ids::PROJECT_CLOSE)),
        ],
    },
    Menu {
        title: "Edit",
        entries: &[
            Entry::Planned("Undo"),
            Entry::Planned("Redo"),
            Entry::Separator,
            Entry::Planned("Cut"),
            Entry::Planned("Copy"),
            Entry::Planned("Paste"),
            Entry::Planned("Duplicate"),
            Entry::Planned("Delete"),
            Entry::Separator,
            Entry::Planned("Select all"),
        ],
    },
    Menu {
        title: "View",
        entries: &[Entry::Panels],
    },
    Menu {
        title: "Plugins",
        entries: &[Entry::Planned("Manage plugins"), Entry::Planned("Plugins folder")],
    },
    Menu {
        title: "Test",
        entries: &[
            item("Play", MenuAction::Play, Some(ids::TEST_PLAY)),
            Entry::Planned("Play here"),
            Entry::Planned("Run"),
            Entry::Separator,
            item("Pause", MenuAction::Pause, Some(ids::TEST_PAUSE)),
            item("Step", MenuAction::Step, Some(ids::TEST_STEP)),
            Entry::Separator,
            item("Stop", MenuAction::Stop, Some(ids::TEST_STOP)),
        ],
    },
    Menu {
        title: "Window",
        entries: &[
            item("Reset layout", MenuAction::ResetLayout, Some(ids::VIEW_RESET_LAYOUT)),
            Entry::Separator,
            item("Settings", MenuAction::Settings, Some(ids::SETTINGS_OPEN)),
        ],
    },
    Menu {
        title: "Help",
        entries: &[
            // Studio does not open links yet.
            Entry::Planned("Documentation"),
            item("Keyboard shortcuts", MenuAction::KeyboardShortcuts, None),
            Entry::Planned("Report a bug"),
            Entry::Separator,
            item("About Metallic Studio", MenuAction::About, None),
        ],
    },
];

/// Where the menu titles were drawn last frame, by title, in the context's temporary data under this id (for tests
/// and screenshots that click them).
pub(crate) fn menu_titles_id() -> Id {
    Id::unique("metallic.menu_titles")
}

/// The progress overlay while the project `name` opens, at step `step` (from 0) of [`STEPS`].
pub(crate) fn opening_overlay(ui: &mut Ui, name: &str, step: usize) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let screen = ui.max_rect();
    ui.painter().rect_filled(screen, 0.0, p.bg);
    let card = Rect::from_center_size(screen.center(), vec2(CARD_WIDTH, 168.0));
    ui.scope_builder(UiBuilder::new().max_rect(card), |ui| {
        Frame::new()
            .fill(p.panel)
            .stroke(Stroke::new(size::BORDER, p.border))
            .corner_radius(CornerRadius::same(radius::PANEL))
            .shadow(shadow::MENU)
            .inner_margin(space::S8)
            .show(ui, |ui| {
                ui.set_width(CARD_WIDTH - 2.0 * space::S8);
                ui.label(
                    RichText::new("OPENING")
                        .font(fonts::named(fonts::STRONG, text::LABEL))
                        .extra_letter_spacing(text::LABEL_TRACKING)
                        .color(p.subtle),
                );
                ui.add_space(space::S2);
                ui.add(
                    egui::Label::new(
                        RichText::new(name)
                            .font(fonts::named(fonts::HEADING, text::TITLE + 4.0))
                            .extra_letter_spacing(text::DISPLAY_TRACKING / 2.0)
                            .color(p.text),
                    )
                    .truncate(),
                );
                ui.add_space(space::S5);
                let step = step.min(STEPS.len() - 1);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(STEPS[step]).size(text::BODY).color(p.muted));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{} of {}", step + 1, STEPS.len()))
                                .size(text::SMALL)
                                .color(p.subtle),
                        );
                    });
                });
                ui.add_space(space::S4);
                // The 2 px accent bar, filled to the steps done.
                let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 2.0), Sense::hover());
                ui.painter().rect_filled(bar, CornerRadius::same(1), p.border);
                let done = (step + 1) as f32 / STEPS.len() as f32;
                let filled = Rect::from_min_size(bar.min, vec2(bar.width() * done, bar.height()));
                ui.painter().rect_filled(filled, CornerRadius::same(1), colors.accent);
            });
    });
    // Polls the worker until it is done.
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
}

/// The top bar: the menus ([`MENUS`]), the play controls and the gear (raised while the Settings window is
/// open, `settings_open`). The project's name is the window's title ([`super::Studio::window_title`]). Returns what was
/// asked for.
pub(crate) fn top_bar(
    ui: &mut Ui,
    editor: &EditorState,
    dock: &Dock,
    keymap: &Keymap,
    settings_open: bool,
) -> Option<MenuAction> {
    let p = current_colors(ui.ctx()).palette;
    let rect = ui.max_rect();
    ui.painter().rect_filled(rect, 0.0, p.header);
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - size::BORDER / 2.0,
        Stroke::new(size::BORDER, p.border),
    );
    let mut action = None;
    let play = PlayState::of(editor);
    let inner = rect.shrink2(vec2(space::S5, 0.0));
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            // No mark here (#299): the window's title bar shows it.
            // A menu bar takes all the width it is given; it gets the titles' only, so the rest of the bar follows it.
            let title_font = egui::FontId::proportional(text::BODY);
            let widths: Vec<f32> = MENUS
                .iter()
                .map(|menu| {
                    let galley = ui
                        .painter()
                        .layout_no_wrap(menu.title.to_owned(), title_font.clone(), p.text);
                    galley.size().x + 2.0 * space::S4
                })
                .collect();
            let menus_width = widths.iter().sum::<f32>() + space::S1 * MENUS.len() as f32;
            let mut titles = Vec::with_capacity(MENUS.len());
            ui.allocate_ui_with_layout(
                vec2(menus_width, BAR_HEIGHT),
                Layout::left_to_right(Align::Center),
                |ui| {
                    egui::MenuBar::new().ui(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = space::S1;
                        for (menu, width) in MENUS.iter().zip(&widths) {
                            let title = egui::Button::new(RichText::new(menu.title).font(title_font.clone()))
                                .min_size(vec2(*width, TITLE_HEIGHT))
                                .corner_radius(CornerRadius::same(radius::ITEM));
                            let (response, _) = egui::containers::menu::MenuButton::from_button(title).ui(ui, |ui| {
                                let width = menu_width(ui, menu, dock, keymap);
                                ui.set_min_width(width);
                                ui.set_max_width(width);
                                if let Some(chosen) = menu_entries(ui, menu, dock, keymap, play, width) {
                                    action = Some(chosen);
                                }
                            });
                            titles.push((menu.title, response.rect));
                        }
                    });
                },
            );
            ui.ctx().data_mut(|data| data.insert_temp(menu_titles_id(), titles));
            // The project's name is the window's title (#254), not the bar's.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let tip = match keymap.shortcut_text(ids::SETTINGS_OPEN) {
                    Some(keys) => format!("Settings ({keys})"),
                    None => "Settings".to_owned(),
                };
                if gear(ui, settings_open).on_hover_text(tip).clicked() {
                    action = Some(MenuAction::Settings);
                }
                ui.add_space(space::S4);
                let (divider, _) = ui.allocate_exact_size(vec2(size::BORDER, TITLE_HEIGHT - 8.0), Sense::hover());
                ui.painter().rect_filled(divider, CornerRadius::ZERO, p.border);
                ui.add_space(space::S4);
                // Right to left: the buttons read Play, Pause, Step, Stop.
                ui.spacing_mut().item_spacing.x = space::S1;
                let pause = if play.paused {
                    (Icon::ToolbarPlay, "Resume")
                } else {
                    (Icon::ToolbarPause, "Pause")
                };
                for (kind, glyph, label, command) in [
                    (MenuAction::Stop, Icon::ToolbarStop, "Stop", ids::TEST_STOP),
                    (MenuAction::Step, Icon::ToolbarStep, "Step", ids::TEST_STEP),
                    (MenuAction::Pause, pause.0, pause.1, ids::TEST_PAUSE),
                    (MenuAction::Play, Icon::ToolbarPlay, "Play", ids::TEST_PLAY),
                ] {
                    let tip = match keymap.shortcut_text(command) {
                        Some(keys) => format!("{label} ({keys})"),
                        None => label.to_owned(),
                    };
                    let look = match kind {
                        MenuAction::Play => Control::Play,
                        MenuAction::Stop => Control::Stop,
                        _ => Control::Plain,
                    };
                    if play_button(ui, glyph, label, play.allows(kind), look)
                        .on_hover_text(tip)
                        .on_disabled_hover_text(label)
                        .clicked()
                    {
                        action = Some(kind);
                    }
                }
            });
        },
    );
    action
}

/// How a play control's icon is drawn (#302): the theme's, which the toolbar's play tools share (#325).
pub(crate) use crate::theme::PlayControl as Control;

/// A play control in the top bar (#302): `glyph` in a square button labelled `label`, drawn as `look` says, with no
/// fill but the hover's; greyed out (the website's disabled strength) and inert unless `enabled`.
fn play_button(ui: &mut Ui, glyph: Icon, label: &str, enabled: bool, look: Control) -> Response {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(TITLE_HEIGHT + 4.0, TITLE_HEIGHT), sense);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    if ui.is_rect_visible(rect) {
        if enabled && response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface_hover);
        }
        // The website's disabled strength: 48 %.
        let ink = if enabled {
            look.ink(&colors)
        } else {
            p.text.gamma_multiply(0.48)
        };
        let icon = Rect::from_center_size(rect.center(), vec2(text::BODY_LG, text::BODY_LG));
        if look.filled() {
            icons::paint_filled(ui.painter(), glyph, icon, ink);
        } else {
            icons::paint(ui.painter(), glyph, icon, ink);
        }
    }
    response
}

/// The gear that opens Settings; raised while Settings is open.
fn gear(ui: &mut Ui, open: bool) -> Response {
    let p = current_colors(ui.ctx()).palette;
    let (rect, response) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, open, "Settings"));
    let fill = if open || response.hovered() {
        p.surface_hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(radius::ITEM), fill);
    icons::paint_at(
        ui.painter(),
        rect.center(),
        Align2::CENTER_CENTER,
        Icon::ToolbarSettings,
        text::BODY_LG + 2.0,
        if open || response.hovered() { p.text } else { p.muted },
    );
    response
}

/// Draws `menu`'s lines into its open popup. Returns what was chosen.
fn menu_entries(
    ui: &mut Ui,
    menu: &Menu,
    dock: &Dock,
    keymap: &Keymap,
    play: PlayState,
    width: f32,
) -> Option<MenuAction> {
    let mut chosen = None;
    ui.spacing_mut().item_spacing.y = 2.0;
    for entry in menu.entries {
        match *entry {
            Entry::Item { label, action, command } => {
                let keys = command.and_then(|id| keymap.shortcut_text(id));
                let label = if action == MenuAction::Pause && play.paused {
                    "Resume"
                } else {
                    label
                };
                if menu_item(ui, width, label, keys, None, play.allows(action)).clicked() {
                    chosen = Some(action);
                }
            }
            Entry::Planned(label) => {
                menu_item(ui, width, label, None, None, false).on_hover_text("Coming in a later version of Studio.");
            }
            Entry::Panels => {
                for panel in dock.panels().iter().filter(|panel| !panel.transient()) {
                    let open = dock.is_open(panel.id());
                    if menu_item(ui, width, panel.title(), None, Some(open), true).clicked() {
                        chosen = Some(MenuAction::Toggle(panel.id()));
                    }
                }
            }
            Entry::Separator => {
                ui.separator();
            }
        }
    }
    chosen
}

/// How wide `menu` is (#276): its longest line, the check column (for the panels' list), the label, the gap and the
/// shortcut, with the item's padding on both sides; never less than [`MENU_MIN_WIDTH`].
fn menu_width(ui: &Ui, menu: &Menu, dock: &Dock, keymap: &Keymap) -> f32 {
    let painter = ui.painter();
    let measure = |text: &str, size: f32| {
        painter
            .layout_no_wrap(text.to_owned(), egui::FontId::proportional(size), Color32::WHITE)
            .size()
            .x
    };
    let mut widest: f32 = 0.0;
    let mut line = |label: &str, shortcut: Option<String>, check: bool| {
        let check = if check { text::BODY + space::S3 } else { 0.0 };
        let keys = shortcut.map_or(0.0, |keys| SHORTCUT_GAP + measure(&keys, text::SMALL));
        widest = widest.max(check + measure(label, text::BODY) + keys);
    };
    for entry in menu.entries {
        match *entry {
            Entry::Item { label, action, command } => {
                let keys = command.and_then(|id| keymap.shortcut_text(id));
                line(label, keys.clone(), false);
                if action == MenuAction::Pause {
                    // Pause reads Resume while paused: the menu keeps one width.
                    line("Resume", keys, false);
                }
            }
            Entry::Planned(label) => line(label, None, false),
            Entry::Panels => {
                for panel in dock.panels().iter().filter(|panel| !panel.transient()) {
                    line(panel.title(), None, true);
                }
            }
            Entry::Separator => {}
        }
    }
    (widest + 2.0 * space::S4).ceil().max(MENU_MIN_WIDTH)
}

/// A menu item `width` wide: a check mark when `checked` is `Some(true)`, the label, and the shortcut on the right. A click closes
/// the menu. Not `enabled`, it is drawn at the theme's disabled strength and does nothing.
fn menu_item(
    ui: &mut Ui,
    width: f32,
    label: &str,
    shortcut: Option<String>,
    checked: Option<bool>,
    enabled: bool,
) -> Response {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    // The menu's width, measured from its items: a popup's available width is not its own, and items sized from it
    // widen the menu every frame.
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(width, ITEM_HEIGHT), sense);
    response.widget_info(|| match checked {
        Some(on) => WidgetInfo::selected(Role::MenuItemCheckBox, enabled, on, label),
        None => WidgetInfo::labeled(Role::MenuItem, enabled, label),
    });
    // The website's disabled strength: 48 %.
    let ink = if enabled { p.text } else { p.text.gamma_multiply(0.48) };
    if ui.is_rect_visible(rect) {
        if enabled && response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface_hover);
        }
        let mut x = rect.left() + space::S4;
        if checked.is_some() {
            if checked == Some(true) {
                icons::paint_at(
                    ui.painter(),
                    pos2(x, rect.center().y),
                    Align2::LEFT_CENTER,
                    Icon::UiCheck,
                    text::BODY,
                    colors.accent,
                );
            }
            x += text::BODY + space::S3;
        }
        ui.painter().text(
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(text::BODY),
            ink,
        );
        if let Some(keys) = shortcut {
            ui.painter().text(
                pos2(rect.right() - space::S4, rect.center().y),
                Align2::RIGHT_CENTER,
                keys,
                egui::FontId::proportional(text::SMALL),
                p.subtle,
            );
        }
    }
    if response.clicked() {
        ui.close();
    }
    response
}

/// The About window while `open`: the mark, Studio's name and version, and a Close button.
pub(crate) fn about_window(ctx: &egui::Context, open: &mut bool, mark: Option<&egui::TextureHandle>) {
    if !*open {
        return;
    }
    let p = current_colors(ctx).palette;
    let mut keep = true;
    let mut close = false;
    egui::Window::new("About Metallic Studio")
        .id(Id::unique("metallic.about"))
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        // Its own header, with Lucide's close button (#278), in place of egui's title bar.
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .default_width(ABOUT_WIDTH)
        .show(ctx, |ui| {
            ui.set_width(ABOUT_WIDTH);
            crate::theme::window_header(ui, "About Metallic Studio", &mut keep);
            ui.vertical_centered(|ui| {
                ui.add_space(space::S4);
                if let Some(mark) = mark {
                    ui.add(egui::Image::new((mark.id(), egui::Vec2::splat(ABOUT_MARK))));
                    ui.add_space(space::S4);
                }
                ui.label(
                    RichText::new("Metallic Studio")
                        .font(fonts::named(fonts::HEADING, text::TITLE))
                        .color(p.text),
                );
                ui.label(
                    RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                        .size(text::BODY)
                        .color(p.muted),
                );
                ui.add_space(space::S3);
                ui.label(
                    RichText::new("The editor of the Metallic game engine and platform.")
                        .size(text::SMALL)
                        .color(p.subtle),
                );
                ui.add_space(space::S5);
                close = button(ui, ButtonKind::Secondary, "Close").clicked();
                ui.add_space(space::S2);
            });
        });
    *open = keep && !close;
}

/// The mark's size in the About window, in points.
pub(crate) const ABOUT_MARK: f32 = 56.0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Overrides;

    #[test]
    fn the_menus_are_roblox_studios_and_name_real_commands() {
        let titles: Vec<&str> = MENUS.iter().map(|menu| menu.title).collect();
        assert_eq!(titles, ["File", "Edit", "View", "Plugins", "Test", "Window", "Help"]);
        let keymap = Keymap::studio(&Overrides::new());
        for menu in MENUS {
            assert!(!menu.entries.is_empty(), "{} is empty", menu.title);
            assert_ne!(menu.entries.first(), Some(&Entry::Separator), "{}", menu.title);
            assert_ne!(menu.entries.last(), Some(&Entry::Separator), "{}", menu.title);
            for entry in menu.entries {
                if let Entry::Item {
                    label,
                    command: Some(id),
                    ..
                } = entry
                {
                    assert!(
                        keymap.registry().commands().iter().any(|command| command.id == *id),
                        "{}: {label} names `{id}`, which is not a command",
                        menu.title
                    );
                }
            }
        }
    }
}
