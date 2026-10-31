//! First-time hints (building-tools spec 14.1): one card at a time, bottom left of the viewport,
//! with a keycap, one sentence, "Show me" and "Got it".
//!
//! Triggers are behaviours, not timers, and each hint fires at most once per user. Rules: never
//! during a drag, Play or camera flight; at most one hint per [`SPACING`]; Esc dismisses;
//! Settings > "Show tips" turns them all off and "Reset tips" brings them back. Which hints were
//! shown lives in `hints.ron` in the config folder.
//!
//! The editor feeds [`HintEvent`]s to [`Hints::note`]; the detectors here turn them into hints.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Area, Frame, Id, Key, Layout, Margin, Order, Rect, RichText, pos2,
};
use metallic_core::InstanceId;
use serde::{Deserialize, Serialize};

use crate::theme::{self, fonts};
use crate::widgets::button::{Button, keycap};
use crate::widgets::plate::Plate;

/// Shortest time between two hints (spec: at most one per 3 minutes).
pub const SPACING: Duration = Duration::from_secs(180);
/// The file name inside the config folder.
pub const FILE_NAME: &str = "hints.ron";
/// Card width in px.
pub const WIDTH: f32 = 320.0;
/// How long "Show me" animates.
pub const SHOW_ME: Duration = Duration::from_secs(4);

/// One hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HintId {
    /// First part selected.
    FirstSelect,
    /// First body drag ends off-grid with snapping off.
    DragSnap,
    /// Third gizmo drag on the same axis within 30 s.
    TypeValue,
    /// Second Ctrl+D followed by a move of the copy.
    SmartDup,
    /// Three parts moved one by one to the same Y within 60 s.
    Align,
    /// A drag ends within 0.2 studs of another part's corner.
    VertexSnap,
    /// Colour changed on 3+ parts one at a time.
    ColourHue,
    /// First time the Look picker is used.
    Style,
}

impl HintId {
    /// Every hint, in the spec's order.
    pub const ALL: [HintId; 8] = [
        HintId::FirstSelect,
        HintId::DragSnap,
        HintId::TypeValue,
        HintId::SmartDup,
        HintId::Align,
        HintId::VertexSnap,
        HintId::ColourHue,
        HintId::Style,
    ];

    /// The id stored in `hints.ron`.
    pub fn id(self) -> &'static str {
        match self {
            HintId::FirstSelect => "first-select",
            HintId::DragSnap => "drag-snap",
            HintId::TypeValue => "type-value",
            HintId::SmartDup => "smart-dup",
            HintId::Align => "align",
            HintId::VertexSnap => "vertex-snap",
            HintId::ColourHue => "colour-hue",
            HintId::Style => "style",
        }
    }

    /// The sentence on the card.
    pub fn text(self) -> &'static str {
        match self {
            HintId::FirstSelect => "Drag to move it. W, E, R switch to move, rotate and scale.",
            HintId::DragSnap => "Hold Ctrl while dragging to snap to the 1-stud grid.",
            HintId::TypeValue => {
                "While dragging, type a number for an exact move. Try 5 then Enter."
            }
            HintId::SmartDup => "Press Ctrl+D again to repeat that offset.",
            HintId::Align => "Select them all and press A to line them up in one go.",
            HintId::VertexSnap => "Hold V to snap corner to corner exactly.",
            HintId::ColourHue => {
                "Select them together; Alt-drag the hue ring to recolour and keep each shade."
            }
            HintId::Style => "You can also give one object its own style: select it and press L.",
        }
    }

    /// The key on the card's keycap.
    pub fn key(self) -> &'static str {
        match self {
            HintId::FirstSelect => "W",
            HintId::DragSnap => "Ctrl",
            HintId::TypeValue => "5",
            HintId::SmartDup => "Ctrl+D",
            HintId::Align => "A",
            HintId::VertexSnap => "V",
            HintId::ColourHue => "Alt",
            HintId::Style => "L",
        }
    }

    fn parse(id: &str) -> Option<HintId> {
        HintId::ALL.into_iter().find(|h| h.id() == id)
    }
}

