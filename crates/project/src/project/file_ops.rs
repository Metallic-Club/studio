//! File operations on a project's members: create, rename or move, and delete, keeping asset ids and the paths that
//! name members correct. Like every edit they stay in memory until [`Project::save`], which writes them all at once,
//! crash-safely. A moved member is read into memory and written at its new path by that save.
//!
//! # References
//!
//! - **By asset id** (`MeshPart.MeshId`, `TextureId`): a rename or move keeps the asset's id in the index and only
//!   changes its path there, so these references need no rewrite.
//! - **By member path**: a script's `Source` ([`PATH_PROPERTIES`]) in every scene, and `metallic.toml`'s
//!   `start_scene`. These are rewritten to the new path. A scene is rewritten in its text, only where a path changes,
//!   so the rest of the file stays byte for byte as it was.
//!
//! A scene that does not read (not UTF-8, not JSON, or a newer format) is left alone and listed in the report as
//! unreadable, so a single broken scene never blocks a rename. Paths inside script code (`require`) are not
//! rewritten.
//!
//! # Delete
//!
//! Deleting is never refused because something refers to the member: [`Project::delete_member`] deletes it and
//! returns what referred to it ([`References`]), for the caller to show; [`Project::references`] gives the same report
//! before deleting. A referenced asset stays in the index as missing, so its scenes still name the path they lost and
//! the id comes back if the member does; an asset nothing refers to is dropped from the index.

use std::collections::BTreeMap;

use super::{Project, ProjectError};
use crate::asset_index::{AssetId, Fingerprint, INDEX_FILE};
use crate::manifest::MANIFEST_FILE;
use crate::scene::{PATH_PROPERTIES, SCENE_EXTENSION, SceneReference, find_references, rewrite_paths};

/// One member a rename or move changed the path of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberMove {
    /// The path it had.
    pub from: String,
    /// The path it has now.
    pub to: String,
    /// Its asset id, which it keeps (`None` for a member not indexed yet).
    pub id: Option<AssetId>,
}

/// What [`Project::rename_member`] or [`Project::rename_folder`] changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MoveReport {
    /// Every member moved, sorted by its old path.
    pub moved: Vec<MemberMove>,
    /// The scenes whose path references were rewritten, at their paths after the move, sorted.
    pub scenes: Vec<String>,
    /// Whether `metallic.toml`'s start scene was one of the moved members, and now names its new path.
    pub start_scene: bool,
    /// Scenes that could not be read, so any path in them was not rewritten, at their paths after the move.
    pub unreadable: Vec<String>,
}

/// What refers to a member ([`Project::references`], [`Project::delete_member`]): data for the caller to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct References {
    /// The member.
    pub member: String,
    /// Its asset id, if it is indexed.
    pub id: Option<AssetId>,
    /// Whether it is `metallic.toml`'s start scene.
    pub start_scene: bool,
    /// The scene properties that refer to it by id or by path, by scene path then in file order.
    pub scenes: Vec<SceneReference>,
    /// Scenes that could not be read, so they may refer to it unseen.
    pub unreadable: Vec<String>,
}

impl References {
    /// Whether nothing refers to the member (as far as the readable scenes tell).
    pub fn is_empty(&self) -> bool {
        !self.start_scene && self.scenes.is_empty()
    }
}

/// Whether `member` is a scene, by its extension.
fn is_scene(member: &str) -> bool {
    member
        .rsplit_once('.')
        .is_some_and(|(_, extension)| extension == SCENE_EXTENSION)
}

impl Project {
    /// Adds a new member with `bytes` and gives it an asset id, which it returns (in memory until
    /// [`Project::save`]). A missing asset indexed at the same path gets its id back. Refuses an unsafe path, the
    /// project's metadata (`metallic.toml`, `assets.json`) and a path that is already a member.
    pub fn create_member(&mut self, member: &str, bytes: impl Into<Vec<u8>>) -> Result<AssetId, ProjectError> {
        self.check_new(member)?;
        let bytes = bytes.into();
        let id = match self.assets.id(member) {
            Some(id) => id,
            None => AssetId::random().map_err(|source| self.index_error(source))?,
        };
        let mut assets = self.assets.clone();
        assets
            .insert(member, id, Fingerprint::of(&bytes))
            .map_err(|source| self.index_error(source))?;
        self.edits.insert(member.to_owned(), Some(bytes));
        self.set_assets(assets);
        Ok(id)
    }

