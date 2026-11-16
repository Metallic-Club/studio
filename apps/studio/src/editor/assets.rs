//! The Assets browser's state and its file operations (#676, ADR 0012): which folder is open, what is selected, and
//! creating, renaming, moving, duplicating and deleting the project's files and folders.
//!
//! The browser reads the project's members and folders every frame, so it shows every change, whoever made it. An
//! operation goes through `metallic_project`'s file operations, which keep asset ids and rewrite the paths that name
//! a moved file in the saved scenes; [`EditorState`] then does the same for what it holds in memory: the scene being
//! edited (its scripts' `Source`), the script editor's open files, and the browser's own paths.

use std::collections::{BTreeSet, HashMap};

use metallic_engine::metallic_core::{DataModel, Id, Value};
use metallic_engine::metallic_project::asset_index::{is_asset, is_editor_data};
use metallic_engine::metallic_project::{MemberMove, PATH_PROPERTIES, Project};

use super::EditorState;
use super::open::asset_rows;
use crate::panels::assets::{AssetKind, ScriptType};

/// The name a new folder gets, made unique with a number.
pub const NEW_FOLDER: &str = "New Folder";

/// What the browser keeps between frames.
#[derive(Clone, Debug)]
pub struct AssetBrowser {
    /// The folder shown on the right: its path, `""` for the project's root.
    pub open: String,
    /// The folders opened before, for Back.
    pub back: Vec<String>,
    /// The tree's open folders.
    pub expanded: BTreeSet<String>,
    /// The selected files and folders, by path.
    pub selected: BTreeSet<String>,
    /// Where a Shift-click's range starts.
    pub anchor: Option<String>,
    /// The search, across the whole project.
    pub query: String,
    /// The file or folder whose name is being edited.
    pub renaming: Option<String>,
    /// Files and folders waiting for Delete's confirmation, with what the dialog says.
    pub deleting: Option<Deleting>,
    /// A command from the keyboard for the panel to run on its next frame (the keymap runs before the panels).
    pub command: Option<crate::editor::TreeEdit>,
    /// The last operation's error, shown under the toolbar until the next one.
    pub error: Option<String>,
    /// How big the tiles are, 1 at first (#696): Ctrl and the wheel change it, between [`MIN_SCALE`] and
    /// [`MAX_SCALE`]. Kept while Studio runs.
    pub scale: f32,
    /// Tiles or a list (#699).
    pub view: View,
    /// How the list is sorted (#699).
    pub sort: Sort,
}

/// How the browser shows a folder (#699).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    /// Tiles that wrap with the panel's width.
    #[default]
    Grid,
    /// A row each, under a header that sorts them.
    List,
}

/// A column of the list (#699), which it can be sorted by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Column {
    /// The file or folder's name.
    #[default]
    Name,
    /// What it is ([`AssetKind::label`]).
    Type,
    /// When it was added to the project (`metallic_project`'s asset index, #710).
    Created,
    /// Its size in bytes.
    Size,
}

impl Column {
    /// Every column, left to right.
    pub const ALL: [Column; 4] = [Column::Name, Column::Type, Column::Created, Column::Size];

    /// Its header.
    pub fn label(self) -> &'static str {
        match self {
            Column::Name => "Name",
            Column::Type => "Type",
            Column::Created => "Date created",
            Column::Size => "Size",
        }
    }
}

/// How the list is sorted (#699): by a column, one way or the other. Folders come first either way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sort {
    /// The column.
    pub column: Column,
    /// Largest, latest or last first.
    pub descending: bool,
}

/// What `entry` is, as the list's Type column says: its kind's name, a folder's "Folder".
pub fn type_label(entry: &Entry) -> &'static str {
    match entry.kind {
        EntryKind::Folder => "Folder",
        EntryKind::File(kind) | EntryKind::Missing(kind) => kind.label(),
    }
}

