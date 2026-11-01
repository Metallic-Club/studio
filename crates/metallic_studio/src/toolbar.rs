//! The editor's top toolbar: the Edit, Play, View and Build [`ToolbarGroup`]s of the
//! [`ActionRegistry`] left to right, with an overflow menu for narrow windows. Tools, Settings
//! and Snap live in the viewport header ([`crate::viewport_header`]), so they are not drawn
//! twice.
//!
//! Groups are drawn in [`ORDER`]. Each frame [`Toolbar::show`] measures every group it draws
//! (separator included). When the groups do not all fit, the ones that fit are kept in
//! [`PRIORITY`] order (core tools first) and the rest collapse into a "»" button at the end of
//! the toolbar ([`MORE_LABEL`]). Its menu lists every collapsed action with its label, shortcut
//! and checked state, and draws the action's accessory (the increment fields) beside it. A group
//! is either drawn whole or collapsed whole, so no button is ever cut in half. When a measured
//! width changes, the pass is discarded and redrawn, so a resize never shows a clipped frame.
//!
//! Registering an action in a group (see [`crate::actions`]) is all it takes to appear here.
use eframe::egui::{self, Rect};

use crate::actions::{ActionRegistry, ToolbarGroup};
use crate::editor::EditorState;
use crate::icons::Icon;
use crate::keymap::Keymap;
use crate::widgets::button::Button;

/// Toolbar groups, left to right.
pub const ORDER: [ToolbarGroup; 4] = [
    ToolbarGroup::Edit,
    ToolbarGroup::Play,
    ToolbarGroup::View,
    ToolbarGroup::Build,
];

/// Most important first: the groups at the end collapse into the overflow menu first.
pub const PRIORITY: [ToolbarGroup; 4] = [
    ToolbarGroup::Play,
    ToolbarGroup::Edit,
    ToolbarGroup::Build,
    ToolbarGroup::View,
];

/// Accessible name and tooltip of the overflow button.
pub const MORE_LABEL: &str = "More tools";

/// Width assumed for the overflow button before it was drawn once.
const CHEVRON_GUESS: f32 = 36.0;

/// Where last frame's toolbar put things (for tests and for tools that anchor to it).
#[derive(Debug, Clone)]
pub struct ToolbarLayout {
    /// The room the toolbar had.
    pub available: Rect,
    /// The groups drawn, left to right, with their rectangles (separator included).
    pub shown: Vec<(ToolbarGroup, Rect)>,
    /// The groups in the overflow menu, left-to-right order.
    pub overflow: Vec<ToolbarGroup>,
    /// The overflow button, when there is one.
    pub chevron: Option<Rect>,
}

impl Default for ToolbarLayout {
    fn default() -> Self {
        ToolbarLayout {
            available: Rect::NOTHING,
            shown: Vec::new(),
            overflow: Vec::new(),
            chevron: None,
        }
    }
}

/// The toolbar's remembered widths and last layout.
#[derive(Debug, Default)]
pub struct Toolbar {
    widths: [Option<f32>; ORDER.len()],
    chevron: Option<f32>,
    layout: ToolbarLayout,
}

fn slot(group: ToolbarGroup) -> usize {
    ORDER.iter().position(|g| *g == group).unwrap_or(0)
}

/// Which groups to draw, by [`ORDER`] slot, when each group is `widths[slot]` wide and the
/// toolbar has `available` points: all of them when they fit, otherwise the longest prefix of
/// [`PRIORITY`] that fits beside a `chevron`-wide overflow button.
pub fn fit(widths: &[f32; ORDER.len()], chevron: f32, available: f32) -> [bool; ORDER.len()] {
    if widths.iter().sum::<f32>() <= available {
        return [true; ORDER.len()];
    }
    let budget = available - chevron;
    let mut keep = [false; ORDER.len()];
    let mut used = 0.0;
    for group in PRIORITY {
        let w = widths[slot(group)];
        if used + w > budget {
            break;
        }
        used += w;
        keep[slot(group)] = true;
    }
    keep
}

impl Toolbar {
    /// A toolbar that has measured nothing yet.
    pub fn new() -> Toolbar {
        Toolbar::default()
    }

    /// Last frame's layout.
    pub fn layout(&self) -> &ToolbarLayout {
        &self.layout
    }

    /// Draws the toolbar into the horizontal `ui`.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        actions: &mut ActionRegistry,
        state: &mut EditorState,
        keymap: &Keymap,
    ) {
        let available = ui.available_rect_before_wrap();
        let shown = if self.widths.iter().any(Option::is_none) {
            // Nothing measured yet: draw everything once to learn the widths.
            [true; ORDER.len()]
        } else {
            let widths = self.widths.map(|w| w.unwrap_or(0.0));
            fit(
                &widths,
                self.chevron.unwrap_or(CHEVRON_GUESS),
                available.width(),
            )
        };
        // Reuse last frame's vectors: no allocation per frame.
        let mut layout = std::mem::take(&mut self.layout);
        layout.available = available;
        layout.shown.clear();
        layout.overflow.clear();
        layout.chevron = None;
        let mut changed = false;
        for (i, group) in ORDER.into_iter().enumerate() {
            if !shown[i] {
                layout.overflow.push(group);
                continue;
            }
            let first = layout.shown.is_empty();
            let rect = ui
                .scope(|ui| {
                    if !first {
                        ui.separator();
                    }
                    actions.toolbar(ui, group, state, keymap);
                })
                .response
                .rect;
            let width = rect.width() + ui.spacing().item_spacing.x;
            let old = &mut self.widths[i];
            changed |= old.is_none_or(|w| (w - width).abs() > 0.5);
            *old = Some(width);
            layout.shown.push((group, rect));
        }
        if !layout.overflow.is_empty() {
            let scope = ui.scope(|ui| {
                ui.separator();
                let button = Button::ghost("")
                    .compact()
                    .glyph(Icon::More)
                    .alt(MORE_LABEL);
                ui.add(button)
            });
            let response = scope.inner;
            let width = scope.response.rect.width() + ui.spacing().item_spacing.x;
            changed |= self.chevron.is_none_or(|w| (w - width).abs() > 0.5);
            self.chevron = Some(width);
            layout.chevron = Some(response.rect);
            let overflow = &layout.overflow;
            egui::Popup::menu(&response).show(|ui| {
                actions.overflow_menu(ui, overflow, state, keymap);
            });
        }
        if changed {
            ui.ctx().request_discard("toolbar overflow");
        }
        self.layout = layout;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_fits_or_the_lowest_priority_collapses_first() {
        // Edit, Play, View, Build.
        let w = [80.0, 250.0, 100.0, 200.0];
        let groups = |keep: [bool; 4]| -> Vec<ToolbarGroup> {
            ORDER
                .iter()
                .zip(keep)
                .filter_map(|(g, k)| k.then_some(*g))
                .collect()
        };
        assert_eq!(groups(fit(&w, 36.0, 2000.0)), ORDER.to_vec());
        let narrow = groups(fit(&w, 36.0, 600.0));
        assert_eq!(
            narrow,
            vec![ToolbarGroup::Edit, ToolbarGroup::Play, ToolbarGroup::Build]
        );
        let tiny = groups(fit(&w, 36.0, 300.0));
        assert_eq!(tiny, vec![ToolbarGroup::Play]);
        let used: f32 = tiny.iter().map(|g| w[slot(*g)]).sum();
        assert!(used + 36.0 <= 300.0);
    }
}
