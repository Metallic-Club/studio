//! The colour popover (#221, [`crate::colour`]) hosted by the editor.
//!
//! It opens from a Color3 row in Properties (`BasePart.Color`, `TextColor3`, ...) and from C
//! over the viewport (the selection's `Color`). The popover never edits the scene; this host
//! turns its events into edits:
//!
//! - [`ColourEvent::Preview`] / [`ColourEvent::PreviewHueShift`]: write the colour to every
//!   target as a live preview (no undo step), the way Properties previews a drag.
//! - [`ColourEvent::Commit`] / [`ColourEvent::CommitHueShift`]: one undo step from the colours
//!   before the gesture to the new ones, named by the popover ("Colour: #5280CC (3 parts)").
//! - [`ColourEvent::Cancel`]: put the colours from before the gesture back.
//! - [`ColourEvent::EyedropperClick`]: the `Color` of the part under the cursor, or with Alt (or
//!   over no part) the window's pixel.
//!
//! The popover closes (committing anything pending) when the selection changes or Play starts.
use std::cell::Cell;
use std::rc::Rc;

use eframe::egui::{self, Pos2};
use metallic_core::{Color3, InstanceId, Value};

use crate::actions::{Action, ActionRegistry, ToolbarGroup};
use crate::colour::{ColourEvent, ColourLibrary, ColourPopover, PaletteStore, RecentColours, Rgb8};
use crate::commands::Command;
use crate::editor::EditorState;
use crate::icons::Icon;

/// Action id of "Colour" (C over the viewport).
pub const OPEN_ID: &str = "colour.open";

/// What the open popover edits.
struct Target {
    prop: String,
    /// Each target and its colour before the current gesture.
    base: Vec<(InstanceId, Rgb8)>,
    selection_revision: u64,
}

/// The editor's colour popover.
pub struct ColourDock {
    popover: ColourPopover,
    library: ColourLibrary,
    target: Option<Target>,
    /// Set by the C action; the dock opens next frame where the pointer is.
    requested: Rc<Cell<bool>>,
}

/// The ids in the selection that have a Color3 `prop`.
fn colour_targets(state: &EditorState, prop: &str) -> Vec<(InstanceId, Rgb8)> {
    state
        .selection
        .ids()
        .iter()
        .filter_map(|&id| match state.world.get_ref(id, prop) {
            Ok(Value::Color3(c)) => Some((id, to_rgb8(*c))),
            _ => None,
        })
        .collect()
}

fn to_rgb8(c: Color3) -> Rgb8 {
    let b = |v: f64| v.round().clamp(0.0, 255.0) as u8;
    Rgb8::new(b(c.r), b(c.g), b(c.b))
}

fn to_color3(c: Rgb8) -> Value {
    Value::Color3(Color3::new(f64::from(c.r), f64::from(c.g), f64::from(c.b)))
}

impl ColourDock {
    /// A closed popover; registers the C action. Recent colours are the user's own only when
    /// the editor may write files (not in screenshot or test windows).
    pub fn new(state: &EditorState, actions: &mut ActionRegistry) -> ColourDock {
        let (recent, palettes) = if state.can_save {
            (
                RecentColours::load_default(),
                PaletteStore::load(&state.project_dir),
            )
        } else {
            (
                RecentColours::load(None),
                PaletteStore::in_memory(Vec::new()),
            )
        };
        let requested = Rc::new(Cell::new(false));
        let flag = requested.clone();
        actions.register(
            Action::new(
                OPEN_ID,
                "Colour",
                Icon::Text("C"),
                ToolbarGroup::Hidden,
                move |_| flag.set(true),
            )
            .enabled(|s| !s.play.is_playing() && !colour_targets(s, "Color").is_empty()),
        );
        ColourDock {
            popover: ColourPopover::new("editor"),
            library: ColourLibrary {
                recent,
                palettes,
                ..Default::default()
            },
            target: None,
            requested,
        }
    }

    /// True while the popover shows.
    pub fn is_open(&self) -> bool {
        self.popover.is_open()
    }

    /// The popover (for tests: its colour and rectangle).
    pub fn popover(&self) -> &ColourPopover {
        &self.popover
    }

    /// Opens the popover on `prop` of the selection, top-left corner at `pos`. Returns false
    /// when no selected instance has that Color3 property.
    pub fn open(&mut self, state: &EditorState, prop: &str, pos: Pos2) -> bool {
        if state.play.is_playing() {
            return false;
        }
        let base = colour_targets(state, prop);
        let Some(&(_, active)) = base.last() else {
            return false;
        };
        let mixed = base.iter().any(|(_, c)| *c != active);
        self.popover.open(active, base.len(), mixed, pos);
        self.target = Some(Target {
            prop: prop.to_string(),
            base,
            selection_revision: state.selection.revision(),
        });
        true
    }