/// Sorts `entries` as `sort` says (#699): folders first, then by the column (an unknown date or size last either way),
/// then by name.
pub fn sort_entries(project: &Project, entries: &mut [Entry], sort: Sort) {
    use std::cmp::Ordering;
    let name = |entry: &Entry| entry.file_name().to_lowercase();
    let size = |entry: &Entry| match entry.kind {
        EntryKind::File(_) => project
            .fingerprint(&entry.path)
            .ok()
            .map(|fingerprint| fingerprint.size),
        _ => None,
    };
    let created = |entry: &Entry| project.assets().added(&entry.path);
    // Known values in the order asked for, unknown ones after them.
    let known_first = |a: Option<u64>, b: Option<u64>| match (a, b) {
        (Some(a), Some(b)) if sort.descending => b.cmp(&a),
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    };
    entries.sort_by(|a, b| {
        let by_column = match sort.column {
            Column::Name => Ordering::Equal,
            Column::Type => {
                let order = type_label(a).to_lowercase().cmp(&type_label(b).to_lowercase());
                if sort.descending { order.reverse() } else { order }
            }
            Column::Created => known_first(created(a), created(b)),
            Column::Size => known_first(size(a), size(b)),
        };
        let by_name = name(a).cmp(&name(b));
        let by_name = if sort.descending && sort.column == Column::Name {
            by_name.reverse()
        } else {
            by_name
        };
        b.is_folder()
            .cmp(&a.is_folder())
            .then(by_column)
            .then(by_name)
            .then_with(|| a.path.cmp(&b.path))
    });
}

impl Default for AssetBrowser {
    fn default() -> Self {
        Self {
            open: String::new(),
            back: Vec::new(),
            expanded: BTreeSet::new(),
            selected: BTreeSet::new(),
            anchor: None,
            query: String::new(),
            renaming: None,
            deleting: None,
            command: None,
            error: None,
            scale: 1.0,
            view: View::Grid,
            sort: Sort::default(),
        }
    }
}

/// The smallest the browser's tiles get (#696).
pub const MIN_SCALE: f32 = 0.6;
/// The biggest the browser's tiles get (#696).
pub const MAX_SCALE: f32 = 2.5;

/// Delete's confirmation: what is deleted and what refers to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deleting {
    /// The files and folders.
    pub paths: Vec<String>,
    /// How many files that is, counting those in the folders.
    pub files: usize,
    /// The scene properties that refer to them, as "scene: property" lines.
    pub references: Vec<String>,
}

/// One thing in a folder: a folder or a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Its path in the project.
    pub path: String,
    /// What it is.
    pub kind: EntryKind,
    /// The asset index's id of a file, as text.
    pub id: Option<String>,
}

/// What an [`Entry`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// A folder.
    Folder,
    /// A file of this kind.
    File(AssetKind),
    /// A file the asset index names but the project no longer has.
    Missing(AssetKind),
}

impl Entry {
    /// Whether it is a folder.
    pub fn is_folder(&self) -> bool {
        self.kind == EntryKind::Folder
    }

    /// Its file or folder name, the last part of its path.
    pub fn file_name(&self) -> &str {
        file_name(&self.path)
    }

    /// The name people see: a file's name without its type's extension (`Door` for `Door.server.luau`).
    pub fn name(&self) -> &str {
        if self.is_folder() {
            self.file_name()
        } else {
            split_name(self.file_name()).0
        }
    }
}

/// The last part of `path`.
pub fn file_name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

/// The folder `path` is in, `""` for the root.
pub fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

/// `name` in `folder`.
pub fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// The extensions that are two parts long: a script's type is in its name (#677).
const DOUBLE_EXTENSIONS: [&str; 4] = [".server.luau", ".client.luau", ".server.lua", ".client.lua"];

/// A file name's stem and extension, the extension with its dot (`("Door", ".server.luau")`). A name that starts
/// with its only dot has no extension.
pub fn split_name(name: &str) -> (&str, &str) {
    let lower = name.to_ascii_lowercase();
    for double in DOUBLE_EXTENSIONS {
        if lower.ends_with(double) && name.len() > double.len() {
            return name.split_at(name.len() - double.len());
        }
    }
    match name.rfind('.') {
        Some(0) | None => (name, ""),
        Some(dot) => name.split_at(dot),
    }
}

/// Whether the browser shows `member`: every asset but the editor's own data.
fn shown(member: &str) -> bool {
    is_asset(member) && !is_editor_data(member)
}

/// The folders the browser shows: the project's, without the editor's data folder.
pub fn folders(project: &Project) -> Vec<String> {
    project
        .folders()
        .into_iter()
        .filter(|folder| !is_editor_data(&format!("{folder}/")))
        .collect()
}

