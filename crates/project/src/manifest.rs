//! `metallic.toml`, the manifest at the root of every project (ADR 0006).
//!
//! This module only turns the manifest's contents into a [`Manifest`] and back ([`Manifest::from_toml`],
//! [`Manifest::from_bytes`], [`Manifest::to_toml`]); finding the file inside a project is the project code's job.
//!
//! Reading is strict about what the engine needs and lenient about the rest: a missing or mistyped field, or a
//! `kind` / `mode` the engine does not support, is an error naming the field; a key this engine does not know
//! (written by a newer engine, or by hand) is a warning and is dropped on the next write, like unknown properties
//! in scenes.
//!
//! Writing is deterministic: the same [`Manifest`] always gives the same bytes, keys in a fixed order, so a file
//! the engine wrote reads and writes back unchanged and diffs only where something changed.

use std::fmt;

use toml::{Table, Value};

/// The manifest's file name at the project's root.
pub const MANIFEST_FILE: &str = "metallic.toml";

/// The engine version this build writes into [`Manifest::engine`]: the workspace version's `major.minor`.
pub const ENGINE_VERSION: &str = "0.1";

/// The scene a new project starts in.
pub const DEFAULT_START_SCENE: &str = "Scenes/Main.scene";

/// The table holding every project setting.
const PROJECT_TABLE: &str = "project";

/// Whether a project is 3D or 2D. Each kind has its own physics, rules and editor tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProjectKind {
    /// A 3D project (`kind = "3d"`), the only kind built so far. `"2d"` is reserved and refused until 2D exists.
    ThreeD,
}

impl ProjectKind {
    /// The value written in `metallic.toml`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ThreeD => "3d",
        }
    }
}

impl fmt::Display for ProjectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a project ships. It changes the player's behaviour, never the file format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ProjectMode {
    /// Packed into `game.pak` and shipped on its own (`mode = "standalone"`).
    #[default]
    Standalone,
    /// Runs inside the Metallic platform, streams its assets and shows the platform UI (`mode = "platform"`).
    Platform,
}

impl ProjectMode {
    /// The value written in `metallic.toml`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standalone => "standalone",
            Self::Platform => "platform",
        }
    }
}

impl fmt::Display for ProjectMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The project settings stored in `metallic.toml` (the `[project]` table).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// The project's display name.
    pub name: String,
    /// A free-text description; empty when there is none.
    pub description: String,
    /// Free-form tags, in the order the user gave them.
    pub tags: Vec<String>,
    /// 3D or 2D.
    pub kind: ProjectKind,
    /// Standalone or platform.
    pub mode: ProjectMode,
    /// The scene the game starts in, relative to the project's root, with `/` separators.
    pub start_scene: String,
    /// The engine version the project was last saved with (`major.minor`).
    pub engine: String,
}

impl Manifest {
    /// A new standalone 3D project called `name`, starting in [`DEFAULT_START_SCENE`], saved by this engine.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            tags: Vec::new(),
            kind: ProjectKind::ThreeD,
            mode: ProjectMode::Standalone,
            start_scene: DEFAULT_START_SCENE.to_owned(),
            engine: ENGINE_VERSION.to_owned(),
        }
    }

    /// Reads a manifest from the text of a `metallic.toml` file.
    ///
    /// `name`, `kind`, `mode`, `start_scene` and `engine` are required; `description` and `tags` default to empty.
    /// Keys this engine does not know come back as warnings.
    pub fn from_toml(text: &str) -> Result<LoadedManifest, ManifestError> {
        let root: Table = text
            .parse()
            .map_err(|error: toml::de::Error| ManifestError::Syntax(error.message().to_owned()))?;

        let mut warnings = Vec::new();
        let mut project = None;
        for (key, value) in &root {
            if key == PROJECT_TABLE {
                project = Some(value.as_table().ok_or(ManifestError::NotATable)?);
            } else {
                warnings.push(ManifestWarning::UnknownKey { key: key.clone() });
            }
        }
        let project = project.ok_or(ManifestError::MissingTable)?;

        for key in project.keys() {
            if !Field::ALL.iter().any(|field| field.key() == key) {
                warnings.push(ManifestWarning::UnknownKey {
                    key: format!("{PROJECT_TABLE}.{key}"),
                });
            }
        }

        // Sorted so the report does not depend on how the TOML crate orders its tables.
        warnings.sort_by(|a, b| a.key().cmp(b.key()));

        let manifest = Self {
            name: required_string(project, Field::Name)?.to_owned(),
            description: optional_string(project, Field::Description)?.to_owned(),
            tags: tags(project)?,
            kind: kind(required_string(project, Field::Kind)?)?,
            mode: mode(required_string(project, Field::Mode)?)?,
            start_scene: required_string(project, Field::StartScene)?.to_owned(),
            engine: required_string(project, Field::Engine)?.to_owned(),
        };
        Ok(LoadedManifest { manifest, warnings })
    }

    /// Reads a manifest from the raw bytes of a `metallic.toml` file, which must be UTF-8 (as TOML requires).
    pub fn from_bytes(bytes: &[u8]) -> Result<LoadedManifest, ManifestError> {
        let text = std::str::from_utf8(bytes).map_err(|_| ManifestError::NotUtf8)?;
        Self::from_toml(text)
    }

    /// The text of a `metallic.toml` file holding this manifest: every key, in a fixed order, with `\n` line ends.
    /// Its UTF-8 bytes (`into_bytes`) are the file's contents.
    pub fn to_toml(&self) -> String {
        let tags = self.tags.iter().map(|tag| string(tag)).collect::<Vec<_>>().join(", ");
        let mut text = String::new();
        text.push_str("[project]\n");
        for (field, value) in [
            (Field::Name, string(&self.name)),
            (Field::Description, string(&self.description)),
            (Field::Tags, format!("[{tags}]")),
            (Field::Kind, string(self.kind.as_str())),
            (Field::Mode, string(self.mode.as_str())),
            (Field::StartScene, string(&self.start_scene)),
            (Field::Engine, string(&self.engine)),
        ] {
            text.push_str(field.key());
            text.push_str(" = ");
            text.push_str(&value);
            text.push('\n');
        }
        text
    }
}

