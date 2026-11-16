//! The Assets panel (#676, ADR 0012): the project's files as a browser. On the left, the project's folders and files
//! as a tree; on the right, a back button, the open folder's path as breadcrumbs and a search, and the open folder's
//! contents as tiles that wrap with the panel's width.
//!
//! - **Navigating:** click a folder in the tree or a breadcrumb, double-click a folder's tile, or go Back. The open
//!   folder is highlighted in the tree.
//! - **Selecting:** click a tile; Ctrl adds or removes one, Shift takes the run from the last click.
//! - **Operations** ([`crate::editor::assets`]): New Folder, Rename (F2), Duplicate (Ctrl+D), Delete (Del, after a
//!   confirmation that says what refers to what goes), and moving by dragging tiles onto a folder: a tile, a tree
//!   row or a breadcrumb.
//! - A file the asset index names but the project lost is still shown where it was, marked missing.

mod grid;
mod tree;

use egui::{
    Align2, CornerRadius, Frame, Id, Label, Layout, Modal, Rect, RichText, Role, Sense, Stroke, Ui, WidgetInfo,
};
use egui::{pos2, vec2};

use super::ids;
use crate::dock::{Panel, PanelContext, Zone};
use crate::editor::assets::{self as browser, Deleting, Entry};
use crate::editor::{EditorState, TreeEdit};
use crate::keymap::Context;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, shadow, size, space, text};
use crate::theme::{ButtonKind, button, current_colors, fonts, menu};

/// The Assets panel.
#[derive(Clone, Copy, Debug)]
pub struct Assets;

/// What an asset is, from its file name's extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    /// A glTF or GLB model.
    Mesh,
    /// A PNG or JPEG picture.
    Image,
    /// A sound.
    Audio,
    /// A scene file.
    Scene,
    /// A script file (#677), of the type its name gives.
    Script(ScriptType),
    /// A material (#678).
    Material,
    /// Anything else.
    Other,
}

/// A script file's type, from its name (#677, ADR 0012): `Door.server.luau` is a Script, `Door.client.luau` a
/// LocalScript and `Door.luau` a ModuleScript, as Rojo names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptType {
    /// A Script, which runs on the server.
    Server,
    /// A LocalScript, which runs on a player's machine.
    Local,
    /// A ModuleScript, which other scripts `require`.
    Module,
}

impl ScriptType {
    /// Every type, in the order the menus offer them.
    pub const ALL: [ScriptType; 3] = [ScriptType::Server, ScriptType::Local, ScriptType::Module];

    /// The type of the script file `path`, or `None` for a file that is not Luau.
    pub fn of(path: &str) -> Option<Self> {
        let lower = path.to_ascii_lowercase();
        let stem = lower.strip_suffix(".luau").or_else(|| lower.strip_suffix(".lua"))?;
        Some(if stem.ends_with(".server") {
            ScriptType::Server
        } else if stem.ends_with(".client") {
            ScriptType::Local
        } else {
            ScriptType::Module
        })
    }

    /// The type of a script instance's class: `Script`, `LocalScript` or `ModuleScript` (or a class of theirs).
    pub fn of_class(dm: &metallic_engine::metallic_core::DataModel, id: metallic_engine::metallic_core::Id) -> Self {
        if crate::viewport::pick::is_a(dm, id, "LocalScript") {
            ScriptType::Local
        } else if crate::viewport::pick::is_a(dm, id, "ModuleScript") {
            ScriptType::Module
        } else {
            ScriptType::Server
        }
    }

    /// The class of the instances that run a file of this type.
    pub fn class(self) -> &'static str {
        match self {
            ScriptType::Server => "Script",
            ScriptType::Local => "LocalScript",
            ScriptType::Module => "ModuleScript",
        }
    }

    /// The end of a file name of this type.
    pub fn extension(self) -> &'static str {
        match self {
            ScriptType::Server => ".server.luau",
            ScriptType::Local => ".client.luau",
            ScriptType::Module => ".luau",
        }
    }

    /// Where a script of this type is put when a file of it is opened with none in the scene: where it runs (or, for
    /// a module, where every side can `require` it).
    pub fn home(self) -> &'static str {
        match self {
            ScriptType::Server => "ServerScriptService",
            ScriptType::Local => "Workspace",
            ScriptType::Module => "ReplicatedStorage",
        }
    }

    /// What a new file of this type holds, as Roblox Studio starts them.
    pub fn starter(self) -> &'static str {
        match self {
            ScriptType::Module => "local module = {}\n\nreturn module\n",
            _ => "print(\"Hello world!\")\n",
        }
    }
}