/// Sorts entries as the browser lists them: folders first, then by name, ignoring case.
fn sort(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        b.is_folder()
            .cmp(&a.is_folder())
            .then_with(|| a.file_name().to_lowercase().cmp(&b.file_name().to_lowercase()))
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// The entry for every file and folder of the project, with the missing assets the index still names.
fn all_entries(project: &Project) -> Vec<Entry> {
    let mut entries: Vec<Entry> = folders(project)
        .into_iter()
        .map(|path| Entry {
            path,
            kind: EntryKind::Folder,
            id: None,
        })
        .collect();
    for row in asset_rows(project) {
        if !shown(&row.path) {
            continue;
        }
        let kind = AssetKind::of(&row.path);
        entries.push(Entry {
            kind: if row.missing {
                EntryKind::Missing(kind)
            } else {
                EntryKind::File(kind)
            },
            path: row.path,
            id: Some(row.id),
        });
    }
    // Members not indexed yet (none, as Studio keeps the index up to date; but never hide a file).
    for member in project.members() {
        if shown(member) && !entries.iter().any(|entry| entry.path == member) {
            entries.push(Entry {
                path: member.to_owned(),
                kind: EntryKind::File(AssetKind::of(member)),
                id: None,
            });
        }
    }
    entries
}

/// What is in `folder` (`""` for the root): its folders, then its files, by name.
pub fn children(project: &Project, folder: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = all_entries(project)
        .into_iter()
        .filter(|entry| parent_of(&entry.path) == folder)
        .collect();
    sort(&mut entries);
    entries
}

/// Every file and folder whose name holds `query` (ignoring case), by name.
pub fn search(project: &Project, query: &str) -> Vec<Entry> {
    let query = query.trim().to_lowercase();
    let mut entries: Vec<Entry> = all_entries(project)
        .into_iter()
        .filter(|entry| entry.file_name().to_lowercase().contains(&query))
        .collect();
    sort(&mut entries);
    entries
}

/// The type of each script file a script of `dm` runs (#701): that script's, whatever the file's name says. Scripts
/// saved before #677 are `Name.luau` whatever they are, which their names alone would make ModuleScripts. Of two
/// scripts running one file, the first in the tree's order counts.
pub fn script_types(dm: &DataModel) -> HashMap<String, ScriptType> {
    let mut types = HashMap::new();
    for (id, member) in crate::code::script_files(dm) {
        types.entry(member).or_insert_with(|| ScriptType::of_class(dm, id));
    }
    types
}

/// Gives each script file among `entries` that a script runs that script's type ([`script_types`]).
pub fn retype(entries: &mut [Entry], types: &HashMap<String, ScriptType>) {
    for entry in entries {
        let Some(&script) = types.get(&entry.path) else {
            continue;
        };
        if let EntryKind::File(kind @ AssetKind::Script(_)) | EntryKind::Missing(kind @ AssetKind::Script(_)) =
            &mut entry.kind
        {
            *kind = AssetKind::Script(script);
        }
    }
}

/// `name` made unique in `folder` with a number before its extension (`New Folder 2`, `Rock 2.png`).
pub fn unique_name(project: &Project, folder: &str, name: &str) -> String {
    let taken = |name: &str| {
        let path = join(folder, name);
        project.contains(&path) || project.has_folder(&path)
    };
    if !taken(name) {
        return name.to_owned();
    }
    let (stem, extension) = split_name(name);
    (2..)
        .map(|n| format!("{stem} {n}{extension}"))
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| name.to_owned())
}

/// What an import did (#706): what it brought in, and what it could not, with why.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Imported {
    /// The files and folders made, by path in the project: each one imported, not what is inside a folder.
    pub paths: Vec<String>,
    /// Each file or folder it could not bring in, by its name, with why.
    pub failed: Vec<String>,
}

impl Imported {
    /// What the browser says when not everything came in: `None` when everything did.
    pub fn error(&self) -> Option<String> {
        match self.failed.as_slice() {
            [] => None,
            [one] => Some(format!("Could not import {one}.")),
            [first, rest @ ..] => Some(format!("Could not import {first}, and {} more.", rest.len())),
        }
    }
}

/// The most files one import brings in, so a folder dropped by mistake (a whole drive) does not fill the project.
pub const IMPORT_LIMIT: usize = 10_000;
/// The deepest folder an import goes into.
pub const IMPORT_DEPTH: usize = 32;

