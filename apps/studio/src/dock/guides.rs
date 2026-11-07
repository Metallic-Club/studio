//! Dock guides (#237): while a panel's tab is dragged, a cross of five squares in the middle of the dock area under
//! the pointer says where it can go, as Visual Studio's and Roblox Studio's docking does.
//!
//! - The **centre** square adds the panel to that area as a tab; the **left, right, top and bottom** squares split the
//!   area and put the panel on that side. The square under the pointer is lit, and the part of the area the panel
//!   would take is shaded.
//! - Dropped on a **tab bar**, egui_tiles' own drop still reorders or joins the tabs.
//! - Dropped **anywhere else**, nothing moves: egui_tiles' guess at the nearest place is undone ([`super::Dock::ui`]).
//!
//! The guides are worked out from the last frame's layout, before the tree is drawn, so the frame that lets go docks
//! where the user saw the guide.

use egui::{Align2, CornerRadius, Painter, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use egui_tiles::{Container, LinearDir, Tile, TileId, Tree};

use super::PanelId;
use super::layout;
use crate::theme::tokens::{radius, shadow, size, space, text};
use crate::theme::{Colors, fonts};

/// A guide square's side, in points.
pub(crate) const SQUARE: f32 = 34.0;
/// The space between the squares of the cross.
pub(crate) const GAP: f32 = 6.0;

/// Where a guide puts a dragged panel in the area it is drawn over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Side {
    /// As a tab in the area.
    Centre,
    /// In a split, left of the area.
    Left,
    /// In a split, right of the area.
    Right,
    /// In a split, above the area.
    Top,
    /// In a split, under the area.
    Bottom,
}

impl Side {
    /// Every guide, the centre first.
    pub(crate) const ALL: [Side; 5] = [Side::Centre, Side::Left, Side::Right, Side::Top, Side::Bottom];

    /// The square of this guide in the cross drawn over `area`.
    pub(crate) fn square(self, area: Rect) -> Rect {
        let step = SQUARE + GAP;
        let offset = match self {
            Side::Centre => vec2(0.0, 0.0),
            Side::Left => vec2(-step, 0.0),
            Side::Right => vec2(step, 0.0),
            Side::Top => vec2(0.0, -step),
            Side::Bottom => vec2(0.0, step),
        };
        Rect::from_center_size(area.center() + offset, vec2(SQUARE, SQUARE))
    }

    /// The part of `area` the panel takes: the whole of it as a tab, or the half on this side.
    pub(crate) fn preview(self, area: Rect) -> Rect {
        match self {
            Side::Centre => area,
            Side::Left => area.split_left_right_at_fraction(0.5).0,
            Side::Right => area.split_left_right_at_fraction(0.5).1,
            Side::Top => area.split_top_bottom_at_fraction(0.5).0,
            Side::Bottom => area.split_top_bottom_at_fraction(0.5).1,
        }
    }
}

/// The guides over one dock area: its tab container and where it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Guides {
    /// The tab container the guides dock into.
    pub container: TileId,
    /// The container's area, its tab bar included.
    pub area: Rect,
}

impl Guides {
    /// The guide whose square is under `at`.
    pub(crate) fn side_at(&self, at: Pos2) -> Option<Side> {
        Side::ALL.into_iter().find(|side| side.square(self.area).contains(at))
    }

    /// Draws the guides with `painter`, the one under `at` lit and the part of the area it gives shaded.
    pub(crate) fn paint(&self, painter: &Painter, colors: &Colors, at: Option<Pos2>) {
        let p = &colors.palette;
        let lit = at.and_then(|at| self.side_at(at));
        if let Some(side) = lit {
            painter.rect(
                side.preview(self.area).shrink(2.0),
                CornerRadius::same(radius::ITEM),
                colors.accent.gamma_multiply(0.18),
                Stroke::new(size::BORDER * 2.0, colors.accent),
                StrokeKind::Inside,
            );
        }
        for side in Side::ALL {
            let square = side.square(self.area);
            let on = lit == Some(side);
            painter.rect(
                square,
                CornerRadius::same(radius::ITEM),
                if on { p.surface_hover } else { p.surface },
                Stroke::new(size::BORDER, if on { colors.accent } else { p.border }),
                StrokeKind::Inside,
            );
            // A small picture of the area, with the part the panel takes filled in.
            let window = square.shrink(8.0);
            let ink = if on { colors.accent } else { p.text };
            painter.rect_stroke(
                window,
                CornerRadius::same(2),
                Stroke::new(size::BORDER, ink),
                StrokeKind::Inside,
            );
            let part = match side {
                // The tab bar along the top, as a tab joins it.
                Side::Centre => Rect::from_min_max(window.min, pos2(window.right(), window.top() + 4.0)),
                _ => side.preview(window),
            };
            painter.rect_filled(part, CornerRadius::same(2), ink);
        }
    }
}

