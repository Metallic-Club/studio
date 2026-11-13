//! The asset index: a stable id for every asset of a project, kept in `assets.json` at the archive's root (ADR 0006).
//!
//! Scenes refer to assets by [`AssetId`], not by member path, so moving or renaming a member does not break them.
//! [`AssetIndex::update`] (through [`Project::refresh_assets`](crate::Project::refresh_assets)) brings the index up
//! to date with a project's members:
//!
//! - a member that is already indexed keeps its id, even if its contents changed;
//! - a new member whose [`Fingerprint`] (size and CRC-32) matches an indexed asset whose member is gone is that asset,
//!   moved or renamed: it takes over the id;
//! - any other new member gets a new random id;
//! - an indexed asset with no member is reported missing and stays in the index, so a scene that refers to it can
//!   say what is missing, and the id comes back if the member does. [`AssetIndex::forget`] drops it.
//!
//! # Which members are assets
//!
//! Every member except the project's own metadata at the root, `metallic.toml` and `assets.json`, and the editor's data
//! under `.studio/` ([`is_asset`]).
//! That is scenes (`Scenes/`), scripts (`Scripts/`) and everything under `Assets/`, and any other member a project
//! holds, so a member moved between folders keeps its id.
//!
//! # Fingerprints
//!
//! A member's fingerprint is its size and CRC-32, the checksum every zip archive stores for every member in its
//! central directory. Members saved in the archive are therefore never read to fingerprint them, however large; only
//! unsaved edits, which are in memory already, are checksummed. A false match needs a member of the same size and
//! the same checksum appearing while another disappears, which does not happen by accident. Empty members all look
//! alike, so they are never matched as moves.
//!
//! # `assets.json`
//!
//! Sorted by member path, one asset per line, with nothing machine-local (no file system paths or dates):
//!
//! ```json
//! {
//!   "format": 1,
//!   "assets": {
//!     "Assets/Textures/grass.png": {"id": "9f3c0a5e1b7d4c2a8e6f0b1d3a5c7e9f", "size": 48213, "crc32": 2774035466},
//!     "Scenes/Main.scene": {"id": "0d4e8b2f6a1c3e5d7f9b0a2c4e6d8f1a", "size": 1532, "crc32": 914311107}
//!   }
//! }
//! ```
//!
//! An id is 128 random bits written as 32 lowercase hex digits. Ids are never reused: a new one is random, so two
//! projects (or two branches of one) do not hand out the same id.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

use crate::manifest::MANIFEST_FILE;
use crate::project::check_member_path;

/// The asset index's member, at the archive's root.
pub const INDEX_FILE: &str = "assets.json";

/// The `format` version of `assets.json` this engine writes and reads.
pub const INDEX_FORMAT: u32 = 1;

/// The folder of the editor's own data in a project (#603): what Studio keeps about the project that is not part of the
/// game, such as the Explorer's folder colours. Its members are not assets and are not packed into a game's pak.
pub const EDITOR_DATA: &str = ".studio/";

/// Whether `member` is the editor's own data ([`EDITOR_DATA`]).
pub fn is_editor_data(member: &str) -> bool {
    member.starts_with(EDITOR_DATA)
}

/// Whether `member` is an asset that gets an id: every member but `metallic.toml`, `assets.json` and the editor's own
/// data under [`EDITOR_DATA`].
pub fn is_asset(member: &str) -> bool {
    member != MANIFEST_FILE && member != INDEX_FILE && !is_editor_data(member)
}

/// A stable id for an asset, unique across projects and never reused. Written as 32 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
// Big-endian bytes rather than a `u128`: the same order, but byte-aligned, which keeps errors that carry an id small.
pub struct AssetId([u8; 16]);

impl AssetId {
    /// The id with this value. For tests and tools that need known ids; new assets get [`AssetId::random`] ones.
    pub const fn from_u128(value: u128) -> Self {
        Self(value.to_be_bytes())
    }

    /// The id's value.
    pub const fn to_u128(self) -> u128 {
        u128::from_be_bytes(self.0)
    }

    /// A new random id, from the operating system's random number generator.
    pub fn random() -> Result<Self, AssetIndexError> {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|error| AssetIndexError::Random(error.to_string()))?;
        Ok(Self(bytes))
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.to_u128())
    }
}

