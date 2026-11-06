//! `game.pak`: a game built from its `.mtproj`, as one read-only file (ADR 0006, "Export").
//!
//! [`Pak::pack`] writes every member of a project (the manifest, the asset index, scenes, scripts and assets) into a
//! pak. [`Pak::open`] reads the header and the index, then `metallic.toml` and `assets.json`; every other member is
//! read when it is asked for, with one seek and one read, and checked against its CRC-32. Nothing is unpacked to
//! disk. A [`Pak`] is read through [`GameFiles`], like a [`Project`], so the engine runs a game the same from either;
//! its [`AssetReader`] reads assets by id from the pak.
//!
//! # Layout (format version 1)
//!
//! Every number is little-endian.
//!
//! | Bytes | What |
//! |---|---|
//! | 0..8 | magic: `MTLCPAK` and a NUL byte |
//! | 8..12 | format version ([`PAK_FORMAT`]) |
//! | 12..16 | number of members |
//! | 16..24 | where the index starts |
//! | 24..28 | the index's length in bytes |
//! | 28..32 | the index's CRC-32 |
//! | 32.. | the members' bytes, each as it is in the project (not compressed), each starting at a multiple of 16 bytes (zero bytes between) |
//! | then | the index, at a multiple of 16 bytes, up to the end of the file |
//!
//! The index has one entry per member, sorted by path (byte order), with no gaps between entries:
//!
//! | Bytes | What |
//! |---|---|
//! | 8 | where the member's bytes start |
//! | 8 | the member's length in bytes |
//! | 4 | the member's CRC-32 |
//! | 1 | flags: bit 0 set when the member is an asset with an id; the other bits are 0 |
//! | 16 | the asset id, most significant byte first (as its hex digits read); zero without one |
//! | 2 | the path's length in bytes |
//! | n | the member path, UTF-8 |
//!
//! Packing the same project twice gives the same bytes: members are sorted, nothing records a date or a machine.
//!
//! # Errors
//!
//! A file that is not a pak, a pak from a newer engine, a truncated pak, an index entry pointing outside the data and a
//! member whose bytes do not match its checksum are each a clear [`PakError`] (inside [`ProjectError::Pak`]), never a
//! panic and never wrong data.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::asset_index::{AssetId, AssetIndex, INDEX_FILE};
use crate::assets::AssetReader;
use crate::game_files::GameFiles;
use crate::manifest::{MANIFEST_FILE, Manifest, ManifestWarning};
use crate::project::{Project, ProjectError, TEMP_SUFFIX, check_member_path};

/// The first 8 bytes of every pak.
pub const PAK_MAGIC: [u8; 8] = *b"MTLCPAK\0";

/// The pak format version this engine writes, and the newest it reads.
pub const PAK_FORMAT: u32 = 1;

/// The usual file name of a built game, shipped next to the player.
pub const PAK_FILE: &str = "game.pak";

/// The header's length in bytes; the first member starts here.
const HEADER_LEN: u64 = 32;

/// Every member, and the index, starts at a multiple of this many bytes, so a member can later be used in place
/// (memory-mapped, uploaded) without copying it to align it.
const ALIGN: u64 = 16;

/// An index entry's flag: the member is an asset with an id.
const HAS_ID: u8 = 1;

