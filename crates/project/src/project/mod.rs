//! `.mtproj` projects: one zip archive holding the manifest, scenes, scripts and assets (ADR 0006).
//!
//! [`Project::open`] reads the archive's central directory and `metallic.toml`; every other member is read from the
//! archive when it is asked for ([`Project::read`]). Edits ([`Project::write`], [`Project::remove`],
//! [`Project::set_manifest`]) stay in memory until [`Project::save`], so the file on disk only changes there.
//! [`Project::create`] makes a new project from a [`Template`].
//!
//! # Writing an archive
//!
//! - **Crash-safe:** a save writes a complete new archive beside the old one (`MyGame.mtproj.tmp`), flushes it to
//!   disk and renames it over the old file. A crash or a failed save leaves the old project as it was.
//! - **Unchanged members are copied raw**, without inflating and deflating them again.
//! - **Compression:** members are deflated, except formats that are already compressed (PNG, JPEG, GLB, audio, ...),
//!   which are stored so they can be read in place ([`compression_for`]).
//! - **Deterministic:** `metallic.toml` comes first, then every other member sorted by path, all dated 1980-01-01
//!   (the zip format's earliest date), so the same contents always give the same bytes.
//!
//! # Member paths
//!
//! Member paths use `/`, are relative to the archive root and are case-sensitive. A path that is absolute, has a
//! drive letter, uses `\`, or has an empty, `.` or `..` part is refused ([`check_member_path`]): opening an archive
//! with such a member fails, so nothing read from a project can point outside it.

mod template;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use metallic_core::DataModel;
use zip::result::ZipError;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::manifest::{ENGINE_VERSION, MANIFEST_FILE, Manifest, ManifestError, ManifestWarning};
use crate::scene::{LoadedScene, SceneError, load_scene_bytes};

pub use template::Template;

/// The file name extension of a project (`MyGame.mtproj`).
pub const PROJECT_EXTENSION: &str = "mtproj";

/// What is appended to a project's file name for the archive a save writes before renaming it over the project.
pub const TEMP_SUFFIX: &str = ".tmp";

/// File name extensions (lowercase) of formats that are already compressed: their members are stored, not deflated.
const STORED_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "ktx2", "glb", "ogg", "mp3", "opus", "flac", "m4a", "zip",
];

/// The archive's file handle type.
type Archive = ZipArchive<BufReader<File>>;

/// An open `.mtproj` project: its manifest, the archive on disk and the edits not saved yet.
pub struct Project {
    path: PathBuf,
    archive: Archive,
    /// Every file member of the archive on disk, by path, with its index in the archive.
    stored: BTreeMap<String, usize>,
    /// Changes since the project was opened or last saved: a member's new contents, or `None` once removed.
    edits: BTreeMap<String, Option<Vec<u8>>>,
    manifest: Manifest,
    warnings: Vec<ManifestWarning>,
}

