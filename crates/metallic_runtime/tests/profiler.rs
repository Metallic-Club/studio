//! #23: a profiled Parkour game records its subsystems and its scripts.
use metallic_core::InstanceId;
use metallic_profiler::{Profiler, Subsystem};
use metallic_runtime::{FrameInput, Game, TICK};
use metallic_script::OutputSink;
use std::path::PathBuf;
use std::rc::Rc;

struct Quiet;

impl OutputSink for Quiet {
    fn print(&self, _: &str) {}
    fn warn(&self, _: &str) {}
    fn error(&self, _: &str) {}
}

fn parkour() -> Game {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../templates/parkour");
    let mut game = Game::open(&dir, None, Rc::new(Quiet)).expect("load Parkour");
    game.start().expect("start");
    game
}

#[test]
fn a_profiled_game_records_its_subsystems_and_scripts() {
    let mut game = parkour();
    let input = FrameInput::default();
    // Not enabled: the host's frame marks and the game's scopes record nothing.
    for _ in 0..5 {
        Profiler::begin_frame();
        game.frame(&input, TICK);
        Profiler::end_frame();
    }
    assert!(Profiler::with_frames(|f| f.is_empty()));

    Profiler::set_enabled(true);
    for _ in 0..30 {
        Profiler::begin_frame();
        game.frame(&input, TICK);
        Profiler::end_frame();
    }
    Profiler::set_enabled(false);
    assert_eq!(Profiler::with_frames(|f| f.len()), 30);

    let frame = Profiler::latest().expect("a frame");
    let names: Vec<_> = frame.scopes().iter().map(|s| s.name).collect();
    for name in [
        "game",
        "tick",
        "characters",
        "characters.body",
        "physics.step",
        "scripts.events",
        "scripts.step",
        "physics.sync",
        "avatar",
        "camera",
    ] {
        assert!(names.contains(&name), "{name} missing from {names:?}");
    }
    assert_eq!(frame.dropped_scopes(), 0);
    for subsystem in [Subsystem::Physics, Subsystem::Character, Subsystem::Script] {
        assert!(frame.subsystem_ns(subsystem) > 0, "{subsystem:?}");
    }

    // Some Parkour script ran in the 30 frames, and each timed script is a Script of the game.
    let world = game.world();
    let world = world.borrow();
    let mut scripts = Vec::new();
    Profiler::with_frames(|frames| {
        for f in frames.iter() {
            scripts.extend(f.scripts().iter().map(|s| s.script));
        }
    });
    assert!(!scripts.is_empty(), "no script time recorded");
    let mut all = Vec::new();
    for &root in world.roots() {
        descendants(&world, root, &mut all);
    }
    for bits in scripts {
        let id = all
            .iter()
            .find(|id| id.to_bits() == bits)
            .unwrap_or_else(|| panic!("timed script {bits} is not in the world"));
        let class = world.class_name(*id).unwrap_or_default();
        assert!(class.ends_with("Script"), "timed {class}");
    }
}

fn descendants(w: &metallic_core::World, id: InstanceId, out: &mut Vec<InstanceId>) {
    out.push(id);
    for &c in w.children(id) {
        descendants(w, c, out);
    }
}
