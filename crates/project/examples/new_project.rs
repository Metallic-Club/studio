//! Makes a new `.mtproj` project from a template folder, until Studio does (M5).
//!
//! ```text
//! cargo run -p metallic_project --example new_project -- templates/baseplate-3d MyGame.mtproj ["My Game"]
//! cargo run -p metallic_player --release -- --project MyGame.mtproj
//! ```
//!
//! The project is named after its file unless a name is given.

use std::path::PathBuf;
use std::process::ExitCode;

use metallic_project::{Project, Template};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (template, path, name) = match args.as_slice() {
        [template, path] => (template, PathBuf::from(path), None),
        [template, path, name] => (template, PathBuf::from(path), Some(name.clone())),
        _ => {
            eprintln!("usage: new_project <template folder> <file.mtproj> [name]");
            return ExitCode::from(2);
        }
    };
    let name = name.unwrap_or_else(|| {
        path.file_stem()
            .map_or_else(|| "My Game".to_owned(), |stem| stem.to_string_lossy().into_owned())
    });
    let made = Template::from_dir(template).and_then(|template| Project::create(&path, &template, &name));
    match made {
        Ok(project) => {
            println!(
                "made {} (`{}`, starts in {})",
                project.path().display(),
                project.manifest().name,
                project.manifest().start_scene
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