impl AssetKind {
    /// The kind of the asset at `path`.
    pub fn of(path: &str) -> Self {
        if let Some(script) = ScriptType::of(path) {
            return AssetKind::Script(script);
        }
        let extension = path.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase());
        match extension.as_deref() {
            Some("glb" | "gltf") => AssetKind::Mesh,
            Some("png" | "jpg" | "jpeg" | "webp" | "ktx2") => AssetKind::Image,
            Some("ogg" | "mp3" | "wav" | "opus" | "flac" | "m4a") => AssetKind::Audio,
            Some("scene") => AssetKind::Scene,
            Some("material") => AssetKind::Material,
            _ => AssetKind::Other,
        }
    }

    /// The name people see.
    pub fn label(self) -> &'static str {
        match self {
            AssetKind::Mesh => "Mesh",
            AssetKind::Image => "Image",
            AssetKind::Audio => "Audio",
            AssetKind::Scene => "Scene",
            AssetKind::Material => "Material",
            AssetKind::Script(script) => script.class(),
            AssetKind::Other => "File",
        }
    }

    /// Its icon.
    pub fn icon(self) -> Icon {
        match self {
            AssetKind::Mesh => Icon::AssetMesh,
            AssetKind::Image => Icon::AssetImage,
            AssetKind::Audio => Icon::AssetAudio,
            AssetKind::Scene => Icon::AssetScene,
            AssetKind::Material => Icon::AssetMaterial,
            AssetKind::Script(ScriptType::Server) => Icon::AssetScript,
            AssetKind::Script(ScriptType::Local) => Icon::AssetLocalScript,
            AssetKind::Script(ScriptType::Module) => Icon::AssetModuleScript,
            AssetKind::Other => Icon::AssetFile,
        }
    }
}

/// A size in bytes as people read it.
pub fn file_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64 / 1024.0;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    if size < 10.0 {
        format!("{size:.1} {}", UNITS[unit])
    } else {
        format!("{size:.0} {}", UNITS[unit])
    }
}

/// The badge on a missing asset.
pub const MISSING: &str = "Missing";

/// What a drag from the browser carries: the paths of the files and folders dragged. Anything that takes assets
/// (a folder here; the viewport and the Explorer later) reads it with egui's drag and drop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetDrag {
    /// The files and folders, by path.
    pub paths: Vec<String>,
}

/// The tree's width, before it is dragged.
const TREE_WIDTH: f32 = 190.0;
/// The narrowest the tree or the tiles get.
const MIN_SIDE: f32 = 110.0;
/// The search field's width.
const SEARCH_WIDTH: f32 = 180.0;
/// The label of the root, in the tree and the breadcrumbs.
pub const ROOT: &str = "Project";

/// What a New menu item makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum New {
    /// A folder.
    Folder,
    /// A script file of this type.
    Script(ScriptType),
    /// A material (#678).
    Material,
}

/// The New items of a folder's menus, in order.
pub(crate) const NEW_ITEMS: [(&str, New); 5] = [
    ("New Folder", New::Folder),
    ("New Script", New::Script(ScriptType::Server)),
    ("New LocalScript", New::Script(ScriptType::Local)),
    ("New ModuleScript", New::Script(ScriptType::Module)),
    ("New Material", New::Material),
];

/// The New item called `label`, if it is one.
pub(crate) fn new_item(label: &str) -> Option<New> {
    NEW_ITEMS.iter().find(|(item, _)| *item == label).map(|(_, new)| *new)
}

