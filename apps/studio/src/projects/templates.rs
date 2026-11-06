//! The templates a new project starts from: the folders of `templates/` (`templates/baseplate-3d` in the repository),
//! each holding a `metallic.toml` (#187's [`Template`](metallic_engine::metallic_project::Template)).

use std::path::{Path, PathBuf};

use metallic_engine::metallic_project::Manifest;
use metallic_engine::metallic_project::manifest::MANIFEST_FILE;

/// The templates folder's name.
pub const TEMPLATES_DIR: &str = "templates";

/// A template, as the projects page lists it. Its files are read only when a project is made from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateInfo {
    /// Its folder's name (`baseplate-3d`).
    pub id: String,
    /// The name its manifest gives (`Baseplate`).
    pub name: String,
    /// Its kind, as its manifest says (`3d`).
    pub kind: String,
    /// Its folder.
    pub dir: PathBuf,
}

/// Where Studio's templates are: `templates` beside the executable or in a folder above it (a build in the
/// repository's `target/`), else the repository's own.
pub fn templates_dir() -> Option<PathBuf> {
    let beside_exe = std::env::current_exe().ok().and_then(|exe| {
        exe.ancestors()
            .skip(1)
            .map(|dir| dir.join(TEMPLATES_DIR))
            .find(|dir| dir.is_dir())
    });
    beside_exe.or_else(|| {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(TEMPLATES_DIR);
        repository.is_dir().then_some(repository)
    })
}

/// The templates in `dir`, sorted by id. A folder without a readable `metallic.toml` is skipped and logged.
pub fn list_templates(dir: &Path) -> Vec<TemplateInfo> {
    let folders = match std::fs::read_dir(dir) {
        Ok(folders) => folders,
        Err(error) => {
            log::warn!("templates: could not read {}: {error}", dir.display());
            return Vec::new();
        }
    };
    let mut templates: Vec<TemplateInfo> = folders
        .filter_map(Result::ok)
        .map(|folder| folder.path())
        .filter(|folder| folder.is_dir())
        .filter_map(|folder| {
            let bytes = std::fs::read(folder.join(MANIFEST_FILE)).ok()?;
            match Manifest::from_bytes(&bytes) {
                Ok(loaded) => Some(TemplateInfo {
                    id: folder.file_name()?.to_string_lossy().into_owned(),
                    name: loaded.manifest.name,
                    kind: loaded.manifest.kind.as_str().to_owned(),
                    dir: folder,
                }),
                Err(error) => {
                    log::warn!("templates: {} is skipped: {error}", folder.display());
                    None
                }
            }
        })
        .collect();
    templates.sort_by(|a, b| a.id.cmp(&b.id));
    templates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_repositorys_templates_are_found_and_listed() {
        let dir = templates_dir().expect("the repository's templates");
        let templates = list_templates(&dir);
        let baseplate = templates
            .iter()
            .find(|template| template.id == "baseplate-3d")
            .expect("baseplate-3d");
        assert_eq!(baseplate.name, "Baseplate");
        assert_eq!(baseplate.kind, "3d");
        assert!(list_templates(&dir.join("no such folder")).is_empty());
    }
}
