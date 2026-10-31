//! The "Recent games" list, stored as a small TOML file in the user's config folder.
//!
//! Loading never fails: a missing or corrupt file gives an empty list, and a failed save is only
//! logged, so a broken config can never stop Studio from starting.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How many recent games are remembered.
pub const MAX_RECENT: usize = 10;

/// One remembered game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentProject {
    /// Display name.
    pub name: String,
    /// The game folder.
    pub path: PathBuf,
    /// Id of the template the game was made from, when Studio knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// When the game was last created or opened (seconds since 1970, UTC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened: Option<u64>,
}

impl RecentProject {
    /// False when the folder (or its `metallic.toml`) is gone: the row stays, with a warning.
    pub fn exists(&self) -> bool {
        self.path.join(crate::templates::PROJECT_FILE).is_file()
    }
}

#[derive(Default, Serialize, Deserialize)]
struct RecentFile {
    #[serde(default)]
    project: Vec<RecentProject>,
}

/// The list of recent games, newest first, backed by one file.
#[derive(Debug, Clone, Default)]
pub struct RecentProjects {
    file: Option<PathBuf>,
    items: Vec<RecentProject>,
}

/// Where Studio keeps its settings: see [`crate::data_dirs::resolve_config_dir`]
/// (`%APPDATA%\MetallicStudio` on Windows).
pub fn config_dir() -> Option<PathBuf> {
    crate::data_dirs::resolve_config_dir(&|name| std::env::var_os(name))
}

impl RecentProjects {
    /// Loads the list from `<config dir>/recent.toml`.
    pub fn load_default() -> Self {
        Self::load(config_dir().map(|d| d.join("recent.toml")))
    }

    /// Loads the list from `file` (or an empty, unsaved list when `None`). Never fails.
    pub fn load(file: Option<PathBuf>) -> Self {
        let items = file
            .as_deref()
            .and_then(|f| fs::read_to_string(f).ok())
            .and_then(|text| match toml::from_str::<RecentFile>(&text) {
                Ok(parsed) => Some(parsed.project),
                Err(e) => {
                    log::warn!("ignoring unreadable recent games file: {}", e.message());
                    None
                }
            })
            .unwrap_or_default();
        Self { file, items }
    }

    /// True when changes are saved to a file (false for an in-memory list).
    pub fn is_persistent(&self) -> bool {
        self.file.is_some()
    }

    /// The remembered games, newest first.
    pub fn items(&self) -> &[RecentProject] {
        &self.items
    }

    /// An unsaved list holding `items` as given (screenshots and tests).
    pub fn in_memory(items: Vec<RecentProject>) -> Self {
        Self { file: None, items }
    }

    /// Moves (or adds) a game to the top, stamps it with the current time, trims the list and
    /// saves it. `template` is the id it was made from; `None` keeps what the list already knew.
    pub fn record(&mut self, name: &str, path: &Path, template: Option<&str>) {
        let known = self
            .items
            .iter()
            .find(|p| p.path == path)
            .and_then(|p| p.template.clone());
        self.items.retain(|p| p.path != path);
        self.items.insert(
            0,
            RecentProject {
                name: name.to_owned(),
                path: path.to_path_buf(),
                template: template.map(str::to_owned).or(known),
                opened: Some(unix_now()),
            },
        );
        self.items.truncate(MAX_RECENT);
        self.save();
    }

    /// Removes the game at `path` from the list (the folder itself is never touched) and saves.
    pub fn remove(&mut self, path: &Path) {
        self.items.retain(|p| p.path != path);
        self.save();
    }

    /// Writes the list; failures are logged, never returned.
    fn save(&self) {
        let Some(file) = &self.file else { return };
        let body = RecentFile {
            project: self.items.clone(),
        };
        let result = toml::to_string(&body)
            .map_err(|e| e.to_string())
            .and_then(|text| {
                if let Some(parent) = file.parent() {
                    crate::data_dirs::create_owned(parent).map_err(|e| e.to_string())?;
                }
                fs::write(file, text).map_err(|e| e.to_string())
            });
        if let Err(e) = result {
            log::warn!("could not save recent games: {e}");
        }
    }
}

