//! `metallic-player`: plays a Metallic project.
//!
//! ```text
//! metallic-player <project dir> [--scene <file relative to project>] [--headless --frames N]
//!                 [--quality low|high] [--fps] [--autopilot] [--frames N]
//!                 [--capture FRAME:PNG]...
//! ```
//! Windowed by default (WASD/arrows move, Space jumps, Shift runs, right-drag orbits, wheel
//! zooms, Esc quits; gamepad left stick, A, right stick). `--headless` runs N frames of 1/60 s
//! without a window or GPU (smoke tests). `--fps` plays 60 s in a fixed 1920x1080 window without
//! vsync and prints frame-rate statistics; with `--autopilot` the character runs in a circle and
//! jumps meanwhile, so the measurement includes the animated avatar. `--frames N` also ends a windowed run after N frames,
//! and `--capture FRAME:PNG` saves that frame (3D view and GUI) as a PNG.
//! Exit code: 0 on a clean exit, 1 when the project cannot be loaded or started, 2 for bad
//! arguments.
#![warn(missing_docs)]

mod args;
mod capture;
mod overlay;
mod window;

use metallic_runtime::{FrameInput, Game, StdoutSink, TICK};
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
    if args.headless {
        run_headless(&mut game, args.frames.unwrap_or(600));
        ExitCode::SUCCESS
    } else {
        window::run(game, &args)
    }
}

/// Runs `frames` frames of exactly one tick each, with no input.
fn run_headless(game: &mut Game, frames: u64) {
    let input = FrameInput::default();
    for _ in 0..frames {
        game.frame(&input, TICK);
    }
    game.stop();
    println!(
        "metallic-player: {} ran {frames} frames ({:.1} s of game time)",
        game.name(),
        game.time()
    );
}
