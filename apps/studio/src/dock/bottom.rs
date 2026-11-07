//! Tab bars under their panels (#230). egui_tiles draws a tab container's bar above its content and nowhere else, so
//! for the bottom the dock hides that bar ([`super::DockBehavior::is_tab_bar_visible`]), lets the open panel have the
//! whole area, and draws the strip itself along the panel's bottom edge:
//!
//! - the same tabs ([`super::tab_button`]), the active one with its accent bar on the top edge, facing the panel;
//! - a click opens a tab, and the close buttons close as they do above;
//! - **dragging** a tab starts egui_tiles' own drag (the tab interacts with the id egui_tiles gives it), so dropping
//!   it on a panel's sides or middle splits or joins as before. Dropped on a bottom strip, egui_tiles cannot see the
//!   strip, so the dock moves the tab into that bar at the place it was dropped ([`drop_into`]), and the preview shows
//!   that place instead of egui_tiles' guess. Dragging the strip's empty part drags the whole tab group.

use egui::{Align, Layout, Pos2, Rect, Sense, Ui, UiBuilder};
use egui_tiles::{Container, TabState, Tile, TileId, Tree};

use super::layout::tree_id;
use super::{DockBehavior, Facing, PanelId, tab_button};
use crate::theme::current_colors;

/// A tab container whose bar the dock draws under its open panel.
#[derive(Clone, Debug)]
pub(super) struct Strip {
    /// The tab container.
    pub container: TileId,
    /// Its open tab: a panel, which draws the strip.
    pub pane: TileId,
    /// Its visible tabs, in order, with the panel each shows (`None`: a group of panels).
    pub tabs: Vec<(TileId, Option<PanelId>)>,
    /// Whether the whole group can be dragged by the strip (not the layout's root).
    pub draggable: bool,
}

/// A bottom tab bar as drawn this frame: where a dragged tab may be dropped into it.
#[derive(Clone, Debug)]
pub(super) struct Drop {
    /// The tab container.
    pub container: TileId,
    /// The strip.
    pub rect: Rect,
    /// Its tabs and where each was drawn.
    pub tabs: Vec<(TileId, Rect)>,
}

impl Drop {
    /// The place among the tabs (other than `dragged`) that a tab dropped at `at` takes.
    pub(super) fn index(&self, dragged: Option<TileId>, at: Pos2) -> usize {
        self.others(dragged).filter(|(_, rect)| rect.center().x < at.x).count()
    }

    /// Where that place is along the strip: the left edge of the tab it goes before, or the end of the last tab.
    pub(super) fn insertion_x(&self, dragged: Option<TileId>, at: Pos2) -> f32 {
        let index = self.index(dragged, at);
        let mut others = self.others(dragged);
        match others.nth(index) {
            Some((_, rect)) => rect.left(),
            None => self
                .others(dragged)
                .last()
                .map_or(self.rect.left(), |(_, rect)| rect.right()),
        }
    }

    fn others(&self, dragged: Option<TileId>) -> impl Iterator<Item = &(TileId, Rect)> {
        self.tabs.iter().filter(move |(tile, _)| Some(*tile) != dragged)
    }
}

/// The strips to draw for `tree`: one for each tab container whose open tab is a panel. (A container whose open tab
/// is itself a group keeps egui_tiles' bar above it; the dock's layouts never make one, but a drag could.)
pub(super) fn strips(tree: &Tree<PanelId>) -> Vec<Strip> {
    let tiles = &tree.tiles;
    tiles
        .iter()
        .filter_map(|(&container, tile)| {
            let Tile::Container(Container::Tabs(tabs)) = tile else {
                return None;
            };
            let pane = tabs.next_active(tiles)?;
            if !matches!(tiles.get(pane), Some(Tile::Pane(_))) {
                return None;
            }
            let tabs = tabs
                .children
                .iter()
                .filter(|&&child| tiles.is_visible(child))
                .map(|&child| {
                    let panel = match tiles.get(child) {
                        Some(Tile::Pane(panel)) => Some(panel.clone()),
                        _ => None,
                    };
                    (child, panel)
                })
                .collect();
            Some(Strip {
                container,
                pane,
                tabs,
                draggable: !tree.is_root(container),
            })
        })
        .collect()
}

