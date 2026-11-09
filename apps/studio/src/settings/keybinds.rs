//! The Keybinds page of the Settings window (ADR 0009, Keybinds): every command in nested sections, searchable by
//! name, key or description. Click a shortcut to change it, + to add one, right-click to remove one, reset one, a
//! section or all; conflicts show under the command, and a new one is settled in its row with Replace, Swap or Cancel.
//! A keymap file can be exported and imported.
//!
//! Recording happens in place (#266): the shortcut being recorded shows "..." (then what is held), and the binding is
//! saved once every key and button is released, so a combination can be pressed in any order. [`KeybindsPage::listen`]
//! is the one place besides a text field that reads keys itself, before anything is drawn; Esc or a click elsewhere
//! cancels. Studio does not run the keymap meanwhile.

use std::path::PathBuf;

use egui::{
    Align, Color32, CornerRadius, Event, FontFamily, FontId, Id, Key, Label, Layout, Rect, Response, RichText, Sense,
    Stroke, TextEdit, Ui, vec2,
};
use metallic_engine::metallic_platform::{AppId, UserPaths};

use super::window::{CONTROL_WIDTH, section_label};
use crate::STUDIO_APP_ID;
use crate::keymap::{Binding, Chord, Command, Context, Keymap, Mods, MouseButton, Resolution, Trigger};
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, size, space, text};
use crate::theme::{ButtonKind, button, current_colors, fonts};

/// The keymap file's name, in Studio's config folder unless the user types another path.
pub const KEYMAP_FILE: &str = "keymap.toml";

/// What the shortcut being recorded shows while nothing is held.
pub const LISTENING: &str = "...";

/// The Keybinds page's own state: its filter, the recording, and the keymap file.
#[derive(Debug, Default)]
pub struct KeybindsPage {
    filter: String,
    capture: Option<Capture>,
    /// The keymap file to export to or import from.
    file: Option<String>,
    /// What the last export or import did: whether it worked, and a line saying so.
    status: Option<(bool, String)>,
}

/// A recording: which binding of which command, and how far it got.
#[derive(Clone, Debug, PartialEq)]
struct Capture {
    command: &'static str,
    /// The binding replaced, or `None` to add one.
    slot: Option<usize>,
    /// Whether to record two steps.
    sequence: bool,
    /// The first step, when recording a sequence.
    first: Option<Chord>,
    /// What is held so far: the last key or button pressed, with every modifier held since. Saved once everything is
    /// released.
    held: Option<Chord>,
    /// The binding recorded, waiting for the user to settle its conflicts.
    recorded: Option<Binding>,
    /// Where the shortcut being recorded was last drawn: a click there records a mouse button, the wheel over it
    /// records the wheel, and a click anywhere else cancels.
    chip: Option<Rect>,
    /// Whether the page drew it last frame: a recording nobody can see (Settings closed, another page, a collapsed
    /// section) is cancelled.
    seen: bool,
}

/// What a command's row asks the page to do.
enum RowAction {
    /// Record binding `slot` (`None`: a new one), in one step or two.
    Record(Option<usize>, bool),
    /// Settle the recorded binding's conflict so, or cancel it (`None`).
    Settle(Option<Resolution>),
}

impl KeybindsPage {
    /// The filter.
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Sets the filter.
    pub fn set_filter(&mut self, filter: impl Into<String>) {
        self.filter = filter.into();
    }

    /// Whether a shortcut is being recorded: Studio does not run the keymap meanwhile.
    pub fn is_capturing(&self) -> bool {
        self.capture.is_some()
    }

    /// Records command `id`'s binding at `slot`, or a new one when `slot` is `None`.
    pub fn capture(&mut self, id: &'static str, slot: Option<usize>) {
        self.record(id, slot, false);
    }

    /// Records command `id`'s binding at `slot` (or a new one), as a two-step sequence when `sequence`.
    fn record(&mut self, id: &'static str, slot: Option<usize>, sequence: bool) {
        self.capture = Some(Capture {
            command: id,
            slot,
            sequence,
            first: None,
            held: None,
            recorded: None,
            chip: None,
            seen: true,
        });
    }

    /// The keymap file's path: as typed, else `keymap.toml` in Studio's config folder.
    pub fn file(&self) -> String {
        self.file.clone().unwrap_or_else(default_file)
    }

