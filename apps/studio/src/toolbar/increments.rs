//! The Snap tool's element (#375): the move and rotate increments, one above the other, as Roblox Studio's Model tab
//! has them. Each row is a check box (snapping on or off), the Move or Rotate icon, the increment as a field to type
//! in or drag, and arrows stepping through common values. It edits the saved increments (`settings::Editing`) in place.

use egui::{Align, Align2, Color32, DragValue, Id, Layout, Rect, Role, Sense, Stroke, Ui, UiBuilder, WidgetInfo, vec2};

use crate::settings::{Editing, MOVE_STEP, TURN_STEP};
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::space;
use crate::theme::{self, current_colors};

/// The width of one increment's row.
pub const WIDTH: f32 = 118.0;
/// The shortest tool row the two increments stack in; in a shorter one (#414) they sit side by side.
const STACKED_FROM: f32 = 40.0;

/// The element's width in a tool row `row` points tall: one row's, or two side by side in a short row.
pub fn width(row: f32) -> f32 {
    if row >= STACKED_FROM { WIDTH } else { 2.0 * WIDTH }
}
/// The common move increments the arrows step through, in studs.
const MOVES: &[f32] = &[
    0.01, 0.05, 0.1, 0.125, 0.2, 0.25, 0.5, 1.0, 2.0, 4.0, 5.0, 10.0, 20.0, 50.0, 100.0,
];
/// The common rotate increments, in degrees.
const TURNS: &[f32] = &[1.0, 5.0, 10.0, 15.0, 22.5, 30.0, 45.0, 90.0, 180.0];
/// The side of the row's icon.
const ICON: f32 = 14.0;
/// The width of the arrows.
const ARROWS: f32 = 12.0;

/// One row: what it edits and how.
struct Row {
    /// The check box's label.
    switch: &'static str,
    /// The field's label.
    field: &'static str,
    icon: Icon,
    common: &'static [f32],
    range: (f32, f32),
    suffix: &'static str,
    /// How much a point dragged changes it.
    speed: f64,
}

const MOVE: Row = Row {
    switch: "Snap moves",
    field: "Move increment",
    icon: Icon::ToolbarMove,
    common: MOVES,
    range: MOVE_STEP,
    suffix: "",
    speed: 0.01,
};

const TURN: Row = Row {
    switch: "Snap turns",
    field: "Rotate increment",
    icon: Icon::ToolbarRotate,
    common: TURNS,
    range: TURN_STEP,
    suffix: "\u{B0}",
    speed: 0.25,
};

/// The next common value above `value` (`up`) or below it; `value` itself past the last.
pub fn step(common: &[f32], value: f32, up: bool) -> f32 {
    let found = if up {
        common.iter().copied().find(|&c| c > value * 1.0001)
    } else {
        common.iter().rev().copied().find(|&c| c < value * 0.9999)
    };
    found.unwrap_or(value)
}

/// Draws the increments in `rect`, editing `editing`. `id` is the element's own, so a toolbar showing them twice gives
/// each its own widgets (#456).
pub fn show(ui: &mut Ui, rect: Rect, editing: &mut Editing, id: Id) {
    // The element's rect is the row's less a little air above and below.
    let (first, second) = if rect.height() + 2.0 * space::S1 >= STACKED_FROM {
        let half = rect.height() / 2.0;
        let top = Rect::from_min_size(rect.min, vec2(rect.width(), half));
        (top, Rect::from_min_size(top.left_bottom(), vec2(rect.width(), half)))
    } else {
        let half = rect.width() / 2.0;
        let left = Rect::from_min_size(rect.min, vec2(half, rect.height()));
        (left, Rect::from_min_size(left.right_top(), vec2(half, rect.height())))
    };
    row(ui, id, first, &MOVE, &mut editing.snap_moves, &mut editing.move_step);
    row(ui, id, second, &TURN, &mut editing.snap_turns, &mut editing.turn_step);
}

fn row(ui: &mut Ui, id: Id, rect: Rect, row: &Row, on: &mut bool, value: &mut f32) {
    let p = current_colors(ui.ctx()).palette;
    let mut ui = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(space::S2, 1.0)))
            .id_salt((id, row.field))
            .layout(Layout::left_to_right(Align::Center)),
    );
    ui.spacing_mut().item_spacing.x = space::S1;
    theme::checkbox_only(&mut ui, on, row.switch);
    let (at, _) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::hover());
    let ink = if *on { p.text } else { p.muted };
    icons::paint(ui.painter(), row.icon, at, ink);
    let field = ui.available_width() - ARROWS - space::S1;
    let height = ui.available_height();
    let response = ui
        .scope(|ui| {
            // A plain field: no box, only a faint one under the pointer.
            let widgets = &mut ui.visuals_mut().widgets;
            for state in [&mut widgets.inactive, &mut widgets.active] {
                state.weak_bg_fill = Color32::TRANSPARENT;
                state.bg_stroke = Stroke::NONE;
            }
            widgets.hovered.bg_stroke = Stroke::NONE;
            ui.spacing_mut().button_padding = vec2(space::S1, 0.0);
            ui.add_enabled_ui(*on, |ui| {
                ui.add_sized(
                    [field, height],
                    DragValue::new(value)
                        .range(row.range.0..=row.range.1)
                        .speed(row.speed)
                        .custom_formatter(|v, _| crate::panels::properties::number(v))
                        .suffix(row.suffix),
                )
            })
            .inner
        })
        .inner;
    response.widget_info(|| WidgetInfo::labeled(Role::SpinButton, *on, row.field));
    // The arrows: the upper half steps up, the lower down.
    let (arrows, _) = ui.allocate_exact_size(vec2(ARROWS, height), Sense::hover());
    for (up, icon) in [(true, Icon::UiMoveUp), (false, Icon::UiMoveDown)] {
        let half = if up {
            Rect::from_min_max(arrows.min, arrows.right_center())
        } else {
            Rect::from_min_max(arrows.left_center(), arrows.max)
        };
        let label = format!("{} {}", row.field, if up { "up" } else { "down" });
        let response = ui.interact(half, ui.scope_id().with(&label), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(Role::Button, *on, &label));
        let ink = if !*on {
            p.subtle
        } else if response.hovered() {
            p.text
        } else {
            p.muted
        };
        let anchor = if up { Align2::CENTER_BOTTOM } else { Align2::CENTER_TOP };
        let point = if up {
            half.center_bottom() + vec2(0.0, 2.0)
        } else {
            half.center_top() - vec2(0.0, 2.0)
        };
        icons::paint_at(ui.painter(), point, anchor, icon, ARROWS, ink);
        if *on && response.clicked() {
            *value = step(row.common, *value, up);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arrows_step_through_common_values() {
        assert_eq!(step(MOVES, 1.0, true), 2.0);
        assert_eq!(step(MOVES, 1.0, false), 0.5);
        assert_eq!(step(MOVES, 0.112, true), 0.125, "from a value typed in, the next one");
        assert_eq!(step(MOVES, 0.112, false), 0.1);
        assert_eq!(step(TURNS, 180.0, true), 180.0, "the last stays");
        assert_eq!(step(TURNS, 1.0, false), 1.0);
        assert_eq!(step(TURNS, 15.0, true), 22.5);
    }
}