/// Seconds since 1970 (UTC), or 0 if the clock is before that.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// How long ago `then` was, in words: "just now", "12 minutes ago", "3 hours ago", "yesterday",
/// "4 days ago", then the date ("24 October", with the year when it is not `now`'s). Both are
/// seconds since 1970; dates are UTC.
pub fn relative_time(then: u64, now: u64) -> String {
    let secs = now.saturating_sub(then);
    let (min, hour, day) = (60, 3600, 86_400);
    let plural = |n: u64, unit: &str| {
        if n == 1 {
            format!("1 {unit} ago")
        } else {
            format!("{n} {unit}s ago")
        }
    };
    match secs {
        s if s < min => "just now".to_owned(),
        s if s < hour => plural(s / min, "minute"),
        s if s < day => plural(s / hour, "hour"),
        s if s < 2 * day => "yesterday".to_owned(),
        s if s < 7 * day => plural(s / day, "day"),
        _ => {
            let (y, m, d) = civil_date(then / day);
            let (now_y, _, _) = civil_date(now / day);
            let month = MONTHS[(m - 1) as usize];
            if y == now_y {
                format!("{d} {month}")
            } else {
                format!("{d} {month} {y}")
            }
        }
    }
}

/// Year, month (1-12) and day for a count of days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_date(days: u64) -> (i64, u32, u32) {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_empty_list() {
        let dir = tempfile::tempdir().expect("tmp");
        assert!(
            RecentProjects::load(Some(dir.path().join("nope.toml")))
                .items()
                .is_empty()
        );
    }

    #[test]
    fn corrupt_file_gives_empty_list() {
        let dir = tempfile::tempdir().expect("tmp");
        let file = dir.path().join("recent.toml");
        fs::write(&file, "[[project]\nname = ").expect("write");
        assert!(RecentProjects::load(Some(file)).items().is_empty());
    }

    #[test]
    fn record_dedupes_orders_and_round_trips() {
        let dir = tempfile::tempdir().expect("tmp");
        let file = dir.path().join("sub").join("recent.toml");
        let mut recent = RecentProjects::load(Some(file.clone()));
        recent.record("A", Path::new("/games/a"), Some("parkour"));
        recent.record("B", Path::new("/games/b"), None);
        recent.record("A again", Path::new("/games/a"), None);
        let names: Vec<_> = recent.items().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["A again", "B"]);
        // Re-opening keeps the template it was made from, and stamps the time.
        assert_eq!(recent.items()[0].template.as_deref(), Some("parkour"));
        assert!(recent.items()[0].opened.is_some());
        let reloaded = RecentProjects::load(Some(file.clone()));
        assert_eq!(reloaded.items(), recent.items());
        recent.remove(Path::new("/games/b"));
        let names: Vec<_> = RecentProjects::load(Some(file))
            .items()
            .iter()
            .map(|p| p.name.clone())
            .collect();
        assert_eq!(names, ["A again"]);
    }

    #[test]
    fn files_from_before_template_and_time_still_load() {
        let dir = tempfile::tempdir().expect("tmp");
        let file = dir.path().join("recent.toml");
        fs::write(
            &file,
            "[[project]]\nname = \"Old\"\npath = \"/games/old\"\n",
        )
        .expect("write");
        let items = RecentProjects::load(Some(file)).items().to_vec();
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].template.clone(), items[0].opened), (None, None));
    }

    #[test]
    fn relative_times_read_like_speech() {
        let now = 1_793_318_400; // 2026-10-30 00:00 UTC
        assert_eq!(relative_time(now - 20, now), "just now");
        assert_eq!(relative_time(now - 60, now), "1 minute ago");
        assert_eq!(relative_time(now - 12 * 60, now), "12 minutes ago");
        assert_eq!(relative_time(now - 3 * 3600, now), "3 hours ago");
        assert_eq!(relative_time(now - 30 * 3600, now), "yesterday");
        assert_eq!(relative_time(now - 4 * 86_400, now), "4 days ago");
        assert_eq!(relative_time(now - 9 * 86_400, now), "21 October");
        assert_eq!(relative_time(now - 400 * 86_400, now), "25 September 2025");
        // A clock that went backwards reads as now.
        assert_eq!(relative_time(now + 50, now), "just now");
    }

    #[test]
    fn civil_dates_match_the_calendar() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(11_016), (2000, 2, 29));
        assert_eq!(civil_date(20_756), (2026, 10, 30));
    }

    #[test]
    fn list_is_capped() {
        let mut recent = RecentProjects::load(None);
        for i in 0..(MAX_RECENT + 5) {
            recent.record("g", Path::new(&format!("/games/{i}")), None);
        }
        assert_eq!(recent.items().len(), MAX_RECENT);
    }
}