/// Draws the name of what is dragged (`icon` and `title`) in a chip: under the cross of `guides` when they show, so it
/// never covers a square, or else beside the pointer at `at`. It stands in for egui_tiles' own label, which sits
/// right on the pointer.
pub(crate) fn paint_dragged(
    painter: &Painter,
    colors: &Colors,
    at: Pos2,
    guides: Option<&Guides>,
    icon: Option<char>,
    title: &str,
) {
    let p = &colors.palette;
    let font = fonts::named(fonts::STRONG, text::SMALL + 1.0);
    let galley = painter.layout_no_wrap(title.to_owned(), font, p.text);
    let icon_width = if icon.is_some() { text::BODY + space::S2 } else { 0.0 };
    let extent = vec2(icon_width + galley.size().x, galley.size().y) + vec2(space::S4, space::S2) * 2.0;
    let top_left = match guides {
        Some(guides) => {
            let bottom = Side::Bottom.square(guides.area);
            pos2(bottom.center().x - extent.x / 2.0, bottom.bottom() + GAP * 2.0)
        }
        None => at + vec2(14.0, 18.0),
    };
    let chip = Rect::from_min_size(top_left, extent);
    let round = CornerRadius::same(radius::ITEM);
    painter.add(shadow::MENU.as_shape(chip, round));
    painter.rect(
        chip,
        round,
        p.surface,
        Stroke::new(size::BORDER, p.border),
        StrokeKind::Inside,
    );
    let mut x = chip.left() + space::S4;
    if let Some(icon) = icon {
        painter.text(
            pos2(x, chip.center().y),
            Align2::LEFT_CENTER,
            icon,
            fonts::named(fonts::ICONS, text::BODY),
            colors.accent,
        );
        x += icon_width;
    }
    painter.galley(pos2(x, chip.center().y - galley.size().y / 2.0), galley, p.text);
}

/// Whether `tile` is `ancestor` or inside it.
fn is_within(tree: &Tree<PanelId>, tile: TileId, ancestor: TileId) -> bool {
    let mut up = Some(tile);
    while let Some(at) = up {
        if at == ancestor {
            return true;
        }
        up = tree.tiles.parent_of(at);
    }
    false
}

/// Whether docking `dragged` into tab container `container` could change anything: not into itself or a group inside
/// it, and not into the area it is already the only tab of.
fn can_dock_into(tree: &Tree<PanelId>, dragged: TileId, container: TileId) -> bool {
    if is_within(tree, container, dragged) {
        return false;
    }
    match tree.tiles.get(container) {
        Some(Tile::Container(Container::Tabs(tabs))) => tabs.children.iter().any(|&child| child != dragged),
        _ => false,
    }
}

/// The tab containers laid out last frame, with their areas, smallest first: the innermost under a point comes first.
fn areas(tree: &Tree<PanelId>) -> Vec<(TileId, Rect)> {
    let mut areas: Vec<(TileId, Rect)> = tree
        .tiles
        .iter()
        .filter(|(_, tile)| matches!(tile, Tile::Container(Container::Tabs(_))))
        .filter_map(|(&id, _)| Some((id, tree.tiles.rect(id)?)))
        .collect();
    areas.sort_by(|a, b| a.1.area().total_cmp(&b.1.area()));
    areas
}

/// Whether `at` is on a tab bar of `tree` (`tab_bar` high), where egui_tiles' own drop reorders and joins tabs.
pub(crate) fn on_tab_bar(tree: &Tree<PanelId>, at: Pos2, tab_bar: f32) -> bool {
    areas(tree)
        .into_iter()
        .any(|(_, area)| Rect::from_min_max(area.min, pos2(area.right(), area.top() + tab_bar)).contains(at))
}

