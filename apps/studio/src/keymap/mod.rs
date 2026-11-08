//! The keymap (ADR 0009, Keybinds): every Studio shortcut is a [`Command`] in one [`Registry`], bound by a [`Keymap`]
//! that turns raw egui input into [`Activation`]s.
//!
//! - **One input path.** Studio feeds each frame's events through [`Keymap::dispatch`] before drawing; panels never
//!   read keys themselves (only a focused text field types, and the capture dialog reads the key it records). A key
//!   that fires a command is taken out of the frame's events, so nothing else acts on it too.
//! - **Context from focus.** The focused panel decides: [`Focus::context`]. Contexts nest (Global, Editor, a panel)
//!   and the most specific binding wins. Hover only starts viewport navigation with the mouse and wheel
//!   ([`Command::on_hover`]). A focused text field blocks every key command not marked [`Command::while_typing`].
//! - **Repeat policy.** Commands ignore auto-repeat unless they opt in ([`Repeat::Allow`]).
//! - **Lookup** is a hash map from a binding's first step, rebuilt only when the bindings change.
//! - **Storage:** only overrides are kept ([`Overrides`], `settings.toml`'s `[keybinds]`). Ids this Studio does not
//!   know are kept. A new default never takes a key the user bound to something else: the command is left without
//!   it, the choice is saved as an override, and [`Keymap::take_notices`] says so once.

pub mod binding;
pub mod command;

use std::collections::{BTreeMap, HashMap};

use egui::{Event, Key, MouseWheelUnit};
use toml::{Table, Value};

pub use binding::{Binding, BindingError, Chord, KeyKind, Mods, MouseButton, Platform, Trigger};
pub use command::{Command, Context, Registry, Repeat, STUDIO_COMMANDS, contexts_overlap, ids};

/// The user's bindings that differ from the defaults: command id to binding texts (`[]` is unbound). Ids this Studio
/// does not know, and texts it cannot read, are kept as they are.
pub type Overrides = BTreeMap<String, Vec<String>>;

/// What an exported keymap file says it is.
pub const KEYMAP_FORMAT: &str = "metallic-keymap";

/// The first line of an exported keymap.
const EXPORT_HEADER: &str = "# A Metallic Studio keymap: the bindings that differ from the defaults. Import it in \
                             Settings, Keybinds.\n";

/// How many points of a smooth (touchpad) scroll make one wheel line.
const POINTS_PER_LINE: f32 = 50.0;
/// How many lines a page of scrolling is.
const LINES_PER_PAGE: f32 = 10.0;

/// Where input goes this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Focus {
    /// The focused context: the focused panel's, else Editor with a project open, else Global.
    pub context: Context,
    /// The panel under the pointer, which starts navigation with the mouse and wheel there.
    pub hovered: Option<Context>,
    /// Whether a text field has focus.
    pub typing: bool,
    /// Whether a running game has the keyboard and mouse (Play in Studio): only commands marked
    /// [`Command::during_play`] fire, and every other key and click is the game's.
    pub playing: bool,
}

impl Focus {
    /// Focus in `context`, nothing hovered, not typing, no game with the input.
    pub const fn new(context: Context) -> Self {
        Self {
            context,
            hovered: None,
            typing: false,
            playing: false,
        }
    }
}

/// How a command fired.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ActivationKind {
    /// Its binding was pressed.
    Press,
    /// Its key auto-repeated (only for [`Repeat::Allow`] commands).
    Repeat,
    /// The wheel turned, by this many lines (positive: away from the user).
    Wheel(f32),
}

/// A command fired.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Activation {
    /// The command's id.
    pub command: &'static str,
    /// How.
    pub kind: ActivationKind,
}

/// What [`Keymap::process`] made of a frame's events.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Processed {
    /// The commands fired, in order.
    pub activations: Vec<Activation>,
    /// The indices of the key events the keymap used, which nothing else should act on.
    pub consumed: Vec<usize>,
}

/// Another command's binding that fires on the same input in an overlapping context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The binding asked about.
    pub mine: Binding,
    /// The other command's id.
    pub command: &'static str,
    /// The other command's binding.
    pub theirs: Binding,
}

/// How the capture dialog settles a conflict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// Take the binding; the other command loses it.
    Replace,
    /// Take the binding; the other command gets the one it replaces.
    Swap,
}

/// Why a keymap file could not be imported.
#[derive(Debug, thiserror::Error)]
pub enum KeymapError {
    /// The file could not be read.
    #[error("could not read the file: {0}")]
    Io(#[from] std::io::Error),
    /// The file is not TOML.
    #[error("the file is not a keymap: {0}")]
    NotToml(String),
    /// The file is TOML, but not a keymap.
    #[error("the file is not a Metallic keymap (it has no [keybinds] table)")]
    NotAKeymap,
}

/// A binding of a command: its index and the binding's index in its list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slot {
    command: usize,
    binding: usize,
}

/// A command held down, and what lets it go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Held {
    command: usize,
    release: Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Release {
    Key { key: Key, physical: Option<Key> },
    Button(MouseButton),
}

/// The registry, the user's overrides, and the bindings they give, ready to match input.
#[derive(Debug)]
pub struct Keymap {
    registry: Registry,
    platform: Platform,
    overrides: Overrides,
    /// Each command's bindings now, by registry index.
    bindings: Vec<Vec<Binding>>,
    /// Every binding by its first step, with canonical modifiers.
    lookup: HashMap<Chord, Vec<Slot>>,
    /// Each command's conflicts, shown inline on the Keybinds page.
    conflicts: Vec<Vec<Conflict>>,
    /// The user's layout as seen so far: physical key to the logical key it gave.
    layout: HashMap<Key, Key>,
    /// The sequences whose first step was just pressed.
    pending: Option<Vec<Slot>>,
    held: Vec<Held>,
    notices: Vec<String>,
    changed: bool,
}

impl Keymap {
    /// The keymap of `registry` on `platform` with the user's `overrides`. A default that would take a key one of the
    /// overrides uses is left out (see the module's documentation).
    pub fn new(registry: Registry, platform: Platform, overrides: &Overrides) -> Self {
        let mut keymap = Self {
            registry,
            platform,
            overrides: overrides.clone(),
            bindings: Vec::new(),
            lookup: HashMap::new(),
            conflicts: Vec::new(),
            layout: HashMap::new(),
            pending: None,
            held: Vec::new(),
            notices: Vec::new(),
            changed: false,
        };
        keymap.protect_user_keys();
        keymap.rebuild();
        keymap
    }

