//! `.mtproj` projects: one zip archive holding the manifest, scenes, scripts and assets (ADR 0006).
//!
//! [`Project::open`] reads the archive's central directory and `metallic.toml`; every other member is read from the
//! archive when it is asked for ([`Project::read`]). Edits ([`Project::write`], [`Project::remove`],
//! [`Project::set_manifest`]) stay in memory until [`Project::save`], so the file on disk only changes there.
//! [`Project::create`] makes a new project from a [`Template`]. The asset index (`assets.json`) is read on open
//! ([`Project::assets`]) and brought up to date by [`Project::refresh_assets`].
//!
//! The file operations ([`Project::create_member`], [`Project::rename_member`], [`Project::rename_folder`],
//! [`Project::delete_member`]) keep asset ids and rewrite the paths that name a moved member; see `file_ops.rs`.
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

mod file_ops;
mod template;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use metallic_core::DataModel;
use zip::result::ZipError;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::asset_index::{AssetId, AssetIndex, AssetIndexError, Fingerprint, INDEX_FILE, IndexReport, is_asset};
use crate::assets::AssetReader;
use crate::game_files::GameFiles;
use crate::manifest::{ENGINE_VERSION, MANIFEST_FILE, Manifest, ManifestError, ManifestWarning};
use crate::pak::PakError;
use crate::scene::{LoadedScene, SceneError};

pub use file_ops::{MemberMove, MoveReport, References};
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
    /// The asset index, with any unsaved change (which is also in `edits`, as `assets.json`).
    assets: AssetIndex,
}

impl Project {
    /// Opens the project at `path`: reads the archive's central directory, its `metallic.toml` and its asset index
    /// (`assets.json`; a project without one has an empty index).
    ///
    /// Fails when the file is missing, is not a zip archive, holds a member whose path is unsafe
    /// ([`check_member_path`]), has no valid `metallic.toml` at its root, or has an `assets.json` that is refused. The
    /// start scene is only read by [`Project::load_start_scene`].
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
            assets: AssetIndex::new(),
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
        if project.contains(INDEX_FILE) {
            let bytes = project.read(INDEX_FILE)?;
            project.assets = AssetIndex::from_bytes(&bytes).map_err(|source| ProjectError::AssetIndex {
                path: project.path.clone(),
                source,
            })?;
        }
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

