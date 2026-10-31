//! Command-line arguments.
use metallic_render::Quality;
use metallic_runtime::input::touch::DeviceProfile;
use std::path::PathBuf;

/// Usage text printed for bad arguments.
pub const USAGE: &str = "usage: metallic-player <project dir> [--scene <file relative to project>] \
[--headless --frames N] [--quality low|high] [--fps] [--autopilot] [--frames N] \
[--capture FRAME:PNG]... [--capture-view X,Y,Z,LX,LY,LZ] [--input-script <file>] [--no-overrides] [--menu main|keyboard|gamepad|assist|touch] \
[--emulate phone|tablet|small-phone[-portrait]] [--reduce-motion]";

/// Parsed arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    /// The project folder (holds `metallic.toml`).
    pub project: PathBuf,
    /// Scene to play instead of the main scene, relative to the project.
    pub scene: Option<PathBuf>,
    /// Run without a window or GPU.
    pub headless: bool,
    /// Stop after this many frames.
    pub frames: Option<u64>,
    /// Renderer quality.
    pub quality: Quality,
    /// Measure frame rate for 60 s (fixed 1920x1080 window, no vsync).
    pub fps: bool,
    /// Play without a person: run forward in a circle and jump every few seconds (to measure the
    /// frame rate with the character animating).
    pub autopilot: bool,
    /// Frames to save as PNG: (frame number, path).
    pub captures: Vec<(u64, PathBuf)>,
    /// Camera position and look-at point for captured frames instead of the game camera (for
    /// overview screenshots; the window still shows the game camera).
    pub capture_view: Option<[f32; 6]>,
    /// Input events to play, one per line: `frame device control state [values]` (see
    /// `metallic_runtime::input::ScriptedSource`). With a script every frame lasts 1/60 s of
    /// input time, so a run plays the same way every time.
    pub input_script: Option<PathBuf>,
    /// Ignore the player's saved controls (and do not save changes).
    pub no_overrides: bool,
    /// Start with the Player menu open: `main`, or a Controls tab (`keyboard`, `gamepad`,
    /// `assist`).
    pub menu: Option<String>,
    /// `--emulate`: play as this touch device (window in its shape, the mouse is a finger,
    /// the touch overlay always shows).
    pub emulate: Option<DeviceProfile>,
    /// `--reduce-motion`: the touch overlay appears and disappears without a fade.
    pub reduce_motion: bool,
}

