//! Docking (ADR 0009, Docking): the editor's panels as tabs and splits the user drags around, on `egui_tiles`.
//!
//! - **A panel registry.** Every panel implements [`Panel`] (a stable id, title, icon, default zone, keymap context
//!   and `ui`), and [`crate::panels::PANELS`] lists them. Adding a panel is one type and one line there.
//! - **Open is in the tree.** A panel is open if and only if the dock's tree holds it: the View menu, the close
//!   buttons and the commands all ask [`Dock::is_open`] and change the tree, never a flag of their own.
//! - **State lives in the editor.** The tree holds panel ids only; what a panel shows and remembers is in
//!   [`EditorState`], so closing and reopening a panel, or resetting the layout, loses nothing.
//! - **Persistence.** The layout is `layout.json` in the config folder ([`layout`]), repaired on load, and written
//!   only when it changed and nothing is being dragged.
//! - **Dock guides** (#237): while a tab is dragged, squares in the middle of the area under the pointer say where it
//!   docks, and a drop anywhere but a square or a tab bar leaves the layout as it was ([`guides`]).
//! - **Panel borders** (#262, #273): the groups sit flush, and with Settings → Appearance → Panel borders on, a 1 px
//!   line parts each from the next; a side along the window's edge has none.

pub(crate) mod guides;
pub mod layout;

use std::path::{Path, PathBuf};

use egui::{
    Align2, Color32, CornerRadius, Id, LayerId, Order, Pos2, Rect, Response, Role, Sense, Stroke, Ui, UiBuilder,
    WidgetInfo, pos2, vec2,
};
use egui_tiles::{Container, ContainerKind, LinearDir, ResizeState, TabState, Tile, TileId, Tiles, Tree, UiResponse};
use serde::{Deserialize, Serialize};

use crate::editor::EditorState;
use crate::keymap::Context;
use crate::settings::StudioSettings;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, size, space, text};
use crate::theme::{current_colors, fonts};
pub use layout::{LAYOUT_FILE, LAYOUT_VERSION, LayoutError};

/// Where a panel goes in the default layout, and where it opens when it is not in the dock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Zone {
    /// The middle: the viewport.
    Centre,
    /// The left column.
    Left,
    /// The right column's upper part (the Explorer, #256).
    Right,
    /// The right column's lower part, under [`Zone::Right`] (Properties, #256).
    RightBelow,
    /// Along the bottom, under the centre.
    Bottom,
}

/// What a panel draws with: the open project and the panels' own state, and Studio's settings.
pub struct PanelContext<'a> {
    /// The open project, its data model, and every panel's state.
    pub editor: &'a mut EditorState,
    /// Studio's settings (the viewport reads its Rendering settings).
    pub settings: &'a mut StudioSettings,
}

/// A dockable panel. Implementations are unit types; their state lives in [`EditorState`].
pub trait Panel: Sync {
    /// Its stable id, saved in `layout.json`. Never renamed once shipped.
    fn id(&self) -> &'static str;
    /// The name on its tab and in the View menu.
    fn title(&self) -> &'static str;
    /// Its icon (the drag chip shows it; tabs show titles only, #255).
    fn icon(&self) -> Icon;
    /// Where it goes in the default layout, and where it opens.
    fn zone(&self) -> Zone;
    /// The keymap context it gives while it has focus.
    fn context(&self) -> Context;
    /// Whether the default layout has it.
    fn open_by_default(&self) -> bool {
        true
    }
    /// Whether the default layout shows it in front of the other tabs of its zone (Output, before Assets: #256).
    /// Otherwise the zone's first panel is in front.
    fn in_front_by_default(&self) -> bool {
        false
    }
    /// Whether Studio alone opens and closes it (the Game tab, while a game runs): it is not in the View menu, and
    /// the layout is saved without it.
    fn transient(&self) -> bool {
        false
    }
    /// Draws it, filling `ui`.
    fn ui(&self, ui: &mut Ui, cx: &mut PanelContext<'_>);
}

/// A panel's id in the dock's tree, as saved.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PanelId(pub String);

impl PanelId {
    /// The id of `panel`.
    pub fn of(panel: &dyn Panel) -> Self {
        Self(panel.id().to_owned())
    }
}

/// What the dock's last frame saw: the panel clicked, and the one under the pointer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DockFocus {
    /// The panel the user last pressed a mouse button in, if it is still open.
    pub focused: Option<&'static str>,
    /// The panel under the pointer.
    pub hovered: Option<&'static str>,
}