/// What the tree and the tiles asked for this frame, done after they are drawn.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Act {
    /// Open this folder on the right.
    Open(String),
    /// Show this file: open its folder and select it.
    Show(String),
    /// A double-click on a file or folder: open it.
    Activate(Entry),
    /// A click on a tile.
    Click { path: String, add: bool, range: bool },
    /// Expand or collapse this folder in the tree.
    Toggle(String),
    /// Edit this one's name in place.
    StartRename(String),
    /// This one's name, as typed.
    Rename(String, String),
    /// Renaming ended without a new name.
    KeepName,
    /// A new folder or file in this folder.
    New(String, New),
    /// Copies of these beside them.
    Duplicate(Vec<String>),
    /// Ask to delete these.
    Delete(Vec<String>),
    /// Move these into this folder.
    Move(Vec<String>, String),
    /// Back to the folder before.
    Back,
}

impl Panel for Assets {
    fn id(&self) -> &'static str {
        ids::ASSETS
    }

    fn title(&self) -> &'static str {
        "Assets"
    }

    fn icon(&self) -> Icon {
        Icon::PanelAssets
    }

    fn zone(&self) -> Zone {
        Zone::Bottom
    }

    fn context(&self) -> Context {
        Context::Assets
    }

    fn ui(&self, ui: &mut Ui, cx: &mut PanelContext<'_>) {
        let editor = &mut *cx.editor;
        let keys = Keys::of(cx.keymap);
        let mut acts = Vec::new();
        if let Some(edit) = editor.browser.command.take() {
            keyboard(editor, edit, &mut acts);
        }
        toolbar(ui, editor, &mut acts);
        if let Some(error) = editor.browser.error.clone() {
            error_line(ui, &error);
        }
        ui.add_space(space::S3);

        let rect = ui.available_rect_before_wrap();
        let width_id = Id::unique("metallic.assets.tree_width");
        let widest = (rect.width() - MIN_SIDE).max(MIN_SIDE);
        let stored = ui.data(|data| data.get_temp::<f32>(width_id)).unwrap_or(TREE_WIDTH);
        let tree_width = stored.clamp(MIN_SIDE, widest);
        let tree_rect = Rect::from_min_max(rect.min, pos2(rect.left() + tree_width, rect.bottom()));
        let grid_rect = Rect::from_min_max(pos2(tree_rect.right() + space::S3, rect.top()), rect.max);

        // The line between the tree and the tiles, dragged to resize them.
        let line = Rect::from_center_size(
            pos2(tree_rect.right() + space::S3 / 2.0, rect.center().y),
            vec2(space::S3, rect.height()),
        );
        let grip = ui.interact(line, Id::unique("metallic.assets.split"), Sense::drag());
        grip.widget_info(|| WidgetInfo::labeled(Role::Splitter, true, "Folder tree width"));
        if grip.dragged() {
            let width = (tree_width + grip.drag_delta().x).clamp(MIN_SIDE, widest);
            ui.data_mut(|data| data.insert_temp(width_id, width));
        }
        if grip.hovered() || grip.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        let column = |rect: Rect| {
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::top_down(egui::Align::Min))
        };
        let mut tree_ui = ui.new_child(column(tree_rect));
        tree_ui.set_clip_rect(tree_rect.intersect(ui.clip_rect()));
        tree::show(&mut tree_ui, editor, &keys, &mut acts);
        let mut grid_ui = ui.new_child(column(grid_rect));
        grid_ui.set_clip_rect(grid_rect.intersect(ui.clip_rect()));
        grid::show(&mut grid_ui, editor, &keys, &mut acts);
        // Drawn after both sides, so nothing they draw covers it (#694).
        let p = current_colors(ui.ctx()).palette;
        let ink = if grip.dragged() { p.subtle } else { p.border };
        ui.painter()
            .vline(line.center().x, rect.y_range(), Stroke::new(size::BORDER, ink));
        ui.advance_cursor_after_rect(rect);

        for act in acts {
            apply(ui, editor, act);
        }
        delete_dialog(ui.ctx(), editor);
    }
}

/// The last operation's error, under the toolbar.
fn error_line(ui: &mut Ui, error: &str) {
    let colors = current_colors(ui.ctx());
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(text::SMALL, text::SMALL), Sense::hover());
        icons::paint_at(
            ui.painter(),
            rect.center(),
            Align2::CENTER_CENTER,
            Icon::UiWarning,
            text::SMALL,
            colors.accent,
        );
        ui.add(Label::new(RichText::new(error).size(text::SMALL).color(colors.palette.muted)).wrap());
    });
}

