//! The viewport header (building-tools spec 3.1): a 36 px strip over the viewport with the tools
//! and their keys, the increment chips, World/Local, Pivot/Centre, the snap chips and, at the
//! right, Collisions.
//!
//! It never scrolls or cuts anything off. The tools and increments always show; the other items
//! stay on the strip by priority ([`priority`]) while they fit, measured from the last frame, and
//! the rest move into a "»" menu at the right end. When even that is too wide the tools drop
//! their keycaps (the keys stay in the tooltips).

use std::collections::HashMap;

use eframe::egui::{self, Align, Layout, Rect, Sense, UiBuilder, vec2};

use crate::actions::{ActionRegistry, Look, ToolbarGroup, increment_chips};
use crate::editor::EditorState;
use crate::keymap::Keymap;
use crate::theme;
use crate::widgets::button::Button;

/// Height of the header in px.
pub const HEIGHT: f32 = 36.0;
/// Width kept for the "»" button when anything overflows.
pub const MORE_WIDTH: f32 = 36.0;
/// Width of one separator with its spacing.
const SEPARATOR: f32 = 12.0;
/// The Collisions chip, drawn at the right end.
pub const COLLISIONS: &str = "snap.collisions";

const TOOLS: &str = "@tools";
const TOOLS_COMPACT: &str = "@tools-compact";
const INCREMENTS: &str = "@increments";

/// How long an item stays on the strip when space runs out: higher stays longer.
pub fn priority(id: &str) -> u8 {
    match id {
        "snap.grid" => 90,
        "snap.surface" => 70,
        "space.toggle" => 60,
        "snap.vertex" => 50,
        "snap.edge" => 45,
        COLLISIONS => 40,
        "pivot.mode" => 20,
        "pivot.edit" => 15,
        "snap.align" => 10,
        _ => 30,
    }
}

/// Which optional items stay on a strip `budget` px wide after `fixed` px of items that always
/// show. `items` are (width, priority) in strip order; `keep` gets one flag per item. When not all
/// fit, `more` px are kept for the overflow button and items go in by priority (strip order on
/// ties) until the first one that does not fit.
pub fn fit(fixed: f32, items: &[(f32, u8)], budget: f32, more: f32, keep: &mut Vec<bool>) {
    keep.clear();
    keep.resize(items.len(), false);
    let all = fixed + items.iter().map(|i| i.0).sum::<f32>();
    if all <= budget {
        keep.fill(true);
        return;
    }
    let mut used = fixed + more;
    for _ in 0..items.len() {
        // The highest-priority item not placed yet (no allocation: the lists are short).
        let mut best: Option<usize> = None;
        for (i, item) in items.iter().enumerate() {
            if !keep[i] && best.is_none_or(|b| item.1 > items[b].1) {
                best = Some(i);
            }
        }
        let Some(b) = best else { break };
        if used + items[b].0 > budget {
            break;
        }
        used += items[b].0;
        keep[b] = true;
    }
}

/// The header's layout state: measured widths and what overflowed this frame.
#[derive(Debug)]
pub struct ViewportHeader {
    widths: HashMap<&'static str, f32>,
    ids: Vec<&'static str>,
    items: Vec<(f32, u8)>,
    keep: Vec<bool>,
    overflow: Vec<&'static str>,
    compact_tools: bool,
    rect: Rect,
}

impl Default for ViewportHeader {
    fn default() -> Self {
        ViewportHeader {
            widths: HashMap::new(),
            ids: Vec::new(),
            items: Vec::new(),
            keep: Vec::new(),
            overflow: Vec::new(),
            compact_tools: false,
            rect: Rect::NOTHING,
        }
    }
}

impl ViewportHeader {
    /// An empty header (widths are measured on the first frames).
    pub fn new() -> ViewportHeader {
        ViewportHeader::default()
    }

    /// The actions in the "»" menu this frame.
    pub fn overflow(&self) -> &[&'static str] {
        &self.overflow
    }

    /// True when the tools show without their keycaps this frame.
    pub fn compact_tools(&self) -> bool {
        self.compact_tools
    }

    /// Where the header was drawn last.
    pub fn rect(&self) -> Rect {
        self.rect
    }

    fn width(&self, id: &str) -> f32 {
        if let Some(&w) = self.widths.get(id) {
            return w;
        }
        match id {
            TOOLS => 300.0,
            TOOLS_COMPACT => 170.0,
            INCREMENTS => 96.0,
            COLLISIONS => 88.0,
            "space.toggle" | "pivot.mode" => 90.0,
            _ => 64.0,
        }
    }

    fn measure(&mut self, id: &'static str, width: f32) {
        self.widths.insert(id, width);
    }

