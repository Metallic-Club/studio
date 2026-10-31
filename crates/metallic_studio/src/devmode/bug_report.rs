//! Report a bug: one zip with what a Studio developer needs to look into a problem, written to
//! `%LOCALAPPDATA%\MetallicStudio\bug-reports\<timestamp>.zip`; Studio then opens the folder so
//! the creator can attach the file wherever they report it. Nothing is uploaded.
//!
//! The zip holds `version.txt` (Studio version, channel, GPU, OS), `log.txt` (the captured log,
//! see [`super::log_view`]), `output.txt` (the Output panel) and `screenshot.png` (the window,
//! when the capture worked). Every text entry goes through a [`Redactor`] first: the stored
//! GitHub session and anything shaped like a GitHub token become `[redacted]`, and the home
//! folder becomes `~`.
//!
//! The zip is written here (stored entries with CRC-32), so this needs no compression crate.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui::{self, ColorImage};

/// The folder name under `%LOCALAPPDATA%\MetallicStudio`.
pub const FOLDER: &str = "bug-reports";
/// What a removed secret is replaced with.
pub const REDACTED: &str = "[redacted]";
/// Prefixes of GitHub tokens; a word starting with one of these is removed.
pub const TOKEN_PREFIXES: &[&str] = &["github_pat_", "ghp_", "gho_", "ghu_", "ghs_", "ghr_"];
/// Shortest stored secret that is looked for (shorter ones would remove ordinary words).
pub const MIN_SECRET: usize = 8;

/// What goes into a report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BugReport {
    /// Studio version ("0.1.1").
    pub version: String,
    /// Build channel ("dev").
    pub channel: String,
    /// GPU adapter name and backend, or "none".
    pub gpu: String,
    /// Operating system and architecture.
    pub os: String,
    /// The captured log.
    pub log: String,
    /// The Output panel's lines.
    pub output: String,
    /// The window as a PNG, when the capture worked.
    pub screenshot_png: Option<Vec<u8>>,
}

/// Removes secrets and personal paths from report text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Redactor {
    secrets: Vec<String>,
    home: Option<String>,
}

impl Redactor {
    /// Removes each of `secrets` (those shorter than [`MIN_SECRET`] are ignored) and anything
    /// shaped like a GitHub token; replaces `home` (the user's folder) with `~`.
    pub fn new(secrets: Vec<String>, home: Option<String>) -> Redactor {
        let mut secrets: Vec<String> = secrets
            .into_iter()
            .filter(|s| s.len() >= MIN_SECRET)
            .collect();
        // Longest first, so a secret containing another goes in one piece.
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        secrets.dedup();
        Redactor {
            secrets,
            home: home.filter(|h| h.len() > 3),
        }
    }

    /// The Redactor for this machine: the stored GitHub session (and every long string inside
    /// it) and the home folder.
    pub fn for_this_machine() -> Redactor {
        use crate::github_auth::SecretStore;
        let stored = crate::github_auth::studio_secret_store()
            .get()
            .ok()
            .flatten()
            .map(|s| secrets_in_stored(&s))
            .unwrap_or_default();
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok();
        Redactor::new(stored, home)
    }

    /// `text` without the secrets.
    pub fn apply(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for s in &self.secrets {
            if out.contains(s.as_str()) {
                out = out.replace(s.as_str(), REDACTED);
            }
        }
        out = remove_token_shapes(&out);
        if let Some(home) = &self.home {
            out = out.replace(home.as_str(), "~");
            let forward = home.replace('\\', "/");
            if forward != *home {
                out = out.replace(forward.as_str(), "~");
            }
        }
        out
    }
}

/// The secrets inside a stored session: the whole text, and every quoted string of
/// [`MIN_SECRET`] or more characters in it (the access and refresh tokens of a JSON session).
pub fn secrets_in_stored(stored: &str) -> Vec<String> {
    let mut out = vec![stored.trim().to_owned()];
    let mut rest = stored;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('"') else { break };
        let inner = &after[..end];
        if inner.len() >= MIN_SECRET {
            out.push(inner.to_owned());
        }
        rest = &after[end + 1..];
    }
    out
}

fn remove_token_shapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < text.len() {
        let here = &text[i..];
        let starts_word = i == 0 || !is_token_byte(bytes[i - 1]);
        if starts_word && let Some(p) = TOKEN_PREFIXES.iter().find(|p| here.starts_with(**p)) {
            let len = here.bytes().take_while(|&b| is_token_byte(b)).count();
            if len > p.len() + 4 {
                out.push_str(REDACTED);
                i += len;
                continue;
            }
        }
        let c = here.chars().next().unwrap_or(' ');
        out.push(c);
        i += c.len_utf8();
    }
    out
}

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