impl Project {
    /// Opens the project at `path`: reads the archive's central directory and its `metallic.toml`.
    ///
    /// Fails when the file is missing, is not a zip archive, holds a member whose path is unsafe
    /// ([`check_member_path`]), or has no valid `metallic.toml` at its root. The start scene is only read by
    /// [`Project::load_start_scene`].
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path).map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                ProjectError::NotFound { path: path.clone() }
            } else {
                ProjectError::Io {
                    path: path.clone(),
                    source,
                }
            }
        })?;
        // The file opened, so a failure here is about its contents.
        let archive = ZipArchive::new(BufReader::new(file)).map_err(|error| ProjectError::NotAZip {
            path: path.clone(),
            reason: error.to_string(),
        })?;
        let stored = index(&archive, &path)?;
        let mut project = Self {
            path,
            archive,
            stored,
            edits: BTreeMap::new(),
            manifest: Manifest::new(""),
            warnings: Vec::new(),
        };
        let bytes = project.read(MANIFEST_FILE).map_err(|error| match error {
            ProjectError::MissingMember { path, .. } => ProjectError::MissingManifest { path },
            other => other,
        })?;
        let loaded = Manifest::from_bytes(&bytes).map_err(|source| ProjectError::Manifest {
            path: project.path.clone(),
            source,
        })?;
        project.check(&loaded.manifest.start_scene)?;
        project.manifest = loaded.manifest;
        project.warnings = loaded.warnings;
        Ok(project)
    }

    /// Makes a new project at `path` from `template`, called `name`, and opens it.
    ///
    /// The template's `metallic.toml` is kept but for its name and engine version (this engine's). Refuses to
    /// replace a file that is already there. The archive is written crash-safely, like [`Project::save`].
    pub fn create(path: impl AsRef<Path>, template: &Template, name: &str) -> Result<Self, ProjectError> {
        let path = path.as_ref();
        if fs::symlink_metadata(path).is_ok() {
            return Err(ProjectError::AlreadyExists {
                path: path.to_path_buf(),
            });
        }
        let mut manifest = template.manifest()?;
        manifest.name = name.to_owned();
        manifest.engine = ENGINE_VERSION.to_owned();
        if template.member(&manifest.start_scene).is_none() {
            return Err(ProjectError::MissingStartScene {
                path: template.dir().to_path_buf(),
                scene: manifest.start_scene,
            });
        }
        let manifest_bytes = manifest.to_toml().into_bytes();
        write_archive(path, |writer| {
            write_member(writer, MANIFEST_FILE, &manifest_bytes)?;
            for (member, bytes) in template.members() {
                if member != MANIFEST_FILE {
                    write_member(writer, member, bytes)?;
                }
            }
            Ok(())
        })?;
        Self::open(path)
    }

    /// The project's file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The project settings, with any unsaved change from [`Project::set_manifest`].
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// What was skipped in `metallic.toml` when the project was opened (the caller reports it).
    pub fn warnings(&self) -> &[ManifestWarning] {
        &self.warnings
    }

    /// Replaces the project settings (in memory until [`Project::save`]). Refuses a start scene whose path is
    /// unsafe ([`check_member_path`]).
    pub fn set_manifest(&mut self, manifest: Manifest) -> Result<(), ProjectError> {
        self.check(&manifest.start_scene)?;
        self.edits
            .insert(MANIFEST_FILE.to_owned(), Some(manifest.to_toml().into_bytes()));
        self.manifest = manifest;
        Ok(())
    }

    /// Every member's path, sorted, with unsaved edits applied.
    pub fn members(&self) -> Vec<&str> {
        let mut members: BTreeSet<&str> = self
            .stored
            .keys()
            .filter(|member| !self.edits.contains_key(*member))
            .map(String::as_str)
            .collect();
        members.extend(
            self.edits
                .iter()
                .filter(|(_, bytes)| bytes.is_some())
                .map(|(member, _)| member.as_str()),
        );
        members.into_iter().collect()
    }

    /// Whether the project has a member at `member`, with unsaved edits applied.
    pub fn contains(&self, member: &str) -> bool {
        match self.edits.get(member) {
            Some(bytes) => bytes.is_some(),
            None => self.stored.contains_key(member),
        }
    }

    /// Whether there are edits [`Project::save`] has not written yet.
    pub fn is_modified(&self) -> bool {
        !self.edits.is_empty()
    }

    /// The contents of `member`: its unsaved edit, or else what the archive holds (read and inflated now).
    pub fn read(&mut self, member: &str) -> Result<Vec<u8>, ProjectError> {
        let missing = || ProjectError::MissingMember {
            path: self.path.clone(),
            member: member.to_owned(),
        };
        match self.edits.get(member) {
            Some(Some(bytes)) => return Ok(bytes.clone()),
            Some(None) => return Err(missing()),
            None => {}
        }
        let index = *self.stored.get(member).ok_or_else(missing)?;
        let read = |archive: &mut Archive| -> io::Result<Vec<u8>> {
            let mut file = archive.by_index(index).map_err(zip_to_io)?;
            let mut bytes = Vec::with_capacity(usize::try_from(file.size()).unwrap_or(0).min(1 << 26));
            file.read_to_end(&mut bytes)?;
            Ok(bytes)
        };
        read(&mut self.archive).map_err(|source| ProjectError::ReadMember {
            path: self.path.clone(),
            member: member.to_owned(),
            source,
        })
    }

    /// [`Project::read`] for a text member (a script, a scene), which must be UTF-8.
    pub fn read_text(&mut self, member: &str) -> Result<String, ProjectError> {
        String::from_utf8(self.read(member)?).map_err(|_| ProjectError::NotText {
            path: self.path.clone(),
            member: member.to_owned(),
        })
    }

    /// Sets the contents of `member`, adding it if it is new (in memory until [`Project::save`]). Writing
    /// `metallic.toml` this way is refused: use [`Project::set_manifest`].
    pub fn write(&mut self, member: &str, bytes: impl Into<Vec<u8>>) -> Result<(), ProjectError> {
        self.check(member)?;
        if member == MANIFEST_FILE {
            return Err(ProjectError::ManifestMember {
                path: self.path.clone(),
            });
        }
        self.edits.insert(member.to_owned(), Some(bytes.into()));
        Ok(())
    }

    /// Removes `member` (in memory until [`Project::save`]). Returns whether it was there. `metallic.toml` cannot
    /// be removed.
    pub fn remove(&mut self, member: &str) -> bool {
        if member == MANIFEST_FILE || !self.contains(member) {
            return false;
        }
        if self.stored.contains_key(member) {
            self.edits.insert(member.to_owned(), None);
        } else {
            self.edits.remove(member);
        }
        true
    }

    /// Writes every edit to the project's file, crash-safely: a new archive beside it, flushed, then renamed over
    /// it. Unchanged members are copied without recompressing. On an error the file on disk is unchanged and the
    /// edits are kept.
    pub fn save(&mut self) -> Result<(), ProjectError> {
        let members: Vec<String> = self.members().into_iter().map(str::to_owned).collect();
        let Self {
            archive, stored, edits, ..
        } = self;
        // The manifest first, so a tool listing the archive sees it at once.
        let ordered = members
            .iter()
            .filter(|member| *member == MANIFEST_FILE)
            .chain(members.iter().filter(|member| *member != MANIFEST_FILE));
        write_archive(&self.path, |writer| {
            for member in ordered {
                match (edits.get(member), stored.get(member)) {
                    (Some(Some(bytes)), _) => write_member(writer, member, bytes)?,
                    (_, Some(&index)) => writer.raw_copy_file(archive.by_index(index)?)?,
                    // `members` only lists members that are edited or stored.
                    _ => {}
                }
            }
            Ok(())
        })?;
        // The old archive is gone; read the new one's central directory.
        let warnings = std::mem::take(&mut self.warnings);
        *self = Self::open(&self.path)?;
        self.warnings = warnings;
        Ok(())
    }

    /// Loads the start scene ([`Manifest::start_scene`]) into `dm`, which must be a whole game (see
    /// [`load_scene_bytes`]). On an error `dm` is unchanged.
    pub fn load_start_scene(&mut self, dm: &mut DataModel) -> Result<LoadedScene, ProjectError> {
        let scene = self.manifest.start_scene.clone();
        let bytes = self.read(&scene).map_err(|error| match error {
            ProjectError::MissingMember { path, member } => ProjectError::MissingStartScene { path, scene: member },
            other => other,
        })?;
        load_scene_bytes(dm, &bytes).map_err(|source| ProjectError::Scene {
            path: self.path.clone(),
            scene,
            source,
        })
    }

    /// Refuses `member` if its path is unsafe.
    fn check(&self, member: &str) -> Result<(), ProjectError> {
        check_member_path(member).map_err(|reason| ProjectError::UnsafeMember {
            path: self.path.clone(),
            member: member.to_owned(),
            reason,
        })
    }
}

