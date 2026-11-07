//! Per-user folders for settings, data and caches.
//!
//! The rules are each OS's own conventions, read from environment variables: the Windows `%APPDATA%` and
//! `%LOCALAPPDATA%` folders, the macOS `~/Library` folders, and the XDG base directories elsewhere. They are written
//! out here rather than taken from `directories`/`dirs`, whose `option-ext` dependency is MPL-2.0, and so that every
//! OS's rules are tested on every host.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::PlatformError;

/// Names an app for its user folders, in reverse-domain form: `qualifier.organization.application`
/// (`dev.metallic.Player`). Each part is ASCII letters, digits, `-` or `_`, so no part can reach outside the app's
/// folder.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AppId {
    qualifier: String,
    organization: String,
    application: String,
}

impl AppId {
    /// Parses `qualifier.organization.application`.
    pub fn parse(id: &str) -> Result<Self, PlatformError> {
        let invalid = |reason| PlatformError::InvalidAppId {
            id: id.to_owned(),
            reason,
        };
        let parts: Vec<&str> = id.split('.').collect();
        let [qualifier, organization, application] = parts[..] else {
            return Err(invalid(
                "expected three dot-separated parts, like `dev.metallic.Player`",
            ));
        };
        for part in [qualifier, organization, application] {
            if part.is_empty() {
                return Err(invalid("a part is empty"));
            }
            if !part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return Err(invalid("parts may only contain ASCII letters, digits, `-` and `_`"));
            }
        }
        Ok(Self {
            qualifier: qualifier.to_owned(),
            organization: organization.to_owned(),
            application: application.to_owned(),
        })
    }

    /// The first part (`dev` in `dev.metallic.Player`).
    pub fn qualifier(&self) -> &str {
        &self.qualifier
    }

    /// The second part (`metallic` in `dev.metallic.Player`).
    pub fn organization(&self) -> &str {
        &self.organization
    }

    /// The third part (`Player` in `dev.metallic.Player`).
    pub fn application(&self) -> &str {
        &self.application
    }
}

/// An app's per-user folders. They are not created; call `std::fs::create_dir_all` before writing.
///
/// | | config | data | cache |
/// |---|---|---|---|
/// | Windows | `%APPDATA%\Metallic\Player\config` | `%LOCALAPPDATA%\Metallic\Player\data` | `%LOCALAPPDATA%\Metallic\Player\cache` |
/// | macOS | `~/Library/Application Support/dev.metallic.Player` | (same as config) | `~/Library/Caches/dev.metallic.Player` |
/// | Linux, BSD | `$XDG_CONFIG_HOME/metallic/player` | `$XDG_DATA_HOME/metallic/player` | `$XDG_CACHE_HOME/metallic/player` |
///
/// On Windows the organization folder is capitalised (`Metallic`), as Windows apps name theirs.
/// Config roams with the user on Windows; data and cache stay on the machine. On macOS config and data are one
/// folder, so file names in them must not collide. The XDG variables default to `~/.config`, `~/.local/share`
/// and `~/.cache`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserPaths {
    app: PathBuf,
    config: PathBuf,
    data: PathBuf,
    cache: PathBuf,
}

impl UserPaths {
    /// Folders a backend found by other means (a mobile sandbox, a console's save-data API).
    pub fn new(config: PathBuf, data: PathBuf, cache: PathBuf) -> Self {
        Self {
            app: config.clone(),
            config,
            data,
            cache,
        }
    }

    /// The app's own roaming folder, which holds its config folder: `%APPDATA%\Metallic\Studio` on Windows. Elsewhere it
    /// is the config folder itself. Files the user may browse to and edit (Studio's icons) go here.
    pub fn app_dir(&self) -> &Path {
        &self.app
    }

    /// The folders for `app` for the current user on the OS this build targets. Fails with
    /// [`PlatformError::Unsupported`] on Android and iOS, whose folders come from the app's sandbox once their
    /// entry points exist (M9), and with [`PlatformError::NoHomeDir`] when the user has no home folder.
    pub fn for_app(app: &AppId) -> Result<Self, PlatformError> {
        let convention = current_convention().ok_or_else(|| PlatformError::unsupported("user folders"))?;
        resolve(convention, app, |name| std::env::var_os(name))
    }

    /// Settings: graphics options, key bindings.
    pub fn config_dir(&self) -> &Path {
        &self.config
    }

