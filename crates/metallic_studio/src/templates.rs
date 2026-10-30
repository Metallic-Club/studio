//! Game templates: listing them from disk and creating a new project from one.
//!
//! A template is a folder with a `template.toml` (`id`, `name`, `description`, `preview`, `order`)
//! next to the project files. Project settings metadata lives in `metallic.toml` as `[[setting]]`
//! and survives creation/renaming. Catalog metadata, previews and root test fixtures stay in
//! the template. Legacy settings in `template.toml` are migrated into the project file.

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
    /// Path of the preview image (may not exist; the UI shows a placeholder).
    pub preview_path: PathBuf,
    /// The template folder.
    pub dir: PathBuf,
    /// Sort key: lower comes first.
    pub order: i64,
}

#[derive(Deserialize)]
struct TemplateFile {
    id: String,
    name: String,
    description: String,
    preview: String,
    order: i64,
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
    let parsed: TemplateFile = toml::from_str(&text).map_err(|e| e.message().to_owned())?;
    if parsed.id.trim().is_empty() || parsed.name.trim().is_empty() {
        return Err("`id` and `name` must not be empty".to_owned());
    }
    Ok(TemplateInfo {
        id: parsed.id,
        name: parsed.name,
        description: parsed.description,
        preview_path: dir.join(parsed.preview),
        dir: dir.to_path_buf(),
        order: parsed.order,
    })
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
    found.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.name.cmp(&b.name)));
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
    let table = match fs::read_to_string(&path) {
        Ok(text) => text
            .parse::<toml::Table>()
            .map_err(|error| StudioError::Metadata {
                path: path.clone(),
                message: error.message().to_owned(),
            })?,
        // TemplateInfo can also be constructed directly, without a catalog file.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => toml::Table::new(),
        Err(error) => return Err(StudioError::io(&path, error)),
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
        if (kind.is_dir() && same_relative_path(&relative_path, Path::new("tests")))
            || (kind.is_file()
                && !same_relative_path(&relative_path, Path::new(PROJECT_FILE))
                && excluded_files
                    .iter()
                    .any(|path| same_relative_path(path, &relative_path)))
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
    let result = copy_tree(
        &template.dir,
        target,
        Path::new(""),
        &metadata.excluded_files,
    )
    .and_then(|()| {
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