/// The guides to show while `dragged` is over `at`: over the innermost dock area there, unless docking into it could
/// change nothing.
pub(crate) fn over(tree: &Tree<PanelId>, dragged: TileId, at: Pos2) -> Option<Guides> {
    let (container, area) = areas(tree).into_iter().find(|(_, area)| area.contains(at))?;
    can_dock_into(tree, dragged, container).then_some(Guides { container, area })
}

/// Docks `dragged` on `side` of tab container `container`: as its last tab (and opened) at the centre, or in a split
/// beside it, each taking half of the area. Nothing happens when that could change nothing (see [`over`]).
pub(crate) fn dock(tree: &mut Tree<PanelId>, dragged: TileId, container: TileId, side: Side) {
    if !can_dock_into(tree, dragged, container) {
        return;
    }
    let (dir, before) = match side {
        Side::Centre => {
            tree.move_tile_to_container(dragged, container, usize::MAX, false);
            if let Some(Tile::Container(Container::Tabs(tabs))) = tree.tiles.get_mut(container) {
                tabs.set_active(dragged);
            }
            layout::tidy(tree);
            return;
        }
        Side::Left => (LinearDir::Horizontal, true),
        Side::Right => (LinearDir::Horizontal, false),
        Side::Top => (LinearDir::Vertical, true),
        Side::Bottom => (LinearDir::Vertical, false),
    };
    for (_, tile) in tree.tiles.iter_mut() {
        if let Tile::Container(parent) = tile {
            parent.remove_child(dragged);
        }
    }
    // Looked up before the split exists, or it would be found as the parent.
    let parent = tree.tiles.parent_of(container);
    let children = if before {
        vec![dragged, container]
    } else {
        vec![container, dragged]
    };
    let split = tree.tiles.insert_container(Container::new_linear(dir, children));
    match parent.and_then(|parent| tree.tiles.get_mut(parent)) {
        Some(Tile::Container(parent)) => {
            // `parent_of` found it there, so it is replaced.
            let _ = parent.replace_child(container, split);
        }
        _ => tree.root = Some(split),
    }
    layout::tidy(tree);
}

#[cfg(test)]
mod tests {
    use egui_tiles::Tiles;

    use super::*;
    use crate::dock::Dock;
    use crate::panels::{PANELS, ids};

    /// The tile showing panel `id`.
    fn pane(tree: &Tree<PanelId>, id: &str) -> TileId {
        tree.tiles.find_pane(&PanelId(id.into())).unwrap()
    }

    /// The tab container holding panel `id`.
    fn bar(tree: &Tree<PanelId>, id: &str) -> TileId {
        tree.tiles.parent_of(pane(tree, id)).unwrap()
    }

    /// The panels in `container`, in order.
    fn tabs(tree: &Tree<PanelId>, container: TileId) -> Vec<String> {
        let Some(Tile::Container(container)) = tree.tiles.get(container) else {
            panic!("not a container");
        };
        container
            .children()
            .filter_map(|child| match tree.tiles.get(*child) {
                Some(Tile::Pane(pane)) => Some(pane.0.clone()),
                _ => None,
            })
            .collect()
    }

    /// The split holding `tile` directly, its direction and children.
    fn split_of(tree: &Tree<PanelId>, tile: TileId) -> (LinearDir, Vec<TileId>) {
        let parent = tree.tiles.parent_of(tile).unwrap();
        match tree.tiles.get(parent) {
            Some(Tile::Container(Container::Linear(linear))) => (linear.dir, linear.children.clone()),
            _ => panic!("not in a split"),
        }
    }

    #[test]
    fn the_cross_sits_in_the_middle_and_each_square_finds_its_side() {
        let area = Rect::from_min_size(pos2(100.0, 100.0), vec2(400.0, 300.0));
        let guides = Guides {
            container: TileId::from_u64(1),
            area,
        };
        assert_eq!(Side::Centre.square(area).center(), area.center());
        for side in Side::ALL {
            assert_eq!(guides.side_at(side.square(area).center()), Some(side), "{side:?}");
        }
        assert!(Side::Left.square(area).right() < Side::Centre.square(area).left());
        assert!(Side::Bottom.square(area).top() > Side::Centre.square(area).bottom());
        assert_eq!(
            guides.side_at(area.left_top() + vec2(4.0, 4.0)),
            None,
            "away from the cross"
        );
        assert_eq!(
            Side::Left.preview(area),
            Rect::from_min_max(area.min, pos2(300.0, 400.0))
        );
        assert_eq!(
            Side::Bottom.preview(area),
            Rect::from_min_max(pos2(100.0, 250.0), area.max)
        );
        assert_eq!(Side::Centre.preview(area), area);
    }