/// What [`Manifest::from_toml`] and [`Manifest::from_bytes`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedManifest {
    /// The project settings.
    pub manifest: Manifest,
    /// Things that were skipped, sorted by key (the caller reports them).
    pub warnings: Vec<ManifestWarning>,
}

/// Something in `metallic.toml` that was skipped rather than refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestWarning {
    /// A key this engine does not know, from a newer engine or added by hand. It is dropped on the next write.
    UnknownKey {
        /// The key's dotted path, such as `project.icon`.
        key: String,
    },
}

impl ManifestWarning {
    /// The dotted key the warning is about.
    pub fn key(&self) -> &str {
        match self {
            Self::UnknownKey { key } => key,
        }
    }
}

impl fmt::Display for ManifestWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKey { key } => write!(f, "metallic.toml: unknown key `{key}` ignored"),
        }
    }
}

/// Why `metallic.toml` was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    /// The bytes are not UTF-8 text.
    #[error("metallic.toml is not UTF-8 text")]
    NotUtf8,
    /// The text is not valid TOML.
    #[error("metallic.toml is not valid TOML: {0}")]
    Syntax(String),
    /// There is no `[project]` table.
    #[error("metallic.toml has no [project] table")]
    MissingTable,
    /// `project` is there but is not a table.
    #[error("metallic.toml: `project` must be a table ([project])")]
    NotATable,
    /// A required field is missing from `[project]`.
    #[error("metallic.toml: `project.{field}` is missing")]
    MissingField {
        /// The field's key.
        field: &'static str,
    },
    /// A field holds the wrong kind of value.
    #[error("metallic.toml: `project.{field}` must be {expected}")]
    WrongType {
        /// The field's key.
        field: &'static str,
        /// What it must hold, such as "a string".
        expected: &'static str,
    },
    /// A field holds a value the engine does not accept, such as `kind = "2d"` before 2D exists.
    #[error("metallic.toml: `project.{field}` is \"{value}\", which is not supported; accepted values: {accepted}")]
    UnsupportedValue {
        /// The field's key.
        field: &'static str,
        /// The value found.
        value: String,
        /// Every value the engine accepts, quoted and comma-separated.
        accepted: &'static str,
    },
}

/// The keys of `[project]`, in the order they are written.
#[derive(Clone, Copy)]
enum Field {
    Name,
    Description,
    Tags,
    Kind,
    Mode,
    StartScene,
    Engine,
}

impl Field {
    const ALL: [Self; 7] = [
        Self::Name,
        Self::Description,
        Self::Tags,
        Self::Kind,
        Self::Mode,
        Self::StartScene,
        Self::Engine,
    ];

    fn key(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Description => "description",
            Self::Tags => "tags",
            Self::Kind => "kind",
            Self::Mode => "mode",
            Self::StartScene => "start_scene",
            Self::Engine => "engine",
        }
    }
}

fn optional_string(project: &Table, field: Field) -> Result<&str, ManifestError> {
    match project.get(field.key()) {
        None => Ok(""),
        Some(value) => value.as_str().ok_or(ManifestError::WrongType {
            field: field.key(),
            expected: "a string",
        }),
    }
}