/// The dock: the panels, the layout's tree, and where it is saved.
pub struct Dock {
    panels: &'static [&'static dyn Panel],
    tree: Tree<PanelId>,
    /// The layout as last read or written; written again only when the tree differs.
    saved: Tree<PanelId>,
    path: Option<PathBuf>,
    focus: DockFocus,
    /// Where the pointer last was: a release can come with the pointer leaving, and a drop needs the place.
    pointer: Option<Pos2>,
    /// Each tab as the last frame drew it: where a dragged tab goes in the bar it is over.
    tab_rects: Vec<(TileId, Rect)>,
}

impl std::fmt::Debug for Dock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dock")
            .field("open", &self.open_panels())
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Default for Dock {
    /// Studio's panels in the default layout, kept in memory only.
    fn default() -> Self {
        Self::in_memory(crate::panels::PANELS)
    }
}

impl Dock {
    /// The default layout of `panels`, kept in memory only.
    pub fn in_memory(panels: &'static [&'static dyn Panel]) -> Self {
        let tree = layout::default_tree(panels);
        Self {
            panels,
            saved: tree.clone(),
            tree,
            path: None,
            focus: DockFocus::default(),
            pointer: None,
            tab_rects: Vec::new(),
        }
    }

    /// The layout of `panels` saved at `path`, repaired, or the default one (see [`layout`]).
    pub fn load(path: PathBuf, panels: &'static [&'static dyn Panel]) -> Self {
        let tree = layout::load(&path, panels);
        Self {
            panels,
            saved: tree.clone(),
            tree,
            path: Some(path),
            focus: DockFocus::default(),
            pointer: None,
            tab_rects: Vec::new(),
        }
    }

    /// The file the layout is saved in, unless it lives in memory.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The registered panels.
    pub fn panels(&self) -> &'static [&'static dyn Panel] {
        self.panels
    }

    /// The panel registered as `id`.
    pub fn panel(&self, id: &str) -> Option<&'static dyn Panel> {
        self.panels.iter().copied().find(|panel| panel.id() == id)
    }

    /// The layout's tree.
    pub fn tree(&self) -> &Tree<PanelId> {
        &self.tree
    }

    /// The tile showing panel `id`, if it is open.
    fn tile_of(&self, id: &str) -> Option<TileId> {
        self.tree
            .tiles
            .iter()
            .find(|(_, tile)| matches!(tile, Tile::Pane(pane) if pane.0 == id))
            .map(|(&tile, _)| tile)
    }

    /// Whether panel `id` is open: in the tree.
    pub fn is_open(&self, id: &str) -> bool {
        self.tile_of(id).is_some()
    }

    /// The open panels' ids, in registry order.
    pub fn open_panels(&self) -> Vec<&'static str> {
        self.panels
            .iter()
            .map(|panel| panel.id())
            .filter(|id| self.is_open(id))
            .collect()
    }

    /// Opens panel `id` and brings its tab to the front: as a tab beside an open panel of its zone, or else at its
    /// zone's side of the layout. Does nothing for an id that is not registered.
    pub fn open(&mut self, id: &str) {
        if let Some(tile) = self.tile_of(id) {
            self.tree.make_active(|tile_id, _| tile_id == tile);
            return;
        }
        let Some(panel) = self.panel(id) else {
            log::warn!("dock: there is no panel `{id}`");
            return;
        };
        let tiles = &mut self.tree.tiles;
        let pane = tiles.insert_pane(PanelId::of(panel));
        // Beside a panel of the same zone, in its tab bar.
        let neighbour = self
            .panels
            .iter()
            .filter(|other| other.zone() == panel.zone() && other.id() != id)
            .find_map(|other| {
                let tile = tiles
                    .iter()
                    .find(|(_, tile)| matches!(tile, Tile::Pane(pane) if pane.0 == other.id()))
                    .map(|(&tile, _)| tile)?;
                let parent = tiles.parent_of(tile)?;
                matches!(tiles.get(parent), Some(Tile::Container(Container::Tabs(_)))).then_some(parent)
            });
        // The right column's two parts stack: a panel of one part goes over or under the other's tab bar.
        let stack = match panel.zone() {
            Zone::Right => Some((Zone::RightBelow, guides::Side::Top)),
            Zone::RightBelow => Some((Zone::Right, guides::Side::Bottom)),
            _ => None,
        }
        .and_then(|(zone, side)| {
            let bar = self
                .panels
                .iter()
                .filter(|other| other.zone() == zone)
                .find_map(|other| tiles.find_pane(&PanelId::of(*other)))
                .and_then(|tile| tiles.parent_of(tile))?;
            Some((bar, side))
        });
        if let Some(Tile::Container(Container::Tabs(tabs))) = neighbour.and_then(|parent| tiles.get_mut(parent)) {
            tabs.add_child(pane);
            tabs.set_active(pane);
        } else if neighbour.is_none()
            && let Some((bar, side)) = stack
        {
            guides::dock(&mut self.tree, pane, bar, side);
            return;
        } else {
            let tabs = tiles.insert_tab_tile(vec![pane]);
            self.tree.root = Some(match self.tree.root {
                None => tabs,
                Some(root) => {
                    let tiles = &mut self.tree.tiles;
                    match panel.zone() {
                        Zone::Left => layout::split(tiles, LinearDir::Horizontal, &[(tabs, 1.0), (root, 3.0)]),
                        Zone::Right | Zone::RightBelow | Zone::Centre => {
                            layout::split(tiles, LinearDir::Horizontal, &[(root, 3.0), (tabs, 1.0)])
                        }
                        Zone::Bottom => layout::split(tiles, LinearDir::Vertical, &[(root, 3.0), (tabs, 1.0)]),
                    }
                }
            });
        }
        layout::tidy(&mut self.tree);
    }

    /// Closes panel `id`, if it is open.
    pub fn close(&mut self, id: &str) {
        if let Some(tile) = self.tile_of(id) {
            self.tree.remove_recursively(tile);
            layout::tidy(&mut self.tree);
        }
        if self.focus.focused == Some(id) {
            self.focus.focused = None;
        }
    }

    /// Opens panel `id` if it is closed, closes it if it is open.
    pub fn toggle(&mut self, id: &str) {
        if self.is_open(id) {
            self.close(id);
        } else {
            self.open(id);
        }
    }

    /// Goes back to the default layout (saved like any other change).
    pub fn reset(&mut self) {
        self.tree = layout::default_tree(self.panels);
        self.focus = DockFocus::default();
    }

    /// The panel with focus and the one under the pointer, as of the last frame.
    pub fn focus(&self) -> DockFocus {
        self.focus
    }

    /// The keymap context of the focused panel, or the Editor's.
    pub fn context(&self) -> Context {
        self.focus
            .focused
            .and_then(|id| self.panel(id))
            .map_or(Context::Editor, |panel| panel.context())
    }

    /// The keymap context of the panel under the pointer.
    pub fn hovered_context(&self) -> Option<Context> {
        self.focus
            .hovered
            .and_then(|id| self.panel(id))
            .map(|panel| panel.context())
    }

    /// Gives panel `id` the focus, as a click in it does.
    pub fn set_focused(&mut self, id: Option<&'static str>) {
        self.focus.focused = id;
    }

    /// Draws the dock filling `ui`, with the dock guides over it while a tab is dragged.
    pub fn ui(&mut self, ui: &mut Ui, cx: &mut PanelContext<'_>) {
        let ctx = ui.ctx().clone();
        if let Some(at) = ctx.input(|input| input.pointer.latest_pos()) {
            self.pointer = Some(at);
        }
        // What is dragged and where, from the last frame's layout, before the tree moves anything this frame.
        let drag = self.tree.dragged_id(&ctx).zip(self.pointer);
        let tab_drop =
            drag.and_then(|(dragged, at)| guides::tab_drop(&self.tree, dragged, at, TAB_HEIGHT, &self.tab_rects));
        let guides = drag.and_then(|(dragged, at)| guides::over(&self.tree, dragged, at));
        let released = ctx.input(|input| input.pointer.any_released());
        // egui_tiles drops a released tab at the nearest place it knows; the dock undoes that guess and docks where its
        // guides or a tab bar say (or nowhere).
        let before = (drag.is_some() && released).then(|| self.tree.clone());
        let colors = current_colors(&ctx);
        let borders = cx.settings.appearance.panel_borders;
        let groups = tab_groups(&self.tree);
        let mut behavior = DockBehavior {
            panels: self.panels,
            borders,
            groups,
            area: ui.max_rect(),
            cx,
            focus: DockFocus {
                focused: self.focus.focused,
                hovered: None,
            },
            close: Vec::new(),
            colors,
            tabs: Vec::new(),
        };
        self.tree.ui(&mut behavior, ui);
        let DockBehavior { focus, close, tabs, .. } = behavior;
        self.focus = focus;
        self.tab_rects = tabs;
        if let Some((dragged, at)) = drag
            && !released
        {
            let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("metallic.dock.guides")));
            if let Some(drop) = &tab_drop {
                guides::paint_tab_drop(&painter, &colors, drop);
            } else if let Some(guides) = &guides {
                guides.paint(&painter, &colors, Some(at));
            }
            let (title, icon) = match self.tree.tiles.get(dragged) {
                Some(Tile::Pane(pane)) => match self.panel(&pane.0) {
                    Some(panel) => (panel.title().to_owned(), Some(panel.icon())),
                    None => (pane.0.clone(), None),
                },
                _ => ("Group".to_owned(), None),
            };
            guides::paint_dragged(&painter, &colors, at, guides.as_ref(), icon, &title);
        }
        if let (Some(before), Some((dragged, at))) = (before, drag) {
            self.tree = before;
            if let Some(drop) = tab_drop {
                guides::drop_into_tabs(&mut self.tree, dragged, &drop);
            } else if let Some(guides) = guides
                && let Some(side) = guides.side_at(at)
            {
                guides::dock(&mut self.tree, dragged, guides.container, side);
            }
        }
        for tile in close {
            self.tree.remove_recursively(tile);
        }
        layout::tidy(&mut self.tree);
        if let Some(focused) = self.focus.focused
            && !self.is_open(focused)
        {
            self.focus.focused = None;
        }
    }

    /// Whether the layout differs from what was last read or written.
    pub fn is_changed(&self) -> bool {
        self.tree != self.saved
    }

    /// Writes the layout if it changed, unless `busy` (a drag is going on: it writes once the drag ends). Returns
    /// whether it wrote. A layout in memory is never written.
    pub fn save_if_changed(&mut self, busy: bool) -> std::io::Result<bool> {
        if busy || !self.is_changed() {
            return Ok(false);
        }
        self.saved = self.tree.clone();
        let Some(path) = &self.path else {
            return Ok(false);
        };
        crate::settings::store::write_atomically(path, &layout::to_json(&self.persisted()))?;
        Ok(true)
    }

    /// The layout as it is saved: without the panels Studio alone opens ([`Panel::transient`], the Game tab), so a
    /// layout saved during a game does not bring back an empty Game tab.
    pub fn persisted(&self) -> Tree<PanelId> {
        let mut tree = self.tree.clone();
        let transient: Vec<TileId> = tree
            .tiles
            .iter()
            .filter_map(|(&tile, kind)| match kind {
                Tile::Pane(pane) if self.panel(&pane.0).is_some_and(|panel| panel.transient()) => Some(tile),
                _ => None,
            })
            .collect();
        for tile in transient {
            tree.remove_recursively(tile);
        }
        layout::tidy(&mut tree);
        tree
    }

    /// Whether a tab is being dragged.
    pub fn is_dragging(&self, ctx: &egui::Context) -> bool {
        self.tree.dragged_id(ctx).is_some()
    }
}