/// Moves `dragged` (just dropped at `at`) into `drop`'s tab container at the place it was dropped, and opens it.
/// Nothing happens when that would put a group inside itself.
pub(super) fn drop_into(tree: &mut Tree<PanelId>, dragged: TileId, drop: &Drop, at: Pos2) {
    let mut up = Some(drop.container);
    while let Some(tile) = up {
        if tile == dragged {
            return;
        }
        up = tree.tiles.parent_of(tile);
    }
    let index = drop.index(Some(dragged), at);
    tree.move_tile_to_container(dragged, drop.container, index, false);
    if let Some(Tile::Container(Container::Tabs(tabs))) = tree.tiles.get_mut(drop.container) {
        tabs.set_active(dragged);
    }
}

impl DockBehavior<'_, '_> {
    /// Draws `strip` in `rect`, under its open panel, noting clicks, closes and where its tabs are.
    pub(super) fn strip_ui(&mut self, ui: &mut Ui, strip: &Strip, rect: Rect) {
        let p = current_colors(ui.ctx()).palette;
        ui.painter().rect_filled(rect, 0.0, p.header);
        let tree = tree_id();
        let mut bar = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .id_salt(("dock.strip", strip.container))
                .layout(Layout::left_to_right(Align::Center)),
        );
        bar.set_clip_rect(rect.intersect(ui.clip_rect()));
        bar.spacing_mut().item_spacing.x = 0.0;
        if strip.draggable {
            // Added before the tabs, so the tabs are on top of it.
            let background = bar.interact(rect, bar.scope_id().with("background"), Sense::click_and_drag());
            if background.drag_started() {
                bar.ctx().set_dragged_id(strip.container.egui_id(tree));
            }
        }
        let ctx = bar.ctx().clone();
        let dragged_now = ctx.dragged_id().or_else(|| ctx.drag_stopped_id());
        let mut drawn = Vec::with_capacity(strip.tabs.len());
        for (child, pane) in &strip.tabs {
            let id = child.egui_id(tree);
            let state = TabState {
                active: *child == strip.pane,
                is_being_dragged: dragged_now == Some(id),
                closable: true,
            };
            let (title, icon) = self.label(pane.as_ref());
            let (response, close) = tab_button(&mut bar, id, &title, icon, &state, Facing::Up);
            if close {
                self.close.push(*child);
            }
            if response.clicked() {
                self.activate.push((strip.container, *child));
            }
            drawn.push((*child, response.rect));
        }
        self.drops.push(Drop {
            container: strip.container,
            rect,
            tabs: drawn,
        });
    }
}

#[cfg(test)]
mod tests {
    use egui::{pos2, vec2};

    use super::*;
    use crate::dock::Dock;
    use crate::panels::{PANELS, ids};

    #[test]
    fn every_tab_bar_of_the_default_layout_is_a_strip() {
        let dock = Dock::in_memory(PANELS);
        let strips = strips(dock.tree());
        assert_eq!(strips.len(), 4, "left, centre, right and bottom");
        let left = strips.iter().find(|strip| strip.tabs.len() == 2).unwrap();
        let names: Vec<&str> = left
            .tabs
            .iter()
            .map(|(_, pane)| pane.as_ref().unwrap().0.as_str())
            .collect();
        assert_eq!(names, [ids::EXPLORER, ids::ASSETS]);
        assert_eq!(left.pane, left.tabs[0].0, "the first tab is open");
        assert!(strips.iter().all(|strip| strip.draggable), "none is the root");
    }

    #[test]
    fn a_drop_takes_the_place_its_pointer_is_at() {
        let tab = |x: f32| Rect::from_min_size(pos2(x, 0.0), vec2(100.0, 32.0));
        let tiles: Vec<TileId> = (1..=3).map(TileId::from_u64).collect();
        let drop = Drop {
            container: TileId::from_u64(9),
            rect: Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 32.0)),
            tabs: vec![(tiles[0], tab(0.0)), (tiles[1], tab(100.0)), (tiles[2], tab(200.0))],
        };
        assert_eq!(drop.index(None, pos2(10.0, 16.0)), 0);
        assert_eq!(drop.index(None, pos2(160.0, 16.0)), 2);
        assert_eq!(drop.index(None, pos2(390.0, 16.0)), 3);
        assert_eq!(drop.insertion_x(None, pos2(160.0, 16.0)), 200.0);
        assert_eq!(drop.insertion_x(None, pos2(390.0, 16.0)), 300.0);
        // The dragged tab itself does not count.
        assert_eq!(drop.index(Some(tiles[0]), pos2(160.0, 16.0)), 1);
    }
}