    /// Studio's commands on this platform with `overrides`.
    pub fn studio(overrides: &Overrides) -> Self {
        Self::new(Registry::studio(), Platform::current(), overrides)
    }

    /// The registry.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The platform "Primary" resolves for.
    pub fn platform(&self) -> Platform {
        self.platform
    }

    /// The overrides, as `settings.toml` stores them.
    pub fn overrides(&self) -> &Overrides {
        &self.overrides
    }

    /// The bindings of command `id` now (none for an unknown id).
    pub fn bindings(&self, id: &str) -> &[Binding] {
        self.registry
            .index(id)
            .and_then(|index| self.bindings.get(index))
            .map_or(&[], Vec::as_slice)
    }

    /// Whether command `id` has its default bindings.
    pub fn is_default(&self, id: &str) -> bool {
        !self.overrides.contains_key(id)
    }

    /// The bindings of other commands that clash with command `id`'s.
    pub fn conflicts(&self, id: &str) -> &[Conflict] {
        self.registry
            .index(id)
            .and_then(|index| self.conflicts.get(index))
            .map_or(&[], Vec::as_slice)
    }

    /// The bindings of commands other than `id` that would clash with `binding` given to it.
    pub fn conflicts_with(&self, id: &str, binding: &Binding) -> Vec<Conflict> {
        let Some(index) = self.registry.index(id) else {
            return Vec::new();
        };
        self.clashes(index, binding)
            .map(|slot| Conflict {
                mine: *binding,
                command: self.registry.commands()[slot.command].id,
                theirs: self.bindings[slot.command][slot.binding],
            })
            .collect()
    }

    /// The other commands' bindings that clash with `binding` on the command at `index`.
    fn clashes<'a>(&'a self, index: usize, binding: &'a Binding) -> impl Iterator<Item = Slot> + 'a {
        let command = &self.registry.commands()[index];
        self.registry
            .commands()
            .iter()
            .enumerate()
            .filter(move |(other, them)| *other != index && contexts_overlap(command, them))
            .flat_map(move |(other, _)| {
                self.bindings[other]
                    .iter()
                    .enumerate()
                    .filter(move |(_, theirs)| binding.overlaps(theirs, self.platform, &self.layout))
                    .map(move |(at, _)| Slot {
                        command: other,
                        binding: at,
                    })
            })
    }

    /// Gives command `id` exactly `bindings` (an empty list unbinds it). The defaults again remove the override.
    pub fn set_bindings(&mut self, id: &str, bindings: Vec<Binding>) {
        if let Some(index) = self.registry.index(id) {
            self.store(index, bindings);
            self.rebuild();
        }
    }

    /// Records `bindings` as command `index`'s, without rebuilding the lookup.
    fn store(&mut self, index: usize, mut bindings: Vec<Binding>) {
        let mut seen = Vec::with_capacity(bindings.len());
        bindings.retain(|binding| {
            let first = !seen.contains(binding);
            seen.push(*binding);
            first
        });
        let id = self.registry.commands()[index].id;
        if bindings == self.registry.defaults(index) {
            self.overrides.remove(id);
        } else {
            self.overrides
                .insert(id.to_owned(), bindings.iter().map(Binding::to_string).collect());
        }
        if let Some(slot) = self.bindings.get_mut(index) {
            *slot = bindings;
        }
        self.changed = true;
    }

    /// Gives command `id` `binding`, in place of its binding at `slot` or added when `slot` is `None`. Clashing
    /// bindings of other commands are settled by `resolution`: `None` keeps them (both stay bound, and the conflict
    /// shows). [`Resolution::Swap`] needs one clash and a binding replaced; otherwise it replaces.
    pub fn assign(&mut self, id: &str, slot: Option<usize>, binding: Binding, resolution: Option<Resolution>) {
        let Some(index) = self.registry.index(id) else {
            return;
        };
        let clashes: Vec<Slot> = self.clashes(index, &binding).collect();
        let mut mine = self.bindings[index].clone();
        let old = slot.and_then(|at| mine.get(at).copied());
        match slot.filter(|at| *at < mine.len()) {
            Some(at) => mine[at] = binding,
            None => mine.push(binding),
        }
        self.store(index, mine);
        if let Some(resolution) = resolution {
            let swap = match (resolution, old, clashes.as_slice()) {
                (Resolution::Swap, Some(old), [one]) => Some((*one, old)),
                _ => None,
            };
            if let Some((other, old)) = swap {
                let mut theirs = self.bindings[other.command].clone();
                theirs[other.binding] = old;
                self.store(other.command, theirs);
            } else {
                // Every clash of one command at once, so the indices stay right.
                let mut by_command: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
                for clash in clashes {
                    by_command.entry(clash.command).or_default().push(clash.binding);
                }
                for (command, lost) in by_command {
                    let theirs = self.bindings[command]
                        .iter()
                        .enumerate()
                        .filter(|(at, _)| !lost.contains(at))
                        .map(|(_, binding)| *binding)
                        .collect();
                    self.store(command, theirs);
                }
            }
        }
        self.rebuild();
    }

    /// Removes command `id`'s binding at `slot`.
    pub fn remove_binding(&mut self, id: &str, slot: usize) {
        let mut bindings = self.bindings(id).to_vec();
        if slot < bindings.len() {
            bindings.remove(slot);
            self.set_bindings(id, bindings);
        }
    }

    /// Gives command `id` its defaults again.
    pub fn reset(&mut self, id: &str) {
        if self.overrides.remove(id).is_some() {
            self.changed = true;
            self.rebuild();
        }
    }

    /// Gives every command its defaults again. Overrides of ids this Studio does not know are kept.
    pub fn reset_all(&mut self) {
        let registry = &self.registry;
        let before = self.overrides.len();
        self.overrides.retain(|id, _| registry.index(id).is_none());
        if self.overrides.len() != before {
            self.changed = true;
            self.rebuild();
        }
    }

