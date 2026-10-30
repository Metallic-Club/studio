//! Command-line arguments.
use metallic_render::Quality;
use std::path::PathBuf;

/// Usage text printed for bad arguments.
pub const USAGE: &str = "usage: metallic-player <project dir> [--scene <file relative to project>] \
[--headless --frames N] [--quality low|high] [--fps] [--frames N] [--capture FRAME:PNG]...";

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
    /// Frames to save as PNG: (frame number, path).
    pub captures: Vec<(u64, PathBuf)>,
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
            captures: Vec::new(),
        };
        while let Some(a) = it.next() {
            let mut value = |name: &str| it.next().ok_or(format!("{name} needs a value"));
            match a.as_str() {
                "--scene" => args.scene = Some(PathBuf::from(value("--scene")?)),
                "--headless" => args.headless = true,
                "--fps" => args.fps = true,
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
    fn rejects_unknown_options_and_missing_project() {
        assert!(parse("p --nope").is_err());
        assert!(parse("--headless").is_err());
        assert!(parse("p --quality ultra").is_err());
    }
}
