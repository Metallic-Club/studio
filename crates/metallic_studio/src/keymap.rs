//! Keyboard shortcuts as data.
//!
//! A [`Keymap`] is a table of (action id, key chord, context) rows. The Metallic preset is
//! [`METALLIC_PRESET`]; the Roblox Studio and Unity-style presets (#129) are more tables of the
//! same shape, and Settings > Keyboard edits a copy. Action ids are the ids of the
//! [`crate::actions::ActionRegistry`], so menus, the toolbar and tooltips all show the binding the
//! keymap holds. One chord means one action per context ([`Keymap::conflicts`] finds clashes).
use eframe::egui::{self, Key, Modifiers};

/// Where a binding applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyContext {
    /// Anywhere in the editor window, unless a text field has focus.
    Global,
    /// While the pointer is over the 3D viewport. Single letters live here.
    Viewport,
    /// While a gizmo or body drag is held: Esc cancels, X/Y/Z lock axes, R/T turn, Ctrl+D
    /// stamps. These keys mean nothing else during a drag.
    Drag,
    /// While a play session runs: Studio keys that win over the game (Esc stops). Viewport
    /// keys are off then; the game gets them.
    Play,
}

/// A key with modifiers, e.g. Ctrl+Shift+Z.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    /// The key.
    pub key: Key,
    /// Ctrl (Cmd on macOS).
    pub ctrl: bool,
    /// Shift.
    pub shift: bool,
    /// Alt.
    pub alt: bool,
}

impl KeyChord {
    /// Parses "Ctrl+Shift+Z", "F5", "Delete", "Q".
    pub fn parse(text: &str) -> Option<KeyChord> {
        let mut chord = KeyChord {
            key: Key::Escape,
            ctrl: false,
            shift: false,
            alt: false,
        };
        let mut key = None;
        for part in text.split('+') {
            match part.trim() {
                "Ctrl" => chord.ctrl = true,
                "Shift" => chord.shift = true,
                "Alt" => chord.alt = true,
                name => key = Some(Key::from_name(name)?),
            }
        }
        chord.key = key?;
        Some(chord)
    }

    /// True when `modifiers` match exactly (Ctrl means Cmd on macOS).
    pub fn matches_modifiers(&self, m: Modifiers) -> bool {
        m.command == self.ctrl && m.shift == self.shift && m.alt == self.alt
    }

    /// Text shown in menus and tooltips.
    pub fn label(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        s.push_str(if self.key == Key::Comma {
            ","
        } else {
            self.key.name()
        });
        s
    }
}

/// One keymap row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// Action id (see [`crate::actions`]).
    pub action: &'static str,
    /// The chord.
    pub chord: KeyChord,
    /// Where it applies.
    pub context: KeyContext,
}