fn required_string(project: &Table, field: Field) -> Result<&str, ManifestError> {
    if !project.contains_key(field.key()) {
        return Err(ManifestError::MissingField { field: field.key() });
    }
    optional_string(project, field)
}

fn tags(project: &Table) -> Result<Vec<String>, ManifestError> {
    let wrong_type = || ManifestError::WrongType {
        field: Field::Tags.key(),
        expected: "an array of strings",
    };
    let Some(value) = project.get(Field::Tags.key()) else {
        return Ok(Vec::new());
    };
    value
        .as_array()
        .ok_or_else(wrong_type)?
        .iter()
        .map(|tag| tag.as_str().map(str::to_owned).ok_or_else(wrong_type))
        .collect()
}

fn kind(value: &str) -> Result<ProjectKind, ManifestError> {
    match value {
        "3d" => Ok(ProjectKind::ThreeD),
        _ => Err(ManifestError::UnsupportedValue {
            field: Field::Kind.key(),
            value: value.to_owned(),
            accepted: "\"3d\" (\"2d\" is reserved until 2D projects exist)",
        }),
    }
}

fn mode(value: &str) -> Result<ProjectMode, ManifestError> {
    match value {
        "standalone" => Ok(ProjectMode::Standalone),
        "platform" => Ok(ProjectMode::Platform),
        _ => Err(ManifestError::UnsupportedValue {
            field: Field::Mode.key(),
            value: value.to_owned(),
            accepted: "\"standalone\", \"platform\"",
        }),
    }
}