/// The dock's [`egui_tiles::Behavior`]: draws each panel and its tab in the website's style, and notes focus and
/// closes.
struct DockBehavior<'a, 'b> {
    panels: &'static [&'static dyn Panel],
    cx: &'a mut PanelContext<'b>,
    focus: DockFocus,
    /// Tabs whose close button was clicked, removed after the frame.
    close: Vec<TileId>,
    /// The theme's colours: the behaviour's colour hooks are not given a context.
    colors: crate::theme::Colors,
    /// Whether groups of tabs are outlined (#262).
    borders: bool,
    /// The groups of tabs, which the borders outline.
    groups: Vec<TileId>,
    /// The dock's whole area: a group's side along it has no border (#273).
    area: Rect,
    /// Each tab drawn this frame and where: where a dragged tab goes in a bar it is dropped on.
    tabs: Vec<(TileId, Rect)>,
}

impl DockBehavior<'_, '_> {
    fn panel(&self, id: &PanelId) -> Option<&'static dyn Panel> {
        self.panels.iter().copied().find(|panel| panel.id() == id.0)
    }

    /// The title and icon of a tab showing `pane` (`None`: a group of panels).
    fn label(&self, pane: Option<&PanelId>) -> (String, Option<Icon>) {
        match pane {
            Some(pane) => match self.panel(pane) {
                Some(panel) => (panel.title().to_owned(), Some(panel.icon())),
                None => (pane.0.clone(), None),
            },
            None => ("Group".to_owned(), None),
        }
    }
}