/// Why a file is not a pak this engine reads, or a member of it is damaged.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PakError {
    /// The file does not start with [`PAK_MAGIC`].
    #[error("not a game.pak (it does not start with the pak's magic bytes)")]
    NotAPak,
    /// The pak was made by a newer engine, in a format this one does not read.
    #[error("made by a newer engine: pak format version {version}, this engine reads up to version {supported}")]
    NewerVersion {
        /// The pak's format version.
        version: u32,
        /// The newest this engine reads ([`PAK_FORMAT`]).
        supported: u32,
    },
    /// The file ends before the part of it the header says is there.
    #[error("truncated: the file has {size} bytes, its {part} needs {needed}")]
    Truncated {
        /// The file's length.
        size: u64,
        /// What does not fit: the header or the index.
        part: &'static str,
        /// The length the file needs to hold it.
        needed: u64,
    },
    /// An index entry puts a member outside the members' bytes (in the header, in the index or past the end).
    #[error(
        "the index puts `{member}` at bytes {offset}..{}, outside the members' bytes ({HEADER_LEN}..{data_end})",
        offset.saturating_add(*length)
    )]
    OutOfRange {
        /// The member.
        member: String,
        /// Where the entry says its bytes start.
        offset: u64,
        /// Its length.
        length: u64,
        /// Where the members' bytes end (the index starts).
        data_end: u64,
    },
    /// A member's bytes do not match the CRC-32 the index records: the file is damaged.
    #[error("`{member}` is damaged: its CRC-32 is {found:08x}, the index says {expected:08x}")]
    Checksum {
        /// The member.
        member: String,
        /// The CRC-32 the index records.
        expected: u32,
        /// The CRC-32 of the bytes read.
        found: u32,
    },
    /// The header or the index is malformed.
    #[error("damaged: {reason}")]
    Damaged {
        /// What is wrong.
        reason: String,
    },
}

impl PakError {
    fn damaged(reason: impl Into<String>) -> Self {
        Self::Damaged { reason: reason.into() }
    }
}

/// A built game: an open `game.pak`. Read it through [`GameFiles`]. See the [module documentation](self).
pub struct Pak {
    file: PakFile,
    manifest: Manifest,
    warnings: Vec<ManifestWarning>,
    assets: AssetIndex,
}

impl Pak {
    /// Packs every member of `project` into a pak at `path`: what [`Project::read`] gives, unsaved edits included,
    /// and the asset ids of its index as of the last [`Project::refresh_assets`].
    ///
    /// Written like a save: a complete file beside `path` (`game.pak.tmp`), flushed to disk, then renamed over it, so a
    /// failed pack leaves what was at `path` as it was.
    pub fn pack(project: &mut Project, path: impl AsRef<Path>) -> Result<(), ProjectError> {
        let path = path.as_ref();
        let mut temp = path.as_os_str().to_owned();
        temp.push(TEMP_SUFFIX);
        let temp = PathBuf::from(temp);
        let result = write_pak(project, &temp, path).and_then(|()| {
            fs::rename(&temp, path).map_err(|source| ProjectError::Save {
                path: path.to_path_buf(),
                source,
            })
        });
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }

    /// Opens the pak at `path`: reads and checks its header and index, then its `metallic.toml` and `assets.json` (a
    /// pak without one has an empty asset index).
    ///
    /// Fails with [`ProjectError::Pak`] when the file is not a pak, is from a newer engine, is truncated or has a
    /// damaged index, and as [`Project::open`] does when a member path is unsafe or the manifest or asset index is
    /// refused.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path).map_err(|source| ProjectError::Io {
            path: path.clone(),
            source,
        })?;
        let entries = read_entries(&path, &mut file)?;
        let mut pak = Self {
            file: PakFile {
                path,
                entries: Arc::new(entries),
                file: Some(file),
            },
            manifest: Manifest::new(""),
            warnings: Vec::new(),
            assets: AssetIndex::new(),
        };
        let bytes = pak.read(MANIFEST_FILE).map_err(|error| match error {
            ProjectError::MissingMember { path, .. } => ProjectError::MissingManifest { path },
            other => other,
        })?;
        let loaded = Manifest::from_bytes(&bytes).map_err(|source| ProjectError::Manifest {
            path: pak.file.path.clone(),
            source,
        })?;
        check_member_path(&loaded.manifest.start_scene).map_err(|reason| ProjectError::UnsafeMember {
            path: pak.file.path.clone(),
            member: loaded.manifest.start_scene.clone(),
            reason,
        })?;
        pak.manifest = loaded.manifest;
        pak.warnings = loaded.warnings;
        if pak.file.entries.members.contains_key(INDEX_FILE) {
            let bytes = pak.read(INDEX_FILE)?;
            pak.assets = AssetIndex::from_bytes(&bytes).map_err(|source| ProjectError::AssetIndex {
                path: pak.file.path.clone(),
                source,
            })?;
        }
        Ok(pak)
    }

    /// What was skipped in `metallic.toml` when the pak was opened (the caller reports it).
    pub fn warnings(&self) -> &[ManifestWarning] {
        &self.warnings
    }

    /// The member path and the bytes of the asset `id`, found by the id its index entry records.
    pub fn read_id(&mut self, id: AssetId) -> Result<(String, Vec<u8>), ProjectError> {
        let member = self
            .file
            .entries
            .ids
            .get(&id)
            .cloned()
            .ok_or_else(|| ProjectError::UnknownAsset {
                path: self.file.path.clone(),
                id,
            })?;
        let bytes = self.read(&member)?;
        Ok((member, bytes))
    }
}