    #[test]
    fn the_centre_adds_the_panel_as_a_tab() {
        let mut tree = Dock::in_memory(PANELS).tree().clone();
        let output = pane(&tree, ids::OUTPUT);
        let target = bar(&tree, ids::PROPERTIES);
        dock(&mut tree, output, target, Side::Centre);
        assert_eq!(tabs(&tree, bar(&tree, ids::PROPERTIES)), [ids::PROPERTIES, ids::OUTPUT]);
    }

    #[test]
    fn each_side_splits_the_area_and_puts_the_panel_there() {
        for (side, dir, first) in [
            (Side::Left, LinearDir::Horizontal, true),
            (Side::Right, LinearDir::Horizontal, false),
            (Side::Top, LinearDir::Vertical, true),
            (Side::Bottom, LinearDir::Vertical, false),
        ] {
            let mut tree = Dock::in_memory(PANELS).tree().clone();
            let output = pane(&tree, ids::OUTPUT);
            let target = bar(&tree, ids::PROPERTIES);
            dock(&mut tree, output, target, side);
            let output_bar = bar(&tree, ids::OUTPUT);
            assert_eq!(tabs(&tree, output_bar), [ids::OUTPUT], "{side:?}: a tab bar of its own");
            let properties_bar = bar(&tree, ids::PROPERTIES);
            assert_eq!(tabs(&tree, properties_bar), [ids::PROPERTIES]);
            // The two side by side (or one over the other), in a split of the side's direction.
            let (found, children) = split_of(&tree, output_bar);
            assert_eq!(found, dir, "{side:?}");
            let at = |tile| children.iter().position(|&child| child == tile).unwrap();
            assert_eq!(at(output_bar) < at(properties_bar), first, "{side:?}");
            assert_eq!(
                at(output_bar).abs_diff(at(properties_bar)),
                1,
                "{side:?}: next to each other"
            );
        }
    }

    #[test]
    fn a_panel_splits_off_its_own_area_but_never_into_itself() {
        let mut tree = Dock::in_memory(PANELS).tree().clone();
        // Assets shares the left area with the Explorer: it can split off below it.
        let assets = pane(&tree, ids::ASSETS);
        let left = bar(&tree, ids::EXPLORER);
        dock(&mut tree, assets, left, Side::Bottom);
        assert_eq!(tabs(&tree, bar(&tree, ids::EXPLORER)), [ids::EXPLORER]);
        let (dir, _) = split_of(&tree, bar(&tree, ids::ASSETS));
        assert_eq!(dir, LinearDir::Vertical);

        // Now the only tab of its area: docking there changes nothing.
        let before = tree.clone();
        let assets = pane(&tree, ids::ASSETS);
        let own = bar(&tree, ids::ASSETS);
        dock(&mut tree, assets, own, Side::Left);
        assert_eq!(tree, before);
        // Nor does a group into an area inside it.
        let middle = tree.tiles.parent_of(bar(&tree, ids::VIEWPORT)).unwrap();
        let output = bar(&tree, ids::OUTPUT);
        dock(&mut tree, middle, output, Side::Right);
        assert_eq!(tree, before);
    }

    #[test]
    fn the_root_area_can_be_split() {
        let mut tiles = Tiles::default();
        let viewport = tiles.insert_pane(PanelId(ids::VIEWPORT.into()));
        let output = tiles.insert_pane(PanelId(ids::OUTPUT.into()));
        let root = tiles.insert_tab_tile(vec![viewport, output]);
        let mut tree = Tree::new(layout::tree_id(), root, tiles);
        dock(&mut tree, output, root, Side::Right);
        let (dir, children) = split_of(&tree, bar(&tree, ids::OUTPUT));
        assert_eq!(dir, LinearDir::Horizontal);
        assert_eq!(children, [bar(&tree, ids::VIEWPORT), bar(&tree, ids::OUTPUT)]);
        assert_eq!(tree.root(), tree.tiles.parent_of(bar(&tree, ids::OUTPUT)));
    }
}
