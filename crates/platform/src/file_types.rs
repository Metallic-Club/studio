//! Per-user file associations: double-clicking a file of an app's type opens the app with it (ADR 0009, Opening
//! `.mtproj` files).
//!
//! Registering is an explicit action ([`register`], [`unregister`]): Studio's `--register-file-types` now, the
//! installer at packaging (M8). Nothing here runs on its own, and tests check [`windows_entries`] and
//! [`xdg_entries`] as data, never by writing them.
//!
//! | OS | What [`register`] does |
//! |---|---|
//! | Windows | `HKEY_CURRENT_USER\Software\Classes`: `.ext` → a ProgID with a name, an icon and `shell\open\command`, then `SHChangeNotify` so Explorer sees it |
//! | Linux, BSD | an XDG shared-mime-info package (`$XDG_DATA_HOME/mime/packages`) and a `.desktop` file with `MimeType=` (`$XDG_DATA_HOME/applications`), then `update-mime-database` and `update-desktop-database` if they are installed |
//! | macOS | nothing: the app bundle's `Info.plist` declares the type (`CFBundleDocumentTypes`, `UTExportedTypeDeclarations`) at packaging |

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::PlatformError;

/// A file type an app opens: its extension, MIME type and names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileType {
    /// The extension, without the dot (`mtproj`).
    pub extension: &'static str,
    /// Its MIME type (`application/x-metallic-project`).
    pub mime_type: &'static str,
    /// The Windows ProgID it maps to (`Metallic.Project`).
    pub prog_id: &'static str,
    /// What a file of this type is, as file managers show it (`Metallic project`).
    pub description: &'static str,
    /// The app's reverse-domain id (`dev.metallic.Studio`): the name of the `.desktop` file and the MIME package.
    pub app_id: &'static str,
    /// The app's name, as "Open with" lists it (`Metallic Studio`).
    pub app_name: &'static str,
}

/// One thing [`register`] writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// A string value under `HKEY_CURRENT_USER` (Windows).
    RegistryValue {
        /// The key, under `HKEY_CURRENT_USER` (`Software\Classes\.mtproj`).
        key: String,
        /// The value's name; `None` is the key's default value.
        name: Option<String>,
        /// The text it holds.
        value: String,
    },
    /// A file in the user's folders (Linux).
    File {
        /// Where it goes.
        path: PathBuf,
        /// What it holds.
        contents: String,
    },
}

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Entry::RegistryValue { key, name, value } => {
                let name = name.as_deref().unwrap_or("(default)");
                write!(f, "HKEY_CURRENT_USER\\{key} {name} = {value}")
            }
            Entry::File { path, .. } => write!(f, "{}", path.display()),
        }
    }
}

/// The registry values that associate `file_type` with the program at `exe` for the current user, under
/// `HKEY_CURRENT_USER`. The icon is the program's first icon resource, so the file shows the program's icon.
pub fn windows_entries(file_type: &FileType, exe: &Path) -> Vec<Entry> {
    let exe = exe.to_string_lossy();
    let [extension, prog_id] = windows_keys(file_type);
    let value = |key: &str, name: Option<&str>, value: String| Entry::RegistryValue {
        key: key.to_owned(),
        name: name.map(str::to_owned),
        value,
    };
    vec![
        value(&extension, None, file_type.prog_id.to_owned()),
        value(&extension, Some("Content Type"), file_type.mime_type.to_owned()),
        value(&prog_id, None, file_type.description.to_owned()),
        value(&format!("{prog_id}\\DefaultIcon"), None, format!("\"{exe}\",0")),
        // Quoted, so a path with spaces reaches the program as one argument.
        value(
            &format!("{prog_id}\\shell\\open\\command"),
            None,
            format!("\"{exe}\" \"%1\""),
        ),
    ]
}

/// The two keys [`windows_entries`] writes under, which [`unregister`] removes with everything in them: the
/// extension's and the ProgID's.
pub fn windows_keys(file_type: &FileType) -> [String; 2] {
    [
        format!("Software\\Classes\\.{}", file_type.extension),
        format!("Software\\Classes\\{}", file_type.prog_id),
    ]
}