impl std::fmt::Debug for Project {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Project")
            .field("path", &self.path)
            .field("manifest", &self.manifest)
            .field("members", &self.members())
            .field("modified", &self.is_modified())
            .finish_non_exhaustive()
    }
}

/// Why a project could not be opened, created, read or saved. Every message names the project's file.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    /// There is no file at the path.
    #[error("{}: no such project file", path.display())]
    NotFound {
        /// The project's file.
        path: PathBuf,
    },
    /// [`Project::create`] would replace a file that is already there.
    #[error("{}: a file is already there; a new project never replaces one", path.display())]
    AlreadyExists {
        /// The project's file.
        path: PathBuf,
    },
    /// The file system refused a read or write.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file or folder concerned.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The file is not a zip archive (or a damaged one).
    #[error("{}: not a Metallic project (a .mtproj is a zip archive): {reason}", path.display())]
    NotAZip {
        /// The project's file.
        path: PathBuf,
        /// What the zip reader said.
        reason: String,
    },
    /// A member path is absolute or would point outside the project ([`check_member_path`]).
    #[error("{}: refused the member path `{member}`: {reason}", path.display())]
    UnsafeMember {
        /// The project's file (or the template's folder).
        path: PathBuf,
        /// The member path.
        member: String,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// There is no `metallic.toml` at the archive's root (or the template folder's).
    #[error("{}: no metallic.toml at the project's root", path.display())]
    MissingManifest {
        /// The project's file (or the template's folder).
        path: PathBuf,
    },
    /// `metallic.toml` is there but is refused.
    #[error("{}: {source}", path.display())]
    Manifest {
        /// The project's file (or the template's folder).
        path: PathBuf,
        /// What is wrong with it.
        source: ManifestError,
    },
    /// `metallic.toml` must be changed through [`Project::set_manifest`], which checks it.
    #[error("{}: metallic.toml is changed through the project settings, not written as a member", path.display())]
    ManifestMember {
        /// The project's file.
        path: PathBuf,
    },
    /// The start scene `metallic.toml` names is not in the project.
    #[error("{}: the start scene `{scene}` is not in the project", path.display())]
    MissingStartScene {
        /// The project's file (or the template's folder).
        path: PathBuf,
        /// The start scene's member path.
        scene: String,
    },
    /// The start scene could not be loaded.
    #[error("{}: the start scene `{scene}` could not be loaded: {source}", path.display())]
    Scene {
        /// The project's file.
        path: PathBuf,
        /// The start scene's member path.
        scene: String,
        /// Why.
        source: SceneError,
    },
    /// A member that is not in the project was asked for.
    #[error("{}: no member `{member}` in the project", path.display())]
    MissingMember {
        /// The project's file.
        path: PathBuf,
        /// The member path.
        member: String,
    },
    /// A member is in the archive but could not be read (a damaged archive, or a compression this engine does not
    /// read).
    #[error("{}: could not read `{member}`: {source}", path.display())]
    ReadMember {
        /// The project's file.
        path: PathBuf,
        /// The member path.
        member: String,
        /// Why.
        source: io::Error,
    },
    /// A member that must be text (a script, a scene) is not UTF-8.
    #[error("{}: `{member}` is not UTF-8 text", path.display())]
    NotText {
        /// The project's file.
        path: PathBuf,
        /// The member path.
        member: String,
    },
    /// The new archive could not be written. The project's file on disk is unchanged.
    #[error("{}: could not save, the file on disk is unchanged: {source}", path.display())]
    Save {
        /// The project's file.
        path: PathBuf,
        /// Why.
        source: io::Error,
    },
}

