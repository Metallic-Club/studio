//! The `metallic_studio` executable: opens Studio. Everything else is in the library.

use std::process::ExitCode;

fn main() -> ExitCode {
    metallic_studio::logging::install(level());
    match metallic_studio::run() {
        Ok(()) => ExitCode::SUCCESS,
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
