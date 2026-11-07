//! The Keybinds page of the Settings window (ADR 0009, Keybinds): every command by category, searchable by name,
//! key or description. Click a shortcut to change it, right-click to add or remove one, reset one or all; conflicts
//! show under the command, and the capture dialog settles a new one with Replace, Swap or Cancel. A keymap file can be
//! exported and imported.
//!
//! The capture dialog is the one place besides a text field that reads keys itself: it records the next chord, and
//! Esc cancels it. Studio does not run the keymap while it is open.

use std::path::PathBuf;

use egui::{
    Align, Color32, CornerRadius, Event, FontFamily, FontId, Id, Key, Label, Layout, Modal, Rect, Response, RichText,
    Sense, Stroke, TextEdit, Ui, vec2,
};
use metallic_engine::metallic_platform::{AppId, UserPaths};

use super::window::{CONTROL_WIDTH, section_label};
use crate::STUDIO_APP_ID;
use crate::keymap::{Binding, Chord, Command, Context, Keymap, Mods, MouseButton, Resolution, Trigger};
use crate::theme::tokens::{radius, size, space, text};
use crate::theme::{ButtonKind, button, current_colors, fonts};

/// The keymap file's name, in Studio's config folder unless the user types another path.
pub const KEYMAP_FILE: &str = "keymap.toml";

/// The Keybinds page's own state: its filter, the capture dialog, and the keymap file.
#[derive(Debug, Default)]
pub struct KeybindsPage {
    filter: String,
    capture: Option<Capture>,
    /// The keymap file to export to or import from.
    file: Option<String>,
    /// What the last export or import did: whether it worked, and a line saying so.
    status: Option<(bool, String)>,
}

/// The capture dialog: which binding of which command is being recorded, and how far it got.
#[derive(Clone, Debug, PartialEq)]
struct Capture {
    command: &'static str,
    /// The binding replaced, or `None` to add one.
    slot: Option<usize>,
    /// Whether to record two steps.
    sequence: bool,
    /// The first step, when recording a sequence.
    first: Option<Chord>,
    /// The binding recorded, waiting for the user to settle its conflicts.
    recorded: Option<Binding>,
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

    /// Whether the capture dialog is open: Studio does not run the keymap meanwhile.
    pub fn is_capturing(&self) -> bool {
        self.capture.is_some()
    }

