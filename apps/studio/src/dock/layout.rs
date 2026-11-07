//! The dock layout on disk: `layout.json` in Studio's config folder, versioned, and repaired when it is read.
//!
//! ```json
//! { "version": 1, "tree": { ...egui_tiles' tree of panel ids... } }
//! ```
//!
//! - **Repair on load:** a panel Studio does not know (from a newer or older Studio) and a second copy of a panel are
//!   dropped, then the tree is tidied (empty tabs and splits go). If no panel is left, or the file is not a layout
//!   this Studio reads, the default layout is used. A layout is the user's convenience, not their work, so a damaged
//!   file is only logged and replaced at the next change.
//! - **Written** atomically (temp file and rename), and only when the layout changed (see [`super::Dock`]).

use std::collections::HashSet;
use std::path::Path;

use egui_tiles::{Container, LinearDir, SimplificationOptions, Tile, TileId, Tiles, Tree, UiResponse};
use serde::{Deserialize, Serialize};

use super::{Panel, PanelId, Zone};

/// The layout file's name in Studio's config folder.
pub const LAYOUT_FILE: &str = "layout.json";

/// The layout file's format version.
pub const LAYOUT_VERSION: u32 = 1;

/// The tree's id: one dock per window, so a constant.
pub(crate) fn tree_id() -> egui::Id {
    egui::Id::unique("metallic.dock")
}

/// The simplifications the dock keeps its tree in: every panel in a tab bar (so it has a title and a close button),
/// no empty containers, no splits in splits of the same direction.
pub(crate) fn simplification() -> SimplificationOptions {
    SimplificationOptions {
        all_panes_must_have_tabs: true,
        ..SimplificationOptions::default()
    }
}

/// The file as written.
#[derive(Serialize)]
struct LayoutRef<'a> {
    version: u32,
    tree: &'a Tree<PanelId>,
}

/// The file as read.
#[derive(Deserialize)]
struct LayoutFile {
    version: u32,
    tree: Tree<PanelId>,
}

/// The layout as the file's JSON.
pub fn to_json(tree: &Tree<PanelId>) -> String {
    let file = LayoutRef {
        version: LAYOUT_VERSION,
        tree,
    };
    // A tree of strings and numbers always serialises; an error here would be a bug in egui_tiles.
    serde_json::to_string_pretty(&file).unwrap_or_else(|error| {
        log::error!("layout: could not write the layout as JSON: {error}");
        String::new()
    })
}

/// Why a layout file was not used.
#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    /// Not JSON, or not a layout.
    #[error("it is not a layout: {0}")]
    NotALayout(String),
    /// A version this Studio does not read.
    #[error("it is version {0}, and this Studio reads version {LAYOUT_VERSION}")]
    Version(u64),
    /// Nothing usable was left after the repair.
    #[error("no panel this Studio knows is in it")]
    Empty,
}

/// Reads a layout file's text, repaired for `panels`.
pub fn from_json(text: &str, panels: &[&dyn Panel]) -> Result<Tree<PanelId>, LayoutError> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| LayoutError::NotALayout(error.to_string()))?;
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| LayoutError::NotALayout("it has no version".to_owned()))?;
    if version != u64::from(LAYOUT_VERSION) {
        return Err(LayoutError::Version(version));
    }
    let file: LayoutFile = serde_json::from_value(value).map_err(|error| LayoutError::NotALayout(error.to_string()))?;
    debug_assert_eq!(file.version, LAYOUT_VERSION);
    repair(file.tree, panels).ok_or(LayoutError::Empty)
}

/// Reads the layout at `path` for `panels`: the default layout when the file is missing, or is not one this Studio
/// can use (which is logged).
pub fn load(path: &Path, panels: &[&dyn Panel]) -> Tree<PanelId> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return default_tree(panels),
        Err(error) => {
            log::warn!(
                "layout: could not read {}: {error}; using the default layout",
                path.display()
            );
            return default_tree(panels);
        }
    };
    match from_json(&text, panels) {
        Ok(tree) => tree,
        Err(error) => {
            log::warn!(
                "layout: {} was not used ({error}); using the default layout",
                path.display()
            );
            default_tree(panels)
        }
    }
}