impl BugReport {
    /// The text of `version.txt`.
    pub fn version_text(&self) -> String {
        format!(
            "Metallic Studio {}\nChannel: {}\nGPU: {}\nOS: {}\n",
            self.version, self.channel, self.gpu, self.os
        )
    }

    /// The zip's entries, text entries redacted by `redactor`.
    pub fn entries(&self, redactor: &Redactor) -> Vec<(String, Vec<u8>)> {
        let mut out = vec![
            (
                "version.txt".to_owned(),
                redactor.apply(&self.version_text()).into_bytes(),
            ),
            ("log.txt".to_owned(), redactor.apply(&self.log).into_bytes()),
            (
                "output.txt".to_owned(),
                redactor.apply(&self.output).into_bytes(),
            ),
        ];
        if let Some(png) = &self.screenshot_png {
            out.push(("screenshot.png".to_owned(), png.clone()));
        }
        out
    }

    /// Writes `<dir>/<stamp>.zip` (`<stamp>-2.zip` and so on when it exists) and returns its
    /// path. `unix_secs` dates the entries.
    pub fn write(
        &self,
        dir: &Path,
        unix_secs: u64,
        redactor: &Redactor,
    ) -> Result<PathBuf, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let stamp = timestamp(unix_secs);
        let mut path = dir.join(format!("{stamp}.zip"));
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("{stamp}-{n}.zip"));
            n += 1;
        }
        let bytes = zip_stored(&self.entries(redactor), dos_time(unix_secs));
        let tmp = path.with_extension("zip.tmp");
        std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })?;
        Ok(path)
    }
}

/// How long Report a bug waits for the window capture before writing the report without it.
pub const SCREENSHOT_WAIT: Duration = Duration::from_secs(2);

/// A report waiting for its window capture.
pub struct ReportJob {
    started: Instant,
    image: Receiver<Arc<ColorImage>>,
}

impl ReportJob {
    /// Asks egui for a capture of the window (it arrives after the next frame is drawn).
    pub fn start(ctx: &egui::Context) -> ReportJob {
        let (tx, rx) = channel();
        ctx.request_screenshot(move |image| {
            let _ = tx.send(image);
        });
        ctx.request_repaint();
        ReportJob {
            started: Instant::now(),
            image: rx,
        }
    }

    /// `Some` once the job is done: the capture as PNG bytes, or `None` inside when it did not
    /// come within [`SCREENSHOT_WAIT`] (or could not be encoded).
    pub fn poll(&self) -> Option<Option<Vec<u8>>> {
        if let Ok(image) = self.image.try_recv() {
            return Some(crate::screenshot::png_bytes(&image).ok());
        }
        (self.started.elapsed() >= SCREENSHOT_WAIT).then_some(None)
    }
}

/// Seconds since 1970 now.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Where reports go: `%LOCALAPPDATA%\MetallicStudio\bug-reports`, else the config folder's
/// `bug-reports`. `None` if neither is known.
pub fn reports_dir() -> Option<PathBuf> {
    reports_dir_in(
        std::env::var_os("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from),
        crate::recent::config_dir(),
    )
}

/// [`reports_dir`] with the folders passed in.
pub fn reports_dir_in(local_app_data: Option<PathBuf>, config: Option<PathBuf>) -> Option<PathBuf> {
    match local_app_data {
        Some(local) => Some(crate::data_dirs::config_dir_in(&local).join(FOLDER)),
        None => config.map(|c| c.join(FOLDER)),
    }
}

/// Opens `dir` in the system file manager.
pub fn open_folder(dir: &Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(program).arg(dir).spawn() {
        log::warn!("could not open the bug reports folder: {e}");
    }
}

/// (year, month, day, hour, minute, second) in UTC.
fn civil(unix_secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (unix_secs / 86_400) as i64;
    let secs = unix_secs % 86_400;
    // Days to civil date (proleptic Gregorian), after H. Hinnant's public-domain algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (
        year,
        month,
        day,
        (secs / 3600) as u32,
        (secs / 60 % 60) as u32,
        (secs % 60) as u32,
    )
}

/// "2026-10-31_14-05-09" (UTC): the report's file name.
pub fn timestamp(unix_secs: u64) -> String {
    let (y, mo, d, h, mi, s) = civil(unix_secs);
    format!("{y:04}-{mo:02}-{d:02}_{h:02}-{mi:02}-{s:02}")
}