/// Checks a member path: relative, with `/` between its parts, and no part empty, `.` or `..`. Returns what is
/// wrong with it. Paths stay inside the project whatever reads them, on any operating system.
pub fn check_member_path(member: &str) -> Result<(), &'static str> {
    if member.is_empty() {
        return Err("the path is empty");
    }
    if member.starts_with('/') {
        return Err("absolute paths are not allowed");
    }
    if member.contains('\\') {
        return Err("member paths separate folders with `/`, not `\\`");
    }
    if member.contains(':') {
        return Err("`:` (a drive letter or stream name) is not allowed");
    }
    if member.contains('\0') {
        return Err("NUL characters are not allowed");
    }
    for part in member.split('/') {
        match part {
            "" => return Err("empty path parts (`//`, or a trailing `/`) are not allowed"),
            "." => return Err("`.` path parts are not allowed"),
            ".." => return Err("`..` path parts are not allowed"),
            _ => {}
        }
    }
    Ok(())
}

/// How a member is written: stored when its format is already compressed (by file name extension), deflated
/// otherwise.
pub fn compression_for(member: &str) -> CompressionMethod {
    let extension = member
        .rsplit_once('.')
        .filter(|(_, extension)| !extension.contains('/'))
        .map(|(_, extension)| extension.to_ascii_lowercase());
    match extension {
        Some(extension) if STORED_EXTENSIONS.contains(&extension.as_str()) => CompressionMethod::Stored,
        _ => CompressionMethod::Deflated,
    }
}