/// A tab's height.
const TAB_HEIGHT: f32 = 32.0;
/// The space between a tab's title and its close button.
const TAB_GAP: f32 = 6.0;
/// The close button's size.
const CLOSE_SIZE: f32 = 18.0;
/// The active tab's accent underline (#255: 1 px, thinner than the 3 px bars that mark selected list items).
pub(crate) const TAB_LINE: f32 = 1.0;

/// Every group of tabs in `tree`: what the panel borders outline.
fn tab_groups(tree: &Tree<PanelId>) -> Vec<TileId> {
    tree.tiles
        .iter()
        .filter(|(_, tile)| matches!(tile, Tile::Container(container) if container.kind() == ContainerKind::Tabs))
        .map(|(&id, _)| id)
        .collect()
}

/// The 1 px lines a group in `rect` draws (#273): borders only between docked panels, never along the dock's edge
/// (`area`, the window's). Each group draws its right and bottom sides when another group is there, so two
/// neighbours share one line rather than two.
fn inner_edges(rect: Rect, area: Rect) -> Vec<Rect> {
    // Within half a point: layouts are fractional.
    const SAME: f32 = 0.5;
    let mut lines = Vec::new();
    if area.right() - rect.right() > SAME {
        lines.push(Rect::from_min_max(
            pos2(rect.right() - size::BORDER, rect.top()),
            rect.right_bottom(),
        ));
    }
    if area.bottom() - rect.bottom() > SAME {
        lines.push(Rect::from_min_max(
            pos2(rect.left(), rect.bottom() - size::BORDER),
            rect.right_bottom(),
        ));
    }
    lines
}

