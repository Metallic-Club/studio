//! The `metallic_player` executable: plays a built-in game in a window.
//!
//! ```text
//! metallic_player                      # the demo, until the window is closed
//! metallic_player --bench 600          # 600 frames after a warm-up, then the stats as JSON on stdout
//! metallic_player --scene demo --preset high
//! ```
//!
//! The graphics settings are read from `graphics.toml` in the user's config folder (made with the Auto preset the
//! first time); `--preset` overrides its preset for one run. Compiled pipelines are kept in the user's cache folder
//! (`pipelines/`, on Vulkan) so the next start compiles less.

use std::io::Write as _;
use std::process::ExitCode;
use std::sync::OnceLock;

use metallic_engine::StdOutput;
use metallic_engine::graphics::{
    graphics_settings_path, load_graphics_settings, pipeline_cache_dir, save_graphics_settings,
};
use metallic_engine::metallic_platform::AppId;
use metallic_engine::metallic_script::{OutputSink, OutputSource};
use metallic_player::{PLAYER_APP_ID, Player, PlayerConfig, Preset, Scene};

const HELP: &str = "\
Metallic Player: runs a game in a window.

Usage: metallic_player [OPTIONS]

Options:
  --scene <NAME>     The built-in game to run [default: demo] (scenes: {scenes})
  --preset <NAME>    The graphics preset for this run, in place of the settings file's (presets: {presets})
  --bench <FRAMES>   Run FRAMES frames after a warm-up, without a frame cap, print the stats as JSON and exit
  --overlay          Start with the performance overlay shown (F3 toggles it)
  -h, --help         Print this help
  -V, --version      Print the version";

/// What the command line asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Args {
    scene: Scene,
    preset: Option<Preset>,
    bench: Option<u64>,
    overlay: bool,
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
                Long("overlay") => parsed.overlay = true,
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
        config.overlay = self.overlay;
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

    let mut config = args.config();
    load_user_graphics(&mut config);
    if args.bench.is_none() {
        eprintln!(
            "Metallic Player {}: scene `{}`. {}F3 shows the performance overlay.",
            metallic_player::VERSION,
            config.scene,
            if config.scene == Scene::Demo {
                "WASD and Q/E move, hold the right mouse button to look, C orbits, Space pauses. "
            } else {
                ""
            }
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

/// Loads the player's graphics settings into `config` from their file in the user's config folder: the Auto preset
/// when there is none (the file is then made, so there is one to edit) or it is damaged (with a warning; the file is
/// left as it is until the settings are next saved). `--preset` still overrides the file's preset for this run.
fn load_user_graphics(config: &mut PlayerConfig) {
    // Compiled pipelines are kept beside the user's other caches (Vulkan only; nothing is written elsewhere).
    config.pipeline_cache = AppId::parse(PLAYER_APP_ID)
        .and_then(|app| pipeline_cache_dir(&app))
        .ok();
    let path = match AppId::parse(PLAYER_APP_ID).and_then(|app| graphics_settings_path(&app)) {
        Ok(path) => path,
        Err(error) => {
            log::warn!("graphics: no settings file ({error}); using the Auto preset");
            return;
        }
    };
    let loaded = load_graphics_settings(&path);
    if let Some(problem) = &loaded.problem {
        log::warn!("graphics: {problem}");
    } else if !loaded.from_file
        && let Err(error) = save_graphics_settings(&path, &loaded.settings)
    {
        log::warn!("graphics: could not write {}: {error}", path.display());
    }
    config.graphics = loaded.settings;
    config.graphics_file = Some(path);
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
        let stress = parse(&["--scene", "parts_100k", "--overlay"]).unwrap();
        assert_eq!(stress.scene.name(), "parts_100k");
        assert!(stress.config().overlay);
        assert_eq!(stress.config().window.size.width, 1920);
        let args = parse(&["--scene", "demo", "--preset=ultra", "--bench", "120"]).unwrap();
        assert_eq!(args.scene, Scene::Demo);
        assert!(!args.overlay);
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
        assert!(text.contains(
            "(scenes: demo, parts_10k, parts_50k, parts_100k, many_lights, overdraw, mobile, see_through_10k)"
        ));
        assert!(text.contains("--overlay"));
        assert!(text.contains("(presets: low, medium, high, ultra, auto)"));
    }
}