/// A TOML string literal for `text`, escaped by the TOML crate so any text round-trips.
fn string(text: &str) -> String {
    Value::String(text.to_owned()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the engine writes for a project with every field set, byte for byte.
    const FULL: &str = "\
[project]
name = \"My Game\"
description = \"A small test game.\"
tags = [\"racing\", \"multiplayer\"]
kind = \"3d\"
mode = \"platform\"
start_scene = \"Scenes/Track.scene\"
engine = \"0.1\"
";

    /// What the engine writes for [`Manifest::new`].
    const NEW: &str = "\
[project]
name = \"My Game\"
description = \"\"
tags = []
kind = \"3d\"
mode = \"standalone\"
start_scene = \"Scenes/Main.scene\"
engine = \"0.1\"
";

    fn read(text: &str) -> Manifest {
        let loaded = Manifest::from_toml(text).unwrap();
        assert_eq!(loaded.warnings, []);
        loaded.manifest
    }

    fn error(text: &str) -> String {
        Manifest::from_toml(text).unwrap_err().to_string()
    }

    /// [`NEW`] with `key = value` swapped for `replacement` (an empty `replacement` removes the line).
    fn with(key: &str, replacement: &str) -> String {
        NEW.lines()
            .filter_map(|line| {
                if line.starts_with(&format!("{key} = ")) {
                    (!replacement.is_empty()).then(|| replacement.to_owned())
                } else {
                    Some(line.to_owned())
                }
            })
            .map(|line| line + "\n")
            .collect()
    }

    #[test]
    fn engine_version_matches_the_workspace_version() {
        let version = env!("CARGO_PKG_VERSION");
        let major_minor = version.rsplit_once('.').unwrap().0;
        assert_eq!(ENGINE_VERSION, major_minor);
    }

    #[test]
    fn reads_every_field() {
        let manifest = read(FULL);
        assert_eq!(
            manifest,
            Manifest {
                name: "My Game".to_owned(),
                description: "A small test game.".to_owned(),
                tags: vec!["racing".to_owned(), "multiplayer".to_owned()],
                kind: ProjectKind::ThreeD,
                mode: ProjectMode::Platform,
                start_scene: "Scenes/Track.scene".to_owned(),
                engine: "0.1".to_owned(),
            }
        );
    }

    #[test]
    fn new_project_writes_the_documented_defaults() {
        assert_eq!(Manifest::new("My Game").to_toml(), NEW);
    }

    #[test]
    fn engine_written_files_round_trip_byte_for_byte() {
        for text in [FULL, NEW] {
            assert_eq!(read(text).to_toml(), text);
        }
    }

    #[test]
    fn awkward_text_round_trips() {
        let mut manifest = Manifest::new("Quotes \" and \\ back'slash");
        manifest.description = "Line one\nLine two\ttabbed, \u{1F680} and ''' and \"\"\"".to_owned();
        manifest.tags = vec![String::new(), "a, b".to_owned(), "]".to_owned(), "日本語".to_owned()];
        manifest.start_scene = "Scenes/With Spaces/Main.scene".to_owned();
        let text = manifest.to_toml();
        assert_eq!(read(&text), manifest);
        assert_eq!(read(&text).to_toml(), text);
    }

    #[test]
    fn the_adr_example_reads() {
        // The example in docs/adr/0006-project-format.md, comments included.
        let manifest = read(
            r#"[project]
name = "My Game"
description = ""
tags = []
kind = "3d"            # "2d" is reserved and refused until 2D exists
mode = "standalone"    # or "platform"
start_scene = "Scenes/Main.scene"
engine = "0.1"         # engine version the project was last saved with
"#,
        );
        assert_eq!(manifest, Manifest::new("My Game"));
    }

    #[test]
    fn hand_edited_files_are_rewritten_in_the_fixed_order() {
        let text = "\
[project]
engine = '0.1'
start_scene = 'Scenes/Main.scene'
mode = 'standalone'
kind = '3d'
name = 'My Game'
";
        assert_eq!(read(text).to_toml(), NEW);
    }

    #[test]
    fn description_and_tags_are_optional() {
        let text = with("description", "");
        let text = text.replace("tags = []\n", "");
        assert_eq!(read(&text), Manifest::new("My Game"));
    }

    #[test]
    fn unknown_keys_warn_and_are_dropped() {
        let text = format!("{NEW}icon = \"icon.png\"\n\n[future]\nanswer = 42\n");
        let loaded = Manifest::from_toml(&text).unwrap();
        assert_eq!(loaded.manifest, Manifest::new("My Game"));
        let warnings: Vec<_> = loaded.warnings.iter().map(ToString::to_string).collect();
        assert_eq!(
            warnings,
            [
                "metallic.toml: unknown key `future` ignored",
                "metallic.toml: unknown key `project.icon` ignored",
            ]
        );
        assert_eq!(loaded.manifest.to_toml(), NEW);
    }

    #[test]
    fn refuses_2d() {
        assert_eq!(
            error(&with("kind", "kind = \"2d\"")),
            "metallic.toml: `project.kind` is \"2d\", which is not supported; \
             accepted values: \"3d\" (\"2d\" is reserved until 2D projects exist)"
        );
    }

    #[test]
    fn refuses_an_unknown_kind() {
        assert_eq!(
            error(&with("kind", "kind = \"3D\"")),
            "metallic.toml: `project.kind` is \"3D\", which is not supported; \
             accepted values: \"3d\" (\"2d\" is reserved until 2D projects exist)"
        );
    }

    #[test]
    fn refuses_an_unknown_mode() {
        assert_eq!(
            error(&with("mode", "mode = \"server\"")),
            "metallic.toml: `project.mode` is \"server\", which is not supported; \
             accepted values: \"standalone\", \"platform\""
        );
    }

    #[test]
    fn refuses_missing_fields() {
        for field in ["name", "kind", "mode", "start_scene", "engine"] {
            assert_eq!(
                error(&with(field, "")),
                format!("metallic.toml: `project.{field}` is missing"),
                "{field}"
            );
        }
    }

    #[test]
    fn refuses_wrong_types() {
        assert_eq!(
            error(&with("kind", "kind = 3")),
            "metallic.toml: `project.kind` must be a string"
        );
        assert_eq!(
            error(&with("description", "description = []")),
            "metallic.toml: `project.description` must be a string"
        );
        assert_eq!(
            error(&with("tags", "tags = \"racing\"")),
            "metallic.toml: `project.tags` must be an array of strings"
        );
        assert_eq!(
            error(&with("tags", "tags = [\"racing\", 1]")),
            "metallic.toml: `project.tags` must be an array of strings"
        );
    }

    #[test]
    fn refuses_a_missing_or_malformed_project_table() {
        assert_eq!(error(""), "metallic.toml has no [project] table");
        assert_eq!(error("name = \"My Game\"\n"), "metallic.toml has no [project] table");
        assert_eq!(
            error("project = \"My Game\"\n"),
            "metallic.toml: `project` must be a table ([project])"
        );
    }

    #[test]
    fn refuses_invalid_toml() {
        assert!(error("[project\n").starts_with("metallic.toml is not valid TOML: "));
    }

    #[test]
    fn bytes_round_trip_byte_for_byte() {
        for text in [FULL, NEW] {
            let loaded = Manifest::from_bytes(text.as_bytes()).unwrap();
            assert_eq!(loaded.manifest.to_toml().into_bytes(), text.as_bytes());
        }
    }

    #[test]
    fn refuses_bytes_that_are_not_utf8() {
        let mut bytes = NEW.as_bytes().to_vec();
        bytes.extend_from_slice(b"# \xFF\n");
        assert_eq!(
            Manifest::from_bytes(&bytes).unwrap_err().to_string(),
            "metallic.toml is not UTF-8 text"
        );
    }
}