    /// Whether the bindings changed since the last call: the caller saves [`Keymap::overrides`].
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// What Studio should tell the user about the keymap (a new default not applied); each only once.
    pub fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notices)
    }

    /// `binding` as people read it here, physical keys as the user's layout labels them.
    pub fn label(&self, binding: &Binding) -> String {
        binding.label(self.platform, &self.layout)
    }

    /// Every binding of command `id`, as people read them, joined by "or"; `None` when it is unbound. Menus and
    /// tooltips show this, never just the first binding.
    pub fn shortcut_text(&self, id: &str) -> Option<String> {
        let labels: Vec<String> = self.bindings(id).iter().map(|binding| self.label(binding)).collect();
        (!labels.is_empty()).then(|| labels.join(" or "))
    }

    /// The user's layout as seen so far: the logical key each physical key gave.
    pub fn layout(&self) -> &HashMap<Key, Key> {
        &self.layout
    }

    /// Whether command `id` is held: pressed (key, button or drag) and not yet released.
    pub fn is_held(&self, id: &str) -> bool {
        self.registry
            .index(id)
            .is_some_and(|index| self.held.iter().any(|held| held.command == index))
    }

    /// Whether the first step of a sequence was pressed and the keymap waits for the second.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Runs this frame's input in `ctx` through the keymap with `focus`, takes out the key events it used, and
    /// returns the commands fired.
    pub fn dispatch(&mut self, ctx: &egui::Context, focus: Focus) -> Vec<Activation> {
        let events = ctx.input(|input| input.events.clone());
        let processed = self.process(&events, focus);
        if !processed.consumed.is_empty() {
            ctx.input_mut(|input| {
                let mut at = 0;
                input.events.retain(|_| {
                    let keep = !processed.consumed.contains(&at);
                    at += 1;
                    keep
                });
            });
        }
        processed.activations
    }

    /// Matches raw egui `events` against the bindings with `focus`.
    pub fn process(&mut self, events: &[Event], focus: Focus) -> Processed {
        let mut out = Processed::default();
        for (at, event) in events.iter().enumerate() {
            match *event {
                Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat,
                    modifiers,
                } => {
                    if pressed && let Some(physical) = physical_key {
                        self.learn_layout(physical, key, modifiers);
                    }
                    if !pressed {
                        self.held.retain(|held| match held.release {
                            Release::Key {
                                key: down,
                                physical: down_physical,
                            } => down != key && (down_physical.is_none() || down_physical != physical_key),
                            Release::Button(_) => true,
                        });
                        continue;
                    }
                    let mods = Mods::from_egui(modifiers, self.platform);
                    if self.key(mods, key, physical_key, repeat, focus, &mut out.activations) {
                        out.consumed.push(at);
                    }
                }
                Event::PointerButton {
                    button,
                    pressed,
                    modifiers,
                    ..
                } => {
                    let button = MouseButton::from_egui(button);
                    if !pressed {
                        self.held.retain(|held| held.release != Release::Button(button));
                        continue;
                    }
                    let mods = Mods::from_egui(modifiers, self.platform);
                    let chords = [
                        Chord::new(mods, Trigger::Mouse(button)),
                        Chord::new(mods, Trigger::Drag(button)),
                    ];
                    if let Some(slot) = self.best(&chords, focus, true) {
                        out.activations.push(self.fire(slot.command, ActivationKind::Press));
                        self.held.push(Held {
                            command: slot.command,
                            release: Release::Button(button),
                        });
                    }
                }
                Event::MouseWheel {
                    unit, delta, modifiers, ..
                } => {
                    let along = if delta.y != 0.0 { delta.y } else { delta.x };
                    let lines = match unit {
                        MouseWheelUnit::Point => along / POINTS_PER_LINE,
                        MouseWheelUnit::Line => along,
                        MouseWheelUnit::Page => along * LINES_PER_PAGE,
                    };
                    if lines == 0.0 {
                        continue;
                    }
                    let mods = Mods::from_egui(modifiers, self.platform);
                    if let Some(slot) = self.best(&[Chord::new(mods, Trigger::Wheel)], focus, true) {
                        out.activations
                            .push(self.fire(slot.command, ActivationKind::Wheel(lines)));
                    }
                }
                Event::WindowFocused(false) => {
                    // Keys released in another window are never seen here.
                    self.held.clear();
                    self.pending = None;
                }
                _ => {}
            }
        }
        out
    }

    /// Handles a key press. Returns whether the keymap used it.
    fn key(
        &mut self,
        mods: Mods,
        key: Key,
        physical: Option<Key>,
        repeat: bool,
        focus: Focus,
        fired: &mut Vec<Activation>,
    ) -> bool {
        let mut chords = vec![Chord::new(mods, Trigger::logical(key))];
        if let Some(physical) = physical {
            chords.push(Chord::new(mods, Trigger::physical(physical)));
        }
        if let Some(waiting) = self.pending.take() {
            if repeat {
                self.pending = Some(waiting);
                return true;
            }
            // The second step: it completes a sequence, or ends the wait (Esc, or any other key).
            let second = waiting.into_iter().find(|slot| {
                self.bindings[slot.command][slot.binding]
                    .second
                    .is_some_and(|step| chords.contains(&step.canonical(self.platform)))
            });
            if let Some(slot) = second {
                fired.push(self.fire(slot.command, ActivationKind::Press));
            }
            return true;
        }
        if let Some(slot) = self.best(&chords, focus, false) {
            let command = &self.registry.commands()[slot.command];
            match (repeat, command.repeat) {
                (false, _) => {
                    fired.push(self.fire(slot.command, ActivationKind::Press));
                    self.held.push(Held {
                        command: slot.command,
                        release: Release::Key { key, physical },
                    });
                }
                (true, Repeat::Allow) => fired.push(self.fire(slot.command, ActivationKind::Repeat)),
                // Swallowed: it is the command's key, just held.
                (true, Repeat::Ignore) => {}
            }
            return true;
        }
        if repeat {
            return false;
        }
        let starts: Vec<Slot> = self.candidates(&chords, focus, false, true).collect();
        if starts.is_empty() {
            return false;
        }
        self.pending = Some(starts);
        true
    }

    /// The most specific active one-step binding whose first step is one of `chords`.
    fn best(&self, chords: &[Chord], focus: Focus, pointer: bool) -> Option<Slot> {
        self.candidates(chords, focus, pointer, false).max_by(|a, b| {
            let depth = |slot: &Slot| self.specificity(slot.command, focus, pointer);
            // The deepest context, then the command registered first.
            depth(a).cmp(&depth(b)).then(b.command.cmp(&a.command))
        })
    }

    /// The active bindings whose first step is one of `chords`: one-step bindings, or the sequences when `sequences`.
    fn candidates<'a>(
        &'a self,
        chords: &'a [Chord],
        focus: Focus,
        pointer: bool,
        sequences: bool,
    ) -> impl Iterator<Item = Slot> + 'a {
        chords
            .iter()
            .filter_map(|chord| self.lookup.get(chord))
            .flatten()
            .copied()
            .filter(move |slot| self.bindings[slot.command][slot.binding].second.is_some() == sequences)
            .filter(move |slot| {
                let command = &self.registry.commands()[slot.command];
                // A text field gets the keys; the wheel and buttons are not typing.
                let typing_blocks = !pointer && focus.typing && !command.while_typing;
                // A game with the input gets everything but the few commands that control it.
                let play_blocks = focus.playing && !command.during_play;
                !typing_blocks && !play_blocks && self.specificity(slot.command, focus, pointer).is_some()
            })
    }

    /// How specific the command at `index` is in `focus` (its deepest active context), or `None` when it is not
    /// active. Mouse and wheel bindings of [`Command::on_hover`] commands follow the hovered panel, not focus.
    fn specificity(&self, index: usize, focus: Focus, pointer: bool) -> Option<u8> {
        let command = &self.registry.commands()[index];
        let at = if pointer && command.on_hover {
            focus.hovered?
        } else {
            focus.context
        };
        command
            .contexts
            .iter()
            .filter(|context| context.contains(at))
            .map(|context| context.depth())
            .max()
    }

    fn fire(&self, index: usize, kind: ActivationKind) -> Activation {
        Activation {
            command: self.registry.commands()[index].id,
            kind,
        }
    }

    /// Notes which logical key `physical` gives. Only from presses without modifiers, which can change the key.
    fn learn_layout(&mut self, physical: Key, logical: Key, modifiers: egui::Modifiers) {
        if !modifiers.is_none() {
            return;
        }
        if self.layout.insert(physical, logical) != Some(logical) {
            // Physical keys clash as the layout labels them.
            self.find_conflicts();
        }
    }

    /// Recomputes every command's bindings, the lookup and the conflicts. Only when something changed.
    fn rebuild(&mut self) {
        self.bindings = (0..self.registry.commands().len())
            .map(|index| self.current(index))
            .collect();
        self.lookup.clear();
        for (command, bindings) in self.bindings.iter().enumerate() {
            for (binding, b) in bindings.iter().enumerate() {
                self.lookup
                    .entry(b.first.canonical(self.platform))
                    .or_default()
                    .push(Slot { command, binding });
            }
        }
        self.pending = None;
        self.find_conflicts();
    }

    fn find_conflicts(&mut self) {
        self.conflicts = (0..self.bindings.len())
            .map(|index| {
                self.bindings[index]
                    .iter()
                    .flat_map(|binding| {
                        self.clashes(index, binding).map(|slot| Conflict {
                            mine: *binding,
                            command: self.registry.commands()[slot.command].id,
                            theirs: self.bindings[slot.command][slot.binding],
                        })
                    })
                    .collect()
            })
            .collect();
    }

    /// The bindings of the command at `index`: its override's, else its defaults.
    fn current(&self, index: usize) -> Vec<Binding> {
        let command = &self.registry.commands()[index];
        match self.overrides.get(command.id) {
            Some(texts) => texts
                .iter()
                .filter_map(|text| {
                    Binding::parse(text)
                        .map_err(|error| log::warn!("keybinds: {}: {error}; ignored", command.id))
                        .ok()
                })
                .collect(),
            None => self.registry.defaults(index).to_vec(),
        }
    }

    /// Leaves out of each command on its defaults any default that clashes with a key the user bound to another
    /// command, saves that as an override (so it is decided once), and says so.
    fn protect_user_keys(&mut self) {
        let user: Vec<(usize, Binding)> = self
            .overrides
            .keys()
            .filter_map(|id| self.registry.index(id))
            .flat_map(|index| self.current(index).into_iter().map(move |binding| (index, binding)))
            .collect();
        for index in 0..self.registry.commands().len() {
            let command = self.registry.commands()[index];
            if self.overrides.contains_key(command.id) {
                continue;
            }
            let mut kept = Vec::new();
            for default in self.registry.defaults(index) {
                let taken = user.iter().find(|(owner, binding)| {
                    *owner != index
                        && contexts_overlap(&command, &self.registry.commands()[*owner])
                        && default.overlaps(binding, self.platform, &self.layout)
                });
                match taken {
                    Some((owner, _)) => {
                        let label = self.label(default);
                        self.notices.push(format!(
                            "\u{201C}{}\u{201D} now has {label} by default, but you use {label} for \u{201C}{}\u{201D}, \
                             so it was left out. You can bind it in Settings, Keybinds.",
                            command.title,
                            self.registry.commands()[*owner].title,
                        ));
                    }
                    None => kept.push(default.to_string()),
                }
            }
            if kept.len() < self.registry.defaults(index).len() {
                self.overrides.insert(command.id.to_owned(), kept);
                self.changed = true;
            }
        }
    }

    /// The overrides as a keymap file.
    pub fn export(&self) -> String {
        let mut document = Table::new();
        document.insert("format".into(), Value::String(KEYMAP_FORMAT.into()));
        document.insert("version".into(), Value::Integer(1));
        let mut keybinds = Table::new();
        write_overrides(&self.overrides, &mut keybinds);
        document.insert("keybinds".into(), Value::Table(keybinds));
        format!("{EXPORT_HEADER}{document}")
    }

    /// Takes the bindings of a keymap file (as [`Keymap::export`] writes, or a `settings.toml`): every command it
    /// names gets its bindings, every other command its defaults. Ids this Studio does not know are kept from both.
    /// Returns how many commands the file names.
    pub fn import(&mut self, text: &str) -> Result<usize, KeymapError> {
        let document: Table = text
            .parse()
            .map_err(|error: toml::de::Error| KeymapError::NotToml(error.message().to_owned()))?;
        if document
            .get("format")
            .is_some_and(|format| format.as_str() != Some(KEYMAP_FORMAT))
        {
            return Err(KeymapError::NotAKeymap);
        }
        let table = document
            .get("keybinds")
            .and_then(Value::as_table)
            .ok_or(KeymapError::NotAKeymap)?;
        let imported = read_overrides(table);
        let count = imported.len();
        let registry = &self.registry;
        self.overrides.retain(|id, _| registry.index(id).is_none());
        self.overrides.extend(imported);
        self.protect_user_keys();
        self.changed = true;
        self.rebuild();
        Ok(count)
    }

    /// Exports the keymap to `path`.
    pub fn export_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(path, self.export())
    }

    /// Imports the keymap file at `path`.
    pub fn import_from(&mut self, path: &std::path::Path) -> Result<usize, KeymapError> {
        let text = std::fs::read_to_string(path)?;
        self.import(&text)
    }
}