    /// Sets the keymap file's path.
    pub fn set_file(&mut self, path: impl Into<String>) {
        self.file = Some(path.into());
    }

    /// Draws the page into `ui`, editing `keymap`.
    pub fn show(&mut self, ui: &mut Ui, keymap: &mut Keymap) {
        let p = current_colors(ui.ctx()).palette;
        ui.label(
            RichText::new("Keybinds")
                .font(fonts::named(fonts::HEADING, text::TITLE))
                .color(p.text),
        );
        ui.label(
            RichText::new("Click a shortcut to change it, or + to add another. Right-click one to remove it.")
                .size(text::BODY)
                .color(p.muted),
        );
        ui.add_space(space::S5);
        ui.horizontal(|ui| {
            let customised = keymap
                .registry()
                .commands()
                .iter()
                .any(|command| !keymap.is_default(command.id));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let reset = ui.add_enabled_ui(customised, |ui| button(ui, ButtonKind::Secondary, "Reset all"));
                if reset.inner.clicked() {
                    keymap.reset_all();
                }
                ui.add(
                    TextEdit::singleline(&mut self.filter)
                        .hint_text("Search by name, key or description")
                        .desired_width(f32::INFINITY)
                        .margin(vec2(space::S4, space::S3)),
                );
            });
        });
        ui.add_space(space::S5);

        let commands: Vec<Command> = keymap
            .registry()
            .commands()
            .iter()
            .filter(|command| matches(command, &self.filter, keymap))
            .copied()
            .collect();
        if commands.is_empty() {
            ui.label(
                RichText::new(format!("No keybinds match \u{201C}{}\u{201D}.", self.filter.trim()))
                    .size(text::BODY_LG)
                    .color(p.muted),
            );
        }
        // While searching, every section with a match is open, whatever the user collapsed.
        let searching = !self.filter.trim().is_empty();
        for section in sections(&commands) {
            self.section(ui, &section, keymap, searching);
            ui.add_space(space::S4);
        }
        ui.add_space(space::S4);

        self.file_row(ui, keymap);
    }

    /// One section (#264): a header that collapses it, with Reset while a command in it differs from its default;
    /// then its commands and the sections inside it. A top-level section is a bordered card; one inside it is
    /// indented along a line.
    fn section(&mut self, ui: &mut Ui, section: &Section, keymap: &mut Keymap, searching: bool) {
        let p = current_colors(ui.ctx()).palette;
        let top = section.depth() == 0;
        let id = Id::unique(("metallic.keybinds.section", &section.path));
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
        if top {
            egui::Frame::new()
                .stroke(Stroke::new(size::BORDER, p.border))
                .corner_radius(CornerRadius::same(radius::PANEL))
                .inner_margin(egui::Margin::symmetric(space::S5 as i8, space::S3 as i8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.section_contents(ui, section, keymap, searching, &mut state);
                });
        } else {
            // Indented, with a line down its left side that ties it to the section around it.
            let start = ui.cursor().min;
            ui.horizontal_top(|ui| {
                ui.add_space(space::S5);
                ui.vertical(|ui| self.section_contents(ui, section, keymap, searching, &mut state));
            });
            let end = ui.min_rect().bottom();
            ui.painter()
                .vline(start.x + space::S1, start.y..=end, Stroke::new(size::BORDER, p.border));
        }
        state.store(ui.ctx());
    }

    /// A section's header, then its body when open (or while searching).
    fn section_contents(
        &mut self,
        ui: &mut Ui,
        section: &Section,
        keymap: &mut Keymap,
        searching: bool,
        state: &mut egui::collapsing_header::CollapsingState,
    ) {
        if section_header(ui, section, keymap, state.is_open() || searching) {
            state.toggle(ui);
        }
        if searching {
            self.section_body(ui, section, keymap, searching);
        } else {
            state.show_body_unindented(ui, |ui| self.section_body(ui, section, keymap, searching));
        }
    }

    /// A section's commands, then the sections inside it.
    fn section_body(&mut self, ui: &mut Ui, section: &Section, keymap: &mut Keymap, searching: bool) {
        for (index, command) in section.commands.iter().enumerate() {
            self.command_row(ui, command, keymap, rule_below(index, section.commands.len()));
        }
        for inner in &section.sections {
            ui.add_space(space::S3);
            self.section(ui, inner, keymap, searching);
        }
    }

    /// The keymap file: its path, Export and Import, and what the last one did.
    fn file_row(&mut self, ui: &mut Ui, keymap: &mut Keymap) {
        let colors = current_colors(ui.ctx());
        let p = colors.palette;
        section_label(ui, "Keymap file");
        ui.label(
            RichText::new("Export your bindings to a file, or import a file exported from another machine.")
                .size(text::SMALL)
                .color(p.muted),
        );
        ui.add_space(space::S3);
        // Looked up once, not every frame.
        let mut path = self.file.get_or_insert_with(default_file).clone();
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, ButtonKind::Secondary, "Import").clicked() {
                    self.status = Some(match keymap.import_from(&PathBuf::from(&path)) {
                        Ok(count) => (true, format!("Imported the bindings of {count} commands from {path}.")),
                        Err(error) => (false, format!("Could not import {path}: {error}.")),
                    });
                }
                if button(ui, ButtonKind::Secondary, "Export").clicked() {
                    self.status = Some(match keymap.export_to(&PathBuf::from(&path)) {
                        Ok(()) => (true, format!("Exported to {path}.")),
                        Err(error) => (false, format!("Could not export to {path}: {error}.")),
                    });
                }
                let field = ui.add(
                    TextEdit::singleline(&mut path)
                        .desired_width(f32::INFINITY)
                        .margin(vec2(space::S4, space::S3)),
                );
                if field.changed() {
                    self.file = Some(path.clone());
                }
            });
        });
        if let Some((ok, line)) = &self.status {
            ui.add(
                Label::new(
                    RichText::new(line)
                        .size(text::SMALL)
                        .color(if *ok { p.muted } else { colors.accent }),
                )
                .wrap(),
            );
        }
    }
}