/// The shortcuts the menus show.
pub(crate) struct Keys {
    pub(crate) rename: Option<String>,
    pub(crate) duplicate: Option<String>,
    pub(crate) delete: Option<String>,
}

impl Keys {
    fn of(keymap: &crate::keymap::Keymap) -> Self {
        use crate::keymap::ids;
        Self {
            rename: keymap.shortcut_text(ids::EDIT_RENAME),
            duplicate: keymap.shortcut_text(ids::EDIT_DUPLICATE),
            delete: keymap.shortcut_text(ids::EDIT_DELETE),
        }
    }
}

/// A key command while the panel has the focus: Rename, Duplicate and Delete act on the selected tiles.
fn keyboard(editor: &EditorState, edit: TreeEdit, acts: &mut Vec<Act>) {
    let selected: Vec<String> = editor.browser.selected.iter().cloned().collect();
    if selected.is_empty() {
        return;
    }
    match edit {
        TreeEdit::Rename if selected.len() == 1 => acts.push(Act::StartRename(selected[0].clone())),
        TreeEdit::Duplicate => acts.push(Act::Duplicate(selected)),
        TreeEdit::Delete => acts.push(Act::Delete(selected)),
        _ => {}
    }
}

/// The toolbar: Back, the open folder's breadcrumbs (each a button, and a place to drop tiles), and the search.
fn toolbar(ui: &mut Ui, editor: &mut EditorState, acts: &mut Vec<Act>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::S1;
        back_button(ui, editor, acts);
        ui.add_space(space::S2);
        let search_width = SEARCH_WIDTH.min(ui.available_width() / 2.0);
        let crumbs_width = (ui.available_width() - search_width - space::S3).max(0.0);
        let layout = Layout::left_to_right(egui::Align::Center);
        ui.allocate_ui_with_layout(vec2(crumbs_width, size::ROW), layout, |ui| {
            ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
            breadcrumbs(ui, editor, acts);
        });
        // At the right end, the magnifier before the field as everywhere else.
        ui.add_space((ui.available_width() - search_width).max(0.0));
        ui.allocate_ui_with_layout(vec2(search_width, size::ROW), layout, |ui| {
            super::search_box(ui, &mut editor.browser.query, "Search the assets");
        });
    });
}

/// Back, greyed out with nowhere to go back to.
fn back_button(ui: &mut Ui, editor: &EditorState, acts: &mut Vec<Act>) {
    let p = current_colors(ui.ctx()).palette;
    let can_back = !editor.browser.back.is_empty();
    let sense = if can_back { Sense::click() } else { Sense::hover() };
    let (rect, back) = ui.allocate_exact_size(vec2(size::ROW, size::ROW), sense);
    back.widget_info(|| WidgetInfo::labeled(Role::Button, can_back, "Back"));
    if back.hovered() && can_back {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface);
    }
    let ink = if can_back { p.text } else { p.text.gamma_multiply(0.48) };
    icons::paint_at(
        ui.painter(),
        rect.center(),
        Align2::CENTER_CENTER,
        Icon::UiBack,
        text::BODY,
        ink,
    );
    if back.clicked() {
        acts.push(Act::Back);
    }
}

/// The open folder's path, `Project > Assets > Trees`, each part a button that opens it and takes dropped tiles.
fn breadcrumbs(ui: &mut Ui, editor: &EditorState, acts: &mut Vec<Act>) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let searching = !editor.browser.query.trim().is_empty();
    let mut crumbs = vec![(ROOT.to_owned(), String::new())];
    for part in editor.browser.open.split('/').filter(|part| !part.is_empty()) {
        let folder = browser::join(&crumbs[crumbs.len() - 1].1, part);
        crumbs.push((part.to_owned(), folder));
    }
    let last = crumbs.len() - 1;
    for (i, (label, folder)) in crumbs.iter().enumerate() {
        if i > 0 {
            let (chevron, _) = ui.allocate_exact_size(vec2(text::SMALL, size::ROW), Sense::hover());
            icons::paint_at(
                ui.painter(),
                chevron.center(),
                Align2::CENTER_CENTER,
                Icon::UiExpand,
                text::SMALL,
                p.subtle,
            );
        }
        let current = i == last && !searching;
        let font = if current {
            fonts::named(fonts::STRONG, text::BODY)
        } else {
            egui::FontId::proportional(text::BODY)
        };
        let galley = ui.painter().layout_no_wrap(label.clone(), font, p.text);
        let width = galley.size().x + 2.0 * space::S2;
        let (rect, crumb) = ui.allocate_exact_size(vec2(width, size::ROW), Sense::click());
        crumb.widget_info(|| WidgetInfo::labeled(Role::Link, true, format!("Open {label}")));
        let dropping = drop_target(&crumb, folder);
        if crumb.hovered() || dropping {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius::ITEM), p.surface);
        }
        if dropping {
            let stroke = Stroke::new(2.0, colors.accent);
            ui.painter()
                .rect_stroke(rect, CornerRadius::same(radius::ITEM), stroke, egui::StrokeKind::Inside);
        }
        let ink = if current || crumb.hovered() { p.text } else { p.muted };
        let at = pos2(rect.left() + space::S2, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(at, galley, ink);
        if crumb.clicked() {
            acts.push(Act::Open(folder.clone()));
        }
        if let Some(paths) = dropped_on(&crumb) {
            acts.push(Act::Move(paths, folder.clone()));
        }
    }
}