/// The overrides in a `[keybinds]` table: each key whose value is a list of texts. Any other value is logged and
/// left out (and stays in the file, see [`write_overrides`]).
pub fn read_overrides(table: &Table) -> Overrides {
    let mut overrides = Overrides::new();
    for (id, value) in table {
        let texts = value.as_array().and_then(|list| {
            list.iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect::<Option<Vec<String>>>()
        });
        match texts {
            Some(texts) => {
                overrides.insert(id.clone(), texts);
            }
            None => log::warn!("keybinds: `{id} = {value}` is not a list of bindings; ignored"),
        }
    }
    overrides
}

/// Writes `overrides` into a `[keybinds]` table, replacing every list of texts there (those are overrides, so one
/// reset is removed) and leaving any other value alone.
pub fn write_overrides(overrides: &Overrides, table: &mut Table) {
    table.retain(|_, value| !value.as_array().is_some_and(|list| list.iter().all(Value::is_str)));
    for (id, texts) in overrides {
        table.insert(
            id.clone(),
            Value::Array(texts.iter().cloned().map(Value::String).collect()),
        );
    }
}

#[cfg(test)]
mod tests {
    use egui::{Modifiers, PointerButton, TouchPhase, pos2, vec2};

    use super::*;

    const PRIMARY: Modifiers = Modifiers::COMMAND;