    /// Opens the capture dialog for command `id`'s binding at `slot`, or to add one when `slot` is `None`.
    pub fn capture(&mut self, id: &'static str, slot: Option<usize>) {
        self.capture = Some(Capture {
            command: id,
            slot,
            sequence: false,
            first: None,
            recorded: None,
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
        for command in &section.commands {
            if let Some(request) = command_row(ui, command, keymap) {
                self.capture(request.0, request.1);
            }
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

    /// The capture dialog, while open: records a chord (or two), then applies it or asks how to settle a conflict.
    pub(super) fn dialog(&mut self, ctx: &egui::Context, keymap: &mut Keymap) {
        let Some(mut capture) = self.capture.take() else {
            return;
        };
        let Some(index) = keymap.registry().index(capture.command) else {
            return;
        };
        let command = keymap.registry().commands()[index];
        let colors = current_colors(ctx);
        let p = colors.palette;
        let mut open = true;
        Modal::new(Id::unique("metallic.keybinds.capture")).show(ctx, |ui| {
            ui.set_width(420.0);
            let verb = if capture.slot.is_some() { "Change" } else { "Add" };
            ui.label(
                RichText::new(format!("{verb} a shortcut for {}", command.title))
                    .font(fonts::named(fonts::STRONG, text::BODY_LG))
                    .color(p.text),
            );
            ui.add_space(space::S4);
            match capture.recorded {
                None => {
                    let shown = match capture.first {
                        Some(first) => format!("{}, \u{2026}", keymap.label(&Binding::single(first))),
                        None => "Press a key".to_owned(),
                    };
                    let pad = key_pad(ui, &shown);
                    if let Some(done) = record(ui.ctx(), &mut capture, pad, keymap, index) {
                        match done {
                            Recorded::Cancelled => open = false,
                            Recorded::Binding(binding) => {
                                if keymap.conflicts_with(command.id, &binding).is_empty() {
                                    keymap.assign(command.id, capture.slot, binding, None);
                                    open = false;
                                } else {
                                    capture.recorded = Some(binding);
                                }
                            }
                        }
                    }
                    ui.add(
                        Label::new(
                            RichText::new("Or click or scroll in the box to use the mouse. Esc cancels.")
                                .size(text::SMALL)
                                .color(p.muted),
                        )
                        .wrap(),
                    );
                    ui.add_space(space::S3);
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut capture.sequence, "Two-step sequence (such as Ctrl+K, then S)");
                    });
                    ui.add_space(space::S4);
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if button(ui, ButtonKind::Secondary, "Cancel").clicked() {
                            open = false;
                        }
                    });
                }
                Some(binding) => {
                    let conflicts = keymap.conflicts_with(command.id, &binding);
                    let label = keymap.label(&binding);
                    ui.label(
                        RichText::new(format!("{label} is already used by"))
                            .size(text::BODY)
                            .color(p.muted),
                    );
                    for conflict in &conflicts {
                        if let Some(other) = keymap.registry().command(conflict.command) {
                            ui.label(
                                RichText::new(format!("{} ({})", other.title, contexts_label(other.contexts)))
                                    .font(fonts::named(fonts::STRONG, text::BODY))
                                    .color(p.text),
                            );
                        }
                    }
                    let old = capture
                        .slot
                        .and_then(|slot| keymap.bindings(command.id).get(slot).copied());
                    let can_swap = conflicts.len() == 1 && old.is_some();
                    ui.add_space(space::S3);
                    let explain = match old {
                        Some(old) if can_swap => format!(
                            "Replace takes it from them. Swap gives them {} instead.",
                            keymap.label(&old)
                        ),
                        _ => "Replace takes it from them.".to_owned(),
                    };
                    ui.add(Label::new(RichText::new(explain).size(text::SMALL).color(p.muted)).wrap());
                    ui.add_space(space::S5);
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if button(ui, ButtonKind::Primary, "Replace").clicked() {
                            keymap.assign(command.id, capture.slot, binding, Some(Resolution::Replace));
                            open = false;
                        }
                        let swap = ui.add_enabled_ui(can_swap, |ui| button(ui, ButtonKind::Secondary, "Swap"));
                        if swap.inner.clicked() {
                            keymap.assign(command.id, capture.slot, binding, Some(Resolution::Swap));
                            open = false;
                        }
                        if button(ui, ButtonKind::Secondary, "Cancel").clicked() {
                            open = false;
                        }
                    });
                    // Esc still cancels here.
                    if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape)) {
                        open = false;
                    }
                }
            }
        });
        if open {
            self.capture = Some(capture);
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
        let chevron = if open {
            crate::panels::icon::CARET_DOWN
        } else {
            crate::panels::icon::CARET_RIGHT
        };
        let mut x = rect.left();
        ui.painter().text(
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            chevron,
            fonts::named(fonts::ICONS, text::BODY),
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

/// What the capture dialog recorded this frame.
enum Recorded {
    Cancelled,
    Binding(Binding),
}

/// Reads this frame's input for the capture dialog: a key (by place for a command that binds places), a mouse button
/// or the wheel in `pad`. Takes the key events, so nothing else in the dialog acts on them.
fn record(ctx: &egui::Context, capture: &mut Capture, pad: Rect, keymap: &Keymap, index: usize) -> Option<Recorded> {
    let physical = keymap.registry().prefers_physical(index);
    let drag = keymap.registry().prefers_drag(index);
    let platform = keymap.platform();
    let events = ctx.input_mut(|input| {
        let events = input.events.clone();
        input
            .events
            .retain(|event| !matches!(event, Event::Key { .. } | Event::Text(_)));
        events
    });
    let hovering = ctx
        .input(|input| input.pointer.hover_pos())
        .is_some_and(|at| pad.contains(at));
    for event in events {
        let chord = match event {
            Event::Key {
                key: Key::Escape,
                pressed: true,
                modifiers,
                ..
            } if modifiers.is_none() => return Some(Recorded::Cancelled),
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
                Chord::new(Mods::from_egui(modifiers, platform), trigger)
            }
            Event::PointerButton {
                pos,
                button,
                pressed: true,
                modifiers,
            } if pad.contains(pos) && capture.first.is_none() => {
                let button = MouseButton::from_egui(button);
                let trigger = if drag {
                    Trigger::Drag(button)
                } else {
                    Trigger::Mouse(button)
                };
                return Some(Recorded::Binding(Binding::single(Chord::new(
                    Mods::from_egui(modifiers, platform),
                    trigger,
                ))));
            }
            Event::MouseWheel { modifiers, .. } if hovering && capture.first.is_none() => {
                return Some(Recorded::Binding(Binding::single(Chord::new(
                    Mods::from_egui(modifiers, platform),
                    Trigger::Wheel,
                ))));
            }
            _ => continue,
        };
        match capture.first {
            Some(first) => return Some(Recorded::Binding(Binding::sequence(first, chord))),
            None if capture.sequence => capture.first = Some(chord),
            None => return Some(Recorded::Binding(Binding::single(chord))),
        }
    }
    None
}

/// The box that shows what is being pressed. Returns where it is, for the mouse.
fn key_pad(ui: &mut Ui, shown: &str) -> Rect {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 72.0), Sense::click());
    ui.painter().rect(
        rect,
        CornerRadius::same(radius::INPUT),
        p.input,
        // The 3 px focus outline: the dialog is listening.
        Stroke::new(size::ACCENT_BAR, colors.accent),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        shown,
        FontId::new(text::BODY_LG, FontFamily::Monospace),
        p.text,
    );
    ui.add_space(space::S3);
    rect
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