    /// Renames or moves the member `from` to `to` (in a zip archive they are the same thing). It keeps its asset id,
    /// and every path that named it is rewritten (see the [module docs](self)). Refuses an unsafe `to`, the project's
    /// metadata, a `from` that is not a member and a `to` that is. On an error nothing changed.
    pub fn rename_member(&mut self, from: &str, to: &str) -> Result<MoveReport, ProjectError> {
        self.check_metadata(from)?;
        if !self.contains(from) {
            return Err(self.missing(from));
        }
        self.check_new(to)?;
        self.move_members(vec![(from.to_owned(), to.to_owned())])
    }

    /// Renames or moves the folder `from` to `to`: every member whose path starts with `from/` moves under `to/`,
    /// keeping its asset id, and every path that named one is rewritten. A folder is only a path prefix, so one with
    /// no members is not there. Refuses unsafe paths, a folder moved into itself, and any member, or `to` itself,
    /// already being there. On an error nothing changed.
    pub fn rename_folder(&mut self, from: &str, to: &str) -> Result<MoveReport, ProjectError> {
        self.check(from)?;
        self.check(to)?;
        if to == from || to.starts_with(&format!("{from}/")) {
            return Err(ProjectError::FolderIntoItself {
                path: self.path.clone(),
                from: from.to_owned(),
                to: to.to_owned(),
            });
        }
        let prefix = format!("{from}/");
        let moves: Vec<(String, String)> = self
            .members()
            .into_iter()
            .filter_map(|member| Some((member.to_owned(), format!("{to}/{}", member.strip_prefix(&prefix)?))))
            .collect();
        if moves.is_empty() {
            return Err(self.missing(from));
        }
        if self.contains(to) {
            return Err(self.exists(to));
        }
        for (_, destination) in &moves {
            self.check_new(destination)?;
        }
        self.move_members(moves)
    }

    /// Deletes `member` (in memory until [`Project::save`]) and returns what referred to it. Deleting is not refused
    /// when something does: the caller shows the report. A referenced asset stays in the index as missing; one that
    /// nothing refers to is dropped from it. Refuses the project's metadata and a member that is not there.
    pub fn delete_member(&mut self, member: &str) -> Result<References, ProjectError> {
        self.check_metadata(member)?;
        if !self.contains(member) {
            return Err(self.missing(member));
        }
        let references = self.references(member)?;
        self.remove(member);
        if let Some(id) = references.id.filter(|_| references.is_empty()) {
            self.forget_asset(id);
        }
        Ok(references)
    }

    /// What refers to `member`: `metallic.toml`'s start scene, and every scene property that holds its asset id or
    /// (for [`PATH_PROPERTIES`]) its path. Reads every scene.
    pub fn references(&mut self, member: &str) -> Result<References, ProjectError> {
        let id = self.assets.id(member);
        let id_text = id.map(|id| id.to_string());
        let mut references = References {
            member: member.to_owned(),
            id,
            start_scene: self.manifest.start_scene == member,
            scenes: Vec::new(),
            unreadable: Vec::new(),
        };
        let scenes: Vec<String> = self
            .members()
            .into_iter()
            .filter(|scene| is_scene(scene) && *scene != member)
            .map(str::to_owned)
            .collect();
        for scene in scenes {
            let bytes = self.read(&scene)?;
            let refers = |property: &str, value: &str| {
                id_text.as_deref() == Some(value) || (PATH_PROPERTIES.contains(&property) && value == member)
            };
            match std::str::from_utf8(&bytes)
                .ok()
                .and_then(|text| find_references(&scene, text, refers).ok())
            {
                Some(found) => references.scenes.extend(found),
                None => references.unreadable.push(scene),
            }
        }
        Ok(references)
    }

