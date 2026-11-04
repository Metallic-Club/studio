//! The pipeline cache on disk (ADR 0008 §9, #86): where the backend can keep compiled pipelines (Vulkan's
//! `VkPipelineCache`, wgpu's [`wgpu::Features::PIPELINE_CACHE`]), every pipeline is made through one cache per
//! device, loaded from a file in the folder the app names ([`crate::ContextOptions::pipeline_cache_dir`], the
//! user's cache folder from `metallic_platform`) and written back after compiling, on a compile worker, never in a
//! frame.
//!
//! The file is named after the adapter's vendor and device, as wgpu's own key ([`wgpu::util::pipeline_cache_key`]);
//! wgpu checks the driver and its validation key inside the data and starts empty when they changed (a driver
//! update), and the next save replaces the file.
//!
//! # The file
//!
//! A 32-byte header, then exactly what [`wgpu::PipelineCache::get_data`] returned:
//!
//! | Bytes | |
//! | --- | --- |
//! | 0–7 | `METALPSO` |
//! | 8–11 | format version (1), little-endian |
//! | 12–15 | zero |
//! | 16–23 | the data's length, little-endian |
//! | 24–31 | the data's FNV-1a 64-bit hash, little-endian |
//!
//! A file whose header, length or hash does not match is ignored, so a torn write or a damaged file never reaches
//! the driver. Saving writes a temporary file beside it and renames it over the old one.
//!
//! Creating a cache from bytes is `unsafe` in wgpu: the data must be what `get_data` returned. This module is the
//! only place in the crate that does it (`#![allow(unsafe_code)]` here, with a `// SAFETY:` comment on the block).

#![allow(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

const MAGIC: [u8; 8] = *b"METALPSO";
const VERSION: u32 = 1;
const HEADER: usize = 32;

/// A device's pipeline cache and the file it is kept in.
pub(crate) struct PipelineCacheStore {
    cache: wgpu::PipelineCache,
    path: PathBuf,
    /// Bytes loaded from the file at start (0: none, or not usable).
    loaded: u64,
    /// Compiles changed the cache since it was last written.
    dirty: AtomicBool,
    /// One save at a time.
    saving: Mutex<()>,
}

impl std::fmt::Debug for PipelineCacheStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipelineCacheStore")
            .field("path", &self.path)
            .field("loaded", &self.loaded)
            .finish_non_exhaustive()
    }
}

impl PipelineCacheStore {
    /// The cache for `device` on `adapter`, filled from its file in `dir` when there is a usable one. `None` where
    /// the device has no pipeline cache (every backend but Vulkan) or wgpu has no key for the adapter.
    pub(crate) fn open(adapter: &wgpu::Adapter, device: &wgpu::Device, dir: &Path) -> Option<Self> {
        if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
            return None;
        }
        let key = wgpu::util::pipeline_cache_key(&adapter.get_info())?;
        let path = dir.join(format!("{key}.bin"));
        let data = std::fs::read(&path)
            .ok()
            .and_then(|file| unwrap_file(&file).map(<[u8]>::to_vec));
        // SAFETY: `data` is exactly the payload of a file this module wrote from `PipelineCache::get_data`: the
        // header's length and FNV-1a hash of it are checked above, so a torn or damaged file is never passed. The
        // file is keyed by `pipeline_cache_key` (vendor and device), and wgpu checks its own header inside the data
        // (adapter key, driver version and validation key) before the driver sees any of it; with `fallback`, data
        // it cannot use gives an empty cache instead of an error.
        let cache = unsafe {
            device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                label: Some("metallic pipelines"),
                data: data.as_deref(),
                fallback: true,
            })
        };
        Some(Self {
            cache,
            path,
            loaded: data.map_or(0, |data| data.len() as u64),
            dirty: AtomicBool::new(false),
            saving: Mutex::new(()),
        })
    }

    /// The cache to make pipelines with.
    pub(crate) fn cache(&self) -> &wgpu::PipelineCache {
        &self.cache
    }

    /// Bytes loaded from the file at start (0 when there was none, or it could not be used).
    pub(crate) fn loaded(&self) -> u64 {
        self.loaded
    }

    /// Notes that compiles added to the cache.
    pub(crate) fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// Writes the cache when compiles added to it since the last write.
    pub(crate) fn save_if_dirty(&self) {
        if self.dirty.swap(false, Ordering::Relaxed) {
            self.write();
        }
    }

    /// Writes the cache to its file now. Failing to write a cache only costs the next start its compiles, so errors
    /// are ignored.
    pub(crate) fn save(&self) {
        self.dirty.store(false, Ordering::Relaxed);
        self.write();
    }

    fn write(&self) {
        let _one_at_a_time = self.saving.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(data) = self.cache.get_data() else {
            return;
        };
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Several renderers (or processes) may save at once: each writes its own temporary file, and the last rename
        // wins with a whole file.
        static SAVES: AtomicU32 = AtomicU32::new(0);
        let unique = SAVES.fetch_add(1, Ordering::Relaxed);
        let temporary = self.path.with_extension(format!("{}-{unique}.tmp", std::process::id()));
        if std::fs::write(&temporary, wrap_file(&data)).is_ok() && std::fs::rename(&temporary, &self.path).is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
    }
}

/// The FNV-1a 64-bit hash of `data`.
fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// `data` with the file's header in front.
fn wrap_file(data: &[u8]) -> Vec<u8> {
    let mut file = Vec::with_capacity(HEADER + data.len());
    file.extend_from_slice(&MAGIC);
    file.extend_from_slice(&VERSION.to_le_bytes());
    file.extend_from_slice(&0u32.to_le_bytes());
    file.extend_from_slice(&(data.len() as u64).to_le_bytes());
    file.extend_from_slice(&fnv1a(data).to_le_bytes());
    file.extend_from_slice(data);
    file
}

/// The data inside a file, when its header is this format's and its length and hash match.
fn unwrap_file(file: &[u8]) -> Option<&[u8]> {
    let (header, data) = file.split_at_checked(HEADER)?;
    let word = |at: usize| u64::from_le_bytes(header[at..at + 8].try_into().unwrap_or([0; 8]));
    let version = u32::from_le_bytes(header[8..12].try_into().unwrap_or([0; 4]));
    let valid = header[..8] == MAGIC
        && version == VERSION
        && word(16) == data.len() as u64
        && word(24) == fnv1a(data)
        && !data.is_empty();
    valid.then_some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_round_trips_and_damage_is_refused() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
        let file = wrap_file(&data);
        assert_eq!(file.len(), HEADER + data.len());
        assert_eq!(unwrap_file(&file), Some(&data[..]));
        // Torn: shorter than its header says.
        assert_eq!(unwrap_file(&file[..file.len() - 1]), None);
        assert_eq!(unwrap_file(&file[..10]), None);
        // A flipped byte in the data, or in the magic.
        let mut flipped = file.clone();
        flipped[HEADER + 500] ^= 1;
        assert_eq!(unwrap_file(&flipped), None);
        let mut magic = file.clone();
        magic[0] = b'X';
        assert_eq!(unwrap_file(&magic), None);
        // Another version of the format.
        let mut version = file;
        version[8] = 2;
        assert_eq!(unwrap_file(&version), None);
        // Nothing to load.
        assert_eq!(unwrap_file(&wrap_file(&[])), None);
    }

    #[test]
    fn the_hash_is_fnv1a() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