impl fmt::Debug for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssetId({self})")
    }
}

impl FromStr for AssetId {
    type Err = ParseAssetIdError;

    /// Reads exactly 32 lowercase hex digits, so every id has one spelling.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let valid = text.len() == 32 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !valid {
            return Err(ParseAssetIdError);
        }
        u128::from_str_radix(text, 16)
            .map(Self::from_u128)
            .map_err(|_| ParseAssetIdError)
    }
}

/// Text that is not an [`AssetId`] (32 lowercase hex digits).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("an asset id is 32 lowercase hex digits")]
pub struct ParseAssetIdError;

/// What identifies a member's contents for move detection: its size and its CRC-32 (the zip format's checksum).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint {
    /// The contents' size in bytes.
    pub size: u64,
    /// The contents' CRC-32, as a zip archive stores it.
    pub crc32: u32,
}

impl Fingerprint {
    /// The fingerprint of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Self {
            size: bytes.len() as u64,
            crc32: crc32fast::hash(bytes),
        }
    }
}

/// An asset: its id and its member path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    /// The asset's id.
    pub id: AssetId,
    /// Its member path.
    pub path: String,
}

/// An asset found at a new path ([`IndexReport::moved`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetMove {
    /// The asset's id, which it keeps.
    pub id: AssetId,
    /// The path it had.
    pub from: String,
    /// The path it has now.
    pub to: String,
}

/// What an [`AssetIndex::update`] found, each list sorted by path. Missing assets are data for the caller to show,
/// not errors.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexReport {
    /// New members, each with a new id.
    pub added: Vec<Asset>,
    /// Members that moved or were renamed, keeping their ids.
    pub moved: Vec<AssetMove>,
    /// Indexed assets with no member (deleted, or moved and changed at once), at the path they had. They stay in the
    /// index until [`AssetIndex::forget`].
    pub missing: Vec<Asset>,
}

/// One asset in the index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    id: AssetId,
    fingerprint: Fingerprint,
}

/// A project's assets: a stable [`AssetId`] for every asset member, by path. See the [module docs](self).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssetIndex {
    by_path: BTreeMap<String, Entry>,
    by_id: BTreeMap<AssetId, String>,
}