/// The tree with every panel Studio does not know, and every second copy of a panel, dropped and the rest tidied.
/// `None` when no panel is left.
pub fn repair(mut tree: Tree<PanelId>, panels: &[&dyn Panel]) -> Option<Tree<PanelId>> {
    let mut seen = HashSet::new();
    let mut drop = Vec::new();
    if let Some(root) = tree.root() {
        let mut order = Vec::new();
        panes_in_order(&tree.tiles, root, &mut order, &mut HashSet::new());
        for (tile, id) in order {
            let known = panels.iter().any(|panel| panel.id() == id.0);
            if !known || !seen.insert(id.0.clone()) {
                log::info!(
                    "layout: dropped the panel `{}` ({})",
                    id.0,
                    if known { "twice" } else { "unknown" }
                );
                drop.push(tile);
            }
        }
    }
    for tile in drop {
        tree.remove_recursively(tile);
    }
    tidy(&mut tree);
    tree.tiles.recompute_next_tile_id();
    let any = tree.tiles.tiles().any(|tile| matches!(tile, Tile::Pane(_)));
    (any && !tree.is_empty()).then_some(tree)
}

/// Collects the panes under `tile`, in the order they are shown, each tile once.
fn panes_in_order(tiles: &Tiles<PanelId>, tile: TileId, out: &mut Vec<(TileId, PanelId)>, seen: &mut HashSet<TileId>) {
    if !seen.insert(tile) {
        return;
    }
    match tiles.get(tile) {
        Some(Tile::Pane(id)) => out.push((tile, id.clone())),
        Some(Tile::Container(container)) => {
            for &child in container.children() {
                panes_in_order(tiles, child, out, seen);
            }
        }
        None => {}
    }
}

/// Drops what nothing reaches and what is empty, as the dock's own pass does each frame.
pub(crate) fn tidy(tree: &mut Tree<PanelId>) {
    tree.simplify(&simplification());
    tree.gc(&mut Tidy);
}

/// The behaviour [`Tree::gc`] needs: it keeps every pane, and draws nothing.
struct Tidy;

impl egui_tiles::Behavior<PanelId> for Tidy {
    fn pane_ui(&mut self, _ui: &mut egui::Ui, _tile: TileId, _pane: &mut PanelId) -> UiResponse {
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &PanelId) -> egui::WidgetText {
        pane.0.clone().into()
    }
}

/// The default layout (ADR 0009, Docking): the centre panels in the middle, the left panels as tabs on the left, the
/// right ones on the right, and the bottom ones under the centre. Panels not open by default are left out.
pub fn default_tree(panels: &[&dyn Panel]) -> Tree<PanelId> {
    let mut tiles = Tiles::default();
    let zone = |zone: Zone, tiles: &mut Tiles<PanelId>| -> Option<TileId> {
        let panes: Vec<TileId> = panels
            .iter()
            .filter(|panel| panel.zone() == zone && panel.open_by_default())
            .map(|panel| tiles.insert_pane(PanelId::of(*panel)))
            .collect();
        (!panes.is_empty()).then(|| tiles.insert_tab_tile(panes))
    };
    let left = zone(Zone::Left, &mut tiles);
    let centre = zone(Zone::Centre, &mut tiles);
    let right = zone(Zone::Right, &mut tiles);
    let bottom = zone(Zone::Bottom, &mut tiles);

    let middle = match (centre, bottom) {
        (Some(centre), Some(bottom)) => Some(split(&mut tiles, LinearDir::Vertical, &[(centre, 3.0), (bottom, 1.0)])),
        (one, other) => one.or(other),
    };
    let columns: Vec<(TileId, f32)> = [(left, 1.0), (middle, 3.4), (right, 1.2)]
        .into_iter()
        .filter_map(|(tile, share)| tile.map(|tile| (tile, share)))
        .collect();
    let root = match columns.as_slice() {
        [] => None,
        [(only, _)] => Some(*only),
        _ => Some(split(&mut tiles, LinearDir::Horizontal, &columns)),
    };
    let mut tree = match root {
        Some(root) => Tree::new(tree_id(), root, tiles),
        None => Tree::empty(tree_id()),
    };
    tidy(&mut tree);
    tree
}

/// A split of `children` in `dir`, each with its share of the space.
pub(crate) fn split(tiles: &mut Tiles<PanelId>, dir: LinearDir, children: &[(TileId, f32)]) -> TileId {
    let mut linear = egui_tiles::Linear::new(dir, children.iter().map(|(tile, _)| *tile).collect());
    for &(tile, share) in children {
        linear.shares.set_share(tile, share);
    }
    tiles.insert_container(Container::Linear(linear))
}