/// Draws one tab (its title and close button; no icon, #255) filling the bar's height, interacting as `id`. Returns
/// its response and whether its close button was clicked.
fn tab_button(ui: &mut Ui, id: Id, title: &str, state: &TabState) -> (Response, bool) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let title_font = fonts::named(fonts::STRONG, text::SMALL + 1.0);
    let galley = ui.painter().layout_no_wrap(title.to_owned(), title_font, p.text);
    let width = space::S5 + galley.size().x + TAB_GAP + CLOSE_SIZE + space::S3;
    let (_, rect) = ui.allocate_space(vec2(width, ui.available_height()));
    let response = ui.interact(rect, id, Sense::click_and_drag());
    response.widget_info(|| WidgetInfo::selected(Role::Tab, ui.is_enabled(), state.active, title));
    let close_rect = Rect::from_center_size(
        pos2(rect.right() - space::S3 - CLOSE_SIZE / 2.0, rect.center().y),
        vec2(CLOSE_SIZE, CLOSE_SIZE),
    );
    let close = ui.interact(close_rect, id.with("close"), Sense::click());
    close.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), format!("Close {title}")));
    if ui.is_rect_visible(rect) && !state.is_being_dragged {
        let hovered = response.hovered() || close.hovered();
        if state.active {
            ui.painter().rect_filled(rect, CornerRadius::ZERO, p.panel);
            // The active tab's accent underline, where it meets its panel.
            let line = Rect::from_min_max(pos2(rect.left(), rect.bottom() - TAB_LINE), rect.right_bottom());
            ui.painter().rect_filled(line, CornerRadius::ZERO, colors.accent);
        } else if hovered {
            ui.painter().rect_filled(rect, CornerRadius::ZERO, p.surface);
        }
        let ink = if state.active || hovered { p.text } else { p.muted };
        let at = pos2(rect.left() + space::S5, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(at, galley, ink);
        if state.active || hovered {
            if close.hovered() {
                ui.painter()
                    .rect_filled(close_rect, CornerRadius::same(radius::ITEM / 2), p.surface_hover);
            }
            icons::paint_at(
                ui.painter(),
                close_rect.center(),
                Align2::CENTER_CENTER,
                Icon::UiClose,
                text::SMALL,
                if close.hovered() { p.text } else { p.subtle },
            );
        }
    }
    (response, close.clicked())
}