impl GameFiles for Pak {
    fn path(&self) -> &Path {
        &self.file.path
    }

    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn members(&self) -> Vec<&str> {
        self.file.entries.members.keys().map(String::as_str).collect()
    }

    fn read(&mut self, member: &str) -> Result<Vec<u8>, ProjectError> {
        self.file.read(member).map_err(|error| match error {
            ReadError::Missing => ProjectError::MissingMember {
                path: self.file.path.clone(),
                member: member.to_owned(),
            },
            ReadError::Io(source) => ProjectError::ReadMember {
                path: self.file.path.clone(),
                member: member.to_owned(),
                source,
            },
            ReadError::Damaged(source) => ProjectError::Pak {
                path: self.file.path.clone(),
                source,
            },
        })
    }

    fn assets(&self) -> &AssetIndex {
        &self.assets
    }

    fn asset_reader(&self) -> AssetReader {
        AssetReader::from_pak(self.assets.clone(), self.file.reopen())
    }
}

impl std::fmt::Debug for Pak {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pak")
            .field("path", &self.file.path)
            .field("manifest", &self.manifest)
            .field("members", &self.file.entries.members.len())
            .finish_non_exhaustive()
    }
}

/// Where one member's bytes are, and what they are.
#[derive(Clone, Copy, Debug)]
struct Entry {
    offset: u64,
    length: u64,
    crc32: u32,
}

/// A pak's index, read and checked.
#[derive(Debug, Default)]
struct Entries {
    members: BTreeMap<String, Entry>,
    /// The asset members, by the id their entry records.
    ids: BTreeMap<AssetId, String>,
}

/// Reads a pak's members by path: [`Pak`]'s, and an [`AssetReader`]'s on another thread, which share the index and
/// open the file each on their own.
pub(crate) struct PakFile {
    path: PathBuf,
    entries: Arc<Entries>,
    /// Opened on first use.
    file: Option<File>,
}

/// Why a member could not be read from a pak.
pub(crate) enum ReadError {
    /// The pak has no such member.
    Missing,
    /// The file could not be read.
    Io(io::Error),
    /// The member's bytes are damaged.
    Damaged(PakError),
}

impl From<ReadError> for io::Error {
    fn from(error: ReadError) -> Self {
        match error {
            ReadError::Missing => io::ErrorKind::NotFound.into(),
            ReadError::Io(error) => error,
            ReadError::Damaged(error) => io::Error::new(io::ErrorKind::InvalidData, error),
        }
    }
}

impl PakFile {
    /// The pak's file.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Another reader of the same pak, with its own file handle (opened on first use).
    fn reopen(&self) -> Self {
        Self {
            path: self.path.clone(),
            entries: Arc::clone(&self.entries),
            file: None,
        }
    }

    /// The bytes of `member`, checked against its CRC-32.
    pub(crate) fn read(&mut self, member: &str) -> Result<Vec<u8>, ReadError> {
        let entry = *self.entries.members.get(member).ok_or(ReadError::Missing)?;
        let file = match self.file.as_mut() {
            Some(file) => file,
            None => self.file.insert(File::open(&self.path).map_err(ReadError::Io)?),
        };
        // The index was checked against the file's length when it was opened, so this is not an untrusted size.
        let length = usize::try_from(entry.length)
            .map_err(|_| ReadError::Io(io::Error::new(io::ErrorKind::OutOfMemory, "the member is too large")))?;
        let mut bytes = vec![0; length];
        file.seek(SeekFrom::Start(entry.offset)).map_err(ReadError::Io)?;
        file.read_exact(&mut bytes).map_err(ReadError::Io)?;
        let found = crc32fast::hash(&bytes);
        if found != entry.crc32 {
            return Err(ReadError::Damaged(PakError::Checksum {
                member: member.to_owned(),
                expected: entry.crc32,
                found,
            }));
        }
        Ok(bytes)
    }
}