    fn press(key: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn repeat(key: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: true,
            modifiers,
        }
    }

    fn release(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    /// A key on an AZERTY keyboard: `physical` is where it is (US names), `logical` what it types.
    fn azerty(physical: Key, logical: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key: logical,
            physical_key: Some(physical),
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn button(button: PointerButton, pressed: bool, modifiers: Modifiers) -> Event {
        Event::PointerButton {
            pos: pos2(10.0, 10.0),
            button,
            pressed,
            modifiers,
        }
    }

    fn wheel(lines: f32, modifiers: Modifiers) -> Event {
        Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(0.0, lines),
            phase: TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers,
        }
    }

    fn fired(keymap: &mut Keymap, events: &[Event], focus: Focus) -> Vec<&'static str> {
        keymap
            .process(events, focus)
            .activations
            .into_iter()
            .map(|activation| activation.command)
            .collect()
    }

    fn studio(overrides: &Overrides) -> Keymap {
        Keymap::new(Registry::studio(), Platform::Other, overrides)
    }

    fn overrides(entries: &[(&str, &[&str])]) -> Overrides {
        entries
            .iter()
            .map(|(id, texts)| (id.to_string(), texts.iter().map(|t| t.to_string()).collect()))
            .collect()
    }

    fn parse(text: &str) -> Binding {
        Binding::parse(text).unwrap()
    }

    /// Commands that share F5 at three depths, and one that only works in the editor.
    const NESTED: &[Command] = &[
        Command {
            defaults: &["F5"],
            ..Command::new("test.global", "Global F5", "Test", "Anywhere.")
        },
        Command {
            contexts: &[Context::Viewport],
            defaults: &["F5"],
            ..Command::new("test.viewport", "Viewport F5", "Test", "In the viewport.")
        },
        Command {
            contexts: &[Context::Editor],
            defaults: &["Primary+D"],
            ..Command::new("edit.duplicate", "Duplicate", "Test", "In the editor.")
        },
        Command {
            contexts: &[Context::Explorer],
            defaults: &["F2"],
            ..Command::new("explorer.rename", "Rename", "Test", "In the explorer.")
        },
    ];

    #[test]
    fn the_most_specific_context_wins() {
        let mut keymap = Keymap::new(Registry::new(NESTED), Platform::Other, &Overrides::new());
        let f5 = [press(Key::F5, Modifiers::NONE)];
        assert_eq!(fired(&mut keymap, &f5, Focus::new(Context::Global)), ["test.global"]);
        assert_eq!(fired(&mut keymap, &f5, Focus::new(Context::Editor)), ["test.global"]);
        assert_eq!(
            fired(&mut keymap, &f5, Focus::new(Context::Viewport)),
            ["test.viewport"]
        );
        // A sibling panel is not inside the viewport: the global binding applies there.
        assert_eq!(fired(&mut keymap, &f5, Focus::new(Context::Explorer)), ["test.global"]);

        let duplicate = [press(Key::D, PRIMARY)];
        assert!(fired(&mut keymap, &duplicate, Focus::new(Context::Global)).is_empty());
        assert_eq!(
            fired(&mut keymap, &duplicate, Focus::new(Context::Assets)),
            ["edit.duplicate"]
        );
        let rename = [press(Key::F2, Modifiers::NONE)];
        assert!(
            fired(&mut keymap, &rename, Focus::new(Context::Assets)).is_empty(),
            "focus, not hover"
        );
        let hovering = Focus {
            hovered: Some(Context::Explorer),
            ..Focus::new(Context::Assets)
        };
        assert!(
            fired(&mut keymap, &rename, hovering).is_empty(),
            "hover never starts key commands"
        );
        assert_eq!(
            fired(&mut keymap, &rename, Focus::new(Context::Explorer)),
            ["explorer.rename"]
        );
        // The shadowed global binding is a conflict the page shows.
        assert_eq!(keymap.conflicts("test.viewport")[0].command, "test.global");
        assert!(keymap.conflicts("explorer.rename").is_empty());
    }