impl egui_tiles::Behavior<PanelId> for DockBehavior<'_, '_> {
    fn pane_ui(&mut self, ui: &mut Ui, _tile: TileId, pane: &mut PanelId) -> UiResponse {
        let p = current_colors(ui.ctx()).palette;
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, p.panel);
        let Some(panel) = self.panel(pane) else {
            return UiResponse::None;
        };
        if ui.rect_contains_pointer(rect) {
            self.focus.hovered = Some(panel.id());
            if ui.input(|input| input.pointer.any_pressed()) {
                self.focus.focused = Some(panel.id());
            }
        }
        let inner = rect.shrink2(vec2(space::S4, space::S4));
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .id_salt(panel.id())
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.set_clip_rect(inner.intersect(ui.clip_rect()));
                panel.ui(ui, self.cx);
            },
        );
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &PanelId) -> egui::WidgetText {
        self.panel(pane)
            .map_or_else(|| pane.0.clone(), |panel| panel.title().to_owned())
            .into()
    }

    fn is_tab_closable(&self, _tiles: &Tiles<PanelId>, _tile: TileId) -> bool {
        true
    }

    fn tab_ui(&mut self, tiles: &mut Tiles<PanelId>, ui: &mut Ui, id: Id, tile: TileId, state: &TabState) -> Response {
        let pane = match tiles.get(tile) {
            Some(Tile::Pane(pane)) => Some(pane.clone()),
            _ => None,
        };
        let (title, _) = self.label(pane.as_ref());
        let (response, close) = tab_button(ui, id, &title, state);
        self.tabs.push((tile, response.rect));
        if close {
            self.close.push(tile);
        }
        response
    }

    fn paint_drag_preview(
        &self,
        _visuals: &egui::Visuals,
        _painter: &egui::Painter,
        _parent_rect: Option<Rect>,
        _preview_rect: Rect,
    ) {
        // egui_tiles' guess is never the drop: the dock's guides and tab bar marker show where a tab goes.
    }

    fn drag_ui(&mut self, _tiles: &Tiles<PanelId>, _ui: &mut Ui, _tile: TileId) {
        // The dock draws what is dragged beside the pointer (guides::paint_dragged), clear of the guides.
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        TAB_HEIGHT
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        // Flush (#273): the borders, not gaps, part the groups.
        0.0
    }

    fn resize_stroke(&self, style: &egui::Style, resize_state: ResizeState) -> Stroke {
        match resize_state {
            ResizeState::Idle => Stroke::NONE,
            ResizeState::Hovering => style.visuals.widgets.hovered.fg_stroke,
            ResizeState::Dragging => style.visuals.widgets.active.fg_stroke,
        }
    }

    fn paint_on_top_of_tile(&self, painter: &egui::Painter, _style: &egui::Style, tile_id: TileId, rect: Rect) {
        if self.borders && self.groups.contains(&tile_id) {
            for line in inner_edges(rect, self.area) {
                painter.rect_filled(line, CornerRadius::ZERO, self.colors.palette.border);
            }
        }
    }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> Color32 {
        self.colors.palette.header
    }

    fn tab_bar_hline_stroke(&self, _visuals: &egui::Visuals) -> Stroke {
        Stroke::new(size::BORDER, self.colors.palette.border)
    }

    fn drag_preview_color(&self, _visuals: &egui::Visuals) -> Color32 {
        self.colors.accent.gamma_multiply(0.25)
    }

    fn drag_preview_stroke(&self, _visuals: &egui::Visuals) -> Stroke {
        Stroke::new(size::BORDER, self.colors.accent)
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        layout::simplification()
    }

    fn cycle_tabs_with_keyboard(&self) -> bool {
        // Every shortcut goes through the keymap (ADR 0009, Keybinds).
        false
    }
}

#[cfg(test)]
mod tests {
    use egui_tiles::Tiles;

    use super::layout::{default_tree, from_json, to_json, tree_id};
    use super::*;
    use crate::panels::{PANELS, ids};

    /// The panels in the same tab bar as `id`, in order.
    fn tab_bar(dock: &Dock, id: &str) -> Vec<String> {
        let tiles = &dock.tree().tiles;
        let tile = dock.tile_of(id).unwrap();
        let parent = tiles.parent_of(tile).unwrap();
        let Some(Tile::Container(Container::Tabs(tabs))) = tiles.get(parent) else {
            panic!("{id} is not in a tab bar");
        };
        tabs.children
            .iter()
            .map(|child| match tiles.get(*child) {
                Some(Tile::Pane(pane)) => pane.0.clone(),
                _ => "?".to_owned(),
            })
            .collect()
    }

    /// Every pane's id in `tree`, sorted.
    fn panes(tree: &Tree<PanelId>) -> Vec<String> {
        let mut ids: Vec<String> = tree
            .tiles
            .tiles()
            .filter_map(|tile| match tile {
                Tile::Pane(pane) => Some(pane.0.clone()),
                Tile::Container(_) => None,
            })
            .collect();
        ids.sort();
        ids
    }