/// A section of the Keybinds page (#264): its commands, then the sections inside it, from the commands' category
/// paths (`Viewport/Fly camera` is Fly camera inside Viewport).
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Section {
    /// Its name: the path's last part.
    pub name: String,
    /// Its whole path, which names it apart from a section of the same name elsewhere.
    pub path: String,
    /// The commands directly in it, in the registry's order.
    pub commands: Vec<Command>,
    /// The sections inside it, in the order their first command comes.
    pub sections: Vec<Section>,
}

impl Section {
    /// How many sections it is inside.
    fn depth(&self) -> usize {
        self.path.matches('/').count()
    }

    /// Every command in it and in the sections inside it.
    pub(super) fn all_commands(&self) -> Vec<Command> {
        let mut all = self.commands.clone();
        for inner in &self.sections {
            all.extend(inner.all_commands());
        }
        all
    }

    /// The section inside it with `name`, made if there is none yet.
    fn child(sections: &mut Vec<Section>, parent: &str, name: &str) -> usize {
        if let Some(at) = sections.iter().position(|section| section.name == name) {
            return at;
        }
        let path = if parent.is_empty() {
            name.to_owned()
        } else {
            format!("{parent}/{name}")
        };
        sections.push(Section {
            name: name.to_owned(),
            path,
            ..Section::default()
        });
        sections.len() - 1
    }
}

/// Whether the row at `index` of `count` has a line under it: between rows only, so a section's last (or only) row
/// ends at the section's own edge (#408).
pub(super) fn rule_below(index: usize, count: usize) -> bool {
    index + 1 < count
}

/// `commands` as the page's sections, nested by their category paths, in the order each first appears.
pub(super) fn sections(commands: &[Command]) -> Vec<Section> {
    let mut top: Vec<Section> = Vec::new();
    for command in commands {
        let mut parts = command
            .category
            .split('/')
            .map(str::trim)
            .filter(|part| !part.is_empty());
        let Some(first) = parts.next() else {
            continue;
        };
        let at = Section::child(&mut top, "", first);
        let mut section = &mut top[at];
        for part in parts {
            let path = section.path.clone();
            let at = Section::child(&mut section.sections, &path, part);
            section = &mut section.sections[at];
        }
        section.commands.push(*command);
    }
    top
}