    /// Writes a project at `path` holding exactly `members` (path and contents), replacing a file already there, and
    /// opens it: a participant's copy of a project another Studio hosts (Team Create, ADR 0011). The archive is
    /// written crash-safely, like [`Project::save`].
    ///
    /// Refuses an unsafe member path ([`check_member_path`]), a path given twice and a set without `metallic.toml`,
    /// before anything is written.
    pub fn from_members<'a>(
        path: impl AsRef<Path>,
        members: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    ) -> Result<Self, ProjectError> {
        let path = path.as_ref();
        let members: Vec<(&str, &[u8])> = members.into_iter().collect();
        let mut seen = std::collections::BTreeSet::new();
        for (member, _) in &members {
            check_member_path(member).map_err(|reason| ProjectError::UnsafeMember {
                path: path.to_path_buf(),
                member: (*member).to_owned(),
                reason,
            })?;
            if !seen.insert(*member) {
                return Err(ProjectError::UnsafeMember {
                    path: path.to_path_buf(),
                    member: (*member).to_owned(),
                    reason: "it is given twice",
                });
            }
        }
        if !seen.contains(MANIFEST_FILE) {
            return Err(ProjectError::MissingManifest {
                path: path.to_path_buf(),
            });
        }
        write_archive(path, |writer| {
            for (member, bytes) in members.iter().filter(|(m, _)| *m == MANIFEST_FILE) {
                write_member(writer, member, bytes)?;
            }
            for (member, bytes) in members.iter().filter(|(m, _)| *m != MANIFEST_FILE) {
                write_member(writer, member, bytes)?;
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
        GameFiles::read_text(self, member)
    }

    /// Sets the contents of `member`, adding it if it is new (in memory until [`Project::save`]). Writing
    /// `metallic.toml` this way is refused (use [`Project::set_manifest`]), and so is writing `assets.json` (it
    /// changes through [`Project::refresh_assets`]).
    pub fn write(&mut self, member: &str, bytes: impl Into<Vec<u8>>) -> Result<(), ProjectError> {
        self.check(member)?;
        if member == MANIFEST_FILE {
            return Err(ProjectError::ManifestMember {
                path: self.path.clone(),
            });
        }
        if member == INDEX_FILE {
            return Err(ProjectError::IndexMember {
                path: self.path.clone(),
            });
        }
        self.edits.insert(member.to_owned(), Some(bytes.into()));
        Ok(())
    }

    /// Removes `member` (in memory until [`Project::save`]). Returns whether it was there. `metallic.toml` and
    /// `assets.json` cannot be removed.
    pub fn remove(&mut self, member: &str) -> bool {
        if member == MANIFEST_FILE || member == INDEX_FILE || !self.contains(member) {
            return false;
        }
        if self.stored.contains_key(member) {
            self.edits.insert(member.to_owned(), None);
        } else {
            self.edits.remove(member);
        }
        true
    }

    /// The fingerprint of `member` (its size and CRC-32), with unsaved edits applied. A saved member's comes from
    /// the archive's central directory, so it is not read, however large it is.
    pub fn fingerprint(&self, member: &str) -> Result<Fingerprint, ProjectError> {
        let missing = || ProjectError::MissingMember {
            path: self.path.clone(),
            member: member.to_owned(),
        };
        match self.edits.get(member) {
            Some(Some(bytes)) => return Ok(Fingerprint::of(bytes)),
            Some(None) => return Err(missing()),
            None => {}
        }
        let index = *self.stored.get(member).ok_or_else(missing)?;
        let entry = self
            .archive
            .by_index_data(index)
            .map_err(|error| ProjectError::ReadMember {
                path: self.path.clone(),
                member: member.to_owned(),
                source: zip_to_io(error),
            })?;
        Ok(Fingerprint {
            size: entry.size(),
            crc32: entry.crc32(),
        })
    }

    /// The asset index: a stable id for every asset member, with unsaved changes. It is as of the last
    /// [`Project::refresh_assets`] (or as saved); members edited since are not in it yet.
    pub fn assets(&self) -> &AssetIndex {
        &self.assets
    }

    /// Brings the asset index up to date with the project's members ([`AssetIndex::update`]), giving new assets
    /// random ids, and says what was added, moved and is missing. A change to the index is an edit, saved by
    /// [`Project::save`]. On an error the index is unchanged.
    pub fn refresh_assets(&mut self) -> Result<IndexReport, ProjectError> {
        let mut members = BTreeMap::new();
        for member in self.members() {
            members.insert(member.to_owned(), self.fingerprint(member)?);
        }
        let mut assets = self.assets.clone();
        let report = assets
            .update(&members, AssetId::random)
            .map_err(|source| ProjectError::AssetIndex {
                path: self.path.clone(),
                source,
            })?;
        self.set_assets(assets);
        Ok(report)
    }

    /// A reader of the project's assets by id that works on any thread ([`AssetReader`]), for decoding meshes and
    /// images off the frame. It sees the asset index and the asset members as they are now, unsaved edits included;
    /// later edits are not seen by it.
    pub fn asset_reader(&self) -> AssetReader {
        let edits = self
            .edits
            .iter()
            .filter(|(member, _)| is_asset(member))
            .map(|(member, bytes)| (member.clone(), bytes.clone()))
            .collect();
        AssetReader::new(&self.path, self.assets.clone(), edits)
    }

    /// Drops the asset `id` from the index ([`AssetIndex::forget`]), for a missing asset that is not coming back.
    /// Returns the path it had. Saved by [`Project::save`].
    pub fn forget_asset(&mut self, id: AssetId) -> Option<String> {
        let mut assets = self.assets.clone();
        let path = assets.forget(id)?;
        self.set_assets(assets);
        Some(path)
    }

    /// Replaces the asset index, recording `assets.json` as an edit when it changed.
    fn set_assets(&mut self, assets: AssetIndex) {
        if assets != self.assets {
            self.edits
                .insert(INDEX_FILE.to_owned(), Some(assets.to_json().into_bytes()));
            self.assets = assets;
        }
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
        GameFiles::load_start_scene(self, dm)
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

impl GameFiles for Project {
    fn path(&self) -> &Path {
        Project::path(self)
    }

    fn manifest(&self) -> &Manifest {
        Project::manifest(self)
    }

    fn members(&self) -> Vec<&str> {
        Project::members(self)
    }

    fn read(&mut self, member: &str) -> Result<Vec<u8>, ProjectError> {
        Project::read(self, member)
    }

    fn assets(&self) -> &AssetIndex {
        Project::assets(self)
    }

    fn asset_reader(&self) -> AssetReader {
        Project::asset_reader(self)
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
    /// `assets.json` changes through [`Project::refresh_assets`], not by writing it.
    #[error("{}: assets.json is changed through the asset index, not written as a member", path.display())]
    IndexMember {
        /// The project's file.
        path: PathBuf,
    },
    /// `assets.json` is refused, or the asset index could not be updated.
    #[error("{}: {source}", path.display())]
    AssetIndex {
        /// The project's file.
        path: PathBuf,
        /// What is wrong.
        source: AssetIndexError,
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
    /// A file operation would put a member where there already is one.
    #[error("{}: there is already a member `{member}`", path.display())]
    MemberExists {
        /// The project's file.
        path: PathBuf,
        /// The member path.
        member: String,
    },
    /// [`Project::rename_folder`] was asked to move a folder into itself.
    #[error("{}: cannot move the folder `{from}` into itself (`{to}`)", path.display())]
    FolderIntoItself {
        /// The project's file.
        path: PathBuf,
        /// The folder.
        from: String,
        /// Where it was to go.
        to: String,
    },
    /// A `game.pak` is not one this engine reads, or is damaged ([`Pak`](crate::Pak)).
    #[error("{}: {source}", path.display())]
    Pak {
        /// The pak's file.
        path: PathBuf,
        /// What is wrong with it.
        source: PakError,
    },
    /// No asset has the id.
    #[error("{}: no asset {id}", path.display())]
    UnknownAsset {
        /// The project's or the pak's file.
        path: PathBuf,
        /// The id.
        id: AssetId,
    },
    /// The new archive (or pak) could not be written. The file on disk is unchanged.
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
fn write_member(writer: &mut ZipWriter<BufWriter<Sink>>, member: &str, bytes: &[u8]) -> Result<(), ZipError> {
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
    fill: impl FnOnce(&mut ZipWriter<BufWriter<Sink>>) -> Result<(), ZipError>,
) -> Result<(), ProjectError> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(TEMP_SUFFIX);
    let temp = PathBuf::from(temp);
    let write = || -> io::Result<()> {
        let mut writer = ZipWriter::new(BufWriter::new(Sink::create(&temp)?));
        fill(&mut writer).map_err(zip_to_io)?;
        let sink = writer
            .finish()
            .map_err(zip_to_io)?
            .into_inner()
            .map_err(io::IntoInnerError::into_error)?;
        // On disk before the rename, so a crash cannot leave a renamed but empty project.
        sink.file.sync_all()?;
        drop(sink);
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

/// The temporary archive a save writes. In unit tests it can be made to fail part-way ([`fail_saves_after`]), to
/// show that such a save leaves the project's file as it was.
struct Sink {
    file: File,
    /// How many more bytes may be written before every write fails.
    #[cfg(test)]
    budget: Option<usize>,
}

#[cfg(test)]
thread_local! {
    /// The budget of the next [`Sink`]s made on this thread.
    static FAIL_AFTER: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Makes every save on this thread fail once it has written `bytes` bytes of its temporary archive (`None`: never).
#[cfg(test)]
pub(crate) fn fail_saves_after(bytes: Option<usize>) {
    FAIL_AFTER.with(|budget| budget.set(bytes));
}

impl Sink {
    fn create(path: &Path) -> io::Result<Self> {
        Ok(Self {
            file: File::create(path)?,
            #[cfg(test)]
            budget: FAIL_AFTER.with(std::cell::Cell::get),
        })
    }
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        #[cfg(test)]
        if let Some(budget) = &mut self.budget {
            if *budget == 0 {
                return Err(io::Error::other("injected failure part-way through the save"));
            }
            let written = self.file.write(&buf[..buf.len().min(*budget)])?;
            *budget -= written;
            return Ok(written);
        }
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for Sink {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

/// A zip error as an I/O error (the zip crate's own I/O errors unwrapped).
fn zip_to_io(error: ZipError) -> io::Error {
    match error {
        ZipError::Io(error) => error,
        other => io::Error::other(other),
    }
}