impl AssetIndex {
    /// An empty index (a project with no `assets.json` yet).
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads `assets.json`. Refuses another `format` version, a path that is not a safe asset path
    /// ([`check_member_path`], [`is_asset`]), a malformed id or two assets with the same id.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, AssetIndexError> {
        let json = |error: serde_json::Error| AssetIndexError::Json(error.to_string());
        // The version first, so a newer file is refused for its version rather than for a field it changed.
        let header: Header = serde_json::from_slice(bytes).map_err(json)?;
        if header.format != INDEX_FORMAT {
            return Err(AssetIndexError::UnsupportedFormat { found: header.format });
        }
        let file: FileIn = serde_json::from_slice(bytes).map_err(json)?;
        let mut by_path = BTreeMap::new();
        for (path, entry) in file.assets {
            check_member_path(&path).map_err(|reason| AssetIndexError::BadPath {
                path: path.clone(),
                reason,
            })?;
            if !is_asset(&path) {
                return Err(AssetIndexError::BadPath {
                    path,
                    reason: "the project's metadata is not an asset",
                });
            }
            let Ok(id) = entry.id.parse() else {
                return Err(AssetIndexError::BadId { path, id: entry.id });
            };
            let fingerprint = Fingerprint {
                size: entry.size,
                crc32: entry.crc32,
            };
            by_path.insert(path, Entry { id, fingerprint });
        }
        Self::from_paths(by_path)
    }

    /// Writes `assets.json`: sorted by path, one asset per line, with a final newline. The same index always gives
    /// the same bytes, and reading them back gives the same index.
    pub fn to_json(&self) -> String {
        let mut out = format!("{{\n  \"format\": {INDEX_FORMAT},\n  \"assets\": {{");
        for (i, (path, entry)) in self.by_path.iter().enumerate() {
            let Fingerprint { size, crc32 } = entry.fingerprint;
            // Serializing a string cannot fail; serde_json escapes it as JSON needs.
            let path = serde_json::to_string(path).unwrap_or_default();
            let separator = if i == 0 { "\n    " } else { ",\n    " };
            out.push_str(&format!(
                "{separator}{path}: {{\"id\": \"{}\", \"size\": {size}, \"crc32\": {crc32}}}",
                entry.id
            ));
        }
        out.push_str(if self.by_path.is_empty() {
            "}\n}\n"
        } else {
            "\n  }\n}\n"
        });
        out
    }

    /// The id of the asset at `path`, if it is indexed (its member may be missing).
    pub fn id(&self, path: &str) -> Option<AssetId> {
        self.by_path.get(path).map(|entry| entry.id)
    }

    /// The path of the asset `id`, if it is indexed (its member may be missing).
    pub fn path(&self, id: AssetId) -> Option<&str> {
        self.by_id.get(&id).map(String::as_str)
    }

    /// The fingerprint the asset at `path` had when it was last indexed.
    pub fn fingerprint(&self, path: &str) -> Option<Fingerprint> {
        self.by_path.get(path).map(|entry| entry.fingerprint)
    }

    /// Every indexed asset, sorted by path.
    pub fn iter(&self) -> impl Iterator<Item = (&str, AssetId)> {
        self.by_path.iter().map(|(path, entry)| (path.as_str(), entry.id))
    }

    /// How many assets are indexed.
    pub fn len(&self) -> usize {
        self.by_path.len()
    }

    /// Whether no asset is indexed.
    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    /// Drops the asset `id` from the index (for a missing asset that is not coming back). Returns the path it had.
    pub fn forget(&mut self, id: AssetId) -> Option<String> {
        let path = self.by_id.remove(&id)?;
        self.by_path.remove(&path);
        Some(path)
    }

    /// Indexes the asset at `path` as `id` with `fingerprint`, replacing what was indexed at `path`. Refuses an id
    /// another path has.
    pub(crate) fn insert(&mut self, path: &str, id: AssetId, fingerprint: Fingerprint) -> Result<(), AssetIndexError> {
        if let Some(other) = self.by_id.get(&id).filter(|other| *other != path) {
            return Err(AssetIndexError::DuplicateId {
                id,
                first: other.clone(),
                second: path.to_owned(),
            });
        }
        if let Some(old) = self.by_path.insert(path.to_owned(), Entry { id, fingerprint }) {
            self.by_id.remove(&old.id);
        }
        self.by_id.insert(id, path.to_owned());
        Ok(())
    }

    /// Records that the asset at `from` is now at `to`, keeping its id. A missing asset indexed at `to` is dropped:
    /// the moved one takes its place. Returns the moved asset's id, or `None` when nothing is indexed at `from`.
    pub(crate) fn rename(&mut self, from: &str, to: &str) -> Option<AssetId> {
        let entry = self.by_path.remove(from)?;
        if let Some(replaced) = self.by_path.insert(to.to_owned(), entry) {
            self.by_id.remove(&replaced.id);
        }
        self.by_id.insert(entry.id, to.to_owned());
        Some(entry.id)
    }

    /// Brings the index up to date with `members`, a project's members by path with their fingerprints, and says
    /// what changed (see the [module docs](self)). Members that are not assets ([`is_asset`]) are skipped. `new_id`
    /// makes the id of each added asset, in path order ([`AssetId::random`] in a project). On an error the index is
    /// unchanged.
    pub fn update(
        &mut self,
        members: &BTreeMap<String, Fingerprint>,
        mut new_id: impl FnMut() -> Result<AssetId, AssetIndexError>,
    ) -> Result<IndexReport, AssetIndexError> {
        let mut report = IndexReport::default();
        let mut by_path = BTreeMap::new();
        let mut added = Vec::new();
        for (path, &fingerprint) in members.iter().filter(|(path, _)| is_asset(path)) {
            match self.by_path.get(path) {
                // Still there: same id, whatever its contents are now.
                Some(entry) => {
                    by_path.insert(
                        path.clone(),
                        Entry {
                            id: entry.id,
                            fingerprint,
                        },
                    );
                }
                None => added.push((path, fingerprint)),
            }
        }
        // Gone members by fingerprint, each list in path order, so moving a folder of identical files pairs them
        // up by name.
        let mut gone: BTreeMap<Fingerprint, VecDeque<(&String, AssetId)>> = BTreeMap::new();
        for (path, entry) in &self.by_path {
            if !members.contains_key(path) {
                gone.entry(entry.fingerprint).or_default().push_back((path, entry.id));
            }
        }
        let mut used: BTreeSet<AssetId> = self.by_id.keys().copied().collect();
        for (path, fingerprint) in added {
            let moved = (fingerprint.size > 0)
                .then(|| gone.get_mut(&fingerprint).and_then(VecDeque::pop_front))
                .flatten();
            let id = match moved {
                Some((from, id)) => {
                    report.moved.push(AssetMove {
                        id,
                        from: from.clone(),
                        to: path.clone(),
                    });
                    id
                }
                None => {
                    let id = new_id()?;
                    if !used.insert(id) {
                        return Err(AssetIndexError::IdInUse(id));
                    }
                    report.added.push(Asset { id, path: path.clone() });
                    id
                }
            };
            by_path.insert(path.clone(), Entry { id, fingerprint });
        }
        for (fingerprint, paths) in gone {
            for (path, id) in paths {
                by_path.insert(path.clone(), Entry { id, fingerprint });
                report.missing.push(Asset { id, path: path.clone() });
            }
        }
        report.missing.sort_by(|a, b| a.path.cmp(&b.path));
        *self = Self::from_paths(by_path)?;
        Ok(report)
    }

    /// The index of `by_path`, refusing two paths with one id.
    fn from_paths(by_path: BTreeMap<String, Entry>) -> Result<Self, AssetIndexError> {
        let mut by_id = BTreeMap::new();
        for (path, entry) in &by_path {
            if let Some(first) = by_id.insert(entry.id, path.clone()) {
                return Err(AssetIndexError::DuplicateId {
                    id: entry.id,
                    first,
                    second: path.clone(),
                });
            }
        }
        Ok(Self { by_path, by_id })
    }
}