/// The files that associate `file_type` with the program at `exe` for the current user on an XDG desktop, under
/// `data_home` (`$XDG_DATA_HOME`, `~/.local/share` by default): the MIME type's package and the `.desktop` file
/// that opens it.
pub fn xdg_entries(file_type: &FileType, exe: &Path, data_home: &Path) -> Vec<Entry> {
    let FileType {
        extension,
        mime_type,
        description,
        app_id,
        app_name,
        ..
    } = file_type;
    let mime = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">\n\
         \x20 <mime-type type=\"{mime_type}\">\n\
         \x20   <comment>{}</comment>\n\
         \x20   <glob pattern=\"*.{extension}\"/>\n\
         \x20 </mime-type>\n\
         </mime-info>\n",
        xml_escape(description)
    );
    let desktop = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={app_name}\n\
         Exec={} %f\n\
         Terminal=false\n\
         MimeType={mime_type};\n",
        desktop_exec_arg(&exe.to_string_lossy())
    );
    vec![
        Entry::File {
            path: data_home.join("mime").join("packages").join(format!("{app_id}.xml")),
            contents: mime,
        },
        Entry::File {
            path: data_home.join("applications").join(format!("{app_id}.desktop")),
            contents: desktop,
        },
    ]
}

/// What [`register`] would write on this OS for the program at `exe`: the registry values on Windows, the files on
/// Linux, nothing on macOS (the bundle declares the type).
pub fn entries(file_type: &FileType, exe: &Path) -> Result<Vec<Entry>, PlatformError> {
    if cfg!(target_os = "windows") {
        Ok(windows_entries(file_type, exe))
    } else if cfg!(target_os = "macos") {
        Ok(Vec::new())
    } else if cfg!(any(target_os = "android", target_os = "ios")) {
        Err(PlatformError::unsupported("file associations"))
    } else {
        Ok(xdg_entries(
            file_type,
            exe,
            &xdg_data_home(|name| std::env::var_os(name))?,
        ))
    }
}

/// Associates `file_type` with the program at `exe` for the current user. Returns a line for each thing done.
pub fn register(file_type: &FileType, exe: &Path) -> Result<Vec<String>, PlatformError> {
    #[cfg(target_os = "windows")]
    {
        registry::register(&windows_entries(file_type, exe))
    }
    #[cfg(target_os = "macos")]
    {
        let _ = (file_type, exe);
        Ok(vec![macos_note()])
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = (file_type, exe);
        Err(PlatformError::unsupported("file associations"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "android", target_os = "ios")))]
    {
        let data_home = xdg_data_home(|name| std::env::var_os(name))?;
        xdg::register(&xdg_entries(file_type, exe, &data_home), &data_home)
    }
}

/// Removes what [`register`] wrote for `file_type`. Returns a line for each thing done; what was not there is said,
/// not an error.
pub fn unregister(file_type: &FileType) -> Result<Vec<String>, PlatformError> {
    #[cfg(target_os = "windows")]
    {
        registry::unregister(&windows_keys(file_type))
    }
    #[cfg(target_os = "macos")]
    {
        let _ = file_type;
        Ok(vec![macos_note()])
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = file_type;
        Err(PlatformError::unsupported("file associations"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "android", target_os = "ios")))]
    {
        let data_home = xdg_data_home(|name| std::env::var_os(name))?;
        // Only the paths matter for removing them; the program's path is not part of them.
        xdg::unregister(&xdg_entries(file_type, Path::new(""), &data_home), &data_home)
    }
}

#[cfg(target_os = "macos")]
fn macos_note() -> String {
    "Nothing to do on macOS: the app bundle's Info.plist declares the file type (packaging, M8).".to_owned()
}