    #[test]
    fn the_default_layout_puts_each_panel_in_its_zone() {
        let dock = Dock::in_memory(PANELS);
        assert_eq!(
            dock.open_panels(),
            [ids::VIEWPORT, ids::EXPLORER, ids::ASSETS, ids::PROPERTIES, ids::OUTPUT]
        );
        // #256: (Viewport over [Assets, Output]) | (Explorer over Properties); Output in front.
        assert_eq!(tab_bar(&dock, ids::OUTPUT), [ids::ASSETS, ids::OUTPUT]);
        assert_eq!(tab_bar(&dock, ids::EXPLORER), [ids::EXPLORER]);
        assert_eq!(tab_bar(&dock, ids::PROPERTIES), [ids::PROPERTIES]);
        let tiles = &dock.tree().tiles;
        let bar = |id: &str| tiles.parent_of(dock.tile_of(id).unwrap()).unwrap();
        let Some(Tile::Container(Container::Tabs(bottom))) = tiles.get(bar(ids::OUTPUT)) else {
            panic!("the bottom is not a tab bar");
        };
        assert_eq!(bottom.active, dock.tile_of(ids::OUTPUT), "Output is open");
        let Some(Tile::Container(Container::Linear(columns))) = dock.tree().root().and_then(|root| tiles.get(root))
        else {
            panic!("the root is not a split");
        };
        assert_eq!(columns.dir, LinearDir::Horizontal);
        assert_eq!(columns.children.len(), 2, "no left column");
        let split = |tile: TileId| match tiles.get(tile) {
            Some(Tile::Container(Container::Linear(linear))) => (linear.dir, linear.children.clone()),
            _ => panic!("not a split"),
        };
        assert_eq!(
            split(columns.children[0]),
            (LinearDir::Vertical, vec![bar(ids::VIEWPORT), bar(ids::OUTPUT)])
        );
        assert_eq!(
            split(columns.children[1]),
            (LinearDir::Vertical, vec![bar(ids::EXPLORER), bar(ids::PROPERTIES)])
        );
    }

    /// The split directly holding the tab bar of panel `id`: its direction and children.
    fn split_of(dock: &Dock, id: &str) -> (LinearDir, Vec<TileId>) {
        let tiles = &dock.tree().tiles;
        let bar = tiles.parent_of(dock.tile_of(id).unwrap()).unwrap();
        match tiles.parent_of(bar).and_then(|split| tiles.get(split)) {
            Some(Tile::Container(Container::Linear(linear))) => (linear.dir, linear.children.clone()),
            _ => panic!("{id}'s bar is not in a split"),
        }
    }

