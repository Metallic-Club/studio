//! Game templates: listing them from disk and creating a new project from one.
//!
//! A template is a folder with a `template.toml` (`id`, `name`, `description`, `preview`, `order`,
//! and for the start screen the optional `kind`, `facts`, `inside` and `look`) next to the project
//! files. Project settings metadata lives in `metallic.toml` as `[[setting]]`
//! and survives creation/renaming. Catalog metadata, previews and root test fixtures stay in
//! the template. Legacy settings in `template.toml` are migrated into the project file.
//!
//! The Baseplate template is also compiled into Studio ([`builtin_baseplate`]), so a game can be
//! created even when the templates folder is missing.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::StudioError;

/// Name of the per-template metadata file (never copied into projects).
pub const TEMPLATE_FILE: &str = "template.toml";
/// Name of the project file that holds the project name and persistent settings metadata.
pub const PROJECT_FILE: &str = "metallic.toml";

/// One template card on the start screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateInfo {
    /// Stable identifier, e.g. `parkour`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// One or two sentences for the card.
    pub description: String,
    /// Path of the preview image (may not exist; the UI shows a placeholder). Empty for a
    /// compiled-in template.
    pub preview_path: PathBuf,
    /// The template folder. Empty for a compiled-in template.
    pub dir: PathBuf,
    /// Sort key: lower comes first.
    pub order: i64,
    /// The template's files when they are compiled into Studio instead of read from `dir`.
    pub embedded: Option<&'static [EmbeddedFile]>,
    /// What kind of game it starts (the start screen's filter and art chip).
    pub kind: TemplateKind,
    /// A short line under the name, e.g. "3D, third-person" (empty when the catalog has none).
    pub facts: String,
    /// What the template contains, for the "Inside:" line (empty when the catalog has none).
    pub inside: String,
    /// The lighting look it ships with, for the art chip (empty when the catalog has none).
    pub look: String,
}

/// The kind of game a template starts, as written in `template.toml` (`kind = "3d"`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum TemplateKind {
    /// A 3D game (the default).
    #[default]
    #[serde(rename = "3d")]
    ThreeD,
    /// A 2D game.
    #[serde(rename = "2d")]
    TwoD,
    /// An empty world to build in.
    #[serde(rename = "empty")]
    Empty,
}

impl TemplateKind {
    /// "3D" or "2D" for the art chip (an empty world is 3D).
    pub fn dimension(self) -> &'static str {
        match self {
            Self::TwoD => "2D",
            Self::ThreeD | Self::Empty => "3D",
        }
    }
}

/// One file of a template that is compiled into Studio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddedFile {
    /// Path relative to the template folder, with `/` separators.
    pub path: &'static str,
    /// The file's contents.
    pub bytes: &'static [u8],
}

/// Id of the Baseplate template, which Studio always offers.
pub const BASEPLATE_ID: &str = "baseplate";

/// `templates/baseplate`, compiled in. A test keeps it byte-identical to the folder.
const BASEPLATE_FILES: &[EmbeddedFile] = &[
    EmbeddedFile {
        path: TEMPLATE_FILE,
        bytes: include_bytes!("../../../templates/baseplate/template.toml"),
    },
    EmbeddedFile {
        path: PROJECT_FILE,
        bytes: include_bytes!("../../../templates/baseplate/metallic.toml"),
    },
    EmbeddedFile {
        path: "main.mscene",
        bytes: include_bytes!("../../../templates/baseplate/main.mscene"),
    },
];

#[derive(Deserialize)]
struct TemplateFile {
    id: String,
    name: String,
    description: String,
    preview: String,
    order: i64,
    #[serde(default)]
    kind: TemplateKind,
    #[serde(default)]
    facts: String,
    #[serde(default)]
    inside: String,
    #[serde(default)]
    look: String,
}

impl TemplateFile {
    /// The catalog entry for a template whose files are at `dir` (or compiled in).
    fn into_info(
        self,
        preview_path: PathBuf,
        dir: PathBuf,
        embedded: Option<&'static [EmbeddedFile]>,
    ) -> TemplateInfo {
        TemplateInfo {
            id: self.id,
            name: self.name,
            description: self.description,
            preview_path,
            dir,
            order: self.order,
            embedded,
            kind: self.kind,
            facts: self.facts,
            inside: self.inside,
            look: self.look,
        }
    }
}

/// The folder templates are read from: env `METALLIC_TEMPLATES_DIR`, else `<exe dir>/templates`.
pub fn templates_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("METALLIC_TEMPLATES_DIR") {
        return PathBuf::from(dir);
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    exe_dir.unwrap_or_default().join("templates")
}