    #[test]
    fn a_focused_text_field_blocks_commands_not_marked_while_typing() {
        let mut keymap = Keymap::new(Registry::new(NESTED), Platform::Other, &Overrides::new());
        let typing = Focus {
            typing: true,
            ..Focus::new(Context::Editor)
        };
        let events = [press(Key::D, PRIMARY), press(Key::F5, Modifiers::NONE)];
        let processed = keymap.process(&events, typing);
        assert!(processed.activations.is_empty());
        assert!(processed.consumed.is_empty(), "the text field gets the keys");

        let mut keymap = studio(&Overrides::new());
        let processed = keymap.process(&[press(Key::Comma, PRIMARY)], typing);
        assert_eq!(
            processed.activations[0].command,
            ids::SETTINGS_OPEN,
            "marked while typing"
        );
        assert_eq!(processed.consumed, [0]);
    }

    #[test]
    fn a_game_with_the_input_gets_every_key_but_the_play_controls() {
        let mut keymap = studio(&Overrides::new());
        let playing = Focus {
            playing: true,
            ..Focus::new(Context::Editor)
        };
        // Save and Close project are the game's keys now; Stop, Pause, Step and Settings are not.
        let events = [
            press(Key::S, PRIMARY),
            press(Key::W, PRIMARY),
            press(Key::F5, Modifiers::SHIFT),
            press(Key::F7, Modifiers::NONE),
            press(Key::F6, Modifiers::NONE),
            press(Key::Comma, PRIMARY),
        ];
        let processed = keymap.process(&events, playing);
        let fired: Vec<&str> = processed
            .activations
            .iter()
            .map(|activation| activation.command)
            .collect();
        assert_eq!(
            fired,
            [ids::TEST_STOP, ids::TEST_PAUSE, ids::TEST_STEP, ids::SETTINGS_OPEN]
        );
        assert_eq!(processed.consumed, [2, 3, 4, 5], "the rest go to the game");
        // Without the game, Primary+S saves.
        assert_eq!(
            fired_in(&mut keymap, &[press(Key::S, PRIMARY)], Focus::new(Context::Editor)),
            [ids::FILE_SAVE]
        );
    }