/// Reads and checks the header and the index of the pak `file` (at `path`).
fn read_entries(path: &Path, file: &mut File) -> Result<Entries, ProjectError> {
    let io = |source| ProjectError::Io {
        path: path.to_path_buf(),
        source,
    };
    let pak = |source| ProjectError::Pak {
        path: path.to_path_buf(),
        source,
    };
    let size = file.metadata().map_err(io)?.len();
    let mut header = [0; HEADER_LEN as usize];
    let have = header.len().min(usize::try_from(size).unwrap_or(usize::MAX));
    file.read_exact(&mut header[..have]).map_err(io)?;
    let magic = have.min(PAK_MAGIC.len());
    if header[..magic] != PAK_MAGIC[..magic] {
        return Err(pak(PakError::NotAPak));
    }
    if size < HEADER_LEN {
        return Err(pak(PakError::Truncated {
            size,
            part: "header",
            needed: HEADER_LEN,
        }));
    }
    let mut fields = Fields(&header[PAK_MAGIC.len()..]);
    let (version, count, index_offset, index_len, index_crc) =
        match (fields.u32(), fields.u32(), fields.u64(), fields.u32(), fields.u32()) {
            (Some(a), Some(b), Some(c), Some(d), Some(e)) => (a, b, c, d, e),
            _ => return Err(pak(PakError::damaged("the header is too short"))),
        };
    if version > PAK_FORMAT {
        return Err(pak(PakError::NewerVersion {
            version,
            supported: PAK_FORMAT,
        }));
    }
    if version == 0 {
        return Err(pak(PakError::damaged("format version 0 does not exist")));
    }
    if index_offset < HEADER_LEN {
        return Err(pak(PakError::damaged("the header puts the index inside itself")));
    }
    let index_end = index_offset
        .checked_add(u64::from(index_len))
        .ok_or_else(|| pak(PakError::damaged("the header puts the index past any file's end")))?;
    if index_end > size {
        return Err(pak(PakError::Truncated {
            size,
            part: "index",
            needed: index_end,
        }));
    }
    if index_end < size {
        return Err(pak(PakError::damaged(format!(
            "{} bytes follow the index, which ends the file",
            size - index_end
        ))));
    }
    let mut index = vec![0; index_len as usize];
    file.seek(SeekFrom::Start(index_offset)).map_err(io)?;
    file.read_exact(&mut index).map_err(io)?;
    let found = crc32fast::hash(&index);
    if found != index_crc {
        return Err(pak(PakError::damaged(format!(
            "the index's CRC-32 is {found:08x}, the header says {index_crc:08x}"
        ))));
    }

    let mut entries = Entries::default();
    let mut fields = Fields(&index);
    let mut last: Option<String> = None;
    for i in 0..count {
        let short = || pak(PakError::damaged(format!("the index ends in the middle of entry {i}")));
        let offset = fields.u64().ok_or_else(short)?;
        let length = fields.u64().ok_or_else(short)?;
        let crc32 = fields.u32().ok_or_else(short)?;
        let flags = fields.take(1).ok_or_else(short)?[0];
        let id = fields.take(16).ok_or_else(short)?;
        let id = u128::from_be_bytes(id.try_into().map_err(|_| short())?);
        let path_len = fields.u16().ok_or_else(short)?;
        let member = fields.take(usize::from(path_len)).ok_or_else(short)?;
        let member = String::from_utf8(member.to_vec())
            .map_err(|_| pak(PakError::damaged(format!("entry {i}'s path is not UTF-8"))))?;
        check_member_path(&member).map_err(|reason| ProjectError::UnsafeMember {
            path: path.to_path_buf(),
            member: member.clone(),
            reason,
        })?;
        if let Some(last) = &last
            && member.as_str() <= last.as_str()
        {
            return Err(pak(PakError::damaged(format!(
                "the index is not sorted by path, or lists a member twice: `{member}` after `{last}`"
            ))));
        }
        if flags & !HAS_ID != 0 {
            return Err(pak(PakError::damaged(format!(
                "`{member}` has flags {flags:#04x}, which this engine does not know"
            ))));
        }
        if offset < HEADER_LEN || offset.checked_add(length).is_none_or(|end| end > index_offset) {
            return Err(pak(PakError::OutOfRange {
                member,
                offset,
                length,
                data_end: index_offset,
            }));
        }
        if flags & HAS_ID != 0 {
            let id = AssetId::from_u128(id);
            if let Some(other) = entries.ids.insert(id, member.clone()) {
                return Err(pak(PakError::damaged(format!(
                    "`{other}` and `{member}` have the same asset id {id}"
                ))));
            }
        }
        entries.members.insert(member.clone(), Entry { offset, length, crc32 });
        last = Some(member);
    }
    if !fields.0.is_empty() {
        return Err(pak(PakError::damaged(format!(
            "{} bytes follow the index's last entry",
            fields.0.len()
        ))));
    }
    Ok(entries)
}