/// Whether `name` can be a file or folder name: not empty, no `/` or `\`, not `.` or `..`.
fn valid_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("A name cannot be empty.".into());
    }
    if name.contains(['/', '\\']) {
        return Err("A name cannot hold / or \\.".into());
    }
    if name == "." || name == ".." {
        return Err("A name cannot be . or ...".into());
    }
    Ok(())
}

impl AssetBrowser {
    /// Opens `folder` on the right, remembering the one before for Back, and shows it in the tree.
    pub fn open_folder(&mut self, folder: &str) {
        if folder == self.open && self.query.is_empty() {
            return;
        }
        self.back.push(std::mem::replace(&mut self.open, folder.to_owned()));
        self.query.clear();
        self.selected.clear();
        self.anchor = None;
        self.reveal(folder);
    }

    /// Sorts the list by `column` (#699): the other way when it is sorted by it already, else that way up.
    pub fn sort_by(&mut self, column: Column) {
        self.sort = if self.sort.column == column {
            Sort {
                column,
                descending: !self.sort.descending,
            }
        } else {
            Sort {
                column,
                descending: false,
            }
        };
    }

    /// Scales the tiles by `factor` (Ctrl and the wheel, #696), between [`MIN_SCALE`] and [`MAX_SCALE`].
    pub fn zoom(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        }
    }

    /// Goes back to the folder opened before this one. Returns whether there was one.
    pub fn go_back(&mut self) -> bool {
        match self.back.pop() {
            Some(folder) => {
                self.open = folder;
                self.selected.clear();
                self.anchor = None;
                true
            }
            None => false,
        }
    }

    /// Expands the tree down to `path`'s folder.
    pub fn reveal(&mut self, path: &str) {
        let mut at = path;
        while !at.is_empty() {
            self.expanded.insert(at.to_owned());
            at = parent_of(at);
        }
    }

    /// A click on `path` among `shown` (in their order): selects it alone, or adds or removes it (`add`), or selects
    /// the run from the anchor to it (`range`).
    pub fn click(&mut self, shown: &[Entry], path: &str, add: bool, range: bool) {
        let position = |p: &str| shown.iter().position(|entry| entry.path == p);
        if range && let (Some(from), Some(to)) = (self.anchor.as_deref().and_then(position), position(path)) {
            let (low, high) = (from.min(to), from.max(to));
            if !add {
                self.selected.clear();
            }
            self.selected
                .extend(shown[low..=high].iter().map(|entry| entry.path.clone()));
            return;
        }
        if add {
            if !self.selected.remove(path) {
                self.selected.insert(path.to_owned());
            }
        } else {
            self.selected.clear();
            self.selected.insert(path.to_owned());
        }
        self.anchor = Some(path.to_owned());
    }

    /// Moves every path the browser keeps that `moved` changed.
    fn follow(&mut self, moved: &[(String, String)]) {
        let remap = |path: &str| -> Option<String> {
            moved.iter().find_map(|(from, to)| {
                if path == from {
                    Some(to.clone())
                } else {
                    path.strip_prefix(&format!("{from}/"))
                        .map(|rest| format!("{to}/{rest}"))
                }
            })
        };
        let map = |path: &mut String| {
            if let Some(new) = remap(path) {
                *path = new;
            }
        };
        map(&mut self.open);
        self.back.iter_mut().for_each(map);
        for set in [&mut self.expanded, &mut self.selected] {
            *set = set
                .iter()
                .map(|path| remap(path).unwrap_or_else(|| path.clone()))
                .collect();
        }
        if let Some(anchor) = &mut self.anchor {
            map(anchor);
        }
    }

    /// Forgets every path under `removed`.
    fn forget(&mut self, removed: &[String]) {
        let gone = |path: &str| removed.iter().any(|r| path == r || path.starts_with(&format!("{r}/")));
        while gone(&self.open) {
            self.open = parent_of(&self.open).to_owned();
        }
        self.back.retain(|path| !gone(path));
        self.expanded.retain(|path| !gone(path));
        self.selected.retain(|path| !gone(path));
        if self.anchor.as_deref().is_some_and(gone) {
            self.anchor = None;
        }
    }
}