/// One command: its name, description and conflicts on the left; its shortcuts and Reset on the right. Returns the
/// binding to capture when one was asked for: the command and the slot (`None` to add).
pub(super) fn command_row(
    ui: &mut Ui,
    command: &Command,
    keymap: &mut Keymap,
) -> Option<(&'static str, Option<usize>)> {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let mut capture = None;
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
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = space::S2;
                        ui.label(
                            RichText::new(fonts::icon::WARNING.to_string())
                                .font(fonts::named(fonts::ICONS, text::SMALL))
                                .color(colors.accent),
                        );
                        ui.add(
                            Label::new(
                                RichText::new(format!("{} is also bound to {other}", keymap.label(&conflict.mine)))
                                    .size(text::SMALL)
                                    .color(p.text),
                            )
                            .wrap(),
                        );
                    });
                }
                ui.add_space(space::S3);
            });
            // Right-click on the command's name or description adds a binding. Over the words only: the chips beside
            // them take their own clicks.
            let response = ui.interact(words.response.rect, ui.scope_id().with("words"), Sense::click());
            response.context_menu(|ui| {
                if ui.button("Add binding").clicked() {
                    capture = Some((command.id, None));
                }
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
                    if add_button(ui, command.title).clicked() {
                        capture = Some((command.id, None));
                    }
                    let bindings = keymap.bindings(command.id).to_vec();
                    if bindings.is_empty() {
                        let chip = chip(ui, "Unbound", false);
                        if chip.clicked() {
                            capture = Some((command.id, None));
                        }
                        chip.context_menu(|ui| {
                            if ui.button("Add binding").clicked() {
                                capture = Some((command.id, None));
                            }
                        });
                    }
                    // Right to left: the last binding first, so they read in order.
                    for (slot, binding) in bindings.iter().enumerate().rev() {
                        let chip = chip(ui, &keymap.label(binding), true);
                        if chip.clicked() {
                            capture = Some((command.id, Some(slot)));
                        }
                        chip.context_menu(|ui| {
                            if ui.button("Add binding").clicked() {
                                capture = Some((command.id, None));
                            }
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
        let rect = ui.min_rect();
        ui.painter().hline(
            rect.x_range(),
            ui.cursor().top() - ui.spacing().item_spacing.y / 2.0,
            Stroke::new(size::BORDER, p.border),
        );
    });
    capture
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
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            fonts::icon::PLUS,
            fonts::named(fonts::ICONS, text::BODY),
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
