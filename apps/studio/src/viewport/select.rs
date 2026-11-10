//! Selecting by dragging in the viewport (#467), after the archive's Select tool (`studio-achive-`, `tools/select.rs`):
//! the Select tool's **Box**, **Lasso** and **Paint** modes, cycled by a keybind.
//!
//! - **Box**: what has its visual centre inside the rectangle dragged out (#474): a box over a floor's corner, or
//!   round the parts standing on it, does not take the floor.
//! - **Lasso**: what has its visual centre inside the shape drawn.
//! - **Paint**: what the [`BRUSH`] circle passes over, and what lies under its path ([`Gesture::finish`]).
//!
//! What can be picked is what a click picks ([`pick::selectable`]: a part, or the model it is in), seen as its visual
//! bounding box: the screen rectangle round the part of its outline ([`pick::outline_box`]) the viewport shows, and
//! that rectangle's middle, its visual centre. When the gesture ends, Shift adds what it picked, Ctrl (Cmd)
//! removes it, and otherwise it becomes the selection ([`SelectOp`]).

use egui::{Painter, Pos2, Rect, Stroke};
use metallic_engine::metallic_core::{DataModel, Id};

use super::View;
use super::pick;
use crate::editor::Selection;

/// The Paint brush's diameter, in points.
pub const BRUSH: f32 = 24.0;
/// The most points a Lasso or Paint trail keeps: a longer one keeps every other point.
const MAX_TRAIL: usize = 512;

/// How the Select tool picks with a drag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectMode {
    /// A rectangle.
    #[default]
    Box,
    /// A freehand shape.
    Lasso,
    /// A brush.
    Paint,
}

impl SelectMode {
    /// The next mode in the cycle: Box, Lasso, Paint, Box.
    pub fn next(self) -> Self {
        match self {
            SelectMode::Box => SelectMode::Lasso,
            SelectMode::Lasso => SelectMode::Paint,
            SelectMode::Paint => SelectMode::Box,
        }
    }

    /// Its name, as the viewport shows it.
    pub fn label(self) -> &'static str {
        match self {
            SelectMode::Box => "Box",
            SelectMode::Lasso => "Lasso",
            SelectMode::Paint => "Paint",
        }
    }
}

/// How what a gesture picked joins the selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectOp {
    /// It becomes the selection.
    #[default]
    Replace,
    /// Shift: added to it.
    Add,
    /// Ctrl (Cmd): taken out of it.
    Subtract,
}

impl SelectOp {
    /// The op for the keys held as the gesture ends; Ctrl wins over Shift.
    pub fn from_modifiers(modifiers: egui::Modifiers) -> Self {
        if modifiers.command {
            SelectOp::Subtract
        } else if modifiers.shift {
            SelectOp::Add
        } else {
            SelectOp::Replace
        }
    }
}

/// Something that can be picked, as the screen shows it this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenItem {
    /// What a click on it selects.
    pub id: Id,
    /// Its visual bounding box: the rectangle round the part of its outline the viewport shows.
    pub rect: Rect,
    /// That rectangle's middle (#474).
    pub centre: Pos2,
}

impl ScreenItem {
    /// `id` with the visual bounding box `rect`.
    pub fn new(id: Id, rect: Rect) -> Self {
        Self {
            id,
            rect,
            centre: rect.center(),
        }
    }
}

/// The visual bounding box of `bounds` as `view` shows it: round its edges' parts in front of the camera, clipped to
/// the viewport. `None` when none of it shows.
fn visual_box(view: &View, bounds: &pick::Bounds) -> Option<Rect> {
    let corners = bounds.corners();
    let points: Vec<Pos2> = pick::EDGES
        .iter()
        .filter_map(|&(a, b)| view.segment(corners[a], corners[b]))
        .flatten()
        .collect();
    if points.is_empty() {
        return None;
    }
    let shown = Rect::from_points(&points).intersect(view.rect);
    shown.is_positive().then_some(shown)
}