/// The Metallic preset (UI design 5.15): action id, chord, context.
pub const METALLIC_PRESET: &[(&str, &str, KeyContext)] = &[
    ("edit.undo", "Ctrl+Z", KeyContext::Global),
    ("edit.redo", "Ctrl+Y", KeyContext::Global),
    ("edit.redo", "Ctrl+Shift+Z", KeyContext::Global),
    ("edit.duplicate", "Ctrl+D", KeyContext::Global),
    ("edit.delete", "Delete", KeyContext::Global),
    ("edit.group", "Ctrl+G", KeyContext::Global),
    ("edit.rename", "F2", KeyContext::Global),
    ("edit.select_all", "Ctrl+A", KeyContext::Global),
    ("file.save", "Ctrl+S", KeyContext::Global),
    ("file.close", "Ctrl+W", KeyContext::Global),
    ("settings.open", "Ctrl+,", KeyContext::Global),
    ("tool.select", "Q", KeyContext::Viewport),
    ("tool.move", "W", KeyContext::Viewport),
    ("tool.rotate", "E", KeyContext::Viewport),
    ("tool.scale", "R", KeyContext::Viewport),
    ("tool.transform", "T", KeyContext::Viewport),
    ("space.toggle", "X", KeyContext::Viewport),
    ("pivot.mode", "Z", KeyContext::Viewport),
    ("pivot.edit", "P", KeyContext::Viewport),
    ("snap.grid", "G", KeyContext::Viewport),
    ("palette.open", "Ctrl+K", KeyContext::Global),
    ("palette.commands", "Ctrl+Shift+P", KeyContext::Global),
    ("insert.at_cursor", "Shift+A", KeyContext::Viewport),
    ("increment.move_down", "[", KeyContext::Viewport),
    ("increment.move_up", "]", KeyContext::Viewport),
    ("increment.rotate_down", "Shift+[", KeyContext::Viewport),
    ("increment.rotate_up", "Shift+]", KeyContext::Viewport),
    // US layouts report Shift+[ as "{".
    ("increment.rotate_down", "Shift+{", KeyContext::Viewport),
    ("increment.rotate_up", "Shift+}", KeyContext::Viewport),
    ("build.rotate90", "Ctrl+R", KeyContext::Viewport),
    ("build.tilt90", "Ctrl+T", KeyContext::Viewport),
    ("build.drop", "End", KeyContext::Viewport),
    (
        "build.snap_to_grid",
        "Ctrl+Shift+Home",
        KeyContext::Viewport,
    ),
    ("hold.vertex", "V", KeyContext::Viewport),
    ("hold.summon", "Space", KeyContext::Viewport),
    ("view.frame_selection", "F", KeyContext::Viewport),
    ("view.frame_all", "Home", KeyContext::Viewport),
    ("play.start", "F5", KeyContext::Global),
    ("play.stop", "Shift+F5", KeyContext::Global),
    ("play.stop", "Escape", KeyContext::Play),
    ("output.clear", "Ctrl+L", KeyContext::Global),
    // Esc never closes anything: it clears the selection, or cancels a gizmo drag.
    ("edit.deselect", "Escape", KeyContext::Viewport),
    ("drag.cancel", "Escape", KeyContext::Drag),
    ("drag.lock_x", "X", KeyContext::Drag),
    ("drag.lock_y", "Y", KeyContext::Drag),
    ("drag.lock_z", "Z", KeyContext::Drag),
    ("drag.plane_x", "Shift+X", KeyContext::Drag),
    ("drag.plane_y", "Shift+Y", KeyContext::Drag),
    ("drag.plane_z", "Shift+Z", KeyContext::Drag),
    ("drag.turn", "R", KeyContext::Drag),
    ("drag.tilt", "T", KeyContext::Drag),
    ("drag.stamp", "Ctrl+D", KeyContext::Drag),
];

