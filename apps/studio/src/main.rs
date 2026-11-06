//! The `metallic_studio` executable: opens Studio. Everything else is in the library.

use std::process::ExitCode;

fn main() -> ExitCode {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(level());
    }
    match metallic_studio::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Logs on stderr: warnings and errors, or the level `METALLIC_LOG` names (`info`, `debug`, ...). Studio's and the
/// engine's crates log at that level; dependencies only from `warn`, because they are chatty.
struct StderrLogger;

static LOGGER: StderrLogger = StderrLogger;

fn level() -> log::LevelFilter {
    std::env::var("METALLIC_LOG")
        .ok()
        .and_then(|name| name.parse().ok())
        .unwrap_or(log::LevelFilter::Warn)
}

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        let ours = metadata.target().starts_with("metallic");
        metadata.level() <= log::max_level() && (ours || metadata.level() <= log::Level::Warn)
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("[{} {}] {}", record.level(), record.target(), record.args());
        }
    }

    fn flush(&self) {}
}
