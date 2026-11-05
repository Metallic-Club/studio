//! [`Template`]: the starting contents of a new project, read from a folder.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::{ProjectError, check_member_path};
use crate::manifest::{MANIFEST_FILE, Manifest};

/// The files a new project starts with ([`Project::create`](super::Project::create)), by member path.
///
/// Templates are kept as plain folders (`templates/baseplate-3d` in the repository), so they diff; creating a
/// project packs one into a `.mtproj`.
#[derive(Clone, Debug)]
pub struct Template {
    dir: PathBuf,
    members: BTreeMap<String, Vec<u8>>,
}

impl Template {
    /// Reads every file under `dir`; their paths relative to `dir` become member paths. `dir` must hold a
    /// `metallic.toml`; a file name that is not a safe member path ([`check_member_path`]) is refused.
    pub fn from_dir(dir: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let dir = dir.as_ref().to_path_buf();
        let mut members = BTreeMap::new();
        read_dir(&dir, "", &mut members)?;
        let template = Self { dir, members };
        template.manifest()?;
        Ok(template)
    }

    /// The folder it was read from.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Every member, sorted by path.
    pub fn members(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.members
            .iter()
            .map(|(member, bytes)| (member.as_str(), bytes.as_slice()))
    }

    /// The member at `member`, if there is one.
    pub fn member(&self, member: &str) -> Option<&[u8]> {
        self.members.get(member).map(Vec::as_slice)
    }

    /// The template's `metallic.toml`.
    pub fn manifest(&self) -> Result<Manifest, ProjectError> {
        let bytes = self
            .member(MANIFEST_FILE)
            .ok_or_else(|| ProjectError::MissingManifest { path: self.dir.clone() })?;
        Manifest::from_bytes(bytes)
            .map(|loaded| loaded.manifest)
            .map_err(|source| ProjectError::Manifest {
                path: self.dir.clone(),
                source,
            })
    }
}

/// Reads the files under `dir` into `members`, `prefix` being `dir`'s member path (empty or ending in `/`).
fn read_dir(dir: &Path, prefix: &str, members: &mut BTreeMap<String, Vec<u8>>) -> Result<(), ProjectError> {
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| ProjectError::Io { path, source }
    };
    for entry in fs::read_dir(dir).map_err(io(dir))? {
        let entry = entry.map_err(io(dir))?;
        let path = entry.path();
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(ProjectError::UnsafeMember {
                path: path.clone(),
                member: name.to_string_lossy().into_owned(),
                reason: "member paths must be UTF-8",
            });
        };
        let member = format!("{prefix}{name}");
        check_member_path(&member).map_err(|reason| ProjectError::UnsafeMember {
            path: dir.to_path_buf(),
            member: member.clone(),
            reason,
        })?;
        if entry.file_type().map_err(io(&path))?.is_dir() {
            read_dir(&path, &format!("{member}/"), members)?;
        } else {
            members.insert(member, fs::read(&path).map_err(io(&path))?);
        }
    }
    Ok(())
}