    /// Draws the header across the top of `ui`.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        actions: &mut ActionRegistry,
        state: &mut EditorState,
        keymap: &Keymap,
    ) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::hover());
        self.rect = rect;
        ui.painter().rect_filled(rect, 0.0, theme::bg_panel());
        let inner = rect.shrink2(vec2(8.0, 4.0));
        let gap = ui.spacing().item_spacing.x.max(4.0);

        // The optional items in strip order: Settings, Snap, then Collisions.
        self.ids.clear();
        actions.ids_in(ToolbarGroup::Settings, &mut self.ids);
        actions.ids_in(ToolbarGroup::Snap, &mut self.ids);
        if actions.get(COLLISIONS).is_some() {
            self.ids.push(COLLISIONS);
        }
        self.items.clear();
        for i in 0..self.ids.len() {
            let id = self.ids[i];
            self.items.push((self.width(id), priority(id)));
        }
        let budget = inner.width();
        let fixed = self.width(TOOLS) + self.width(INCREMENTS) + 3.0 * SEPARATOR;
        let compact = self.width(TOOLS_COMPACT) + self.width(INCREMENTS) + 3.0 * SEPARATOR;
        self.compact_tools = fixed + MORE_WIDTH > budget
            && fixed + self.items.iter().map(|i| i.0).sum::<f32>() > budget;
        let fixed = if self.compact_tools { compact } else { fixed };
        fit(fixed, &self.items, budget, MORE_WIDTH, &mut self.keep);
        self.overflow.clear();
        for (i, &id) in self.ids.iter().enumerate() {
            if !self.keep[i] {
                self.overflow.push(id);
            }
        }

        // Right end first: "»" and Collisions.
        let mut right = ui.new_child(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::right_to_left(Align::Center)),
        );
        right.spacing_mut().item_spacing.x = 4.0;
        if !self.overflow.is_empty() {
            let more = right.add(Button::ghost("»").compact().alt("More header options"));
            egui::Popup::menu(&more).show(|ui| {
                actions.check_items(ui, &self.overflow, state, keymap);
            });
        }
        let collisions_kept = self
            .ids
            .iter()
            .position(|&id| id == COLLISIONS)
            .is_some_and(|i| self.keep[i]);
        if collisions_kept {
            let w = right
                .scope(|ui| actions.draw_one(ui, COLLISIONS, Look::ChipFull, state, keymap))
                .response
                .rect
                .width();
            self.measure(COLLISIONS, w + gap);
        }
        let used = right.min_rect().width();

        // Left: tools, increments, settings, snap chips; clipped so nothing can spill over.
        let mut left_rect = inner;
        left_rect.set_right(inner.right() - used - 4.0);
        let mut left = ui.new_child(
            UiBuilder::new()
                .max_rect(left_rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        left.set_clip_rect(left_rect.intersect(ui.clip_rect()));
        let ui = &mut left;
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.spacing_mut().interact_size.y = 28.0;
        let keys = !self.compact_tools;
        let w = ui
            .scope(|ui| actions.icon_keys(ui, ToolbarGroup::Tools, keys, state, keymap))
            .response
            .rect
            .width();
        self.measure(if keys { TOOLS } else { TOOLS_COMPACT }, w + gap);
        ui.separator();
        let w = ui
            .scope(|ui| increment_chips(ui, state))
            .response
            .rect
            .width();
        self.measure(INCREMENTS, w + gap);
        let mut last_group = None;
        for i in 0..self.ids.len() {
            let id = self.ids[i];
            if id == COLLISIONS || !self.keep[i] {
                continue;
            }
            let Some(group) = actions.get(id).map(|a| a.group) else {
                continue;
            };
            if last_group != Some(group) {
                ui.separator();
                last_group = Some(group);
            }
            let look = if group == ToolbarGroup::Snap {
                Look::Chip
            } else {
                Look::Toolbar
            };
            let w = ui
                .scope(|ui| actions.draw_one(ui, id, look, state, keymap))
                .response
                .rect
                .width();
            self.measure(id, w + gap);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_fits_on_a_wide_strip() {
        let mut keep = Vec::new();
        fit(
            400.0,
            &[(60.0, 90), (60.0, 10)],
            1000.0,
            MORE_WIDTH,
            &mut keep,
        );
        assert_eq!(keep, [true, true]);
    }

    #[test]
    fn low_priority_items_overflow_first() {
        let mut keep = Vec::new();
        // 400 fixed + 36 for "»": 124 left at 560 (one item), 164 at 600 (two).
        let items = [(80.0, 10), (80.0, 90), (80.0, 70), (80.0, 40)];
        fit(400.0, &items, 560.0, MORE_WIDTH, &mut keep);
        assert_eq!(keep, [false, true, false, false]);
        fit(400.0, &items, 600.0, MORE_WIDTH, &mut keep);
        assert_eq!(keep, [false, true, true, false]);
    }

    #[test]
    fn the_overflow_button_is_paid_for_only_when_needed() {
        let mut keep = Vec::new();
        fit(400.0, &[(100.0, 10)], 500.0, MORE_WIDTH, &mut keep);
        assert_eq!(keep, [true]);
        fit(400.0, &[(100.0, 10)], 499.0, MORE_WIDTH, &mut keep);
        assert_eq!(keep, [false]);
    }
}