/// A section's header: a chevron, its name and how many commands it holds, all of which collapse it; and Reset on
/// the right while one of its commands differs from its default. Returns whether it was clicked to collapse or open.
fn section_header(ui: &mut Ui, section: &Section, keymap: &mut Keymap, open: bool) -> bool {
    let p = current_colors(ui.ctx()).palette;
    let top = section.depth() == 0;
    let commands = section.all_commands();
    let changed = commands.iter().any(|command| !keymap.is_default(command.id));
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), size::CONTROL), Sense::hover());
    let reset_width = if changed { 72.0 } else { 0.0 };
    let toggle_rect = Rect::from_min_max(rect.min, egui::pos2(rect.right() - reset_width, rect.bottom()));
    let toggle = ui.interact(
        toggle_rect,
        ui.scope_id().with(("section", &section.path)),
        Sense::click(),
    );
    toggle.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &section.name);
        info.selected = Some(open);
        info
    });
    if ui.is_rect_visible(rect) {
        let ink = if toggle.hovered() { p.text } else { p.muted };
        let chevron = if open { Icon::UiCollapse } else { Icon::UiExpand };
        let mut x = rect.left();
        icons::paint_at(
            ui.painter(),
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            chevron,
            text::BODY,
            ink,
        );
        x += text::BODY + space::S3;
        let title = ui.painter().text(
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            &section.name,
            fonts::named(fonts::STRONG, if top { text::BODY_LG } else { text::BODY }),
            p.text,
        );
        ui.painter().text(
            egui::pos2(title.right() + space::S3, rect.center().y),
            egui::Align2::LEFT_CENTER,
            commands.len().to_string(),
            FontId::proportional(text::SMALL),
            p.subtle,
        );
    }
    if changed {
        let reset_rect = Rect::from_min_max(egui::pos2(rect.right() - reset_width, rect.top()), rect.max);
        let reset = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(reset_rect)
                    .layout(Layout::right_to_left(Align::Center)),
                |ui| {
                    button(ui, ButtonKind::Quiet, "Reset")
                        .on_hover_text(format!("Back to the defaults for everything in {}", section.name))
                },
            )
            .inner;
        if reset.clicked() {
            for command in &commands {
                keymap.reset(command.id);
            }
        }
    }
    toggle.clicked()
}

