//! `metallic-player`: plays a Metallic project.
//!
//! ```text
//! metallic-player <project dir> [--scene <file relative to project>] [--headless --frames N]
//!                 [--quality low|high] [--fps] [--autopilot] [--frames N]
//!                 [--capture FRAME:PNG]... [--input-script <file>]
//! ```
//! Windowed by default. Input is read as actions (`metallic_runtime::input`); the default
//! `CharacterControls`: WASD/arrows or the d-pad move, Space or A jumps, Shift or L3 runs,
//! right-drag or the right stick turns the camera, the wheel or R1/L1 zoom; the left stick
//! moves; Esc quits. `--headless` runs N frames of 1/60 s without a window or GPU (smoke tests).
//! `--input-script <file>` plays input events (`frame device control state [values]`, for
//! example `30 keyboard Space down`) in either mode, with 1/60 s of input time per frame. `--fps` plays 60 s in a fixed 1920x1080 window without
//! vsync and prints frame-rate statistics; with `--autopilot` the character runs in a circle and
//! jumps meanwhile, so the measurement includes the animated avatar. `--frames N` also ends a windowed run after N frames,
//! and `--capture FRAME:PNG` saves that frame (3D view and GUI) as a PNG.
//! Exit code: 0 on a clean exit, 1 when the project cannot be loaded or started, 2 for bad
//! arguments.
#![warn(missing_docs)]

mod args;
mod capture;
mod devices;
mod overlay;
mod window;

use metallic_runtime::input::ScriptedSource;
use metallic_runtime::{Game, InputRuntime, StdoutSink, TICK};
use std::process::ExitCode;
use std::rc::Rc;

fn main() -> ExitCode {
    let args = match args::Args::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("metallic-player: {msg}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let mut game = match Game::open(&args.project, args.scene.as_deref(), Rc::new(StdoutSink)) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    if let Err(e) = game.start() {
        eprintln!("error: {e}");
        return ExitCode::from(1);
    }
    let script = match args.input_script.as_deref().map(load_script) {
        Some(Ok(s)) => Some(s),
        Some(Err(e)) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
        None => None,
    };
    if args.headless {
        run_headless(&mut game, args.frames.unwrap_or(600), script);
        ExitCode::SUCCESS
    } else {
        window::run(game, &args, script)
    }
}

/// Reads and parses an `--input-script` file.
fn load_script(path: &std::path::Path) -> Result<ScriptedSource, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read input script {}: {e}", path.display()))?;
    ScriptedSource::parse(&text, TICK).map_err(|e| format!("input script {}: {e}", path.display()))
}

/// Runs `frames` frames of exactly one tick each, with the events of `script` (none: no input).
fn run_headless(game: &mut Game, frames: u64, mut script: Option<ScriptedSource>) {
    let mut input = InputRuntime::from_world(&game.world().borrow());
    for frame in 0..frames {
        let now = frame as f64 * TICK;
        if let Some(script) = script.as_mut() {
            let mut events = Vec::new();
            script.poll_frame(frame, now, &mut events);
            for e in events {
                input.push(e);
            }
        }
        if let Ok(world) = game.world().try_borrow() {
            input.sync(&world);
        }
        let frame_input = input.frame(now, TICK as f32);
        game.frame(&frame_input, TICK);
    }
    game.stop();
    println!(
        "metallic-player: {} ran {frames} frames ({:.1} s of game time)",
        game.name(),
        game.time()
    );
}
