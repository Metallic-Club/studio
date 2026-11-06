//! Games from `.mtproj` projects ([`metallic_project::Project`], ADR 0006) or from their builds (`game.pak`,
//! [`metallic_project::Pak`]), both read through [`GameFiles`].
//!
//! An app opens a project (or a pak), makes its engine with the game's scripts as its sources ([`project_sources`]),
//! then loads the start scene into the engine's game before starting it:
//!
//! ```no_run
//! # #[cfg(feature = "luau")] {
//! use metallic_engine::metallic_project::Project;
//! use metallic_engine::{Engine, EngineConfig, RunContext, project_sources};
//!
//! let mut project = Project::open("MyGame.mtproj")?;
//! let config = EngineConfig::new(RunContext::Local).with_sources(project_sources(&mut project)?);
//! let mut engine = Engine::new(config)?;
//! project.load_start_scene(engine.data_model_mut())?;
//! engine.start()?;
//! # }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use metallic_project::{GameFiles, ProjectError};
use metallic_script::MemorySources;

/// The file name extension of a script member: scripts' `Source` names such members (`Scripts/Door.luau`).
const SCRIPT_EXTENSION: &str = ".luau";

/// Every script in `project` (a [`Project`](metallic_project::Project) or a [`Pak`](metallic_project::Pak): its
/// `.luau` members, which must be UTF-8), by member path, for
/// [`EngineConfig::with_sources`](crate::EngineConfig::with_sources).
pub fn project_sources<G: GameFiles + ?Sized>(project: &mut G) -> Result<MemorySources, ProjectError> {
    let scripts: Vec<String> = project
        .members()
        .into_iter()
        .filter(|member| member.ends_with(SCRIPT_EXTENSION))
        .map(str::to_owned)
        .collect();
    let mut sources = MemorySources::new();
    for member in scripts {
        let text = project.read_text(&member)?;
        sources.insert(&member, &text);
    }
    Ok(sources)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use metallic_project::{Project, Template};
    use metallic_script::SourceLoader;

    use super::*;

    #[test]
    fn a_projects_scripts_are_its_sources() {
        let dir = std::env::temp_dir().join(format!("metallic-engine-project-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let template =
            Template::from_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d")).unwrap();
        let mut project = Project::create(dir.join("Game.mtproj"), &template, "Game").unwrap();
        project.write("Scripts/Main.luau", "print('hi')").unwrap();
        project.write("Assets/notes.txt", "not a script").unwrap();

        let sources = project_sources(&mut project).unwrap();
        assert_eq!(&*sources.load("Scripts/Main.luau").unwrap(), "print('hi')");
        assert!(sources.load("Assets/notes.txt").is_err());

        project.write("Scripts/Bad.luau", vec![0xFF, 0xFE]).unwrap();
        let error = project_sources(&mut project).unwrap_err();
        assert!(
            error.to_string().contains("`Scripts/Bad.luau` is not UTF-8 text"),
            "{error}"
        );
        drop(project);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