impl KeybindsPage {
    /// Reads this frame's input for the recording, before anything is drawn, and takes the key events so they reach
    /// nothing else: a key (by place for a command that binds places), a mouse button pressed on the shortcut being
    /// recorded, or the wheel over it. Everything held counts, in any order, and the binding is saved once every key
    /// and button is released (#266); with a conflict, the command's row asks how to settle it. Esc, or a click
    /// anywhere else, cancels.
    pub fn listen(&mut self, ctx: &egui::Context, keymap: &mut Keymap) {
        let Some(mut capture) = self.capture.take() else {
            return;
        };
        let Some(index) = keymap.registry().index(capture.command) else {
            return;
        };
        if !capture.seen {
            // Not drawn last frame: nobody can see it, so it stops.
            return;
        }
        capture.seen = false;
        let events = ctx.input_mut(|input| {
            let events = input.events.clone();
            input
                .events
                .retain(|event| !matches!(event, Event::Key { .. } | Event::Text(_)));
            events
        });
        if capture.recorded.is_some() {
            // Its row's buttons settle it; Esc still cancels.
            let escape = events.iter().any(|event| {
                matches!(
                    event,
                    Event::Key {
                        key: Key::Escape,
                        pressed: true,
                        ..
                    }
                )
            });
            if !escape {
                self.capture = Some(capture);
            }
            return;
        }
        let physical = keymap.registry().prefers_physical(index);
        let drag = keymap.registry().prefers_drag(index);
        let platform = keymap.platform();
        let (modifiers, keys_down, pointer_down, hover) = ctx.input(|input| {
            (
                input.modifiers,
                !input.keys_down.is_empty(),
                input.pointer.any_down(),
                input.pointer.hover_pos(),
            )
        });
        let on_chip = |at: egui::Pos2| capture.chip.is_some_and(|chip| chip.contains(at));
        let mut pressed = Vec::new();
        for event in events {
            match event {
                Event::Key {
                    key: Key::Escape,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.is_none() && capture.held.is_none() => return,
                Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                } => {
                    let trigger = if physical {
                        Trigger::physical(physical_key.unwrap_or(key))
                    } else {
                        Trigger::logical(key)
                    };
                    pressed.push((modifiers, trigger));
                }
                Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    modifiers,
                } => {
                    if on_chip(pos) && capture.first.is_none() {
                        let button = MouseButton::from_egui(button);
                        let trigger = if drag {
                            Trigger::Drag(button)
                        } else {
                            Trigger::Mouse(button)
                        };
                        pressed.push((modifiers, trigger));
                    } else if capture.held.is_none() && pressed.is_empty() {
                        // A click anywhere else cancels.
                        return;
                    }
                }
                Event::MouseWheel { modifiers, .. } if hover.is_some_and(on_chip) && capture.first.is_none() => {
                    pressed.push((modifiers, Trigger::Wheel));
                }
                _ => {}
            }
        }
        // The last key or button pressed, with every modifier held since the first: held before it or pressed after.
        for (modifiers, trigger) in pressed {
            let mods = capture.held.map_or(Mods::NONE, |held| held.mods);
            capture.held = Some(Chord::new(union(mods, Mods::from_egui(modifiers, platform)), trigger));
        }
        if let Some(held) = &mut capture.held {
            held.mods = union(held.mods, Mods::from_egui(modifiers, platform));
        }
        let released = !keys_down && !pointer_down && modifiers.is_none();
        if released && let Some(chord) = capture.held.take() {
            match capture.first {
                None if capture.sequence => capture.first = Some(chord),
                first => {
                    let binding = match first {
                        Some(first) => Binding::sequence(first, chord),
                        None => Binding::single(chord),
                    };
                    if keymap.conflicts_with(capture.command, &binding).is_empty() {
                        keymap.assign(capture.command, capture.slot, binding, None);
                        return;
                    }
                    capture.recorded = Some(binding);
                }
            }
        }
        if capture.held.is_some() || capture.first.is_some() {
            // What is held shows as it changes.
            ctx.request_repaint();
        }
        self.capture = Some(capture);
    }

    /// One command: its name, description and conflicts on the left; its shortcuts, + and Reset on the right; and
    /// while a binding it recorded clashes, how to settle that under it.
    /// One command's row: its name, what it does and its shortcuts, with a line under it when `rule` (between rows,
    /// never after the last: #408).
    pub(super) fn command_row(&mut self, ui: &mut Ui, command: &Command, keymap: &mut Keymap, rule: bool) {
        let colors = current_colors(ui.ctx());
        let p = colors.palette;
        let mut action = None;
        // This command's recording, if it has one: which slot, and what it shows.
        let recording = self.capture.as_ref().filter(|capture| capture.command == command.id);
        let listening = recording.map(|capture| (capture.slot, listening_text(capture, keymap)));
        let clash = recording.and_then(|capture| capture.recorded.map(|binding| (binding, capture.slot)));
        let mut chip_rect = None;
        ui.push_id(command.id, |ui| {
            ui.horizontal(|ui| {
                let words = (ui.available_width() - CONTROL_WIDTH - space::S6).max(160.0);
                let words = ui.allocate_ui_with_layout(vec2(words, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(words);
                    ui.add_space(space::S3);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(command.title)
                                .font(fonts::named(fonts::STRONG, text::BODY))
                                .color(p.text),
                        );
                        if command.contexts != [Context::Global] {
                            ui.label(
                                RichText::new(contexts_label(command.contexts))
                                    .size(text::SMALL)
                                    .color(p.subtle),
                            );
                        }
                    });
                    ui.add(Label::new(RichText::new(command.description).size(text::SMALL).color(p.muted)).wrap());
                    for conflict in keymap.conflicts(command.id) {
                        let other = keymap
                            .registry()
                            .command(conflict.command)
                            .map_or(conflict.command, |other| other.title);
                        warning_line(
                            ui,
                            &format!("{} is also bound to {other}", keymap.label(&conflict.mine)),
                        );
                    }
                    ui.add_space(space::S3);
                });
                // Right-click on the command's name or description adds a binding. Over the words only: the chips
                // beside them take their own clicks.
                let response = ui.interact(words.response.rect, ui.scope_id().with("words"), Sense::click());
                response.context_menu(|ui| {
                    add_menu(ui, &mut action);
                    if !keymap.is_default(command.id) && ui.button("Reset to default").clicked() {
                        keymap.reset(command.id);
                    }
                });
                ui.allocate_ui_with_layout(
                    vec2(CONTROL_WIDTH, size::CONTROL),
                    Layout::right_to_left(Align::Center),
                    |ui| {
                        ui.set_width(CONTROL_WIDTH);
                        ui.spacing_mut().item_spacing.x = space::S3;
                        if !keymap.is_default(command.id) {
                            let reset = button(ui, ButtonKind::Quiet, "Reset").on_hover_text("Back to the default");
                            if reset.clicked() {
                                keymap.reset(command.id);
                            }
                        }
                        // #265: + after the shortcuts adds another.
                        let add = add_button(ui, command.title);
                        if add.clicked() {
                            action = Some(RowAction::Record(None, false));
                        }
                        add.context_menu(|ui| add_menu(ui, &mut action));
                        // A new binding being recorded: after the others.
                        if let Some((None, shown)) = &listening {
                            chip_rect = Some(listening_chip(ui, shown).rect);
                        }
                        let bindings = keymap.bindings(command.id).to_vec();
                        if bindings.is_empty() && listening.is_none() {
                            let chip = chip(ui, "Unbound", false);
                            if chip.clicked() {
                                action = Some(RowAction::Record(None, false));
                            }
                            chip.context_menu(|ui| add_menu(ui, &mut action));
                        }
                        // Right to left: the last binding first, so they read in order.
                        for (slot, binding) in bindings.iter().enumerate().rev() {
                            if let Some((Some(at), shown)) = &listening
                                && *at == slot
                            {
                                chip_rect = Some(listening_chip(ui, shown).rect);
                                continue;
                            }
                            let chip = chip(ui, &keymap.label(binding), true);
                            if chip.clicked() {
                                action = Some(RowAction::Record(Some(slot), false));
                            }
                            chip.context_menu(|ui| {
                                if ui.button("Record a two-step sequence").clicked() {
                                    action = Some(RowAction::Record(Some(slot), true));
                                }
                                add_menu(ui, &mut action);
                                if ui.button("Remove binding").clicked() {
                                    keymap.remove_binding(command.id, slot);
                                }
                                if !keymap.is_default(command.id) && ui.button("Reset to default").clicked() {
                                    keymap.reset(command.id);
                                }
                            });
                        }
                    },
                );
            });
            if let Some((binding, slot)) = clash
                && let Some(settle) = clash_row(ui, command, binding, slot, keymap)
            {
                action = Some(RowAction::Settle(settle));
            }
            if rule {
                let rect = ui.min_rect();
                ui.painter().hline(
                    rect.x_range(),
                    ui.cursor().top() - ui.spacing().item_spacing.y / 2.0,
                    Stroke::new(size::BORDER, p.border),
                );
            }
        });
        if let Some(capture) = self.capture.as_mut().filter(|capture| capture.command == command.id) {
            capture.seen = true;
            capture.chip = chip_rect;
        }
        match action {
            Some(RowAction::Record(slot, sequence)) => self.record(command.id, slot, sequence),
            Some(RowAction::Settle(settle)) => {
                if let (Some(resolution), Some(capture)) = (settle, &self.capture)
                    && let Some(binding) = capture.recorded
                {
                    keymap.assign(command.id, capture.slot, binding, Some(resolution));
                }
                self.capture = None;
            }
            None => {}
        }
    }
}