/// Something the creator did, as the hint detectors need it.
#[derive(Debug, Clone, PartialEq)]
pub enum HintEvent {
    /// The selection changed; `parts` is true when it holds a part.
    Selected {
        /// The selection has a part.
        parts: bool,
    },
    /// A gizmo or body drag ended with a step.
    DragEnded {
        /// The axis of an axis-handle drag.
        axis: Option<usize>,
        /// A body drag (not a handle).
        body: bool,
        /// Grid snapping was off for it.
        snapping_off: bool,
        /// The moved parts end off the grid.
        off_grid: bool,
        /// A moved corner ended within 0.2 studs of another part's corner (not on it).
        near_corner: bool,
    },
    /// Ctrl+D duplicated the selection.
    Duplicated,
    /// One part was moved by one step; `y` is its new height.
    MovedOne {
        /// The part.
        id: InstanceId,
        /// Its Position.Y after the move.
        y: f64,
    },
    /// The selection (the copies, after a duplicate) moved.
    MovedSelection,
    /// One part's Color changed by one step.
    RecolouredOne(InstanceId),
    /// The Look picker was used.
    LookPicker,
}

/// Largest `hints.ron` read; anything bigger is treated as corrupt.
const MAX_FILE_BYTES: u64 = 64 * 1024;