impl EditorState {
    /// What the browser shows in `folder` ([`children`]), script files of the type their scripts are (#701).
    pub fn asset_children(&self, folder: &str) -> Vec<Entry> {
        let mut entries = children(&self.project, folder);
        retype(&mut entries, &script_types(&self.dm));
        entries
    }

    /// What the browser's search for `query` finds ([`search`]), script files of the type their scripts are (#701).
    pub fn asset_search(&self, query: &str) -> Vec<Entry> {
        let mut entries = search(&self.project, query);
        retype(&mut entries, &script_types(&self.dm));
        entries
    }

    /// The type of the script file `path` (#701): the type of a script that runs it, else the type its name gives;
    /// `None` for a file that is not a script.
    pub fn script_type(&self, path: &str) -> Option<ScriptType> {
        let named = ScriptType::of(path)?;
        Some(script_types(&self.dm).get(path).copied().unwrap_or(named))
    }

    /// Makes a new, empty folder in `parent` with a unique name, and returns its path.
    pub fn new_folder(&mut self, parent: &str) -> Result<String, String> {
        let name = unique_name(&self.project, parent, NEW_FOLDER);
        let path = join(parent, &name);
        self.project.create_folder(&path).map_err(|error| error.to_string())?;
        self.browser.reveal(parent);
        Ok(path)
    }

    /// Copies the files and folders at `sources` on the computer into the project's `folder` (#706): each a new asset
    /// (or folder, with what is in it), named as it is, made unique where the name is taken. Shows the folder, and
    /// selects what came in. Up to [`IMPORT_LIMIT`] files; what cannot be read or added is noted and skipped.
    pub fn import_files(&mut self, folder: &str, sources: &[std::path::PathBuf]) -> Imported {
        let mut imported = Imported::default();
        let mut files = 0;
        for source in sources {
            match self.import_one(folder, source, &mut files, 0) {
                Ok(path) => imported.paths.push(path),
                Err(why) => imported.failed.push(why),
            }
        }
        self.assets = asset_rows(&self.project);
        if !imported.paths.is_empty() {
            if self.browser.open != folder || !self.browser.query.is_empty() {
                self.browser.open_folder(folder);
            }
            self.browser.reveal(folder);
            self.browser.selected = imported.paths.iter().cloned().collect();
        }
        imported
    }