/// The file members of `archive` by path, each checked. Folder entries (paths ending in `/`, which zip tools add)
/// are checked too, then left out.
fn index(archive: &Archive, path: &Path) -> Result<BTreeMap<String, usize>, ProjectError> {
    let mut stored = BTreeMap::new();
    for (i, name) in archive.file_names().enumerate() {
        let name = name.map_err(|error| ProjectError::NotAZip {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })?;
        let (member, is_folder) = match name.strip_suffix('/') {
            Some(folder) => (folder, true),
            None => (name.as_ref(), false),
        };
        check_member_path(member).map_err(|reason| ProjectError::UnsafeMember {
            path: path.to_path_buf(),
            member: name.to_string(),
            reason,
        })?;
        if !is_folder {
            stored.insert(member.to_owned(), i);
        }
    }
    Ok(stored)
}

/// Writes one member, compressed as [`compression_for`] says, dated 1980-01-01 so archives are deterministic.
fn write_member(writer: &mut ZipWriter<BufWriter<File>>, member: &str, bytes: &[u8]) -> Result<(), ZipError> {
    let options = SimpleFileOptions::default()
        .compression_method(compression_for(member))
        .last_modified_time(DateTime::default())
        .large_file(u32::try_from(bytes.len()).is_err());
    writer.start_file(member, options)?;
    writer.write_all(bytes)?;
    Ok(())
}

/// Writes a complete archive with `fill` to `path`'s temporary file, flushes it to disk and renames it over
/// `path`. On an error the temporary file is removed and `path` is untouched.
fn write_archive(
    path: &Path,
    fill: impl FnOnce(&mut ZipWriter<BufWriter<File>>) -> Result<(), ZipError>,
) -> Result<(), ProjectError> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(TEMP_SUFFIX);
    let temp = PathBuf::from(temp);
    let write = || -> io::Result<()> {
        let mut writer = ZipWriter::new(BufWriter::new(File::create(&temp)?));
        fill(&mut writer).map_err(zip_to_io)?;
        let file = writer
            .finish()
            .map_err(zip_to_io)?
            .into_inner()
            .map_err(io::IntoInnerError::into_error)?;
        // On disk before the rename, so a crash cannot leave a renamed but empty project.
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    };
    write().map_err(|source| {
        let _ = fs::remove_file(&temp);
        ProjectError::Save {
            path: path.to_path_buf(),
            source,
        }
    })
}

/// A zip error as an I/O error (the zip crate's own I/O errors unwrapped).
fn zip_to_io(error: ZipError) -> io::Error {
    match error {
        ZipError::Io(error) => error,
        other => io::Error::other(other),
    }
}