/// The hint ids in `file`: none when it is missing; when it is unreadable, too big or not valid,
/// a warning is logged and the file is moved aside so it cannot fail again.
fn read_seen(file: &std::path::Path) -> HashSet<HintId> {
    let text = match std::fs::metadata(file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return HashSet::new(),
        Ok(m) if m.len() > MAX_FILE_BYTES => Err(format!("{} bytes", m.len())),
        _ => std::fs::read_to_string(file).map_err(|e| e.to_string()),
    };
    let parsed = text.and_then(|t| ron::from_str::<HintsFile>(&t).map_err(|e| e.to_string()));
    match parsed {
        Ok(f) => f.seen.iter().filter_map(|s| HintId::parse(s)).collect(),
        Err(e) => {
            log::warn!("hints file is unreadable ({e}); starting with every tip unseen");
            let aside = file.with_extension("ron.bad");
            if let Err(e) = std::fs::rename(file, &aside) {
                log::warn!("could not move the unreadable hints file aside: {e}");
            }
            HashSet::new()
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct HintsFile {
    #[serde(default)]
    seen: Vec<String>,
}

/// The hint state of one user.
#[derive(Debug)]
pub struct Hints {
    /// Settings > "Show tips".
    pub show_tips: bool,
    seen: HashSet<HintId>,
    file: Option<PathBuf>,
    current: Option<HintId>,
    show_me: Option<Instant>,
    last_shown: Option<Instant>,
    axis_drags: VecDeque<(usize, Instant)>,
    after_dup: bool,
    dup_moves: u32,
    moves: VecDeque<(InstanceId, f64, Instant)>,
    recoloured: HashSet<InstanceId>,
    /// Last frame: the pointer was over the card or one of its buttons had keyboard focus.
    engaged: bool,
}

impl Default for Hints {
    fn default() -> Self {
        Hints {
            show_tips: true,
            seen: HashSet::new(),
            file: None,
            current: None,
            show_me: None,
            last_shown: None,
            axis_drags: VecDeque::new(),
            after_dup: false,
            dup_moves: 0,
            moves: VecDeque::new(),
            recoloured: HashSet::new(),
            engaged: false,
        }
    }
}

impl Hints {
    /// Hints that are not saved (tests, captures).
    pub fn in_memory() -> Hints {
        Hints::default()
    }

    /// Loads `<config dir>/hints.ron`.
    pub fn load_default() -> Hints {
        Hints::load(crate::recent::config_dir().map(|d| d.join(FILE_NAME)))
    }

    /// Loads `file` (a missing or broken file means no hint was shown yet).
    ///
    /// A file that cannot be read or parsed is logged and moved aside to `hints.ron.bad`
    /// (replacing an older one), and the hints start fresh; the next save writes a clean file.
    pub fn load(file: Option<PathBuf>) -> Hints {
        let seen = file.as_deref().map(read_seen).unwrap_or_default();
        Hints {
            seen,
            file,
            ..Hints::default()
        }
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        let mut seen: Vec<String> = self.seen.iter().map(|h| h.id().to_string()).collect();
        seen.sort();
        let Ok(text) = ron::to_string(&HintsFile { seen }) else {
            return;
        };
        if let Some(parent) = file.parent()
            && let Err(e) = crate::data_dirs::create_owned(parent)
        {
            log::warn!("could not save hints: {e}");
            return;
        }
        if let Err(e) = crate::colour::palette::write_atomic(file, &text) {
            log::warn!("could not save hints: {e}");
        }
    }

    /// Settings > "Reset tips": every hint can show again.
    pub fn reset(&mut self) {
        self.seen.clear();
        self.current = None;
        self.last_shown = None;
        self.save();
    }

    /// True once `hint` has been shown.
    pub fn was_shown(&self, hint: HintId) -> bool {
        self.seen.contains(&hint)
    }

    /// The card showing now.
    pub fn current(&self) -> Option<HintId> {
        self.current
    }

    /// Closes the card ("Got it" or Esc).
    pub fn dismiss(&mut self) {
        self.current = None;
        self.show_me = None;
        self.engaged = false;
    }

    /// Feeds one event. `blocked` is true during a drag, Play or camera flight: no hint fires
    /// then (the trigger can fire again later).
    pub fn note(&mut self, event: &HintEvent, now: Instant, blocked: bool) {
        let hint = self.detect(event, now);
        if let Some(h) = hint {
            self.fire(h, now, blocked);
        }
    }

    fn detect(&mut self, event: &HintEvent, now: Instant) -> Option<HintId> {
        match *event {
            HintEvent::Selected { parts } => parts.then_some(HintId::FirstSelect),
            HintEvent::DragEnded {
                axis,
                body,
                snapping_off,
                off_grid,
                near_corner,
            } => {
                if let Some(a) = axis {
                    self.axis_drags
                        .retain(|(_, t)| now.duration_since(*t) <= Duration::from_secs(30));
                    self.axis_drags.push_back((a, now));
                    if self.axis_drags.iter().filter(|(x, _)| *x == a).count() >= 3 {
                        self.axis_drags.clear();
                        return Some(HintId::TypeValue);
                    }
                }
                if near_corner {
                    return Some(HintId::VertexSnap);
                }
                (body && snapping_off && off_grid).then_some(HintId::DragSnap)
            }
            HintEvent::Duplicated => {
                self.after_dup = true;
                None
            }
            HintEvent::MovedSelection => {
                if std::mem::take(&mut self.after_dup) {
                    self.dup_moves += 1;
                    if self.dup_moves >= 2 {
                        return Some(HintId::SmartDup);
                    }
                }
                None
            }
            HintEvent::MovedOne { id, y } => {
                self.moves
                    .retain(|(_, _, t)| now.duration_since(*t) <= Duration::from_secs(60));
                self.moves.retain(|(m, _, _)| *m != id);
                self.moves.push_back((id, y, now));
                let same = self
                    .moves
                    .iter()
                    .filter(|(_, my, _)| (my - y).abs() < 1e-3)
                    .count();
                (same >= 3).then(|| {
                    self.moves.clear();
                    HintId::Align
                })
            }
            HintEvent::RecolouredOne(id) => {
                self.recoloured.insert(id);
                (self.recoloured.len() >= 3).then_some(HintId::ColourHue)
            }
            HintEvent::LookPicker => Some(HintId::Style),
        }
    }

    /// Shows `hint` when the rules allow it. True when it shows.
    pub fn fire(&mut self, hint: HintId, now: Instant, blocked: bool) -> bool {
        let spaced = self
            .last_shown
            .is_none_or(|t| now.duration_since(t) >= SPACING);
        if !self.show_tips
            || blocked
            || self.seen.contains(&hint)
            || self.current.is_some()
            || !spaced
        {
            return false;
        }
        self.seen.insert(hint);
        self.current = Some(hint);
        self.show_me = None;
        self.last_shown = Some(now);
        self.save();
        true
    }

    /// Draws the card at the bottom left of `viewport` (hidden while `blocked`).
    pub fn ui(&mut self, ctx: &egui::Context, viewport: Rect, blocked: bool) {
        let Some(hint) = self.current else { return };
        if !self.show_tips {
            self.dismiss();
            return;
        }
        if blocked {
            return;
        }
        // Esc dismisses the card when the creator is on it (pointer over it or a button focused);
        // otherwise Esc keeps its editor meaning (clear the selection, cancel a drag).
        if self.engaged && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.dismiss();
            return;
        }
        let at = pos2(viewport.left() + 12.0, viewport.bottom() - 12.0);
        let showing = self.show_me.is_some_and(|t| t.elapsed() < SHOW_ME);
        if showing {
            ctx.request_repaint();
        }
        let mut close = false;
        let mut focused = false;
        let mut card = Rect::NOTHING;
        let mut show_me = false;
        Area::new(Id::NULL.with("hint-card"))
            .order(Order::Foreground)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .fixed_pos(at)
            .show(ctx, |ui| {
                let frame = Frame::new().inner_margin(Margin::same(14));
                let inner = frame.show(ui, |ui| {
                    ui.set_width(WIDTH - 28.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        let pulse = if showing {
                            let t = self.show_me.map_or(0.0, |s| s.elapsed().as_secs_f32());
                            (t * std::f32::consts::TAU).sin() * 0.5 + 0.5
                        } else {
                            1.0
                        };
                        ui.scope(|ui| {
                            ui.set_opacity(0.4 + 0.6 * pulse);
                            keycap(ui, hint.key());
                        });
                        ui.add(
                            egui::Label::new(
                                RichText::new(hint.text())
                                    .font(fonts::body())
                                    .color(theme::text_1()),
                            )
                            .wrap(),
                        );
                    });
                    ui.add_space(8.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let got = ui.add(Button::secondary("Got it").compact());
                        let me = ui.add(Button::ghost("Show me").compact());
                        close = got.clicked();
                        show_me = me.clicked();
                        focused = got.has_focus() || me.has_focus();
                    });
                });
                // The plate goes under the content (drawn after layout, on a lower layer).
                let rect = inner.response.rect;
                card = rect;
                let painter = ui.painter().clone().with_layer_id(egui::LayerId::new(
                    Order::Middle,
                    Id::NULL.with("hint-card-plate"),
                ));
                Plate::floating().paint(&painter, rect);
            });
        let hovered = ctx.pointer_hover_pos().is_some_and(|p| card.contains(p));
        self.engaged = hovered || focused;
        if close {
            self.dismiss();
        }
        if show_me {
            self.show_me = Some(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> InstanceId {
        let mut w = metallic_core::World::new_with_services();
        let mut last = None;
        for _ in 0..n {
            last = Some(w.create("Part", None).expect("part"));
        }
        last.expect("n > 0")
    }

    #[test]
    fn each_hint_fires_once_and_never_while_blocked() {
        let mut h = Hints::in_memory();
        let t0 = Instant::now();
        let sel = HintEvent::Selected { parts: true };
        h.note(&sel, t0, true);
        assert_eq!(h.current(), None, "not during a drag or Play");
        h.note(&sel, t0, false);
        assert_eq!(h.current(), Some(HintId::FirstSelect));
        h.dismiss();
        h.note(&sel, t0 + SPACING * 2, false);
        assert_eq!(h.current(), None, "once per user");
    }

    #[test]
    fn at_most_one_hint_per_three_minutes() {
        let mut h = Hints::in_memory();
        let t0 = Instant::now();
        h.note(&HintEvent::Selected { parts: true }, t0, false);
        h.dismiss();
        h.note(&HintEvent::LookPicker, t0 + Duration::from_secs(60), false);
        assert_eq!(h.current(), None);
        h.note(&HintEvent::LookPicker, t0 + SPACING, false);
        assert_eq!(h.current(), Some(HintId::Style));
    }

    #[test]
    fn show_tips_off_silences_everything() {
        let mut h = Hints::in_memory();
        h.show_tips = false;
        h.note(&HintEvent::Selected { parts: true }, Instant::now(), false);
        assert_eq!(h.current(), None);
        assert!(!h.was_shown(HintId::FirstSelect));
    }

    #[test]
    fn the_behaviour_triggers() {
        let t0 = Instant::now();
        let s = |n: u64| t0 + Duration::from_secs(n);
        let drag = |axis: Option<usize>, body, off, near| HintEvent::DragEnded {
            axis,
            body,
            snapping_off: off,
            off_grid: off,
            near_corner: near,
        };
        let check = |events: &[(HintEvent, Instant)], want: HintId| {
            let mut h = Hints::in_memory();
            for (e, t) in events {
                h.note(e, *t, false);
            }
            assert_eq!(h.current(), Some(want), "{events:?}");
        };
        check(&[(drag(None, true, true, false), s(0))], HintId::DragSnap);
        check(
            &[
                (drag(Some(0), false, false, false), s(0)),
                (drag(Some(0), false, false, false), s(10)),
                (drag(Some(0), false, false, false), s(20)),
            ],
            HintId::TypeValue,
        );
        check(&[(drag(None, true, false, true), s(0))], HintId::VertexSnap);
        check(
            &[
                (HintEvent::Duplicated, s(0)),
                (HintEvent::MovedSelection, s(1)),
                (HintEvent::Duplicated, s(2)),
                (HintEvent::MovedSelection, s(3)),
            ],
            HintId::SmartDup,
        );
        check(
            &[
                (HintEvent::MovedOne { id: id(1), y: 4.0 }, s(0)),
                (HintEvent::MovedOne { id: id(2), y: 4.0 }, s(10)),
                (HintEvent::MovedOne { id: id(3), y: 4.0 }, s(20)),
            ],
            HintId::Align,
        );
        check(
            &[
                (HintEvent::RecolouredOne(id(1)), s(0)),
                (HintEvent::RecolouredOne(id(2)), s(1)),
                (HintEvent::RecolouredOne(id(3)), s(2)),
            ],
            HintId::ColourHue,
        );
        check(&[(HintEvent::LookPicker, s(0))], HintId::Style);
        // Three axis drags spread over more than 30 s, or on different axes, do not count.
        let mut h = Hints::in_memory();
        for (a, t) in [(0, 0), (0, 40), (1, 50), (0, 80)] {
            h.note(&drag(Some(a), false, false, false), s(t), false);
        }
        assert_eq!(h.current(), None);
    }

    #[test]
    fn a_corrupt_hints_file_is_moved_aside_and_the_hints_start_fresh() {
        let dir = tempfile::tempdir().expect("tmp");
        let file = dir.path().join(FILE_NAME);
        std::fs::write(&file, "(seen: [\"first-select\"").expect("write");
        let mut h = Hints::load(Some(file.clone()));
        assert!(!h.was_shown(HintId::FirstSelect));
        assert!(dir.path().join("hints.ron.bad").exists(), "kept for a look");
        assert!(!file.exists());
        // The next save writes a clean file that loads.
        h.note(&HintEvent::Selected { parts: true }, Instant::now(), false);
        assert!(Hints::load(Some(file)).was_shown(HintId::FirstSelect));
    }

    #[test]
    fn shown_hints_are_saved_and_reset_brings_them_back() {
        let dir = tempfile::tempdir().expect("tmp");
        let file = dir.path().join(FILE_NAME);
        let mut h = Hints::load(Some(file.clone()));
        h.note(&HintEvent::Selected { parts: true }, Instant::now(), false);
        let again = Hints::load(Some(file.clone()));
        assert!(again.was_shown(HintId::FirstSelect));
        let mut again = again;
        again.reset();
        assert!(!Hints::load(Some(file)).was_shown(HintId::FirstSelect));
    }
}