/// Both sets of modifiers: what was held at any time while recording.
fn union(a: Mods, b: Mods) -> Mods {
    Mods {
        primary: a.primary || b.primary,
        ctrl: a.ctrl || b.ctrl,
        shift: a.shift || b.shift,
        alt: a.alt || b.alt,
        super_key: a.super_key || b.super_key,
    }
}

/// What the shortcut being recorded shows: "..." until something is held, then what is held; a sequence's first
/// step before it.
fn listening_text(capture: &Capture, keymap: &Keymap) -> String {
    let first = capture
        .first
        .map(|first| format!("{}, ", keymap.label(&Binding::single(first))));
    let held = match capture.held {
        Some(held) => keymap.label(&Binding::single(held)),
        None => LISTENING.to_owned(),
    };
    match capture.recorded {
        Some(recorded) => keymap.label(&recorded),
        None => format!("{}{held}", first.unwrap_or_default()),
    }
}

/// The right-click choices that add a binding.
fn add_menu(ui: &mut Ui, action: &mut Option<RowAction>) {
    if ui.button("Add binding").clicked() {
        *action = Some(RowAction::Record(None, false));
    }
    if ui.button("Add a two-step sequence").clicked() {
        *action = Some(RowAction::Record(None, true));
    }
}

/// A warning icon and a line beside it.
fn warning_line(ui: &mut Ui, line: &str) {
    let colors = current_colors(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::S2;
        icons::show(ui, Icon::UiWarning, text::SMALL, colors.accent);
        ui.add(Label::new(RichText::new(line).size(text::SMALL).color(colors.palette.text)).wrap());
    });
}

