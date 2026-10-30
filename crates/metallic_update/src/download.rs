//! Downloading a release file and checking it before it is used.
//!
//! [`download_verified`] writes to `<name>.part`, stops when more bytes arrive than the release
//! says, compares the SHA-256 with the one GitHub published, and only then renames the file to
//! `<name>`. Any failure deletes the part file, and a `.part` left by a killed run is never
//! reused: the next download starts from nothing.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::feed::{ReleaseAsset, UpdateError};
use crate::http::{HttpClient, Token};

/// No download is larger than this, whatever the release says.
const MAX_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;
const USER_AGENT: &str = "metallic-launcher-updater";

/// Hashes what passes through and refuses to write past `limit` bytes.
struct CheckedWriter<W: Write> {
    inner: W,
    hasher: Sha256,
    written: u64,
    limit: u64,
}

impl<W: Write> Write for CheckedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.written + buf.len() as u64 > self.limit {
            return Err(io::Error::other("the download is larger than expected"));
        }
        let count = self.inner.write(buf)?;
        self.hasher.update(&buf[..count]);
        self.written += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// A file name with no folder part, so it cannot point outside the download folder.
fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':', '\0'])
        && !name.ends_with(".part")
}

/// Downloads `asset` into `dir` and returns the path of the verified file `<dir>/<name>`.
///
/// `progress` receives 0 to 100. The token, when given, is sent to the asset address and is
/// dropped by the client if the download is redirected to another host. On any failure no
/// `<name>` and no `<name>.part` is left behind; an earlier `<name>` or `.part` is replaced.
pub fn download_verified(
    http: &dyn HttpClient,
    token: Option<&Token>,
    asset: &ReleaseAsset,
    dir: &Path,
    mut progress: impl FnMut(u8),
) -> Result<PathBuf, UpdateError> {
    let install = |what: &str, error: &dyn std::fmt::Display| {
        UpdateError::Install(format!("{what}: {error}"))
    };
    if !is_plain_file_name(&asset.name) {
        return Err(UpdateError::BadResponse(
            "the release file has an unusable name".into(),
        ));
    }
    if asset.size == 0 || asset.size > MAX_DOWNLOAD_BYTES {
        return Err(UpdateError::BadResponse(
            "the release file has an unusable size".into(),
        ));
    }
    std::fs::create_dir_all(dir)
        .map_err(|e| install("could not create the download folder", &e))?;
    let final_path = dir.join(&asset.name);
    let part_path = dir.join(format!("{}.part", asset.name));
    // Whatever an earlier run left is not trusted and not reused.
    for stale in [&part_path, &final_path] {
        if let Err(error) = std::fs::remove_file(stale)
            && error.kind() != io::ErrorKind::NotFound
        {
            return Err(install("could not remove an old download", &error));
        }
    }

    let outcome = write_part(http, token, asset, &part_path, &mut progress).and_then(|()| {
        std::fs::rename(&part_path, &final_path)
            .map_err(|e| install("could not finish the download", &e))
    });
    match outcome {
        Ok(()) => Ok(final_path),
        Err(error) => {
            // Best effort: a part file that cannot be removed is still never used.
            let _ = std::fs::remove_file(&part_path);
            Err(error)
        }
    }
}

fn write_part(
    http: &dyn HttpClient,
    token: Option<&Token>,
    asset: &ReleaseAsset,
    part_path: &Path,
    progress: &mut impl FnMut(u8),
) -> Result<(), UpdateError> {
    let file = File::create_new(part_path)
        .map_err(|e| UpdateError::Install(format!("could not create the download file: {e}")))?;
    let mut writer = CheckedWriter {
        inner: file,
        hasher: Sha256::new(),
        written: 0,
        limit: asset.size,
    };
    let authorization = token.map(|token| format!("Bearer {}", token.expose()));
    let mut headers = vec![
        ("Accept", "application/octet-stream"),
        ("User-Agent", USER_AGENT),
    ];
    if let Some(value) = &authorization {
        headers.push(("Authorization", value.as_str()));
    }
    let mut last_percent = None;
    let mut report = |bytes: u64| {
        let percent = (bytes.saturating_mul(100) / asset.size).min(100) as u8;
        if last_percent != Some(percent) {
            last_percent = Some(percent);
            progress(percent);
        }
    };
    http.get_to_writer(&asset.api_url, &headers, &mut writer, &mut report)
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    writer
        .flush()
        .and_then(|()| writer.inner.sync_all())
        .map_err(|e| UpdateError::Install(format!("could not write the download: {e}")))?;
    if writer.written != asset.size {
        return Err(UpdateError::Install(
            "the download is smaller than expected".into(),
        ));
    }
    let digest: [u8; 32] = writer.hasher.finalize().into();
    if digest != asset.sha256 {
        return Err(UpdateError::Install(
            "the download does not match its SHA-256 and was deleted".into(),
        ));
    }
    Ok(())
}