    /// Imports the file or folder `source` into `folder` (see [`EditorState::import_files`]), counting the files in
    /// `files`, `depth` folders down. Returns its path, or its name and why it could not come in.
    fn import_one(
        &mut self,
        folder: &str,
        source: &std::path::Path,
        files: &mut usize,
        depth: usize,
    ) -> Result<String, String> {
        let shown = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| source.display().to_string());
        let name = unique_name(&self.project, folder, &shown);
        valid_name(&name).map_err(|why| format!("{shown} ({why})"))?;
        let path = join(folder, &name);
        if source.is_dir() {
            // Deep enough for any real folder; a link back up the tree stops here rather than going round for ever.
            if depth >= IMPORT_DEPTH {
                return Err(format!("{shown} (folders more than {IMPORT_DEPTH} deep)"));
            }
            self.project
                .create_folder(&path)
                .map_err(|error| format!("{shown} ({error})"))?;
            let mut inside: Vec<std::path::PathBuf> = std::fs::read_dir(source)
                .map_err(|error| format!("{shown} ({error})"))?
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .collect();
            inside.sort();
            for item in inside {
                // What is inside that cannot come in is left out; the folder still does.
                if let Err(why) = self.import_one(&path, &item, files, depth + 1) {
                    log::warn!("assets: could not import {why}");
                }
            }
            return Ok(path);
        }
        if *files >= IMPORT_LIMIT {
            return Err(format!("{shown} (more than {IMPORT_LIMIT} files at once)"));
        }
        let bytes = std::fs::read(source).map_err(|error| format!("{shown} ({error})"))?;
        self.project
            .create_member(&path, bytes)
            .map_err(|error| format!("{shown} ({error})"))?;
        *files += 1;
        Ok(path)
    }

    /// Makes a new material in `parent` (#678): plain plastic, called Material and made unique. Returns its path.
    pub fn new_material_file(&mut self, parent: &str) -> Result<String, String> {
        use metallic_engine::metallic_project::material::{MATERIAL_EXTENSION, Material};
        let name = unique_name(&self.project, parent, &format!("Material.{MATERIAL_EXTENSION}"));
        let path = join(parent, &name);
        self.project
            .create_member(&path, Material::default().to_json())
            .map_err(|error| error.to_string())?;
        self.browser.reveal(parent);
        self.assets = asset_rows(&self.project);
        Ok(path)
    }

    /// Makes a new script file of type `script` in `parent` (#677), named after its class and made unique, holding
    /// the type's starter code. Returns its path.
    pub fn new_script_file(&mut self, parent: &str, script: ScriptType) -> Result<String, String> {
        let name = unique_name(
            &self.project,
            parent,
            &format!("{}{}", script.class(), script.extension()),
        );
        let path = join(parent, &name);
        self.project
            .create_member(&path, script.starter())
            .map_err(|error| error.to_string())?;
        self.browser.reveal(parent);
        self.assets = asset_rows(&self.project);
        Ok(path)
    }

    /// Opens the script file `path` in the script editor (#677): in the tab of the editor's script that runs it. A
    /// file no script runs yet gets one, of its type, where that type runs ([`ScriptType::home`]), as an edit of the
    /// scene that undo takes back.
    pub fn open_script_file(&mut self, path: &str) -> Result<(), String> {
        let Some(script) = self.script_type(path) else {
            return Err(format!("{} is not a script.", file_name(path)));
        };
        let running = crate::code::script_files(&self.dm)
            .into_iter()
            .find(|(_, member)| member == path)
            .map(|(id, _)| id);
        let id = match running {
            Some(id) => id,
            None => {
                if self.is_playing() {
                    return Err("Stop the game to open a script no object runs yet.".into());
                }
                let home = self
                    .dm
                    .registry()
                    .class_id(script.home())
                    .and_then(|class| self.dm.service(class));
                let Some(home) = home else {
                    return Err(format!("There is no {} to put its script in.", script.home()));
                };
                let id = self.script_from_file(path, home)?;
                log::info!(
                    "assets: {} had no script in the scene, so a {} for it was added to {}",
                    file_name(path),
                    script.class(),
                    script.home()
                );
                id
            }
        };
        self.open_script(id);
        Ok(())
    }

    /// Adds a script of the file `path`'s type inside `parent`, named after the file, its `Source` the file, as an
    /// edit of the scene (#677: a script file dragged into the Explorer). Returns the new script.
    pub fn script_from_file(&mut self, path: &str, parent: Id) -> Result<Id, String> {
        let Some(script) = self.script_type(path) else {
            return Err(format!("{} is not a script.", file_name(path)));
        };
        let class = self
            .dm
            .registry()
            .class_id(script.class())
            .ok_or_else(|| format!("There is no {} class.", script.class()))?;
        let name = split_name(file_name(path)).0.to_owned();
        let mut scene = self.scene();
        let id = scene.insert(class, parent, None).map_err(|error| error.to_string())?;
        scene.rename(id, &name).map_err(|error| error.to_string())?;
        scene
            .set_by_name(id, "Source", Value::String(path.into()))
            .map_err(|error| error.to_string())?;
        Ok(id)
    }

    /// Whether something is selected to add a script to (#704: the button on a script file's tile).
    pub fn can_add_script(&mut self) -> bool {
        !self.scene().selection.is_empty()
    }

    /// Adds a script running the file `path`, of its type ([`EditorState::script_type`]), inside each selected object
    /// (#704), as one undo step, and selects the new scripts. Returns them.
    pub fn add_script_to_selection(&mut self, path: &str) -> Result<Vec<Id>, String> {
        let parents: Vec<Id> = self.scene().selection.ids().to_vec();
        if parents.is_empty() {
            return Err("Select an object to add the script to.".into());
        }
        let mut made = Vec::with_capacity(parents.len());
        for parent in parents {
            made.push(self.script_from_file(path, parent)?);
        }
        let scene = self.scene();
        scene.selection.clear();
        scene.selection.extend(made.iter().copied());
        Ok(made)
    }

    /// Renames the file or folder at `path` to `name`, keeping a file's extension (its type) when `name` has none
    /// of its own. Returns the new path.
    pub fn rename_asset(&mut self, path: &str, name: &str) -> Result<String, String> {
        let name = name.trim();
        valid_name(name)?;
        let is_folder = !self.project.contains(path);
        let name = if is_folder {
            name.to_owned()
        } else {
            let (_, extension) = split_name(file_name(path));
            let (_, typed) = split_name(name);
            if typed.eq_ignore_ascii_case(extension) {
                name.to_owned()
            } else {
                format!("{name}{extension}")
            }
        };
        let to = join(parent_of(path), &name);
        if to == path {
            return Ok(to);
        }
        self.move_one(path, &to)?;
        Ok(to)
    }

    /// Moves the files and folders at `paths` into `folder`. Refuses a folder moved into itself; nothing is moved
    /// once one fails, but those before it.
    pub fn move_assets(&mut self, paths: &[String], folder: &str) -> Result<(), String> {
        for path in paths {
            if parent_of(path) == folder {
                continue;
            }
            if folder == path || folder.starts_with(&format!("{path}/")) {
                return Err(format!("{} cannot go into itself.", file_name(path)));
            }
            let to = join(folder, file_name(path));
            self.move_one(path, &to)?;
        }
        Ok(())
    }

    /// Moves one file or folder from `from` to `to`, and makes what the editor holds follow it.
    fn move_one(&mut self, from: &str, to: &str) -> Result<(), String> {
        let report = if self.project.contains(from) {
            self.project.rename_member(from, to)
        } else {
            self.project.rename_folder(from, to)
        }
        .map_err(|error| error.to_string())?;
        self.follow_moves(&report.moved);
        self.browser.follow(&[(from.to_owned(), to.to_owned())]);
        self.assets = asset_rows(&self.project);
        Ok(())
    }

    /// Makes the scene being edited and the script editor follow files that moved: a script's `Source` that named
    /// one names its new path.
    fn follow_moves(&mut self, moved: &[MemberMove]) {
        if moved.is_empty() {
            return;
        }
        let ids: Vec<Id> = match self.dm.root().and_then(|root| self.dm.descendants(root).ok()) {
            Some(all) => all.collect(),
            None => Vec::new(),
        };
        for id in ids {
            for property in PATH_PROPERTIES {
                let Ok(Value::String(path)) = self.dm.get_by_name(id, property) else {
                    continue;
                };
                if let Some(to) = moved.iter().find(|m| *m.from == *path).map(|m| m.to.clone()) {
                    let _ = self.dm.set_by_name(id, property, Value::String(to.into()));
                }
            }
        }
        for m in moved {
            self.scripts.moved(&m.from, &m.to);
        }
    }

    /// Duplicates the files and folders at `paths` beside themselves (`Rock 2.png`), each copy a new asset, and
    /// returns the copies' paths.
    pub fn duplicate_assets(&mut self, paths: &[String]) -> Result<Vec<String>, String> {
        let mut copies = Vec::new();
        for path in paths {
            let folder = parent_of(path);
            let copy = join(folder, &unique_name(&self.project, folder, file_name(path)));
            if self.project.contains(path) {
                let bytes = self.project.read(path).map_err(|error| error.to_string())?;
                self.project
                    .create_member(&copy, bytes)
                    .map_err(|error| error.to_string())?;
            } else {
                self.project.create_folder(&copy).map_err(|error| error.to_string())?;
                let prefix = format!("{path}/");
                let inside: Vec<String> = self
                    .project
                    .members()
                    .into_iter()
                    .filter(|member| member.starts_with(&prefix))
                    .map(str::to_owned)
                    .collect();
                for member in inside {
                    let bytes = self.project.read(&member).map_err(|error| error.to_string())?;
                    let to = format!("{copy}/{}", &member[prefix.len()..]);
                    self.project
                        .create_member(&to, bytes)
                        .map_err(|error| error.to_string())?;
                }
                for folder in self.project.folders() {
                    if let Some(rest) = folder.strip_prefix(&prefix) {
                        let to = format!("{copy}/{rest}");
                        if !self.project.has_folder(&to) {
                            self.project.create_folder(&to).map_err(|error| error.to_string())?;
                        }
                    }
                }
            }
            copies.push(copy);
        }
        self.assets = asset_rows(&self.project);
        Ok(copies)
    }

    /// What deleting `paths` would delete and what refers to it, for Delete's confirmation. Refuses the scene being
    /// edited, and a folder holding it.
    pub fn plan_delete(&mut self, paths: &[String]) -> Result<Deleting, String> {
        let editing = self.project.manifest().start_scene.clone();
        let mut members = Vec::new();
        for path in paths {
            if self.project.contains(path) {
                members.push(path.clone());
            } else {
                let prefix = format!("{path}/");
                members.extend(
                    self.project
                        .members()
                        .into_iter()
                        .filter(|member| member.starts_with(&prefix))
                        .map(str::to_owned),
                );
            }
        }
        if members.contains(&editing) {
            return Err(format!("{editing} is the scene being edited, so it cannot be deleted."));
        }
        let mut references = Vec::new();
        for member in &members {
            let found = self.project.references(member).map_err(|error| error.to_string())?;
            for reference in found.scenes {
                references.push(format!("{}: {}", reference.scene, reference.property));
            }
        }
        references.sort();
        references.dedup();
        Ok(Deleting {
            paths: paths.to_vec(),
            files: members.len(),
            references,
        })
    }

    /// Deletes the files and folders at `paths` (until the project is saved, they are only gone from memory).
    pub fn delete_assets(&mut self, paths: &[String]) -> Result<(), String> {
        for path in paths {
            if self.project.contains(path) {
                self.project.delete_member(path).map_err(|error| error.to_string())?;
            } else if self.project.has_folder(path) {
                self.project.delete_folder(path).map_err(|error| error.to_string())?;
            }
        }
        self.browser.forget(paths);
        self.assets = asset_rows(&self.project);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_split_before_their_type_s_extension() {
        assert_eq!(split_name("Door.server.luau"), ("Door", ".server.luau"));
        assert_eq!(split_name("Door.Client.luau"), ("Door", ".Client.luau"));
        assert_eq!(split_name("Util.luau"), ("Util", ".luau"));
        assert_eq!(split_name("bark.tex.png"), ("bark.tex", ".png"));
        assert_eq!(split_name("README"), ("README", ""));
        assert_eq!(split_name(".hidden"), (".hidden", ""));
        assert_eq!(split_name(".server.luau"), (".server", ".luau"));
    }

    #[test]
    fn paths_join_and_split() {
        assert_eq!(join("", "Assets"), "Assets");
        assert_eq!(join("Assets", "Trees"), "Assets/Trees");
        assert_eq!(parent_of("Assets/Trees/oak.png"), "Assets/Trees");
        assert_eq!(parent_of("Assets"), "");
        assert_eq!(file_name("Assets/Trees/oak.png"), "oak.png");
    }

    #[test]
    fn back_returns_to_the_folders_opened_before() {
        let mut browser = AssetBrowser::default();
        browser.open_folder("Assets");
        browser.open_folder("Assets/Trees");
        assert!(browser.expanded.contains("Assets") && browser.expanded.contains("Assets/Trees"));
        assert!(browser.go_back());
        assert_eq!(browser.open, "Assets");
        assert!(browser.go_back());
        assert_eq!(browser.open, "");
        assert!(!browser.go_back());
    }

    #[test]
    fn clicks_select_one_add_and_range() {
        let entry = |path: &str| Entry {
            path: path.into(),
            kind: EntryKind::Folder,
            id: None,
        };
        let shown = [entry("a"), entry("b"), entry("c"), entry("d")];
        let mut browser = AssetBrowser::default();
        browser.click(&shown, "b", false, false);
        browser.click(&shown, "d", false, true);
        assert_eq!(browser.selected.iter().collect::<Vec<_>>(), ["b", "c", "d"]);
        browser.click(&shown, "c", true, false);
        assert_eq!(browser.selected.iter().collect::<Vec<_>>(), ["b", "d"]);
        browser.click(&shown, "a", false, false);
        assert_eq!(browser.selected.iter().collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn paths_follow_a_move_and_a_delete() {
        let mut browser = AssetBrowser {
            open: "Assets/Trees/Pines".into(),
            ..Default::default()
        };
        browser.selected.insert("Assets/Trees/oak.png".into());
        browser.follow(&[("Assets/Trees".into(), "Forest".into())]);
        assert_eq!(browser.open, "Forest/Pines");
        assert!(browser.selected.contains("Forest/oak.png"));
        browser.forget(&["Forest".into()]);
        assert_eq!(browser.open, "");
        assert!(browser.selected.is_empty());
    }
}