    /// Moves each `(from, to)` member, checked by the caller, and rewrites the paths that named them. Everything that
    /// can fail (reading members and scenes) happens before anything changes.
    fn move_members(&mut self, mut moves: Vec<(String, String)>) -> Result<MoveReport, ProjectError> {
        moves.sort();
        let remap: BTreeMap<String, String> = moves.iter().cloned().collect();
        let new_path = |member: &str| remap.get(member).cloned();

        let mut writes: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for (from, to) in &moves {
            writes.insert(to.clone(), self.read(from)?);
        }

        let mut report = MoveReport::default();
        let scenes: Vec<String> = self
            .members()
            .into_iter()
            .filter(|member| is_scene(member))
            .map(str::to_owned)
            .collect();
        for scene in scenes {
            let at = new_path(&scene).unwrap_or_else(|| scene.clone());
            let bytes = match writes.get(&at) {
                Some(bytes) => bytes.clone(),
                None => self.read(&scene)?,
            };
            let rewritten = std::str::from_utf8(&bytes)
                .ok()
                .and_then(|text| rewrite_paths(text, new_path).ok());
            match rewritten {
                Some(Some(text)) => {
                    writes.insert(at.clone(), text.into_bytes());
                    report.scenes.push(at);
                }
                Some(None) => {}
                None => report.unreadable.push(at),
            }
        }
        report.scenes.sort();
        report.unreadable.sort();

        let mut assets = self.assets.clone();
        for (from, to) in &moves {
            report.moved.push(MemberMove {
                from: from.clone(),
                to: to.clone(),
                id: assets.rename(from, to),
            });
        }

        // Nothing below fails but the start scene's path check, which comes first and was done for `to` already.
        if let Some(start_scene) = new_path(&self.manifest.start_scene) {
            let mut manifest = self.manifest.clone();
            manifest.start_scene = start_scene;
            self.set_manifest(manifest)?;
            report.start_scene = true;
        }
        for (from, _) in &moves {
            self.remove(from);
        }
        for (member, bytes) in writes {
            self.edits.insert(member, Some(bytes));
        }
        self.set_assets(assets);
        Ok(report)
    }

    /// Refuses `member` as a new member's path: unsafe, the project's metadata, or already a member.
    fn check_new(&self, member: &str) -> Result<(), ProjectError> {
        self.check(member)?;
        self.check_metadata(member)?;
        if self.contains(member) {
            return Err(self.exists(member));
        }
        Ok(())
    }

    /// Refuses `metallic.toml` and `assets.json`, which only change through the project settings and the index.
    fn check_metadata(&self, member: &str) -> Result<(), ProjectError> {
        let path = self.path.clone();
        match member {
            MANIFEST_FILE => Err(ProjectError::ManifestMember { path }),
            INDEX_FILE => Err(ProjectError::IndexMember { path }),
            _ => Ok(()),
        }
    }

    fn missing(&self, member: &str) -> ProjectError {
        ProjectError::MissingMember {
            path: self.path.clone(),
            member: member.to_owned(),
        }
    }

    fn exists(&self, member: &str) -> ProjectError {
        ProjectError::MemberExists {
            path: self.path.clone(),
            member: member.to_owned(),
        }
    }

    fn index_error(&self, source: crate::asset_index::AssetIndexError) -> ProjectError {
        ProjectError::AssetIndex {
            path: self.path.clone(),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::project::{TEMP_SUFFIX, Template, fail_saves_after};

    /// A new project made from the Baseplate template, in an empty folder for one test.
    fn new_project(test: &str) -> Project {
        let dir = std::env::temp_dir().join(format!("metallic-file-ops-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let template =
            Template::from_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d")).unwrap();
        Project::create(dir.join("Game.mtproj"), &template, "Game").unwrap()
    }

    #[test]
    fn a_file_operation_save_that_fails_part_way_leaves_the_project_unchanged() {
        let mut project = new_project("failed-save");
        let path: PathBuf = project.path().to_path_buf();
        project.create_member("Assets/rock.glb", vec![7; 4096]).unwrap();
        project.save().unwrap();
        let before = fs::read(&path).unwrap();

        project
            .rename_member("Scenes/Main.scene", "Scenes/Level 1.scene")
            .unwrap();
        project.rename_folder("Assets", "Models").unwrap();
        // Past the first members, into the middle of the archive.
        fail_saves_after(Some(before.len() / 2));
        let error = project.save().unwrap_err();
        fail_saves_after(None);
        assert!(matches!(error, ProjectError::Save { .. }), "{error}");
        assert!(error.to_string().contains("injected failure"), "{error}");
        assert_eq!(fs::read(&path).unwrap(), before, "the project's file is as it was");
        let mut temp = path.clone().into_os_string();
        temp.push(TEMP_SUFFIX);
        assert!(!Path::new(&temp).exists(), "the half-written archive is removed");
        assert!(project.is_modified(), "the edits are kept");

        // The project on disk still opens as it was, and the kept edits save once writing works.
        let old = Project::open(&path).unwrap();
        assert_eq!(old.manifest().start_scene, "Scenes/Main.scene");
        assert!(old.contains("Assets/rock.glb"));
        drop(old);
        project.save().unwrap();
        let reopened = Project::open(&path).unwrap();
        assert_eq!(reopened.manifest().start_scene, "Scenes/Level 1.scene");
        assert!(reopened.contains("Models/rock.glb") && !reopened.contains("Assets/rock.glb"));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