    #[test]
    fn panel_borders_are_lines_between_groups_only() {
        let dock = Dock::in_memory(PANELS);
        // Viewport, Assets and Output, Explorer, Properties.
        assert_eq!(tab_groups(dock.tree()).len(), 4);

        let area = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 600.0));
        let left_top = Rect::from_min_max(pos2(0.0, 0.0), pos2(700.0, 400.0));
        let left_bottom = Rect::from_min_max(pos2(0.0, 400.0), pos2(700.0, 600.0));
        let right = Rect::from_min_max(pos2(700.0, 0.0), pos2(1000.0, 600.0));
        // Top left: a line along its right (the column beside it) and its bottom (the group under it).
        assert_eq!(
            inner_edges(left_top, area),
            [
                Rect::from_min_max(pos2(699.0, 0.0), pos2(700.0, 400.0)),
                Rect::from_min_max(pos2(0.0, 399.0), pos2(700.0, 400.0)),
            ]
        );
        // Bottom left: its bottom and left touch the window's edge, so only its right has a line.
        assert_eq!(
            inner_edges(left_bottom, area),
            [Rect::from_min_max(pos2(699.0, 400.0), pos2(700.0, 600.0))]
        );
        // The right column touches the window on three sides and its neighbour draws the line between: none.
        assert!(inner_edges(right, area).is_empty());
        // Fractional layouts still count as touching.
        assert!(inner_edges(Rect::from_min_max(pos2(700.0, 0.0), pos2(999.8, 600.2)), area).is_empty());
    }

    #[test]
    fn a_layout_round_trips_through_its_json() {
        let mut dock = Dock::in_memory(PANELS);
        dock.close(ids::OUTPUT);
        dock.close(ids::EXPLORER);
        dock.open(ids::EXPLORER);
        let json = to_json(dock.tree());
        assert!(json.contains("\"version\": 1"), "{json}");
        let back = from_json(&json, PANELS).unwrap();
        assert_eq!(&back, dock.tree());
    }

    #[test]
    fn repair_drops_unknown_and_second_panels() {
        let mut tiles = Tiles::default();
        let first = tiles.insert_pane(PanelId(ids::EXPLORER.into()));
        let again = tiles.insert_pane(PanelId(ids::EXPLORER.into()));
        let unknown = tiles.insert_pane(PanelId("teleporter".into()));
        // The Settings panel of #216, a window since #238.
        let settings = tiles.insert_pane(PanelId("settings".into()));
        let output = tiles.insert_pane(PanelId(ids::OUTPUT.into()));
        let left = tiles.insert_tab_tile(vec![first, again, unknown, settings]);
        let bottom = tiles.insert_tab_tile(vec![output]);
        let root = tiles.insert_horizontal_tile(vec![left, bottom]);
        let json = to_json(&Tree::new(tree_id(), root, tiles));

        let repaired = from_json(&json, PANELS).unwrap();
        assert_eq!(panes(&repaired), [ids::EXPLORER, ids::OUTPUT]);
        assert!(repaired.tiles.get(again).is_none() && repaired.tiles.get(unknown).is_none());
        assert!(repaired.tiles.get(settings).is_none());
        assert!(repaired.tiles.get(first).is_some(), "the first copy is kept");
    }

    #[test]
    fn a_layout_with_nothing_usable_gives_way_to_the_default() {
        let mut tiles = Tiles::default();
        let unknown = tiles.insert_pane(PanelId("teleporter".into()));
        let root = tiles.insert_tab_tile(vec![unknown]);
        let only_unknown = to_json(&Tree::new(tree_id(), root, tiles));
        assert!(matches!(from_json(&only_unknown, PANELS), Err(LayoutError::Empty)));
        assert!(matches!(
            from_json("{ not json", PANELS),
            Err(LayoutError::NotALayout(_))
        ));
        assert!(matches!(
            from_json("{\"tree\": 1}", PANELS),
            Err(LayoutError::NotALayout(_))
        ));
        let newer = to_json(&default_tree(PANELS)).replace("\"version\": 1", "\"version\": 99");
        assert!(matches!(from_json(&newer, PANELS), Err(LayoutError::Version(99))));

        let dir = std::env::temp_dir().join(format!("metallic-studio-layout-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(LAYOUT_FILE);
        std::fs::write(&path, &only_unknown).unwrap();
        let dock = Dock::load(path, PANELS);
        assert_eq!(dock.tree(), &default_tree(PANELS));
    }

    #[test]
    fn panels_open_beside_their_zone_and_close() {
        let mut dock = Dock::in_memory(PANELS);
        dock.close(ids::PROPERTIES);
        assert!(!dock.is_open(ids::PROPERTIES));
        dock.open(ids::PROPERTIES);
        assert!(dock.is_open(ids::PROPERTIES));
        assert_eq!(tab_bar(&dock, ids::PROPERTIES), [ids::PROPERTIES]);

        // The right column's parts stack again when one comes back: Explorer over Properties, Properties under it.
        let bar = |dock: &Dock, id: &str| {
            let tiles = &dock.tree().tiles;
            tiles.parent_of(dock.tile_of(id).unwrap()).unwrap()
        };
        let column = |dock: &Dock| {
            (
                LinearDir::Vertical,
                vec![bar(dock, ids::EXPLORER), bar(dock, ids::PROPERTIES)],
            )
        };
        assert_eq!(
            split_of(&dock, ids::PROPERTIES),
            column(&dock),
            "Properties under the Explorer"
        );
        dock.toggle(ids::EXPLORER);
        assert!(!dock.is_open(ids::EXPLORER));
        dock.toggle(ids::EXPLORER);
        assert_eq!(tab_bar(&dock, ids::EXPLORER), [ids::EXPLORER]);
        assert_eq!(
            split_of(&dock, ids::EXPLORER),
            column(&dock),
            "the Explorer over Properties"
        );
        // Assets beside Output.
        dock.close(ids::ASSETS);
        dock.open(ids::ASSETS);
        assert_eq!(tab_bar(&dock, ids::ASSETS), [ids::OUTPUT, ids::ASSETS]);

        dock.open("teleporter");
        assert!(!dock.is_open("teleporter"));

        // Everything closed leaves an empty dock; a panel opened then is the whole layout.
        for id in dock.open_panels() {
            dock.close(id);
        }
        assert!(dock.tree().is_empty());
        dock.open(ids::OUTPUT);
        assert_eq!(dock.open_panels(), [ids::OUTPUT]);

        dock.reset();
        assert_eq!(dock.tree(), &default_tree(PANELS));
    }

    #[test]
    fn the_layout_is_written_only_when_it_changed_and_nothing_is_dragged() {
        let dir = std::env::temp_dir().join(format!("metallic-studio-layout-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(LAYOUT_FILE);
        let mut dock = Dock::load(path.clone(), PANELS);
        assert!(!dock.save_if_changed(false).unwrap());
        assert!(!path.exists(), "the default is not written");

        dock.close(ids::OUTPUT);
        assert!(!dock.save_if_changed(true).unwrap(), "not during a drag");
        assert!(dock.save_if_changed(false).unwrap());
        assert!(!dock.save_if_changed(false).unwrap(), "written once");
        assert!(!Dock::load(path, PANELS).is_open(ids::OUTPUT));
    }
}
