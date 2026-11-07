//! The `metallic_studio` executable: opens Studio, or a project straight into the editor, or registers Studio for
//! `.mtproj` files (`--help`). Everything else is in the library.

use std::process::ExitCode;

use metallic_engine::metallic_platform::file_types;
use metallic_studio::PROJECT_FILE_TYPE;
use metallic_studio::cli::{Command, HELP};

fn main() -> ExitCode {
    let command = match Command::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("error: {error}\n\n{HELP}");
            return ExitCode::from(2);
        }
    };
    let open = match command {
        Command::Run { open } => open,
        Command::Help => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Command::Version => {
            println!("Metallic Studio {}", metallic_studio::VERSION);
            return ExitCode::SUCCESS;
        }
        // Only ever on this explicit command: Studio never registers itself (ADR 0009).
        Command::RegisterFileTypes => {
            return report(
                std::env::current_exe()
                    .map_err(|error| format!("could not find Studio's own executable: {error}"))
                    .and_then(|exe| file_types::register(&PROJECT_FILE_TYPE, &exe).map_err(|error| error.to_string())),
            );
        }
        Command::UnregisterFileTypes => {
            return report(file_types::unregister(&PROJECT_FILE_TYPE).map_err(|error| error.to_string()));
        }
    };
    metallic_studio::logging::install(level());
    match metallic_studio::run(open) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Prints what a file-type command did, or why it failed.
fn report(done: Result<Vec<String>, String>) -> ExitCode {
    match done {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// What stderr gets: warnings and errors, or the level `METALLIC_LOG` names (`info`, `debug`, ...). The Output panel
/// keeps `info` and above whatever this says.
fn level() -> log::LevelFilter {
    std::env::var("METALLIC_LOG")
        .ok()
        .and_then(|name| name.parse().ok())
        .unwrap_or(log::LevelFilter::Warn)
}