/// Everything in Workspace that can be picked and shows in the viewport, as `view` shows it.
pub fn screen_items(dm: &DataModel, view: &View) -> Vec<ScreenItem> {
    let Some(world) = pick::workspace(dm) else {
        return Vec::new();
    };
    let mut ids: Vec<Id> = Vec::new();
    for part in dm.descendants(world).into_iter().flatten() {
        if pick::part_box(dm, part).is_none() {
            continue;
        }
        let id = pick::selectable(dm, part);
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids.into_iter()
        .filter_map(|id| Some(ScreenItem::new(id, visual_box(view, &pick::outline_box(dm, id)?)?)))
        .collect()
}

/// What has its visual centre inside `area` (#474).
pub fn box_hits(area: Rect, items: &[ScreenItem]) -> Vec<Id> {
    items
        .iter()
        .filter(|item| area.contains(item.centre))
        .map(|item| item.id)
        .collect()
}

/// What has its middle inside the closed shape `trail`.
pub fn lasso_hits(trail: &[Pos2], items: &[ScreenItem]) -> Vec<Id> {
    if trail.len() < 3 {
        return Vec::new();
    }
    items
        .iter()
        .filter(|item| inside(trail, item.centre))
        .map(|item| item.id)
        .collect()
}

/// What a brush `diameter` across, drawn along `trail`, passes over.
pub fn paint_hits(trail: &[Pos2], diameter: f32, items: &[ScreenItem]) -> Vec<Id> {
    let reach = diameter * 0.5;
    items
        .iter()
        .filter(|item| {
            trail
                .windows(2)
                .any(|pair| segment_reaches(item.rect, pair[0], pair[1], reach))
                || trail.first().is_some_and(|&p| item.rect.distance_to_pos(p) <= reach)
        })
        .map(|item| item.id)
        .collect()
}

/// Whether the segment `a`–`b` comes within `reach` of `rect`, tested in steps no longer than `reach`.
fn segment_reaches(rect: Rect, a: Pos2, b: Pos2, reach: f32) -> bool {
    let steps = ((b - a).length() / reach.max(1.0)).ceil().max(1.0) as usize;
    (0..=steps).any(|i| rect.distance_to_pos(a.lerp(b, i as f32 / steps as f32)) <= reach)
}

/// Whether `point` is inside the closed `polygon` (even-odd).
fn inside(polygon: &[Pos2], point: Pos2) -> bool {
    let mut odd = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let (a, b) = (polygon[i], polygon[j]);
        if (a.y > point.y) != (b.y > point.y) && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x {
            odd = !odd;
        }
        j = i;
    }
    odd
}

/// Puts `hits` into `selection` as `op` says.
pub fn combine(selection: &mut Selection, hits: &[Id], op: SelectOp) {
    match op {
        SelectOp::Replace => *selection = Selection::of(hits.iter().copied()),
        SelectOp::Add => hits.iter().for_each(|&id| selection.add(id)),
        SelectOp::Subtract => hits.iter().for_each(|&id| selection.remove(id)),
    }
}

/// A selecting drag under way: its mode and the points the pointer has gone through.
#[derive(Clone, Debug, PartialEq)]
pub struct Gesture {
    /// How it picks.
    pub mode: SelectMode,
    trail: Vec<Pos2>,
}

impl Gesture {
    /// A gesture in `mode` starting at `at`.
    pub fn start(mode: SelectMode, at: Pos2) -> Self {
        Self { mode, trail: vec![at] }
    }

    /// Follows the pointer to `at`.
    pub fn extend(&mut self, at: Pos2) {
        if self.trail.last().is_some_and(|last| last.distance(at) <= 2.0) {
            return;
        }
        if self.mode == SelectMode::Box {
            // A box needs only where it began and where it is now.
            self.trail.truncate(1);
        } else if self.trail.len() >= MAX_TRAIL {
            // Every other point, the first kept so the lasso still closes on it.
            let mut keep = false;
            self.trail.retain(|_| {
                keep = !keep;
                keep
            });
        }
        self.trail.push(at);
    }

    /// The box dragged out so far.
    fn area(&self) -> Option<Rect> {
        Some(Rect::from_two_pos(*self.trail.first()?, *self.trail.last()?))
    }

    /// What it picks among `items`.
    pub fn hits(&self, items: &[ScreenItem]) -> Vec<Id> {
        match self.mode {
            SelectMode::Box => self.area().map_or_else(Vec::new, |area| box_hits(area, items)),
            SelectMode::Lasso => lasso_hits(&self.trail, items),
            SelectMode::Paint => paint_hits(&self.trail, BRUSH, items),
        }
    }

    /// What it picks in `dm` as `view` shows it, now it has ended. Paint also picks whatever lies under the middle of
    /// its path, as a click would: what is under the brush, where its visual bounding box also takes in what is not.
    pub fn finish(&self, dm: &DataModel, view: &View) -> Vec<Id> {
        let mut hits = self.hits(&screen_items(dm, view));
        if self.mode == SelectMode::Paint {
            let step = BRUSH * 0.5;
            let mut under = |at: Pos2| {
                if let Some(id) = view.ray(at).and_then(|ray| pick::pick(dm, ray))
                    && !hits.contains(&id)
                {
                    hits.push(id);
                }
            };
            if let Some(&first) = self.trail.first() {
                under(first);
            }
            for pair in self.trail.windows(2) {
                let steps = ((pair[1] - pair[0]).length() / step).ceil().max(1.0) as usize;
                for i in 1..=steps {
                    under(pair[0].lerp(pair[1], i as f32 / steps as f32));
                }
            }
        }
        hits
    }