/// Why `assets.json` could not be read, or the index could not be updated.
#[derive(Debug, thiserror::Error)]
pub enum AssetIndexError {
    /// `assets.json` is not JSON, or not shaped like an asset index.
    #[error("{INDEX_FILE} is not a valid asset index: {0}")]
    Json(String),
    /// `assets.json` has a `format` version this engine does not read.
    #[error("{INDEX_FILE} has format {found}; this engine reads format {INDEX_FORMAT}")]
    UnsupportedFormat {
        /// The file's version.
        found: u32,
    },
    /// An indexed path is not a safe member path, or names the project's metadata.
    #[error("{INDEX_FILE}: `{path}` is not an asset's member path: {reason}")]
    BadPath {
        /// The path.
        path: String,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// An id is not 32 lowercase hex digits.
    #[error("{INDEX_FILE}: the id `{id}` of `{path}` is not 32 lowercase hex digits")]
    BadId {
        /// The asset's path.
        path: String,
        /// The id as written.
        id: String,
    },
    /// Two assets have the same id.
    #[error("{INDEX_FILE}: `{first}` and `{second}` have the same id {id}")]
    DuplicateId {
        /// The id.
        id: AssetId,
        /// One asset's path.
        first: String,
        /// The other's.
        second: String,
    },
    /// The random number generator failed, so no new id could be made.
    #[error("no new asset id: the random number generator failed: {0}")]
    Random(String),
    /// A new id was already in use (only an id source other than [`AssetId::random`] does that).
    #[error("the new asset id {0} is already in use")]
    IdInUse(AssetId),
}

/// The start of `assets.json`, read on its own to check the version.
#[derive(Deserialize)]
struct Header {
    format: u32,
}

/// `assets.json`, as read.
#[derive(Deserialize)]
struct FileIn {
    #[serde(default)]
    assets: BTreeMap<String, EntryIn>,
}

/// One asset, as read.
#[derive(Deserialize)]
struct EntryIn {
    id: String,
    size: u64,
    crc32: u32,
}

#[cfg(test)]
mod tests;
