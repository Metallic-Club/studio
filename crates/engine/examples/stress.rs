//! Runs a stress scene (ADR 0008 §10) in a window for a number of frames, prints the frame statistics as JSON and
//! exits.
//!
//! ```text
//! cargo run --release -p metallic_engine --example stress -- --scene parts_100k
//! cargo run --release -p metallic_engine --example stress -- --scene all --frames 2000 > results.json
//! cargo run --release -p metallic_engine --example stress -- --list
//! ```
//!
//! Options: `--scene <name|all>`, `--frames <n>` (measured, 1000), `--warm-up <n>` (120), `--seed <n>` (1),
//! `--size <width>x<height>` (logical pixels; the scene's own by default), `--vsync` (off by default, so frame
//! times show the work), `--overlay` (show the performance overlay while running), `--no-cull` (draw every part, the
//! numbers before culling), `--view-distance <m>` and `--min-screen-size <px>` (the culling thresholds; infinite and
//! 1 by default), `--fov <degrees>` (narrow or widen the scene's camera). `WGPU_BACKEND=dx12` (or `vulkan`, `gl`)
//! picks the backend.
//!
//! `--scene all` runs every scene, each in its own process (the event loop can only be made once per process), and
//! prints a JSON array. The JSON's fields are `metallic_engine::stress::BenchReport`'s; times are in milliseconds.

use std::error::Error;
use std::process::{Command, ExitCode};

use metallic_engine::stress::{BenchOptions, StressKind, run_bench};
use metallic_render::CullSettings;

fn usage() -> String {
    format!(
        "usage: stress --scene <name|all> [--frames N] [--warm-up N] [--seed N] [--size WxH] [--vsync] [--overlay]\n\
         \x20             [--no-cull] [--view-distance M] [--min-screen-size PX] [--fov DEGREES]\n\
         scenes: {}",
        StressKind::names().join(", ")
    )
}

/// The parsed command line: the scene (or `None` for all) and the options for each.
fn parse(args: &[String]) -> Result<Option<(Option<StressKind>, BenchOptions)>, String> {
    let mut scene = None;
    let mut options = BenchOptions::new(StressKind::Parts10k);
    let mut it = args.iter();
    let number = |value: Option<&String>, flag: &str| -> Result<u64, String> {
        value
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| format!("{flag} needs a number"))
    };
    let decimal = |value: Option<&String>, flag: &str| -> Result<f32, String> {
        value
            .and_then(|v| v.parse().ok())
            .filter(|v: &f32| v.is_finite() && *v >= 0.0)
            .ok_or_else(|| format!("{flag} needs a number"))
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--list" => {
                for kind in StressKind::ALL {
                    println!("{}", kind.name());
                }
                return Ok(None);
            }
            "--scene" => {
                let name = it.next().ok_or("--scene needs a name")?;
                scene = Some(if name == "all" {
                    None
                } else {
                    Some(StressKind::from_name(name).ok_or_else(|| format!("no scene {name:?}"))?)
                });
            }
            "--frames" => options.frames = u32::try_from(number(it.next(), "--frames")?).map_err(|e| e.to_string())?,
            "--warm-up" => {
                options.warm_up = u32::try_from(number(it.next(), "--warm-up")?).map_err(|e| e.to_string())?;
            }
            "--seed" => options.seed = number(it.next(), "--seed")?,
            "--size" => {
                let size = it.next().ok_or("--size needs WxH")?;
                let (w, h) = size.split_once('x').ok_or("--size needs WxH")?;
                let w = w.parse().map_err(|_| "--size needs WxH")?;
                let h = h.parse().map_err(|_| "--size needs WxH")?;
                options.window = Some((w, h));
            }
            "--vsync" => options.vsync = true,
            "--overlay" => options.overlay = true,
            "--no-cull" => options.cull = CullSettings::OFF,
            "--view-distance" => options.cull.max_distance = decimal(it.next(), "--view-distance")?,
            "--min-screen-size" => options.cull.min_screen_size = decimal(it.next(), "--min-screen-size")?,
            "--fov" => options.field_of_view = Some(decimal(it.next(), "--fov")?),
            // `cargo bench`-style runners pass this; nothing to do.
            "--bench" => {}
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let scene = scene.ok_or("--scene is required")?;
    if let Some(kind) = scene {
        options.scene = kind;
    }
    Ok(Some((scene, options)))
}

/// Runs every scene, each in a child process with the same options, and prints a JSON array.
fn run_all(args: &[String]) -> Result<(), Box<dyn Error>> {
    let exe = std::env::current_exe()?;
    let mut reports = Vec::new();
    for kind in StressKind::ALL {
        eprintln!("stress: running {}", kind.name());
        let mut child_args: Vec<String> = Vec::with_capacity(args.len());
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            if arg == "--scene" {
                it.next();
                continue;
            }
            child_args.push(arg.clone());
        }
        let output = Command::new(&exe)
            .args(&child_args)
            .args(["--scene", kind.name()])
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "{} failed: {}",
                kind.name(),
                String::from_utf8_lossy(&output.stderr).trim()
            )
            .into());
        }
        let stdout = String::from_utf8(output.stdout)?;
        let json = stdout
            .lines()
            .rev()
            .find(|line| line.starts_with('{'))
            .ok_or_else(|| format!("{} printed no report", kind.name()))?;
        reports.push(json.to_owned());
    }
    println!("[\n{}\n]", reports.join(",\n"));
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match parse(&args) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => return ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}\n{}", usage());
            return ExitCode::FAILURE;
        }
    };
    let result = match parsed {
        (None, _) => run_all(&args),
        (Some(_), options) => run_bench(&options)
            .map(|report| println!("{}", report.to_json()))
            .map_err(Into::into),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("stress: {error}");
            ExitCode::FAILURE
        }
    }
}