/// `$XDG_DATA_HOME`, or `~/.local/share`, reading environment variables through `var`. A relative value is
/// ignored, as the XDG spec requires (the same rule as [`crate::UserPaths`]).
fn xdg_data_home(var: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, PlatformError> {
    let absolute = |name: &str| var(name).map(PathBuf::from).filter(|path| path.is_absolute());
    match absolute("XDG_DATA_HOME") {
        Some(path) => Ok(path),
        None => Ok(absolute("HOME")
            .ok_or(PlatformError::NoHomeDir)?
            .join(".local")
            .join("share")),
    }
}

/// `text` with XML's special characters escaped.
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// `arg` as one argument of a `.desktop` file's `Exec=` key. The Desktop Entry spec quotes an argument holding a
/// reserved character in double quotes, with `"`, `` ` ``, `$` and `\` escaped by a backslash inside them; `%` is
/// doubled, since `%f` and the like are field codes; and then the value's own string escaping doubles every
/// backslash again.
fn desktop_exec_arg(arg: &str) -> String {
    const RESERVED: &[char] = &[
        ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(', ')', '`',
    ];
    let quoted = if arg.is_empty() || arg.contains(RESERVED) {
        let mut quoted = String::from("\"");
        for c in arg.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                quoted.push('\\');
            }
            quoted.push(c);
        }
        quoted.push('"');
        quoted
    } else {
        arg.to_owned()
    };
    quoted.replace('%', "%%").replace('\\', "\\\\")
}

/// Writing and removing the association in the registry (Windows).
#[cfg(target_os = "windows")]
mod registry {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegDeleteTreeW, RegSetKeyValueW};
    use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};

    use super::Entry;
    use crate::error::PlatformError;

    /// `text` as a nul-terminated UTF-16 string.
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn failed(what: String, code: u32) -> PlatformError {
        PlatformError::FileAssociation(format!("{what}: {}", std::io::Error::from_raw_os_error(code as i32)))
    }

    pub(super) fn register(entries: &[Entry]) -> Result<Vec<String>, PlatformError> {
        let mut done = Vec::new();
        for entry in entries {
            let Entry::RegistryValue { key, name, value } = entry else {
                continue;
            };
            let key_w = wide(key);
            let name_w = name.as_deref().map(wide);
            let value_w = wide(value);
            let bytes = u32::try_from(value_w.len() * 2)
                .map_err(|_| PlatformError::FileAssociation(format!("{entry}: the value is too long")))?;
            // SAFETY: every pointer is to a nul-terminated UTF-16 buffer that outlives the call, and `bytes` is the
            // value buffer's size in bytes, its nul included, as REG_SZ requires. The key is created if missing.
            let code = unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key_w.as_ptr(),
                    name_w.as_ref().map_or(std::ptr::null(), |name| name.as_ptr()),
                    REG_SZ,
                    value_w.as_ptr().cast(),
                    bytes,
                )
            };
            if code != ERROR_SUCCESS {
                return Err(failed(format!("could not set {entry}"), code));
            }
            done.push(format!("Set {entry}"));
        }
        notify();
        done.push("Told Explorer that file associations changed.".to_owned());
        Ok(done)
    }

    pub(super) fn unregister(keys: &[String]) -> Result<Vec<String>, PlatformError> {
        let mut done = Vec::new();
        for key in keys {
            let key_w = wide(key);
            // SAFETY: `key_w` is a nul-terminated UTF-16 string that outlives the call; the key named is deleted
            // with its subkeys and values.
            let code = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key_w.as_ptr()) };
            match code {
                ERROR_SUCCESS => done.push(format!("Removed HKEY_CURRENT_USER\\{key}")),
                ERROR_FILE_NOT_FOUND => done.push(format!("HKEY_CURRENT_USER\\{key} was not there")),
                code => return Err(failed(format!("could not remove HKEY_CURRENT_USER\\{key}"), code)),
            }
        }
        notify();
        done.push("Told Explorer that file associations changed.".to_owned());
        Ok(done)
    }

    /// Tells the shell that associations changed, so Explorer shows the new icon and handler without a restart.
    fn notify() {
        // SAFETY: SHCNE_ASSOCCHANGED takes no items; both item pointers must be null, and are.
        unsafe {
            SHChangeNotify(
                SHCNE_ASSOCCHANGED as i32,
                SHCNF_IDLIST,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
    }
}

/// Writing and removing the association's files, then refreshing the desktop's caches (Linux, BSD).
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "android", target_os = "ios")))]
mod xdg {
    use std::io::ErrorKind;
    use std::path::Path;
    use std::process::Command;

    use super::Entry;
    use crate::error::PlatformError;

    pub(super) fn register(entries: &[Entry], data_home: &Path) -> Result<Vec<String>, PlatformError> {
        let mut done = Vec::new();
        for entry in entries {
            let Entry::File { path, contents } = entry else {
                continue;
            };
            let write = || -> std::io::Result<()> {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, contents)
            };
            write().map_err(|error| {
                PlatformError::FileAssociation(format!("could not write {}: {error}", path.display()))
            })?;
            done.push(format!("Wrote {entry}"));
        }
        refresh(data_home, &mut done);
        Ok(done)
    }

    pub(super) fn unregister(entries: &[Entry], data_home: &Path) -> Result<Vec<String>, PlatformError> {
        let mut done = Vec::new();
        for entry in entries {
            let Entry::File { path, .. } = entry else {
                continue;
            };
            match std::fs::remove_file(path) {
                Ok(()) => done.push(format!("Removed {entry}")),
                Err(error) if error.kind() == ErrorKind::NotFound => done.push(format!("{entry} was not there")),
                Err(error) => {
                    return Err(PlatformError::FileAssociation(format!(
                        "could not remove {}: {error}",
                        path.display()
                    )));
                }
            }
        }
        refresh(data_home, &mut done);
        Ok(done)
    }

    /// Rebuilds the MIME and desktop-file caches under `data_home` with the desktop's own tools, when installed.
    /// Without them the desktop picks the files up on its own, later.
    fn refresh(data_home: &Path, done: &mut Vec<String>) {
        for (tool, folder) in [
            ("update-mime-database", data_home.join("mime")),
            ("update-desktop-database", data_home.join("applications")),
        ] {
            match Command::new(tool).arg(&folder).status() {
                Ok(status) if status.success() => done.push(format!("Ran {tool} {}", folder.display())),
                Ok(status) => done.push(format!("{tool} {} failed ({status})", folder.display())),
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    done.push(format!("{tool} is not installed; skipped it"));
                }
                Err(error) => done.push(format!("Could not run {tool}: {error}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MTPROJ: FileType = FileType {
        extension: "mtproj",
        mime_type: "application/x-metallic-project",
        prog_id: "Metallic.Project",
        description: "Metallic project",
        app_id: "dev.metallic.Studio",
        app_name: "Metallic Studio",
    };

    fn value(key: &str, name: Option<&str>, value: &str) -> Entry {
        Entry::RegistryValue {
            key: key.to_owned(),
            name: name.map(str::to_owned),
            value: value.to_owned(),
        }
    }

    #[test]
    fn windows_maps_the_extension_to_a_prog_id_that_opens_the_program() {
        let exe = Path::new(r"C:\Program Files\Metallic\metallic_studio.exe");
        assert_eq!(
            windows_entries(&MTPROJ, exe),
            [
                value(r"Software\Classes\.mtproj", None, "Metallic.Project"),
                value(
                    r"Software\Classes\.mtproj",
                    Some("Content Type"),
                    "application/x-metallic-project"
                ),
                value(r"Software\Classes\Metallic.Project", None, "Metallic project"),
                value(
                    r"Software\Classes\Metallic.Project\DefaultIcon",
                    None,
                    r#""C:\Program Files\Metallic\metallic_studio.exe",0"#
                ),
                value(
                    r"Software\Classes\Metallic.Project\shell\open\command",
                    None,
                    r#""C:\Program Files\Metallic\metallic_studio.exe" "%1""#
                ),
            ]
        );
        assert_eq!(
            windows_keys(&MTPROJ),
            [r"Software\Classes\.mtproj", r"Software\Classes\Metallic.Project"]
        );
        // Every value is under one of the keys unregister removes.
        for entry in windows_entries(&MTPROJ, exe) {
            let Entry::RegistryValue { key, .. } = entry else {
                panic!()
            };
            assert!(
                windows_keys(&MTPROJ).iter().any(|root| key.starts_with(root.as_str())),
                "{key}"
            );
        }
        assert_eq!(
            windows_entries(&MTPROJ, exe)[0].to_string(),
            r"HKEY_CURRENT_USER\Software\Classes\.mtproj (default) = Metallic.Project"
        );
    }

    #[test]
    fn xdg_writes_a_mime_package_and_a_desktop_file_under_the_data_home() {
        let home = Path::new("/home/ada/.local/share");
        let entries = xdg_entries(&MTPROJ, Path::new("/home/ada/My Tools/metallic_studio"), home);
        assert_eq!(
            entries,
            [
                Entry::File {
                    path: home.join("mime").join("packages").join("dev.metallic.Studio.xml"),
                    contents: "\
<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">
  <mime-type type=\"application/x-metallic-project\">
    <comment>Metallic project</comment>
    <glob pattern=\"*.mtproj\"/>
  </mime-type>
</mime-info>
"
                    .to_owned(),
                },
                Entry::File {
                    path: home.join("applications").join("dev.metallic.Studio.desktop"),
                    contents: "\
[Desktop Entry]
Type=Application
Name=Metallic Studio
Exec=\"/home/ada/My Tools/metallic_studio\" %f
Terminal=false
MimeType=application/x-metallic-project;
"
                    .to_owned(),
                },
            ]
        );
    }

    #[test]
    fn exec_arguments_are_quoted_and_escaped_as_the_desktop_entry_spec_says() {
        assert_eq!(desktop_exec_arg("/usr/bin/metallic_studio"), "/usr/bin/metallic_studio");
        assert_eq!(desktop_exec_arg("/opt/My App/studio"), "\"/opt/My App/studio\"");
        // `%` doubled; `$` and `"` backslashed inside the quotes, and that backslash doubled by string escaping.
        assert_eq!(desktop_exec_arg("/opt/100%/studio"), "/opt/100%%/studio");
        assert_eq!(desktop_exec_arg("/opt/$x/studio"), "\"/opt/\\\\$x/studio\"");
        assert_eq!(desktop_exec_arg("/opt/a\"b/studio"), "\"/opt/a\\\\\"b/studio\"");
        // A literal backslash takes four.
        assert_eq!(desktop_exec_arg("/opt/a\\b"), "\"/opt/a\\\\\\\\b\"");
        assert_eq!(desktop_exec_arg(""), "\"\"");
    }

    #[test]
    fn the_data_home_is_xdg_data_home_or_local_share() {
        let data_home = |vars: &[(&str, &Path)]| {
            xdg_data_home(|name| {
                vars.iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.as_os_str().to_owned())
            })
        };
        // Absolute on whatever OS runs the test.
        let home = std::env::temp_dir().join("ada");
        let data = home.join("data");
        assert_eq!(data_home(&[("HOME", &home), ("XDG_DATA_HOME", &data)]).unwrap(), data);
        assert_eq!(
            data_home(&[("HOME", &home), ("XDG_DATA_HOME", Path::new("relative"))]).unwrap(),
            home.join(".local").join("share")
        );
        assert!(matches!(data_home(&[]), Err(PlatformError::NoHomeDir)));
    }

    #[test]
    fn entries_are_this_os_s_own() {
        let exe = Path::new("metallic_studio");
        let entries = entries(&MTPROJ, exe);
        if cfg!(windows) {
            assert_eq!(entries.unwrap(), windows_entries(&MTPROJ, exe));
        } else if cfg!(target_os = "macos") {
            assert!(entries.unwrap().is_empty());
        } else if let Ok(entries) = entries {
            assert!(entries.iter().all(|entry| matches!(entry, Entry::File { .. })));
        }
    }
}