/// MS-DOS (time, date) as zip headers store them; years before 1980 become 1980.
fn dos_time(unix_secs: u64) -> (u16, u16) {
    let (y, mo, d, h, mi, s) = civil(unix_secs);
    let y = (y - 1980).clamp(0, 127) as u16;
    let time = ((h as u16) << 11) | ((mi as u16) << 5) | (s as u16 / 2);
    let date = (y << 9) | ((mo as u16) << 5) | d as u16;
    (time, date)
}

/// CRC-32 (IEEE, as zip uses).
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// A zip of `entries` stored without compression, names in UTF-8.
pub fn zip_stored(entries: &[(String, Vec<u8>)], (time, date): (u16, u16)) -> Vec<u8> {
    fn u16le(out: &mut Vec<u8>, v: u16) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    fn u32le(out: &mut Vec<u8>, v: u32) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    const UTF8_NAMES: u16 = 1 << 11;
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        let name_len = name.len() as u16;
        // Local file header.
        u32le(&mut out, 0x0403_4b50);
        u16le(&mut out, 20);
        u16le(&mut out, UTF8_NAMES);
        u16le(&mut out, 0);
        u16le(&mut out, time);
        u16le(&mut out, date);
        u32le(&mut out, crc);
        u32le(&mut out, size);
        u32le(&mut out, size);
        u16le(&mut out, name_len);
        u16le(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        // Central directory record.
        u32le(&mut central, 0x0201_4b50);
        u16le(&mut central, 20);
        u16le(&mut central, 20);
        u16le(&mut central, UTF8_NAMES);
        u16le(&mut central, 0);
        u16le(&mut central, time);
        u16le(&mut central, date);
        u32le(&mut central, crc);
        u32le(&mut central, size);
        u32le(&mut central, size);
        u16le(&mut central, name_len);
        u16le(&mut central, 0);
        u16le(&mut central, 0);
        u16le(&mut central, 0);
        u16le(&mut central, 0);
        u32le(&mut central, 0);
        u32le(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let central_at = out.len() as u32;
    let central_len = central.len() as u32;
    out.extend_from_slice(&central);
    // End of central directory.
    u32le(&mut out, 0x0605_4b50);
    u16le(&mut out, 0);
    u16le(&mut out, 0);
    u16le(&mut out, entries.len() as u16);
    u16le(&mut out, entries.len() as u16);
    u32le(&mut out, central_len);
    u32le(&mut out, central_at);
    u16le(&mut out, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn timestamps_are_utc_calendar_dates() {
        assert_eq!(timestamp(0), "1970-01-01_00-00-00");
        // 2026-10-31 14:05:09 UTC.
        assert_eq!(timestamp(1_793_455_509), "2026-10-31_14-05-09");
        assert_eq!(timestamp(951_782_400), "2000-02-29_00-00-00");
    }

    #[test]
    fn token_shapes_and_home_are_removed() {
        let r = Redactor::new(vec![], Some(r"C:\Users\someone".into()));
        let text =
            r"token ghp_abcdefghijklmnop0123 in C:\Users\someone\game and C:/Users/someone/x";
        let out = r.apply(text);
        assert!(!out.contains("ghp_abc"), "{out}");
        assert!(!out.contains("someone"), "{out}");
        assert!(out.contains(r"~\game") && out.contains("~/x"), "{out}");
        assert_eq!(r.apply("ghp_ alone"), "ghp_ alone", "a bare prefix is kept");
    }

    #[test]
    fn stored_json_sessions_give_up_each_token() {
        let s = secrets_in_stored(
            r#"{"access":"abcdefgh12345678","refresh":"zyxwvuts98765432","n":"x"}"#,
        );
        assert!(s.contains(&"abcdefgh12345678".to_owned()));
        assert!(s.contains(&"zyxwvuts98765432".to_owned()));
        assert!(!s.contains(&"x".to_owned()));
    }

    #[test]
    fn reports_go_under_local_app_data() {
        let dir = reports_dir_in(Some(PathBuf::from("L")), Some(PathBuf::from("C")));
        assert_eq!(
            dir,
            Some(Path::new("L").join("MetallicStudio").join(FOLDER))
        );
        let dir = reports_dir_in(None, Some(PathBuf::from("C")));
        assert_eq!(dir, Some(Path::new("C").join(FOLDER)));
    }
}