/// Whether tiles dragged from the browser are over `response` and could go into `folder`.
pub(crate) fn drop_target(response: &egui::Response, folder: &str) -> bool {
    response
        .dnd_hover_payload::<AssetDrag>()
        .is_some_and(|drag| accepts(&drag.paths, folder))
}

/// The paths dropped on `response` this frame.
pub(crate) fn dropped_on(response: &egui::Response) -> Option<Vec<String>> {
    response
        .dnd_release_payload::<AssetDrag>()
        .map(|drag| drag.paths.clone())
}

/// Whether `paths` can move into `folder`: not into one of themselves, and not where they all are already.
fn accepts(paths: &[String], folder: &str) -> bool {
    let into_itself = paths
        .iter()
        .any(|path| folder == path || folder.starts_with(&format!("{path}/")));
    !into_itself && paths.iter().any(|path| browser::parent_of(path) != folder)
}

/// Does what the tree or the tiles asked for.
fn apply(ui: &Ui, editor: &mut EditorState, act: Act) {
    let result = match act {
        Act::Open(folder) => {
            editor.browser.open_folder(&folder);
            Ok(())
        }
        Act::Show(path) => {
            editor.browser.open_folder(browser::parent_of(&path));
            editor.browser.selected.insert(path.clone());
            editor.browser.anchor = Some(path);
            Ok(())
        }
        Act::Activate(entry) => match entry.kind {
            browser::EntryKind::Folder => {
                editor.browser.open_folder(&entry.path);
                Ok(())
            }
            browser::EntryKind::File(AssetKind::Script(_)) => editor.open_script_file(&entry.path),
            browser::EntryKind::File(AssetKind::Material) => editor.open_material(&entry.path),
            browser::EntryKind::File(AssetKind::Image) => editor.open_image(&entry.path),
            _ => Ok(()),
        },
        Act::Click { path, .. } if path.is_empty() => {
            editor.browser.selected.clear();
            editor.browser.anchor = None;
            Ok(())
        }
        Act::Click { path, add, range } => {
            let shown = shown_entries(editor);
            editor.browser.click(&shown, &path, add, range);
            // A plain click on a material gives it to the selected parts (#680); with none selected, nothing changes.
            if !add
                && !range
                && AssetKind::of(&path) == AssetKind::Material
                && let Some(id) = editor.project.assets().id(&path)
            {
                editor.apply_material(id);
            }
            Ok(())
        }
        Act::Toggle(folder) => {
            if !editor.browser.expanded.remove(&folder) {
                editor.browser.expanded.insert(folder);
            }
            Ok(())
        }
        Act::StartRename(path) => {
            editor.browser.renaming = Some(path);
            Ok(())
        }
        Act::Rename(path, name) => {
            editor.browser.renaming = None;
            editor.rename_asset(&path, &name).map(|_| ())
        }
        Act::KeepName => {
            editor.browser.renaming = None;
            Ok(())
        }
        Act::New(parent, new) => match new {
            New::Folder => editor.new_folder(&parent),
            New::Script(script) => editor.new_script_file(&parent, script),
            New::Material => editor.new_material_file(&parent),
        }
        .map(|path| {
            if parent != editor.browser.open {
                editor.browser.open_folder(&parent);
            }
            editor.browser.selected = [path.clone()].into();
            editor.browser.renaming = Some(path);
        }),
        Act::Duplicate(paths) => editor.duplicate_assets(&paths).map(|copies| {
            editor.browser.selected = copies.into_iter().collect();
        }),
        Act::Delete(paths) => editor
            .plan_delete(&paths)
            .map(|plan| editor.browser.deleting = Some(plan)),
        Act::Move(paths, folder) => editor.move_assets(&paths, &folder),
        Act::Back => {
            editor.browser.go_back();
            Ok(())
        }
    };
    editor.browser.error = result.err();
    ui.ctx().request_repaint();
}