/// Reads one template folder; `Err` carries a short reason for the log.
fn read_template(dir: &Path) -> Result<TemplateInfo, String> {
    let file = dir.join(TEMPLATE_FILE);
    let text =
        fs::read_to_string(&file).map_err(|e| format!("cannot read {TEMPLATE_FILE}: {e}"))?;
    let parsed = parse_template_file(&text)?;
    let preview = dir.join(&parsed.preview);
    Ok(parsed.into_info(preview, dir.to_path_buf(), None))
}

/// Parses and checks a `template.toml`; `Err` carries a short reason for the log.
fn parse_template_file(text: &str) -> Result<TemplateFile, String> {
    let parsed: TemplateFile = toml::from_str(text).map_err(|e| e.message().to_owned())?;
    if parsed.id.trim().is_empty() || parsed.name.trim().is_empty() {
        return Err("`id` and `name` must not be empty".to_owned());
    }
    Ok(parsed)
}

/// The compiled-in `template.toml` of `files` as text, if there is one.
fn embedded_catalog(files: &[EmbeddedFile]) -> Option<Result<&str, std::str::Utf8Error>> {
    files
        .iter()
        .find(|file| file.path == TEMPLATE_FILE)
        .map(|file| std::str::from_utf8(file.bytes))
}

/// The Baseplate template compiled into Studio: a grey floor and a spawn point. `None` (with a
/// warning in the log) only if its compiled-in `template.toml` is broken, which a test rules out.
pub fn builtin_baseplate() -> Option<TemplateInfo> {
    let parsed = match embedded_catalog(BASEPLATE_FILES) {
        Some(Ok(text)) => parse_template_file(text),
        Some(Err(e)) => Err(e.to_string()),
        None => Err(format!("{TEMPLATE_FILE} is missing")),
    };
    match parsed {
        Ok(parsed) => Some(parsed.into_info(PathBuf::new(), PathBuf::new(), Some(BASEPLATE_FILES))),
        Err(reason) => {
            log::warn!("built-in Baseplate template is broken: {reason}");
            None
        }
    }
}

/// Adds the compiled-in Baseplate to `templates` unless one with its id is already there, so
/// Studio can always create a game. Keeps the `order` sorting of [`list_templates`].
pub fn with_builtin(mut templates: Vec<TemplateInfo>) -> Vec<TemplateInfo> {
    if !templates.iter().any(|t| t.id == BASEPLATE_ID)
        && let Some(baseplate) = builtin_baseplate()
    {
        templates.push(baseplate);
        sort_templates(&mut templates);
    }
    templates
}

fn sort_templates(templates: &mut [TemplateInfo]) {
    templates.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.name.cmp(&b.name)));
}

/// Lists the templates inside `dir`, sorted by `order` (then name). Folders with a missing or
/// invalid `template.toml` are skipped with a warning in the log; a missing `dir` gives an empty list.
pub fn list_templates(dir: &Path) -> Vec<TemplateInfo> {
    let Ok(entries) = fs::read_dir(dir) else {
        log::warn!("templates folder {} cannot be read", dir.display());
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        match read_template(&path) {
            Ok(info) => found.push(info),
            Err(reason) => log::warn!("skipping template folder {}: {reason}", path.display()),
        }
    }
    sort_templates(&mut found);
    found
}

/// Rejects names that cannot be a folder name on Windows (and are unhelpful elsewhere).
pub fn validate_project_name(name: &str) -> Result<(), StudioError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(StudioError::InvalidName("Give your game a name.".into()));
    }
    if trimmed.ends_with('.')
        || trimmed
            .chars()
            .any(|c| "\\/:*?\"<>|".contains(c) || c.is_control())
    {
        return Err(StudioError::InvalidName(
            "Names can't contain \\ / : * ? \" < > | or end with a dot.".into(),
        ));
    }
    let stem = trimmed.split('.').next().unwrap_or("").trim_end();
    if is_reserved_windows_name(stem) {
        return Err(StudioError::InvalidName(format!(
            "\"{stem}\" is a reserved name on Windows. Pick a different name."
        )));
    }
    Ok(())
}

/// Windows device names (CON, PRN, AUX, NUL, COM1-9, LPT1-9), which cannot be folder names,
/// with or without an extension.
fn is_reserved_windows_name(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            upper
                .strip_prefix(prefix)
                .is_some_and(|n| matches!(n.as_bytes(), [b'1'..=b'9']))
        })
}