    /// Shows the popover and applies its events. `hovered_part` is the part under the cursor in
    /// the viewport (for the eyedropper); `pointer` is where C opens the popover.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        state: &mut EditorState,
        hovered_part: Option<InstanceId>,
    ) {
        if self.requested.take() {
            let at = ctx.pointer_latest_pos().unwrap_or(Pos2::new(200.0, 120.0));
            self.open(state, "Color", at + egui::vec2(16.0, -120.0));
        }
        let stale = self.target.as_ref().is_some_and(|t| {
            t.selection_revision != state.selection.revision() || state.play.is_playing()
        });
        if stale && self.popover.is_open() {
            self.popover.close(&mut self.library);
        }
        let events = self.popover.show(ctx, &mut self.library);
        for event in events {
            self.handle(ctx, state, hovered_part, event);
        }
        if !self.popover.is_open() {
            self.target = None;
        }
    }

    fn write(&self, state: &mut EditorState, colour: impl Fn(Rgb8) -> Rgb8) {
        let Some(t) = &self.target else { return };
        for &(id, before) in &t.base {
            let _ = state.world.set(id, &t.prop, to_color3(colour(before)));
        }
    }

    /// One undo step from the gesture's start colours to `colour(start)`.
    fn commit(&mut self, state: &mut EditorState, name: String, colour: impl Fn(Rgb8) -> Rgb8) {
        let Some(t) = &mut self.target else { return };
        let mut commands = Vec::new();
        for (id, before) in t.base.iter_mut() {
            let after = colour(*before);
            let _ = state.world.set(*id, &t.prop, to_color3(after));
            if after != *before {
                commands.push(Command::set(
                    *id,
                    &t.prop,
                    to_color3(*before),
                    to_color3(after),
                ));
            }
            *before = after;
        }
        // A batch even for one target, so the step keeps the popover's name.
        if !commands.is_empty() {
            state.record(Command::Batch { name, commands });
        }
    }

    fn handle(
        &mut self,
        ctx: &egui::Context,
        state: &mut EditorState,
        hovered_part: Option<InstanceId>,
        event: ColourEvent,
    ) {
        match event {
            ColourEvent::Preview(c) => self.write(state, |_| c),
            ColourEvent::PreviewHueShift(d) => {
                self.write(state, |before| crate::colour::maths::shift_hue(before, d))
            }
            ColourEvent::Commit { colour, undo_name } => {
                self.commit(state, undo_name, |_| colour);
            }
            ColourEvent::CommitHueShift { degrees, undo_name } => {
                self.commit(state, undo_name, |before| {
                    crate::colour::maths::shift_hue(before, degrees)
                });
            }
            ColourEvent::Cancel => self.write(state, |before| before),
            ColourEvent::Closed => {}
            ColourEvent::EyedropperClick { pos, alt, .. } => {
                let picked = hovered_part.filter(|_| !alt).and_then(|id| {
                    match state.world.get_ref(id, "Color") {
                        Ok(Value::Color3(c)) => Some(to_rgb8(*c)),
                        _ => None,
                    }
                });
                match picked {
                    Some(c) => self.popover.pick(c, &mut self.library),
                    None => self.popover.pick_screen_pixel(ctx, pos),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::OutputLog;
    use metallic_core::World;
    use std::path::Path;

    #[test]
    fn preview_writes_and_commit_is_one_step_and_cancel_restores() {
        let log = OutputLog::new(16);
        let mut world = World::new_with_services();
        let ws = world.root("Workspace").unwrap();
        let a = world.create("Part", Some(ws)).unwrap();
        let b = world.create("Part", Some(ws)).unwrap();
        let start = world.get(a, "Color").unwrap();
        let mut s = EditorState::new(world, Path::new("."), Path::new("x"), "T", log.sink());
        s.can_save = false;
        s.selection.set([a, b]);
        let mut actions = ActionRegistry::new();
        let mut dock = ColourDock::new(&s, &mut actions);
        assert!(dock.open(&s, "Color", Pos2::ZERO));
        let ctx = egui::Context::default();
        let blue = Rgb8::from_u32(0x5280CC);
        for c in [Rgb8::from_u32(0x102030), blue] {
            dock.handle(&ctx, &mut s, None, ColourEvent::Preview(c));
        }
        assert_eq!(s.world.get(b, "Color").unwrap(), to_color3(blue));
        assert!(s.history.is_empty(), "previews are not undo steps");
        dock.handle(
            &ctx,
            &mut s,
            None,
            ColourEvent::Commit {
                colour: blue,
                undo_name: "Colour: #5280CC (2 parts)".into(),
            },
        );
        assert_eq!(s.history.len(), 1);
        dock.handle(&ctx, &mut s, None, ColourEvent::Preview(Rgb8::new(1, 2, 3)));
        dock.handle(&ctx, &mut s, None, ColourEvent::Cancel);
        assert_eq!(s.world.get(a, "Color").unwrap(), to_color3(blue));
        s.undo();
        assert_eq!(s.world.get(a, "Color").unwrap(), start);
        assert_eq!(s.world.get(b, "Color").unwrap(), start);
    }
}