/// The tiles shown on the right, in order: the search's results, or the open folder's contents.
pub(crate) fn shown_entries(editor: &EditorState) -> Vec<Entry> {
    let query = editor.browser.query.trim();
    if query.is_empty() {
        editor.asset_children(&editor.browser.open)
    } else {
        editor.asset_search(query)
    }
}

/// One line of a right-click menu: its label (empty for a separator), its shortcut, and whether it can be chosen.
pub(crate) type MenuLine<'a> = (&'a str, Option<String>, bool);

/// A right-click menu on `response` of `items`, drawn as Studio's menus are (#660). Returns the index of the item
/// chosen.
pub(crate) fn context_menu(response: &egui::Response, items: &[MenuLine<'_>]) -> Option<usize> {
    let p = current_colors(&response.ctx).palette;
    let mut chosen = None;
    egui::Popup::context_menu(response)
        .style(egui::style::StyleModifier::default())
        .frame(
            Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(size::BORDER, p.border))
                .corner_radius(CornerRadius::same(radius::ITEM))
                .shadow(shadow::MENU)
                .inner_margin(space::S2),
        )
        .show(|ui| {
            ui.spacing_mut().item_spacing.y = menu::ITEM_GAP;
            let lines = items.iter().map(|(label, keys, _)| (*label, keys.as_deref(), false));
            let width = menu::menu_width(ui, lines);
            ui.set_min_width(width);
            ui.set_max_width(width);
            for (i, (label, keys, enabled)) in items.iter().enumerate() {
                if label.is_empty() {
                    ui.separator();
                } else if menu::menu_item(ui, width, label, keys.clone(), None, *enabled).clicked() {
                    chosen = Some(i);
                }
            }
        });
    chosen
}

/// A file or folder's name being edited in place, in `field`: focused, all of it selected at first. Enter, or a click
/// elsewhere, keeps what is typed; Escape keeps the old name.
pub(crate) fn rename_field(ui: &mut Ui, field: Rect, path: &str, name: &str, centred: bool) -> Option<Act> {
    let id = Id::unique(("metallic.assets.rename", path));
    let field_id = id.with("field");
    let first = ui.data_mut(|data| data.get_temp::<String>(id)).is_none();
    let mut typed = ui
        .data_mut(|data| data.get_temp::<String>(id))
        .unwrap_or_else(|| name.to_owned());
    let size = if centred { text::SMALL } else { text::BODY };
    let mut edit = egui::TextEdit::singleline(&mut typed)
        .id(field_id)
        .margin(vec2(space::S1, 0.0))
        .font(egui::FontId::proportional(size));
    if centred {
        edit = edit.align(egui::Align2::CENTER_CENTER);
    }
    let response = ui.put(field, edit);
    response.widget_info(|| WidgetInfo::labeled(Role::TextInput, true, format!("Name of {name}")));
    if first {
        response.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), field_id) {
            let all = egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(typed.chars().count()),
            );
            state.cursor.set_char_range(Some(all));
            state.store(ui.ctx(), field_id);
        }
    } else if response.lost_focus() || !response.has_focus() {
        ui.data_mut(|data| data.remove::<String>(id));
        let escape = ui.input(|input| input.key_pressed(egui::Key::Escape));
        return Some(if escape || typed == name {
            Act::KeepName
        } else {
            Act::Rename(path.to_owned(), typed)
        });
    }
    ui.data_mut(|data| data.insert_temp(id, typed));
    None
}

