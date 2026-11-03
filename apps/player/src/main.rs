//! The `metallic_player` executable: plays a built-in game in a window.
//!
//! ```text
//! metallic_player                      # the demo, until the window is closed
//! metallic_player --bench 600          # 600 frames after a warm-up, then the stats as JSON on stdout
//! metallic_player --scene demo --preset high
//! ```

use std::io::Write as _;
use std::process::ExitCode;
use std::sync::OnceLock;

use metallic_engine::StdOutput;
use metallic_engine::metallic_script::{OutputSink, OutputSource};
use metallic_player::{Player, PlayerConfig, Preset, Scene};

const HELP: &str = "\
Metallic Player: runs a game in a window.

Usage: metallic_player [OPTIONS]

Options:
  --scene <NAME>     The built-in game to run [default: demo] (scenes: {scenes})
  --preset <NAME>    The graphics preset (presets: {presets}); kept until graphics settings land (#84)
  --bench <FRAMES>   Run FRAMES frames after a warm-up, without a frame cap, print the stats as JSON and exit
  -h, --help         Print this help
  -V, --version      Print the version";

/// What the command line asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Args {
    scene: Scene,
    preset: Option<Preset>,
    bench: Option<u64>,
    help: bool,
    version: bool,
}

impl Args {
    /// Parses the arguments after the program name.
    fn parse(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Self, lexopt::Error> {
        use lexopt::prelude::*;

        let mut parsed = Args::default();
        let mut parser = lexopt::Parser::from_args(args);
        while let Some(arg) = parser.next()? {
            match arg {
                Long("scene") => parsed.scene = parser.value()?.parse()?,
                Long("preset") => parsed.preset = Some(parser.value()?.parse()?),
                Long("bench") => {
                    let frames: u64 = parser.value()?.parse()?;
                    if frames == 0 {
                        return Err("--bench needs at least 1 frame".into());
                    }
                    parsed.bench = Some(frames);
                }
                Short('h') | Long("help") => parsed.help = true,
                Short('V') | Long("version") => parsed.version = true,
                _ => return Err(arg.unexpected()),
            }
        }
        Ok(parsed)
    }

    fn config(&self) -> PlayerConfig {
        let mut config = match self.bench {
            Some(frames) => PlayerConfig::bench(self.scene, frames),
            None => PlayerConfig::new(self.scene),
        };
        config.preset = self.preset;
        config
    }
}

fn help() -> String {
    let names = |names: Vec<&str>| names.join(", ");
    HELP.replace("{scenes}", &names(Scene::ALL.iter().map(|s| s.name()).collect()))
        .replace("{presets}", &names(Preset::ALL.iter().map(|p| p.name()).collect()))
}

fn main() -> ExitCode {
    let args = match Args::parse(std::env::args_os().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("error: {error}\n\n{}", help());
            return ExitCode::from(2);
        }
    };
    if args.help {
        println!("{}", help());
        return ExitCode::SUCCESS;
    }
    if args.version {
        println!("Metallic Player {}", metallic_player::VERSION);
        return ExitCode::SUCCESS;
    }
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(LOGGER.level());
    }

    let config = args.config();
    if args.bench.is_none() {
        eprintln!(
            "Metallic Player {}: scene `{}`. WASD and Q/E move, hold the right mouse button to look, C orbits, \
             Space pauses.",
            metallic_player::VERSION,
            config.scene
        );
    }
    let result = if args.bench.is_some() {
        // Stdout carries only the JSON, so it can be piped.
        Player::with_output(config, PrintsToStderr).and_then(Player::run)
    } else {
        Player::new(config).and_then(Player::run)
    };
    match result {
        Ok(report) => {
            if args.bench.is_some() {
                println!("{}", report.to_json());
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Script output for `--bench`: everything on stderr, prints included.
struct PrintsToStderr;

impl OutputSink for PrintsToStderr {
    fn print(&mut self, _from: &OutputSource<'_>, message: &str) {
        let _ = writeln!(std::io::stderr().lock(), "{message}");
    }

    fn warn(&mut self, from: &OutputSource<'_>, message: &str) {
        StdOutput.warn(from, message);
    }

    fn error(&mut self, from: &OutputSource<'_>, message: &str) {
        StdOutput.error(from, message);
    }
}

/// Engine and renderer logs on stderr: warnings and errors, or the level `METALLIC_LOG` names (`info`, `debug`,
/// ...). Our own crates log at that level; wgpu and the other dependencies only from `warn`, unless
/// `METALLIC_LOG` ends in `!` (`debug!`), because they are very chatty.
struct StderrLogger;

static LOGGER: StderrLogger = StderrLogger;

impl StderrLogger {
    /// The level, and whether dependencies log at it too: read from `METALLIC_LOG` once.
    fn setting() -> (log::LevelFilter, bool) {
        static SETTING: OnceLock<(log::LevelFilter, bool)> = OnceLock::new();
        *SETTING.get_or_init(|| {
            let value = std::env::var("METALLIC_LOG").unwrap_or_default();
            let (name, everything) = match value.strip_suffix('!') {
                Some(name) => (name, true),
                None => (value.as_str(), false),
            };
            (name.parse().unwrap_or(log::LevelFilter::Warn), everything)
        })
    }

    fn level(&self) -> log::LevelFilter {
        Self::setting().0
    }
}

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        let (level, everything) = Self::setting();
        let ours = metadata.target().starts_with("metallic");
        metadata.level() <= level && (ours || everything || metadata.level() <= log::Level::Warn)
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("[{} {}] {}", record.level(), record.target(), record.args());
        }
    }

    fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse(args.iter().map(|arg| (*arg).into())).map_err(|error| error.to_string())
    }

    #[test]
    fn no_arguments_play_the_demo() {
        let args = parse(&[]).unwrap();
        assert_eq!(args, Args::default());
        let config = args.config();
        assert_eq!(config.scene, Scene::Demo);
        assert_eq!(config.max_frames, None);
        assert!(config.frame_cap.is_some());
    }

    #[test]
    fn every_flag_is_read() {
        let args = parse(&["--scene", "demo", "--preset=ultra", "--bench", "120"]).unwrap();
        assert_eq!(args.scene, Scene::Demo);
        assert_eq!(args.preset, Some(Preset::Ultra));
        assert_eq!(args.bench, Some(120));
        let config = args.config();
        assert_eq!(config.max_frames, Some(120));
        assert_eq!(config.preset, Some(Preset::Ultra));
        assert_eq!(config.frame_cap, None, "a bench is paced by vsync only");
        assert!(parse(&["-h"]).unwrap().help);
        assert!(parse(&["--version"]).unwrap().version);
    }

    #[test]
    fn bad_arguments_say_what_is_wrong() {
        let error = parse(&["--scene", "nowhere"]).unwrap_err();
        assert!(error.contains("unknown scene `nowhere`"), "{error}");
        let error = parse(&["--preset", "epic"]).unwrap_err();
        assert!(error.contains("unknown preset `epic`"), "{error}");
        assert!(parse(&["--bench", "lots"]).is_err());
        assert!(parse(&["--bench", "0"]).unwrap_err().contains("at least 1 frame"));
        assert!(parse(&["--bench"]).is_err(), "a value is required");
        assert!(parse(&["--fullscreen"]).unwrap_err().contains("--fullscreen"));
        assert!(parse(&["extra"]).is_err());
    }

    #[test]
    fn the_help_lists_scenes_and_presets() {
        let text = help();
        assert!(text.contains("(scenes: demo)"));
        assert!(text.contains("(presets: low, medium, high, ultra, auto)"));
    }
}