    /// Data the app keeps: downloaded games, saves, logs.
    pub fn data_dir(&self) -> &Path {
        &self.data
    }

    /// Data the app can rebuild and the OS may delete: shader and asset caches.
    pub fn cache_dir(&self) -> &Path {
        &self.cache
    }
}

/// Which OS's folder rules apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Convention {
    Windows,
    MacOs,
    /// The XDG base directories (Linux, the BSDs).
    Xdg,
}

fn current_convention() -> Option<Convention> {
    if cfg!(target_os = "windows") {
        Some(Convention::Windows)
    } else if cfg!(target_os = "macos") {
        Some(Convention::MacOs)
    } else if cfg!(any(target_os = "android", target_os = "ios")) {
        None
    } else {
        Some(Convention::Xdg)
    }
}

/// `name` with its first letter in capitals (`metallic` → `Metallic`).
fn capitalised(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Applies `convention`'s rules for `app`, reading environment variables through `var`.
fn resolve(
    convention: Convention,
    app: &AppId,
    var: impl Fn(&str) -> Option<OsString>,
) -> Result<UserPaths, PlatformError> {
    // Relative values are ignored, as the XDG spec requires; a relative folder would depend on the working folder.
    let absolute = |name: &str| var(name).map(PathBuf::from).filter(|path| path.is_absolute());
    let home = || absolute("HOME").ok_or(PlatformError::NoHomeDir);
    match convention {
        Convention::Windows => {
            let profile = || absolute("USERPROFILE").ok_or(PlatformError::NoHomeDir);
            let roaming = match absolute("APPDATA") {
                Some(path) => path,
                None => profile()?.join("AppData").join("Roaming"),
            };
            let local = match absolute("LOCALAPPDATA") {
                Some(path) => path,
                None => profile()?.join("AppData").join("Local"),
            };
            let folder = Path::new(&capitalised(app.organization())).join(app.application());
            Ok(UserPaths {
                app: roaming.join(&folder),
                config: roaming.join(&folder).join("config"),
                data: local.join(&folder).join("data"),
                cache: local.join(&folder).join("cache"),
            })
        }
        Convention::MacOs => {
            let library = home()?.join("Library");
            let bundle = format!("{}.{}.{}", app.qualifier(), app.organization(), app.application());
            let support = library.join("Application Support").join(&bundle);
            Ok(UserPaths::new(
                support.clone(),
                support,
                library.join("Caches").join(&bundle),
            ))
        }
        Convention::Xdg => {
            let base = |name: &str, fallback: &[&str]| -> Result<PathBuf, PlatformError> {
                match absolute(name) {
                    Some(path) => Ok(path),
                    None => Ok(fallback.iter().fold(home()?, |path, part| path.join(part))),
                }
            };
            let folder = Path::new(&app.organization().to_lowercase()).join(app.application().to_lowercase());
            Ok(UserPaths::new(
                base("XDG_CONFIG_HOME", &[".config"])?.join(&folder),
                base("XDG_DATA_HOME", &[".local", "share"])?.join(&folder),
                base("XDG_CACHE_HOME", &[".cache"])?.join(&folder),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn player() -> AppId {
        AppId::parse("dev.metallic.Player").unwrap()
    }

    /// An absolute folder on whatever OS runs the test.
    fn root(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    fn env(vars: &[(&str, PathBuf)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let vars: HashMap<String, OsString> = vars
            .iter()
            .map(|(name, path)| ((*name).to_owned(), path.clone().into_os_string()))
            .collect();
        move |name| vars.get(name).cloned()
    }

    #[test]
    fn app_id_splits_into_its_three_parts() {
        let id = player();
        assert_eq!(
            (id.qualifier(), id.organization(), id.application()),
            ("dev", "metallic", "Player")
        );
        assert!(AppId::parse("com.my-studio.My_Game2").is_ok());
    }

    #[test]
    fn malformed_app_ids_are_rejected() {
        for bad in [
            "",
            "metallic",
            "dev.metallic",
            "dev.metallic.player.extra",
            "dev..player",
            "dev.metal lic.x",
            "dev.metallic.a/b",
            "dev.metallic.a\\b",
        ] {
            assert!(
                matches!(AppId::parse(bad), Err(PlatformError::InvalidAppId { .. })),
                "{bad:?} was accepted"
            );
        }
    }

    #[test]
    fn windows_config_roams_and_data_and_cache_stay_local() {
        let (roaming, local) = (root("Roaming"), root("Local"));
        let vars = env(&[("APPDATA", roaming.clone()), ("LOCALAPPDATA", local.clone())]);
        let paths = resolve(Convention::Windows, &player(), vars).unwrap();
        let app = Path::new("Metallic").join("Player");
        assert_eq!(paths.app_dir(), roaming.join(&app));
        assert_eq!(paths.config_dir(), roaming.join(&app).join("config"));
        assert_eq!(paths.data_dir(), local.join(&app).join("data"));
        assert_eq!(paths.cache_dir(), local.join(&app).join("cache"));
    }

    #[test]
    fn windows_falls_back_to_the_user_profile() {
        let profile = root("alice");
        let paths = resolve(Convention::Windows, &player(), env(&[("USERPROFILE", profile.clone())])).unwrap();
        let local = profile.join("AppData").join("Local");
        assert_eq!(
            paths.config_dir(),
            profile
                .join("AppData")
                .join("Roaming")
                .join("Metallic")
                .join("Player")
                .join("config")
        );
        assert_eq!(paths.cache_dir(), local.join("Metallic").join("Player").join("cache"));
        assert!(matches!(
            resolve(Convention::Windows, &player(), env(&[])),
            Err(PlatformError::NoHomeDir)
        ));
    }

    #[test]
    fn macos_uses_the_bundle_id_under_library() {
        let home = root("alice");
        let paths = resolve(Convention::MacOs, &player(), env(&[("HOME", home.clone())])).unwrap();
        let support = home
            .join("Library")
            .join("Application Support")
            .join("dev.metallic.Player");
        assert_eq!(paths.config_dir(), support);
        assert_eq!(paths.data_dir(), support);
        assert_eq!(
            paths.cache_dir(),
            home.join("Library").join("Caches").join("dev.metallic.Player")
        );
    }

    #[test]
    fn xdg_reads_the_base_directory_variables() {
        let vars = env(&[
            ("HOME", root("alice")),
            ("XDG_CONFIG_HOME", root("cfg")),
            ("XDG_DATA_HOME", root("share")),
            ("XDG_CACHE_HOME", root("tmp-cache")),
        ]);
        let paths = resolve(Convention::Xdg, &player(), vars).unwrap();
        let app = Path::new("metallic").join("player");
        assert_eq!(paths.config_dir(), root("cfg").join(&app));
        assert_eq!(paths.data_dir(), root("share").join(&app));
        assert_eq!(paths.cache_dir(), root("tmp-cache").join(&app));
    }

    #[test]
    fn xdg_defaults_under_home_and_ignores_relative_variables() {
        let home = root("alice");
        let vars = env(&[("HOME", home.clone()), ("XDG_CONFIG_HOME", PathBuf::from("relative"))]);
        let paths = resolve(Convention::Xdg, &player(), vars).unwrap();
        let app = Path::new("metallic").join("player");
        assert_eq!(paths.config_dir(), home.join(".config").join(&app));
        assert_eq!(paths.data_dir(), home.join(".local").join("share").join(&app));
        assert_eq!(paths.cache_dir(), home.join(".cache").join(&app));
        assert!(matches!(
            resolve(Convention::Xdg, &player(), env(&[])),
            Err(PlatformError::NoHomeDir)
        ));
    }

    #[test]
    fn the_current_os_resolves_absolute_distinct_folders() {
        let Ok(paths) = UserPaths::for_app(&player()) else {
            // Android and iOS have no folders yet; desktop CI always has a home folder.
            assert!(current_convention().is_none());
            return;
        };
        for dir in [paths.config_dir(), paths.data_dir(), paths.cache_dir()] {
            assert!(dir.is_absolute(), "{} is relative", dir.display());
        }
        assert_ne!(paths.config_dir(), paths.cache_dir());
        assert_ne!(paths.data_dir(), paths.cache_dir());
    }

    #[test]
    fn backends_can_supply_their_own_folders() {
        let paths = UserPaths::new("c".into(), "d".into(), "k".into());
        assert_eq!(paths.config_dir(), Path::new("c"));
        assert_eq!(paths.data_dir(), Path::new("d"));
        assert_eq!(paths.cache_dir(), Path::new("k"));
    }
}