/// Under a command whose new binding clashes: who has it, and Replace, Swap (when there is one other and an old
/// binding to give it) or Cancel. Returns the choice: `Some(None)` to cancel.
fn clash_row(
    ui: &mut Ui,
    command: &Command,
    binding: Binding,
    slot: Option<usize>,
    keymap: &Keymap,
) -> Option<Option<Resolution>> {
    let p = current_colors(ui.ctx()).palette;
    let conflicts = keymap.conflicts_with(command.id, &binding);
    let owners: Vec<String> = conflicts
        .iter()
        .filter_map(|conflict| keymap.registry().command(conflict.command))
        .map(|other| format!("{} ({})", other.title, contexts_label(other.contexts)))
        .collect();
    let old = slot.and_then(|slot| keymap.bindings(command.id).get(slot).copied());
    let can_swap = conflicts.len() == 1 && old.is_some();
    let mut choice = None;
    ui.add_space(space::S2);
    warning_line(
        ui,
        &format!("{} is already used by {}.", keymap.label(&binding), owners.join(", ")),
    );
    let explain = match old {
        Some(old) if can_swap => format!(
            "Replace takes it from them. Swap gives them {} instead.",
            keymap.label(&old)
        ),
        _ => "Replace takes it from them.".to_owned(),
    };
    ui.add(Label::new(RichText::new(explain).size(text::SMALL).color(p.muted)).wrap());
    ui.add_space(space::S2);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::S3;
        if button(ui, ButtonKind::Primary, "Replace").clicked() {
            choice = Some(Some(Resolution::Replace));
        }
        let swap = ui.add_enabled_ui(can_swap, |ui| button(ui, ButtonKind::Secondary, "Swap"));
        if swap.inner.clicked() {
            choice = Some(Some(Resolution::Swap));
        }
        if button(ui, ButtonKind::Secondary, "Cancel").clicked() {
            choice = Some(None);
        }
    });
    ui.add_space(space::S3);
    choice
}

/// The shortcut being recorded: what it shows, outlined in the accent while Studio listens.
fn listening_chip(ui: &mut Ui, shown: &str) -> Response {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let text = RichText::new(shown)
        .font(FontId::new(text::MONO, FontFamily::Monospace))
        .color(p.text);
    // Hover only: a click on it records a mouse button (KeybindsPage::listen), it does not start again.
    ui.add(
        egui::Button::new(text)
            .sense(Sense::hover())
            .fill(p.input)
            .stroke(Stroke::new(size::BORDER, colors.accent))
            .corner_radius(CornerRadius::same(radius::ITEM))
            .min_size(vec2(26.0, 26.0)),
    )
    .on_hover_text("Press the keys, then let go. Click here for a mouse button. Esc cancels.")
}

/// Whether every word of `query` is in the command's name, description, category or id, or one of its keys as shown.
pub(super) fn matches(command: &Command, query: &str, keymap: &Keymap) -> bool {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return true;
    }
    let mut haystack = format!(
        "{} {} {} {}",
        command.title, command.description, command.category, command.id
    )
    .to_lowercase();
    for binding in keymap.bindings(command.id) {
        haystack.push(' ');
        haystack.push_str(&keymap.label(binding).to_lowercase());
    }
    words.iter().all(|word| haystack.contains(word.as_str()))
}