    fn fired_in(keymap: &mut Keymap, events: &[Event], focus: Focus) -> Vec<&'static str> {
        fired(keymap, events, focus)
    }

    #[test]
    fn commands_ignore_auto_repeat_unless_they_opt_in() {
        let mut keymap = Keymap::new(Registry::new(NESTED), Platform::Other, &Overrides::new());
        let held = [press(Key::D, PRIMARY), repeat(Key::D, PRIMARY), repeat(Key::D, PRIMARY)];
        let processed = keymap.process(&held, Focus::new(Context::Editor));
        assert_eq!(processed.activations.len(), 1, "holding Primary+D duplicates once");
        assert_eq!(processed.consumed, [0, 1, 2], "the repeats are still the command's");

        let mut keymap = studio(&Overrides::new());
        let viewport = Focus::new(Context::Viewport);
        let held = [
            press(Key::W, Modifiers::NONE),
            repeat(Key::W, Modifiers::NONE),
            repeat(Key::W, Modifiers::NONE),
        ];
        let kinds: Vec<ActivationKind> = keymap
            .process(&held, viewport)
            .activations
            .iter()
            .map(|activation| activation.kind)
            .collect();
        assert_eq!(
            kinds,
            [ActivationKind::Press, ActivationKind::Repeat, ActivationKind::Repeat]
        );
        assert!(keymap.is_held(ids::FLY_FORWARD));
        keymap.process(&[release(Key::W)], viewport);
        assert!(!keymap.is_held(ids::FLY_FORWARD));
    }

    #[test]
    fn mouse_buttons_drags_and_the_wheel_start_on_hover_over_the_viewport() {
        let mut keymap = studio(&Overrides::new());
        let right = button(PointerButton::Secondary, true, Modifiers::NONE);
        // Focused but not hovered: the pointer is elsewhere, so it is not navigation.
        assert!(fired(&mut keymap, std::slice::from_ref(&right), Focus::new(Context::Viewport)).is_empty());
        let over = Focus {
            hovered: Some(Context::Viewport),
            ..Focus::new(Context::Explorer)
        };
        assert_eq!(fired(&mut keymap, &[right], over), [ids::LOOK]);
        assert!(keymap.is_held(ids::LOOK), "a drag is held until the button is released");
        keymap.process(&[button(PointerButton::Secondary, false, Modifiers::NONE)], over);
        assert!(!keymap.is_held(ids::LOOK));

        assert_eq!(
            fired(
                &mut keymap,
                &[button(PointerButton::Primary, true, Modifiers::ALT)],
                over
            ),
            [ids::ORBIT]
        );
        assert!(
            fired(
                &mut keymap,
                &[button(PointerButton::Primary, true, Modifiers::NONE)],
                over
            )
            .is_empty(),
            "a plain click is not bound"
        );

        let processed = keymap.process(&[wheel(2.0, Modifiers::NONE), wheel(-1.0, Modifiers::SHIFT)], over);
        assert_eq!(
            processed.activations,
            [
                Activation {
                    command: ids::ZOOM,
                    kind: ActivationKind::Wheel(2.0)
                },
                Activation {
                    command: ids::SPEED,
                    kind: ActivationKind::Wheel(-1.0)
                },
            ]
        );
        assert!(processed.consumed.is_empty(), "pointer input stays for the panels");
        // A text field does not block the wheel over the viewport.
        let typing = Focus { typing: true, ..over };
        assert_eq!(fired(&mut keymap, &[wheel(1.0, Modifiers::NONE)], typing), [ids::ZOOM]);
        // Losing the window lets go of everything held.
        keymap.process(&[button(PointerButton::Secondary, true, Modifiers::NONE)], over);
        keymap.process(&[Event::WindowFocused(false)], over);
        assert!(!keymap.is_held(ids::LOOK));
    }

    #[test]
    fn movement_follows_the_place_and_shortcuts_follow_the_label_on_azerty() {
        let undo = Command {
            contexts: &[Context::Editor],
            defaults: &["Primary+Z"],
            ..Command::new("edit.undo", "Undo", "Edit", "Undoes.")
        };
        let mut commands = STUDIO_COMMANDS.to_vec();
        commands.push(undo);
        let mut keymap = Keymap::new(Registry::new(&commands), Platform::Other, &Overrides::new());
        let viewport = Focus::new(Context::Viewport);
        // AZERTY: the key where W is on QWERTY types Z, and the key where Q is types A.
        assert_eq!(
            fired(&mut keymap, &[azerty(Key::W, Key::Z, Modifiers::NONE)], viewport),
            [ids::FLY_FORWARD]
        );
        assert_eq!(
            fired(&mut keymap, &[azerty(Key::Q, Key::A, Modifiers::NONE)], viewport),
            [ids::FLY_DOWN]
        );
        // The key labelled A (QWERTY's Q) is not fly-left, which is QWERTY's A place (AZERTY's Q).
        assert_eq!(
            fired(&mut keymap, &[azerty(Key::A, Key::Q, Modifiers::NONE)], viewport),
            [ids::FLY_LEFT]
        );
        // Primary+Z is the key labelled Z, wherever it is.
        assert_eq!(
            fired(&mut keymap, &[azerty(Key::W, Key::Z, PRIMARY)], viewport),
            ["edit.undo"]
        );
        // And Primary+W is the key labelled W (QWERTY's Z place): Close project.
        assert_eq!(
            fired(&mut keymap, &[azerty(Key::Z, Key::W, PRIMARY)], viewport),
            [ids::PROJECT_CLOSE]
        );
        // The page shows the user's layout: fly forward reads "Z".
        assert_eq!(keymap.shortcut_text(ids::FLY_FORWARD).as_deref(), Some("Z"));
        assert_eq!(keymap.shortcut_text(ids::FLY_DOWN).as_deref(), Some("A"));
    }

    #[test]
    fn two_step_sequences_wait_for_their_second_key() {
        let mut keymap = studio(&overrides(&[(ids::SETTINGS_OPEN, &["Primary+K, S", "F10"])]));
        let global = Focus::new(Context::Global);
        let processed = keymap.process(&[press(Key::K, PRIMARY)], global);
        assert!(processed.activations.is_empty());
        assert_eq!(processed.consumed, [0], "the first step is the keymap's");
        assert!(keymap.is_pending());
        assert_eq!(
            fired(&mut keymap, &[press(Key::S, Modifiers::NONE)], global),
            [ids::SETTINGS_OPEN]
        );
        assert!(!keymap.is_pending());

        // Any other key ends the wait, and is not passed on.
        keymap.process(&[press(Key::K, PRIMARY)], global);
        let processed = keymap.process(&[press(Key::D, Modifiers::NONE)], global);
        assert!(processed.activations.is_empty());
        assert_eq!(processed.consumed, [0]);
        assert!(fired(&mut keymap, &[press(Key::S, Modifiers::NONE)], global).is_empty());
        // Esc too.
        keymap.process(&[press(Key::K, PRIMARY), press(Key::Escape, Modifiers::NONE)], global);
        assert!(!keymap.is_pending());
        assert_eq!(
            keymap.shortcut_text(ids::SETTINGS_OPEN).as_deref(),
            Some("Ctrl+K, S or F10"),
            "every binding is shown"
        );
    }

    #[test]
    fn conflicts_are_replaced_or_swapped() {
        let mut keymap = studio(&Overrides::new());
        let close = parse("Primary+W");
        let conflicts = keymap.conflicts_with(ids::SETTINGS_OPEN, &close);
        assert_eq!(conflicts.len(), 1, "Global overlaps Editor");
        assert_eq!(conflicts[0].command, ids::PROJECT_CLOSE);
        assert_eq!(
            keymap.conflicts_with(ids::FLY_FORWARD, &parse("Wheel"))[0].command,
            ids::ZOOM
        );
        assert!(
            keymap
                .conflicts_with(ids::FLY_FORWARD, &parse("Primary+Wheel"))
                .is_empty(),
            "other modifiers, other binding"
        );
        assert!(
            keymap.conflicts_with(ids::FLY_LEFT, &parse("F4")).is_empty(),
            "a key nothing uses"
        );
        assert!(
            keymap.conflicts_with(ids::FLY_FORWARD, &parse("Primary+K")).is_empty(),
            "nothing else uses Primary+K"
        );

        // Replace: Settings takes Primary+W; Close project loses it and is saved unbound.
        keymap.assign(ids::SETTINGS_OPEN, Some(0), close, Some(Resolution::Replace));
        assert_eq!(keymap.bindings(ids::SETTINGS_OPEN), [close]);
        assert!(keymap.bindings(ids::PROJECT_CLOSE).is_empty());
        assert_eq!(keymap.overrides()[ids::PROJECT_CLOSE], Vec::<String>::new());
        assert!(keymap.take_changed());

        // Swap: Settings takes Primary+W, and Close project gets the Primary+Comma it replaced.
        let mut keymap = studio(&Overrides::new());
        keymap.assign(ids::SETTINGS_OPEN, Some(0), close, Some(Resolution::Swap));
        assert_eq!(keymap.bindings(ids::SETTINGS_OPEN), [close]);
        assert_eq!(keymap.bindings(ids::PROJECT_CLOSE), [parse("Primary+Comma")]);
        assert!(keymap.conflicts(ids::SETTINGS_OPEN).is_empty());
        let editor = Focus::new(Context::Editor);
        assert_eq!(
            fired(&mut keymap, &[press(Key::W, PRIMARY)], editor),
            [ids::SETTINGS_OPEN]
        );
        assert_eq!(
            fired(&mut keymap, &[press(Key::Comma, PRIMARY)], editor),
            [ids::PROJECT_CLOSE]
        );

        // Added rather than replacing, a swap has nothing to give and replaces.
        let mut keymap = studio(&Overrides::new());
        keymap.assign(ids::SETTINGS_OPEN, None, close, Some(Resolution::Swap));
        assert_eq!(keymap.bindings(ids::SETTINGS_OPEN), [parse("Primary+Comma"), close]);
        assert!(keymap.bindings(ids::PROJECT_CLOSE).is_empty());

        // Kept both: the clash is shown inline on both commands.
        let mut keymap = studio(&Overrides::new());
        keymap.assign(ids::SETTINGS_OPEN, None, close, None);
        assert_eq!(keymap.conflicts(ids::SETTINGS_OPEN)[0].command, ids::PROJECT_CLOSE);
        assert_eq!(keymap.conflicts(ids::PROJECT_CLOSE)[0].command, ids::SETTINGS_OPEN);
    }

    #[test]
    fn studios_defaults_never_clash() {
        let keymap = studio(&Overrides::new());
        for command in keymap.registry().commands() {
            assert!(keymap.conflicts(command.id).is_empty(), "{}", command.id);
        }
        let mac = Keymap::new(Registry::studio(), Platform::Mac, &Overrides::new());
        assert_eq!(mac.shortcut_text(ids::SETTINGS_OPEN).as_deref(), Some("Cmd+,"));
    }

    #[test]
    fn only_overrides_are_stored_and_unknown_ids_are_kept() {
        let stored = overrides(&[
            ("future.teleport", &["Primary+J"]),
            (ids::FLY_UP, &["[R]", "not a binding"]),
        ]);
        let mut keymap = studio(&stored);
        assert_eq!(
            keymap.bindings(ids::FLY_UP),
            [parse("[R]")],
            "an unreadable text is ignored"
        );
        assert!(keymap.bindings("future.teleport").is_empty());
        assert!(!keymap.take_changed(), "nothing to save on load");

        // Back to the defaults: the override goes. Unbound: `[]`.
        keymap.set_bindings(ids::FLY_UP, vec![parse("[E]")]);
        assert!(!keymap.overrides().contains_key(ids::FLY_UP));
        keymap.set_bindings(ids::ZOOM, Vec::new());
        assert_eq!(keymap.overrides()[ids::ZOOM], Vec::<String>::new());
        keymap.remove_binding(ids::SETTINGS_OPEN, 0);
        keymap.reset_all();
        assert_eq!(
            keymap.overrides(),
            &overrides(&[("future.teleport", &["Primary+J"])]),
            "a newer Studio's bindings survive"
        );

        // Through a `[keybinds]` table and back.
        let mut table: Table = "\"future.teleport\" = [\"Primary+J\"]\nnote = 3\n".parse().unwrap();
        keymap.set_bindings(ids::LOOK, vec![parse("DragMiddle")]);
        write_overrides(keymap.overrides(), &mut table);
        assert_eq!(table["note"].as_integer(), Some(3));
        assert_eq!(read_overrides(&table), *keymap.overrides());
        keymap.reset(ids::LOOK);
        write_overrides(keymap.overrides(), &mut table);
        assert!(table.get(ids::LOOK).is_none(), "a reset removes the override");
    }

    #[test]
    fn a_new_default_never_takes_a_key_the_user_uses() {
        // The user put Settings on Primary+W, and this Studio's Close project has Primary+W by default.
        let stored = overrides(&[(ids::SETTINGS_OPEN, &["Primary+W"])]);
        let mut keymap = studio(&stored);
        assert!(keymap.bindings(ids::PROJECT_CLOSE).is_empty());
        let editor = Focus::new(Context::Editor);
        assert_eq!(
            fired(&mut keymap, &[press(Key::W, PRIMARY)], editor),
            [ids::SETTINGS_OPEN]
        );
        let notices = keymap.take_notices();
        assert_eq!(notices.len(), 1);
        assert!(
            notices[0].contains("Close project") && notices[0].contains("Ctrl+W"),
            "{notices:?}"
        );
        assert!(keymap.take_notices().is_empty());
        // Decided once: saved as an override, so the next start says nothing.
        assert!(keymap.take_changed());
        let saved = keymap.overrides().clone();
        assert_eq!(saved[ids::PROJECT_CLOSE], Vec::<String>::new());
        let mut again = studio(&saved);
        assert!(again.take_notices().is_empty());
        assert!(again.bindings(ids::PROJECT_CLOSE).is_empty());
        // Resetting it gives the default back, with the clash shown.
        again.reset(ids::PROJECT_CLOSE);
        assert_eq!(again.bindings(ids::PROJECT_CLOSE), [parse("Primary+W")]);
        assert!(!again.conflicts(ids::PROJECT_CLOSE).is_empty());
    }

    #[test]
    fn a_keymap_exports_and_imports() {
        let mut keymap = studio(&overrides(&[("future.teleport", &["Primary+J"])]));
        keymap.set_bindings(ids::SETTINGS_OPEN, vec![parse("F10"), parse("Primary+K, S")]);
        keymap.set_bindings(ids::ZOOM, Vec::new());
        let file = keymap.export();
        assert!(file.contains("format = \"metallic-keymap\""), "{file}");

        let mut other = studio(&overrides(&[(ids::FLY_UP, &["[R]"]), ("later.thing", &["F1"])]));
        assert_eq!(other.import(&file).unwrap(), 3);
        assert_eq!(other.bindings(ids::SETTINGS_OPEN), keymap.bindings(ids::SETTINGS_OPEN));
        assert!(other.bindings(ids::ZOOM).is_empty());
        assert_eq!(
            other.bindings(ids::FLY_UP),
            [parse("[E]")],
            "not in the file: the default"
        );
        assert!(other.overrides().contains_key("future.teleport"));
        assert!(
            other.overrides().contains_key("later.thing"),
            "this Studio's unknown ids stay"
        );
        assert!(other.take_changed());

        // Through a file.
        let path = std::env::temp_dir().join(format!("metallic-keymap-{}.toml", std::process::id()));
        keymap.export_to(&path).unwrap();
        let mut third = studio(&Overrides::new());
        assert_eq!(third.import_from(&path).unwrap(), 3);
        assert_eq!(third.overrides(), keymap.overrides());
        let _ = std::fs::remove_file(&path);

        assert!(matches!(other.import("[keybinds"), Err(KeymapError::NotToml(_))));
        assert!(matches!(other.import("x = 1"), Err(KeymapError::NotAKeymap)));
        assert!(matches!(
            other.import("format = \"other\"\n[keybinds]\n"),
            Err(KeymapError::NotAKeymap)
        ));
        assert_eq!(
            other.bindings(ids::SETTINGS_OPEN),
            keymap.bindings(ids::SETTINGS_OPEN),
            "unchanged"
        );
    }
}