/// True when `dir` does not exist or has no entries.
fn is_empty_or_missing(dir: &Path) -> Result<bool, StudioError> {
    match fs::read_dir(dir) {
        Ok(mut entries) => Ok(entries.next().is_none()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(StudioError::io(dir, e)),
    }
}

struct CreationMetadata {
    excluded_files: Vec<PathBuf>,
    legacy_settings: Option<toml::Value>,
}

/// Normalize a template-relative asset path without allowing an absolute path or root escape.
fn relative_asset(path: &Path) -> Option<PathBuf> {
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => relative.push(name),
            Component::CurDir => {}
            Component::ParentDir => {
                if !relative.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!relative.as_os_str().is_empty()).then_some(relative)
}

fn same_relative_path(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.to_str()
            .zip(right.to_str())
            .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn creation_metadata(template: &TemplateInfo) -> Result<CreationMetadata, StudioError> {
    let path = template.dir.join(TEMPLATE_FILE);
    let text = match template.embedded {
        Some(files) => match embedded_catalog(files) {
            Some(Ok(text)) => Some(text.to_owned()),
            Some(Err(error)) => {
                return Err(StudioError::Metadata {
                    path,
                    message: error.to_string(),
                });
            }
            None => None,
        },
        None => match fs::read_to_string(&path) {
            Ok(text) => Some(text),
            // TemplateInfo can also be constructed directly, without a catalog file.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(StudioError::io(&path, error)),
        },
    };
    let table = match text {
        Some(text) => text
            .parse::<toml::Table>()
            .map_err(|error| StudioError::Metadata {
                path: path.clone(),
                message: error.message().to_owned(),
            })?,
        None => toml::Table::new(),
    };
    let mut excluded_files = [TEMPLATE_FILE, "preview.png", "preview.webm"]
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if let Some(preview) = template
        .preview_path
        .strip_prefix(&template.dir)
        .ok()
        .and_then(relative_asset)
    {
        excluded_files.push(preview);
    }
    for field in ["preview", "preview_video"] {
        if let Some(path) = table
            .get(field)
            .and_then(toml::Value::as_str)
            .and_then(|path| relative_asset(Path::new(path)))
        {
            excluded_files.push(path);
        }
    }
    Ok(CreationMetadata {
        excluded_files,
        legacy_settings: table.get("setting").cloned(),
    })
}

/// True for template entries that stay out of a new project: the root `tests/` directory and
/// the catalog files in `excluded_files` (never `metallic.toml`).
fn is_excluded(relative_path: &Path, is_dir: bool, excluded_files: &[PathBuf]) -> bool {
    if is_dir {
        return same_relative_path(relative_path, Path::new("tests"));
    }
    !same_relative_path(relative_path, Path::new(PROJECT_FILE))
        && excluded_files
            .iter()
            .any(|path| same_relative_path(path, relative_path))
}

/// Writes compiled-in template files into `to`, with the same exclusions as [`copy_tree`].
fn write_embedded(
    files: &[EmbeddedFile],
    to: &Path,
    excluded_files: &[PathBuf],
) -> Result<(), StudioError> {
    fs::create_dir_all(to).map_err(|e| StudioError::io(to, e))?;
    for file in files {
        let Some(relative_path) = relative_asset(Path::new(file.path)) else {
            log::warn!("skipping compiled-in template file {}", file.path);
            continue;
        };
        let in_tests = relative_path
            .components()
            .next()
            .is_some_and(|first| is_excluded(Path::new(first.as_os_str()), true, excluded_files));
        if in_tests || is_excluded(&relative_path, false, excluded_files) {
            continue;
        }
        let dst = to.join(&relative_path);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| StudioError::io(parent, e))?;
        }
        fs::write(&dst, file.bytes).map_err(|e| StudioError::io(&dst, e))?;
    }
    Ok(())
}

/// Copies project content while excluding catalog files and the root template test directory.
fn copy_tree(
    from: &Path,
    to: &Path,
    relative: &Path,
    excluded_files: &[PathBuf],
) -> Result<(), StudioError> {
    fs::create_dir_all(to).map_err(|e| StudioError::io(to, e))?;
    for entry in fs::read_dir(from).map_err(|e| StudioError::io(from, e))? {
        let entry = entry.map_err(|e| StudioError::io(from, e))?;
        let src = entry.path();
        let relative_path = relative.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| StudioError::io(&src, e))?;
        if kind.is_symlink() {
            log::warn!(
                "skipping symlink {} while copying a template",
                src.display()
            );
            continue;
        }
        if (kind.is_dir() || kind.is_file())
            && is_excluded(&relative_path, kind.is_dir(), excluded_files)
        {
            continue;
        }
        let dst = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&src, &dst, &relative_path, excluded_files)?;
        } else {
            fs::copy(&src, &dst).map_err(|e| StudioError::io(&src, e))?;
        }
    }
    Ok(())
}