/// The contexts a command applies in, for people.
fn contexts_label(contexts: &[Context]) -> String {
    contexts
        .iter()
        .map(|context| context.label())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The small + after a command's shortcuts (#265): adds another, as right-click → Add binding does.
fn add_button(ui: &mut Ui, title: &str) -> Response {
    let p = current_colors(ui.ctx()).palette;
    let (rect, response) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            ui.is_enabled(),
            format!("Add a shortcut to {title}"),
        )
    });
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        if hovered {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius::ITEM / 2), p.surface_hover);
        }
        icons::paint_at(
            ui.painter(),
            rect.center(),
            egui::Align2::CENTER_CENTER,
            Icon::UiAdd,
            text::BODY,
            if hovered { p.text } else { p.subtle },
        );
    }
    response.on_hover_text("Add another shortcut")
}

/// A shortcut as a key cap: monospace on the input colour with a border; `bound` false for the muted "Unbound".
fn chip(ui: &mut Ui, label: &str, bound: bool) -> Response {
    let p = current_colors(ui.ctx()).palette;
    let text = RichText::new(label)
        .font(FontId::new(text::MONO, FontFamily::Monospace))
        .color(if bound { p.text } else { p.subtle });
    ui.add(
        egui::Button::new(text)
            .fill(if bound { p.input } else { Color32::TRANSPARENT })
            .stroke(Stroke::new(size::BORDER, p.border))
            .corner_radius(CornerRadius::same(radius::ITEM))
            .min_size(vec2(0.0, 26.0)),
    )
    .on_hover_text("Click to change. Right-click to add or remove.")
}

/// `keymap.toml` in Studio's config folder, or in the working folder when there is none.
fn default_file() -> String {
    AppId::parse(STUDIO_APP_ID)
        .and_then(|app| UserPaths::for_app(&app))
        .map(|paths| paths.config_dir().join(KEYMAP_FILE))
        .unwrap_or_else(|_| PathBuf::from(KEYMAP_FILE))
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(id: &'static str, category: &'static str) -> Command {
        Command::new(id, id, category, "A command.")
    }

    #[test]
    fn lines_go_between_rows_never_after_the_last() {
        assert!(!rule_below(0, 1), "a lone row has none");
        assert!(rule_below(0, 3) && rule_below(1, 3));
        assert!(!rule_below(2, 3));
    }

    #[test]
    fn sections_nest_by_category_path_in_first_seen_order() {
        let commands = [
            command("a", "General"),
            command("b", "Viewport/Fly camera"),
            command("c", "Viewport/Mouse"),
            command("d", "General"),
            command("e", "Viewport"),
            command("f", "Viewport/Fly camera/Fast"),
            command("g", " Viewport / Mouse "),
        ];
        let sections = sections(&commands);
        let names = |sections: &[Section]| sections.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        let ids = |section: &Section| section.commands.iter().map(|c| c.id).collect::<Vec<_>>();
        assert_eq!(names(&sections), ["General", "Viewport"]);
        assert_eq!(ids(&sections[0]), ["a", "d"]);
        let viewport = &sections[1];
        assert_eq!(ids(viewport), ["e"]);
        assert_eq!(names(&viewport.sections), ["Fly camera", "Mouse"]);
        let fly = &viewport.sections[0];
        assert_eq!(fly.path, "Viewport/Fly camera");
        assert_eq!(fly.depth(), 1);
        assert_eq!(names(&fly.sections), ["Fast"]);
        assert_eq!(fly.sections[0].path, "Viewport/Fly camera/Fast");
        assert_eq!(fly.sections[0].depth(), 2);
        // Spaces around a part do not make another section.
        assert_eq!(ids(&viewport.sections[1]), ["c", "g"]);
        let all: Vec<&str> = viewport.all_commands().iter().map(|c| c.id).collect();
        assert_eq!(all, ["e", "b", "f", "c", "g"]);
    }

    #[test]
    fn studio_commands_nest_the_viewport_s_two_sections() {
        let sections = sections(crate::keymap::STUDIO_COMMANDS);
        let viewport = sections
            .iter()
            .find(|s| s.name == "Viewport")
            .expect("a Viewport section");
        let inner: Vec<&str> = viewport.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(inner, ["Fly camera", "Mouse"]);
    }
}