impl Args {
    /// Parses the arguments after the program name.
    pub fn parse(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
        let mut project = None;
        let mut args = Args {
            project: PathBuf::new(),
            scene: None,
            headless: false,
            frames: None,
            quality: Quality::High,
            fps: false,
            autopilot: false,
            captures: Vec::new(),
            capture_view: None,
            input_script: None,
            no_overrides: false,
            menu: None,
            emulate: None,
            reduce_motion: false,
        };
        while let Some(a) = it.next() {
            let mut value = |name: &str| it.next().ok_or(format!("{name} needs a value"));
            match a.as_str() {
                "--scene" => args.scene = Some(PathBuf::from(value("--scene")?)),
                "--headless" => args.headless = true,
                "--input-script" => {
                    args.input_script = Some(PathBuf::from(value("--input-script")?))
                }
                "--emulate" => {
                    let v = value("--emulate")?;
                    args.emulate = Some(crate::touch_overlay::parse_emulation(&v).ok_or(
                        format!("unknown device `{v}` (phone, tablet or small-phone, optionally -portrait)"),
                    )?);
                }
                "--reduce-motion" => args.reduce_motion = true,
                "--fps" => args.fps = true,
                "--no-overrides" => args.no_overrides = true,
                "--menu" => {
                    let v = value("--menu")?;
                    if !matches!(
                        v.as_str(),
                        "main" | "keyboard" | "gamepad" | "assist" | "touch"
                    ) {
                        return Err(format!(
                            "unknown menu page `{v}` (main, keyboard, gamepad, assist or touch)"
                        ));
                    }
                    args.menu = Some(v);
                }
                "--autopilot" => args.autopilot = true,
                "--frames" => {
                    let v = value("--frames")?;
                    args.frames = Some(v.parse().map_err(|_| format!("bad frame count `{v}`"))?);
                }
                "--quality" => {
                    args.quality = match value("--quality")?.as_str() {
                        "low" => Quality::Low,
                        "high" => Quality::High,
                        other => return Err(format!("unknown quality `{other}` (low or high)")),
                    }
                }
                "--capture" => {
                    let v = value("--capture")?;
                    let (frame, path) = v
                        .split_once(':')
                        .ok_or(format!("--capture wants FRAME:PNG, got `{v}`"))?;
                    let frame = frame
                        .parse()
                        .map_err(|_| format!("bad capture frame `{frame}`"))?;
                    args.captures.push((frame, PathBuf::from(path)));
                }
                "--capture-view" => {
                    let v = value("--capture-view")?;
                    let numbers: Vec<f32> = v
                        .split(',')
                        .map(|n| n.trim().parse::<f32>())
                        .collect::<Result<_, _>>()
                        .map_err(|_| format!("bad --capture-view `{v}`"))?;
                    let view: [f32; 6] = numbers
                        .try_into()
                        .map_err(|_| format!("--capture-view wants 6 numbers, got `{v}`"))?;
                    args.capture_view = Some(view);
                }
                flag if flag.starts_with("--") => return Err(format!("unknown option `{flag}`")),
                path if project.is_none() => project = Some(PathBuf::from(path)),
                extra => return Err(format!("unexpected argument `{extra}`")),
            }
        }
        args.project = project.ok_or("missing project folder")?;
        Ok(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Args, String> {
        Args::parse(s.split_whitespace().map(String::from))
    }

    #[test]
    fn parses_the_smoke_test_command() {
        let a =
            parse("templates/parkour --scene tests/smoke.mscene --headless --frames 1200").unwrap();
        assert_eq!(a.project, PathBuf::from("templates/parkour"));
        assert_eq!(a.scene, Some(PathBuf::from("tests/smoke.mscene")));
        assert!(a.headless);
        assert_eq!(a.frames, Some(1200));
    }

    #[test]
    fn capture_keeps_windows_drive_colons_in_the_path() {
        let a = parse("p --capture 120:C:/shots/start.png --quality low").unwrap();
        assert_eq!(a.captures, vec![(120, PathBuf::from("C:/shots/start.png"))]);
        assert_eq!(a.quality, Quality::Low);
    }

    #[test]
    fn parses_the_running_fps_command() {
        let a = parse("templates/parkour --fps --autopilot").unwrap();
        assert!(a.fps && a.autopilot);
        assert!(!parse("p").unwrap().autopilot);
    }

    #[test]
    fn parses_the_input_script_option() {
        let a = parse("p --headless --input-script runs/jump.txt --frames 90").unwrap();
        assert_eq!(a.input_script, Some(PathBuf::from("runs/jump.txt")));
        assert!(parse("p --input-script").is_err());
        assert_eq!(parse("p").unwrap().input_script, None);
    }

    #[test]
    fn parses_the_emulate_option() {
        let a = parse("p --emulate phone --capture 60:shot.png").unwrap();
        assert_eq!(a.emulate, Some(DeviceProfile::phone()));
        let a = parse("p --emulate tablet-portrait --reduce-motion").unwrap();
        assert_eq!(a.emulate.map(|p| p.size), Some([820.0, 1180.0]));
        assert!(a.reduce_motion);
        assert!(parse("p --emulate watch").is_err());
        assert!(parse("p --emulate").is_err());
        assert_eq!(parse("p").unwrap().emulate, None);
    }

    #[test]
    fn capture_view_takes_six_numbers() {
        let a = parse("p --capture-view 1,2,3,4,5,-6").unwrap();
        assert_eq!(a.capture_view, Some([1.0, 2.0, 3.0, 4.0, 5.0, -6.0]));
        assert!(parse("p --capture-view 1,2,3").is_err());
    }

    #[test]
    fn parses_the_menu_and_override_options() {
        let a = parse("p --menu gamepad --no-overrides").unwrap();
        assert_eq!(a.menu.as_deref(), Some("gamepad"));
        assert!(a.no_overrides);
        assert!(parse("p --menu shop").is_err());
        assert_eq!(
            parse("p --menu touch").unwrap().menu.as_deref(),
            Some("touch")
        );
        assert_eq!(parse("p").unwrap().menu, None);
    }

    #[test]
    fn rejects_unknown_options_and_missing_project() {
        assert!(parse("p --nope").is_err());
        assert!(parse("--headless").is_err());
        assert!(parse("p --quality ultra").is_err());
    }
}