    /// Draws it in `ink`: the box with a faint fill, the lasso's shape closed back to its start, the paint's trail.
    pub fn paint(&self, painter: &Painter, ink: egui::Color32) {
        let stroke = Stroke::new(1.5, ink);
        match self.mode {
            SelectMode::Box => {
                if let Some(area) = self.area() {
                    painter.rect_filled(area, 0.0, ink.gamma_multiply(0.12));
                    painter.rect_stroke(area, 0.0, stroke, egui::StrokeKind::Inside);
                }
            }
            SelectMode::Lasso | SelectMode::Paint => {
                for pair in self.trail.windows(2) {
                    painter.line_segment([pair[0], pair[1]], stroke);
                }
                if self.mode == SelectMode::Lasso
                    && let (Some(&first), Some(&last)) = (self.trail.first(), self.trail.last())
                {
                    painter.line_segment([last, first], Stroke::new(1.0, ink.gamma_multiply(0.5)));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::{pos2, vec2};
    use metallic_engine::metallic_core::DataModel;

    use super::*;

    /// Two instances to pick: one round (100, 100), one round (300, 100), each 40 points square.
    fn items() -> (DataModel, Vec<ScreenItem>) {
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let class = dm.registry().class_id("Part").unwrap();
        let a = dm.create(class).unwrap();
        let b = dm.create(class).unwrap();
        let item = |id, x: f32| ScreenItem::new(id, Rect::from_center_size(pos2(x, 100.0), vec2(40.0, 40.0)));
        (dm, vec![item(a, 100.0), item(b, 300.0)])
    }

    #[test]
    fn a_box_picks_what_has_its_visual_centre_inside_it() {
        let (_dm, items) = items();
        let mut gesture = Gesture::start(SelectMode::Box, pos2(50.0, 50.0));
        gesture.extend(pos2(150.0, 150.0));
        assert_eq!(gesture.hits(&items), [items[0].id]);
        // Over the second's edge, short of its middle (300, 100): not picked (#474).
        gesture.extend(pos2(295.0, 150.0));
        assert_eq!(gesture.hits(&items), [items[0].id]);
        // Past its middle, though not round all of it: picked.
        gesture.extend(pos2(305.0, 105.0));
        assert_eq!(gesture.hits(&items), [items[0].id, items[1].id]);
        // A floor under both, far wider than the box: its middle is outside it, so not picked.
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let floor = dm.create(dm.registry().class_id("Part").unwrap()).unwrap();
        let mut with_floor = items.clone();
        with_floor.push(ScreenItem::new(
            floor,
            Rect::from_min_max(pos2(0.0, 60.0), pos2(1000.0, 600.0)),
        ));
        assert_eq!(gesture.hits(&with_floor), [items[0].id, items[1].id]);
    }

    #[test]
    fn a_lasso_picks_what_has_its_middle_inside_it() {
        let (_dm, items) = items();
        // A triangle round the first's middle only, though it cuts through both boxes.
        let mut gesture = Gesture::start(SelectMode::Lasso, pos2(60.0, 140.0));
        for at in [pos2(100.0, 60.0), pos2(290.0, 140.0)] {
            gesture.extend(at);
        }
        assert_eq!(gesture.hits(&items), [items[0].id]);
    }

    #[test]
    fn paint_picks_what_the_brush_passes_over() {
        let (_dm, items) = items();
        // Along y = 131: 11 points below the boxes' edge, within the brush's reach of 12.
        let mut gesture = Gesture::start(SelectMode::Paint, pos2(0.0, 131.0));
        gesture.extend(pos2(200.0, 131.0));
        assert_eq!(gesture.hits(&items), [items[0].id]);
        gesture.extend(pos2(400.0, 131.0));
        assert_eq!(gesture.hits(&items), [items[0].id, items[1].id]);
        // Further off, nothing.
        let mut far = Gesture::start(SelectMode::Paint, pos2(0.0, 140.0));
        far.extend(pos2(400.0, 140.0));
        assert!(far.hits(&items).is_empty());
    }

    #[test]
    fn shift_adds_ctrl_takes_out_and_otherwise_it_replaces() {
        let (_dm, items) = items();
        let (a, b) = (items[0].id, items[1].id);
        let mut selection = Selection::of([a]);
        combine(&mut selection, &[b], SelectOp::from_modifiers(egui::Modifiers::SHIFT));
        assert_eq!(selection.ids(), [a, b]);
        combine(&mut selection, &[a], SelectOp::from_modifiers(egui::Modifiers::COMMAND));
        assert_eq!(selection.ids(), [b]);
        combine(&mut selection, &[a], SelectOp::from_modifiers(egui::Modifiers::NONE));
        assert_eq!(selection.ids(), [a]);
    }

    #[test]
    fn the_modes_cycle() {
        assert_eq!(SelectMode::default(), SelectMode::Box);
        assert_eq!(SelectMode::Box.next(), SelectMode::Lasso);
        assert_eq!(SelectMode::Lasso.next(), SelectMode::Paint);
        assert_eq!(SelectMode::Paint.next(), SelectMode::Box);
    }
}