/// Writes `name` into `<target>/metallic.toml`, keeping any other keys; creates the file if missing.
fn set_project_name(
    target: &Path,
    name: &str,
    legacy_settings: Option<&toml::Value>,
) -> Result<(), StudioError> {
    let file = target.join(PROJECT_FILE);
    let mut table = match fs::read_to_string(&file) {
        Ok(text) => text
            .parse::<toml::Table>()
            .map_err(|e| StudioError::Metadata {
                path: file.clone(),
                message: e.message().to_owned(),
            })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml::Table::new(),
        Err(e) => return Err(StudioError::io(&file, e)),
    };
    if let Some(legacy) = legacy_settings {
        match table.get("setting") {
            None => { table.insert("setting".to_owned(), legacy.clone()); }
            Some(settings) if settings == legacy => {}
            Some(_) => return Err(StudioError::Metadata {
                path: file,
                message: "The template defines different settings in both template.toml and metallic.toml. Keep one [[setting]] definition in the template's metallic.toml.".to_owned(),
            }),
        }
    }
    table.insert("name".to_owned(), toml::Value::String(name.to_owned()));
    let text = toml::to_string(&table).map_err(|e| StudioError::Metadata {
        path: file.clone(),
        message: e.to_string(),
    })?;
    fs::write(&file, text).map_err(|e| StudioError::io(&file, e))
}

/// Removes everything inside `dir` but keeps `dir` itself.
fn clear_dir(dir: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// Creates a new project at `target` from `template`, named `project_name`.
///
/// `target` must not exist or be empty; otherwise [`StudioError::FolderNotEmpty`] is returned and
/// nothing is written. If copying fails half-way, the files this call created are removed again.
/// Root `template.toml`, `preview.png`, `preview.webm`, configured preview files and the root
/// `tests/` directory are excluded. Tutorials and nested project assets are kept. Legacy
/// `template.toml` settings migrate to `metallic.toml`; conflicting dual definitions fail
/// with [`StudioError::Metadata`] and the same rollback as a copy failure.
/// A directly constructed [`TemplateInfo`] does not require a catalog file; its relative preview is still excluded.
/// A compiled-in template ([`TemplateInfo::embedded`]) is written from its files by the same rules.
pub fn create_from_template(
    template: &TemplateInfo,
    target: &Path,
    project_name: &str,
) -> Result<(), StudioError> {
    validate_project_name(project_name)?;
    if !is_empty_or_missing(target)? {
        return Err(StudioError::FolderNotEmpty(target.to_path_buf()));
    }
    let metadata = creation_metadata(template)?;
    let existed = target.exists();
    let written = match template.embedded {
        Some(files) => write_embedded(files, target, &metadata.excluded_files),
        None => copy_tree(
            &template.dir,
            target,
            Path::new(""),
            &metadata.excluded_files,
        ),
    };
    let result = written.and_then(|()| {
        set_project_name(
            target,
            project_name.trim(),
            metadata.legacy_settings.as_ref(),
        )
    });
    if result.is_err() {
        // Best-effort cleanup: the folder was empty or new before, so everything in it is ours.
        let _ = if existed {
            clear_dir(target)
        } else {
            fs::remove_dir_all(target)
        };
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_screen_fields_are_optional_and_parsed() {
        let base =
            "id = \"a\"\nname = \"A\"\ndescription = \"d\"\npreview = \"p.png\"\norder = 1\n";
        let plain = parse_template_file(base).expect("old catalogs still parse");
        assert_eq!(plain.kind, TemplateKind::ThreeD);
        assert!(plain.facts.is_empty() && plain.inside.is_empty() && plain.look.is_empty());
        let full =
            format!("{base}kind = \"2d\"\nfacts = \"2D, side view\"\nlook = \"Paper look\"\n");
        let parsed = parse_template_file(&full).expect("parse");
        assert_eq!(parsed.kind, TemplateKind::TwoD);
        assert_eq!(parsed.kind.dimension(), "2D");
        assert_eq!(
            (parsed.facts.as_str(), parsed.look.as_str()),
            ("2D, side view", "Paper look")
        );
        assert!(parse_template_file(&format!("{base}kind = \"4d\"\n")).is_err());
    }

    #[test]
    fn the_built_in_baseplate_is_an_empty_world() {
        let baseplate = builtin_baseplate().expect("compiled-in catalog parses");
        assert_eq!(baseplate.kind, TemplateKind::Empty);
        assert!(!baseplate.inside.is_empty());
    }
}