/// Delete's confirmation, while it is asked: what goes, how many files, and what refers to them.
fn delete_dialog(ctx: &egui::Context, editor: &mut EditorState) {
    let Some(Deleting {
        paths,
        files,
        references,
    }) = editor.browser.deleting.clone()
    else {
        return;
    };
    let p = current_colors(ctx).palette;
    let frame = Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(size::BORDER, p.border))
        .corner_radius(CornerRadius::same(radius::DIALOG))
        .shadow(shadow::MENU)
        .inner_margin(space::S8);
    let (mut delete, mut cancel) = (false, false);
    let title = match paths.as_slice() {
        [one] => format!("Delete {}?", browser::file_name(one)),
        many => format!("Delete {} items?", many.len()),
    };
    let modal = Modal::new(Id::unique("metallic.assets.delete"))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(400.0);
            ui.label(
                RichText::new(title)
                    .font(fonts::named(fonts::HEADING, text::TITLE))
                    .color(p.text),
            );
            let what = match files {
                0 => "Nothing is in it.".to_owned(),
                1 => "1 file goes.".to_owned(),
                n => format!("{n} files go."),
            };
            let line = format!("{what} It leaves the project's file when you save.");
            ui.add(Label::new(RichText::new(line).size(text::BODY).color(p.muted)).wrap());
            if !references.is_empty() {
                ui.add_space(space::S4);
                ui.label(crate::projects::section_label("Still used by", p.subtle));
                for line in references.iter().take(8) {
                    ui.label(RichText::new(line).size(text::SMALL).color(p.muted));
                }
                if references.len() > 8 {
                    let more = format!("and {} more", references.len() - 8);
                    ui.label(RichText::new(more).size(text::SMALL).color(p.subtle));
                }
            }
            ui.add_space(space::S6);
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                delete = button(ui, ButtonKind::Primary, "Delete").clicked();
                cancel = button(ui, ButtonKind::Secondary, "Cancel").clicked();
            });
        });
    if modal.should_close() {
        cancel = true;
    }
    if delete {
        editor.browser.deleting = None;
        editor.browser.error = editor.delete_assets(&paths).err();
    } else if cancel {
        editor.browser.deleting = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_come_from_the_extension() {
        assert_eq!(AssetKind::of("Meshes/Rock.GLB"), AssetKind::Mesh);
        assert_eq!(AssetKind::of("Textures/a.b/rock.jpeg"), AssetKind::Image);
        assert_eq!(AssetKind::of("Sounds/hit.ogg"), AssetKind::Audio);
        assert_eq!(AssetKind::of("Scenes/Main.scene"), AssetKind::Scene);
        assert_eq!(AssetKind::of("notes"), AssetKind::Other);
    }

    #[test]
    fn a_script_file_s_type_is_in_its_name() {
        let script = |path| AssetKind::of(path);
        assert_eq!(
            script("Scripts/Door.server.luau"),
            AssetKind::Script(ScriptType::Server)
        );
        assert_eq!(script("Scripts/Hud.CLIENT.luau"), AssetKind::Script(ScriptType::Local));
        assert_eq!(script("Scripts/Util.luau"), AssetKind::Script(ScriptType::Module));
        assert_eq!(script("old/Thing.server.lua"), AssetKind::Script(ScriptType::Server));
        assert_eq!(AssetKind::Script(ScriptType::Local).label(), "LocalScript");
        for script in ScriptType::ALL {
            let file = format!("Door{}", script.extension());
            assert_eq!(ScriptType::of(&file), Some(script), "{file}");
        }
        assert_eq!(ScriptType::of("Door.txt"), None);
    }

    #[test]
    fn sizes_read_well() {
        assert_eq!(file_size(512), "512 B");
        assert_eq!(file_size(1536), "1.5 KB");
        assert_eq!(file_size(25 * 1024 * 1024), "25 MB");
    }

    #[test]
    fn tiles_go_into_other_folders_only() {
        let paths = ["Assets/Trees".to_owned(), "Assets/oak.png".to_owned()];
        assert!(accepts(&paths, "Scenes"));
        assert!(!accepts(&paths, "Assets/Trees"), "into itself");
        assert!(!accepts(&paths, "Assets/Trees/Pines"), "inside itself");
        assert!(!accepts(&paths, "Assets"), "where they are");
        assert!(accepts(&paths[1..], ""));
    }
}