/// A set of bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Keymap {
    /// Builds a keymap from preset rows; rows whose chord does not parse are skipped (a test
    /// keeps the shipped preset free of those).
    pub fn from_rows(rows: &[(&'static str, &str, KeyContext)]) -> Keymap {
        Keymap {
            bindings: rows
                .iter()
                .filter_map(|&(action, chord, context)| {
                    Some(Binding {
                        action,
                        chord: KeyChord::parse(chord)?,
                        context,
                    })
                })
                .collect(),
        }
    }

    /// The Metallic preset.
    pub fn metallic() -> Keymap {
        Keymap::from_rows(METALLIC_PRESET)
    }

    /// All rows.
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// The first chord bound to `action` (shown in menus and tooltips).
    pub fn chord(&self, action: &str) -> Option<KeyChord> {
        self.bindings
            .iter()
            .find(|b| b.action == action)
            .map(|b| b.chord)
    }

    /// Menu text for the shortcut of `action`, or "".
    pub fn label(&self, action: &str) -> String {
        self.chord(action).map(|c| c.label()).unwrap_or_default()
    }

    /// Adds or replaces the binding of `action` in `context` (Settings > Keyboard).
    pub fn bind(&mut self, action: &'static str, chord: KeyChord, context: KeyContext) {
        self.bindings
            .retain(|b| !(b.action == action && b.context == context));
        self.bindings.push(Binding {
            action,
            chord,
            context,
        });
    }

    /// Pairs of rows that give one chord two actions in the same context.
    pub fn conflicts(&self) -> Vec<(&Binding, &Binding)> {
        let mut out = Vec::new();
        for (i, a) in self.bindings.iter().enumerate() {
            for b in &self.bindings[i + 1..] {
                if a.chord == b.chord && a.context == b.context && a.action != b.action {
                    out.push((a, b));
                }
            }
        }
        out
    }

    /// Actions whose chord was pressed this frame in one of `active` contexts. Modifiers must
    /// match exactly (Ctrl+Shift+Z is not Ctrl+Z). Matching key presses are consumed so egui
    /// widgets do not also react. `out` is cleared first.
    pub fn pressed(&self, ctx: &egui::Context, active: &[KeyContext], out: &mut Vec<&'static str>) {
        self.pressed_where(ctx, active, out, |_| true);
    }

    /// Like [`Keymap::pressed`], but only bindings whose action `owns` claims are matched; any
    /// other key, bound or not, stays in egui's event list. While playing, a disabled editing
    /// shortcut (Ctrl+Z, Delete, F2...) is thereby left for the game instead of being swallowed.
    pub fn pressed_where(
        &self,
        ctx: &egui::Context,
        active: &[KeyContext],
        out: &mut Vec<&'static str>,
        owns: impl Fn(&'static str) -> bool,
    ) {
        out.clear();
        ctx.input_mut(|input| {
            input.events.retain(|event| {
                let egui::Event::Key {
                    key,
                    modifiers,
                    pressed: true,
                    ..
                } = event
                else {
                    return true;
                };
                let Some(b) = self.bindings.iter().find(|b| {
                    active.contains(&b.context)
                        && b.chord.key == *key
                        && b.chord.matches_modifiers(*modifiers)
                        && owns(b.action)
                }) else {
                    return true;
                };
                if !out.contains(&b.action) {
                    out.push(b.action);
                }
                false
            });
        });
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap::metallic()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_parses_and_has_no_conflicts() {
        let map = Keymap::metallic();
        assert_eq!(map.bindings().len(), METALLIC_PRESET.len());
        assert!(map.conflicts().is_empty(), "{:?}", map.conflicts());
    }

    #[test]
    fn keys_of_actions_not_owned_stay_in_the_event_list() {
        let map = Keymap::metallic();
        let ctx = egui::Context::default();
        let ctrl_z = egui::Event::Key {
            key: egui::Key::Z,
            physical_key: Some(egui::Key::Z),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        };
        let escape = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: Some(egui::Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let raw = egui::RawInput {
            events: vec![ctrl_z, escape],
            ..Default::default()
        };
        let mut pressed = Vec::new();
        let mut left = 0;
        let mut out = ctx.run_ui(raw, |ui| {
            map.pressed_where(
                ui.ctx(),
                &[KeyContext::Global, KeyContext::Play],
                &mut pressed,
                |id| id.starts_with("play."),
            );
            left = ui.ctx().input(|i| i.events.len());
        });
        out.textures_delta.clear();
        assert_eq!(pressed, ["play.stop"]);
        assert_eq!(left, 1, "Ctrl+Z is left for the game");
    }

    #[test]
    fn labels_and_rebinding() {
        let mut map = Keymap::metallic();
        assert_eq!(map.label("edit.undo"), "Ctrl+Z");
        assert_eq!(map.label("play.stop"), "Shift+F5");
        let chord = KeyChord::parse("Ctrl+Shift+U").unwrap();
        map.bind("edit.undo", chord, KeyContext::Global);
        assert_eq!(map.label("edit.undo"), "Ctrl+Shift+U");
        map.bind("edit.redo", chord, KeyContext::Global);
        assert_eq!(map.conflicts().len(), 1);
    }
}