/// Little-endian fields read off the front of a byte slice.
struct Fields<'a>(&'a [u8]);

impl<'a> Fields<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(head)
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
}

/// Writes the pak of `project` to `temp` and flushes it to disk; errors name `pak`, the file it is for.
fn write_pak(project: &mut Project, temp: &Path, pak: &Path) -> Result<(), ProjectError> {
    let save = |source: io::Error| ProjectError::Save {
        path: pak.to_path_buf(),
        source,
    };
    let too_large = |what: String| save(io::Error::new(io::ErrorKind::InvalidInput, what));
    let members: Vec<String> = project.members().into_iter().map(str::to_owned).collect();
    let count = u32::try_from(members.len()).map_err(|_| too_large("more members than a pak holds".to_owned()))?;
    let mut out = BufWriter::new(File::create(temp).map_err(save)?);
    // The header is written last, once the index's place is known.
    out.write_all(&[0; HEADER_LEN as usize]).map_err(save)?;
    let mut at = HEADER_LEN;
    let mut index = Vec::new();
    for member in &members {
        let bytes = project.read(member)?;
        let path_len = u16::try_from(member.len())
            .map_err(|_| too_large(format!("the member path `{member}` is longer than 65535 bytes")))?;
        let id = project.assets().id(member);
        index.extend_from_slice(&at.to_le_bytes());
        index.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        index.extend_from_slice(&crc32fast::hash(&bytes).to_le_bytes());
        index.push(if id.is_some() { HAS_ID } else { 0 });
        index.extend_from_slice(&id.map_or(0, AssetId::to_u128).to_be_bytes());
        index.extend_from_slice(&path_len.to_le_bytes());
        index.extend_from_slice(member.as_bytes());
        out.write_all(&bytes).map_err(save)?;
        at += bytes.len() as u64;
        at = pad(&mut out, at).map_err(save)?;
    }
    let index_len = u32::try_from(index.len()).map_err(|_| too_large("the index is larger than 4 GiB".to_owned()))?;
    out.write_all(&index).map_err(save)?;
    let mut header = Vec::with_capacity(HEADER_LEN as usize);
    header.extend_from_slice(&PAK_MAGIC);
    header.extend_from_slice(&PAK_FORMAT.to_le_bytes());
    header.extend_from_slice(&count.to_le_bytes());
    header.extend_from_slice(&at.to_le_bytes());
    header.extend_from_slice(&index_len.to_le_bytes());
    header.extend_from_slice(&crc32fast::hash(&index).to_le_bytes());
    out.seek(SeekFrom::Start(0)).map_err(save)?;
    out.write_all(&header).map_err(save)?;
    let file = out.into_inner().map_err(|error| save(error.into_error()))?;
    // On disk before the rename, so a crash cannot leave a renamed but empty pak.
    file.sync_all().map_err(save)
}

/// Writes zero bytes after `at` up to the next multiple of [`ALIGN`], and returns it.
fn pad(out: &mut impl Write, at: u64) -> io::Result<u64> {
    let padding = (ALIGN - at % ALIGN) % ALIGN;
    out.write_all(&[0; ALIGN as usize][..padding as usize])?;
    Ok(at + padding)
}
