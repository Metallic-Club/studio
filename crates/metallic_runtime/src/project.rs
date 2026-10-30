//! `metallic.toml`, the project file at the root of every game folder.
//!
//! ```toml
//! name = "Parkour"
//! main_scene = "main.mscene"
//! ```
use crate::RuntimeError;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// File name of the project file inside a project folder.
pub const PROJECT_FILE: &str = "metallic.toml";

/// The contents of `metallic.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProjectConfig {
    /// The game's display name.
    pub name: String,
    /// The scene the game starts with, relative to the project folder.
    pub main_scene: PathBuf,
}

impl ProjectConfig {
    /// Reads `<dir>/metallic.toml`.
    ///
    /// Errors: [`RuntimeError::Config`] when the file is missing, unreadable or lacks `name` or
    /// `main_scene`.
    pub fn read(dir: &Path) -> Result<ProjectConfig, RuntimeError> {
        let path = dir.join(PROJECT_FILE);
        let fail = |reason: String| RuntimeError::Config {
            path: path.clone(),
            reason,
        };
        let text = std::fs::read_to_string(&path).map_err(|e| fail(e.to_string()))?;
        toml::from_str(&text).map_err(|e| fail(e.message().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_name_and_main_scene() {
        let c: ProjectConfig =
            toml::from_str("name = \"Parkour\"\nmain_scene = \"main.mscene\"\n").unwrap();
        assert_eq!(c.name, "Parkour");
        assert_eq!(c.main_scene, PathBuf::from("main.mscene"));
    }

    #[test]
    fn missing_file_is_a_config_error() {
        let err = ProjectConfig::read(Path::new("this/folder/does/not/exist")).unwrap_err();
        assert!(matches!(err, RuntimeError::Config { .. }), "{err}");
    }
}
