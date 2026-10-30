//! Plays the whole Parkour course with a simple bot that uses only player input (move, run,
//! jump), so every jump in the level is proven reachable with the real character controller.
//!
//! The bot walks from waypoint to waypoint (platform centres and the safe lanes of the lava walk),
//! jumps when there is no ground just ahead of it, waits for the moving platform and hops across
//! the spinning-bar arena. After a death it resumes from its last checkpoint. It must finish
//! within the time and death limits below, and the HUD must show the checkpoints and the time.
use metallic_core::{InstanceId, Value, Vec3, World};
use metallic_physics::{HumanoidState, PhysicsWorld};
use metallic_runtime::{FrameInput, Game, StdoutSink, TICK};
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Debug, Clone, Copy)]
enum Step {
    /// Walk (and jump) to this point on a platform top: x, top y, z.
    Go(f64, f64, f64),
    /// After a death, continue from the step after the last `Resume` passed.
    Resume,
    /// Stand still until the moving platform's Z is at least this.
    MoverAtLeast(f64),
    /// Walk onto the moving platform's centre.
    Mover,
    /// Ride the moving platform until its Z is at most this.
    MoverAtMost(f64),
    /// Keep jumping (true) or stop (false) during the following `Go` steps.
    Hop(bool),
}

use Step::*;

const ROUTE: &[Step] = &[
    Resume,
    Go(0.0, 0.0, -12.0),
    Go(0.0, 0.0, -22.0),
    Go(3.0, 0.0, -34.0),
    Go(-2.0, 1.5, -46.0),
    Go(2.0, 3.0, -58.0),
    Go(0.0, 4.5, -70.0),
    Go(0.0, 6.0, -80.0),
    Go(0.0, 7.5, -89.0),
    Go(0.0, 9.0, -98.0),
    Go(0.0, 10.5, -107.0),
    Go(0.0, 12.0, -121.0),
    Resume,
    // The lava walk: through each safe lane, turning only on the floor tiles between strips.
    Go(5.0, 12.0, -134.0),
    Go(5.0, 12.0, -143.0),
    Go(-5.0, 12.0, -147.0),
    Go(-5.0, 12.0, -155.0),
    Go(0.0, 12.0, -159.0),
    Go(0.0, 12.0, -167.0),
    Go(5.25, 12.0, -171.0),
    Go(5.25, 12.0, -180.5),
    Go(0.0, 12.0, -181.5),
    MoverAtLeast(-188.0),
    Mover,
    MoverAtMost(-201.0),
    Go(0.0, 12.0, -214.0),
    Resume,
    Go(4.0, 12.0, -220.0),
    Hop(true),
    Go(4.0, 12.0, -252.0),
    Hop(false),
    Go(4.0, 13.5, -262.0),
    Go(-3.0, 15.0, -273.0),
    Go(-3.0, 16.5, -283.0),
    Go(-3.0, 16.5, -299.0),
    Go(-3.0, 16.5, -309.0),
    Go(-3.0, 18.5, -319.0),
    Go(2.0, 18.5, -325.0),
    Go(-2.0, 18.5, -331.0),
    Go(3.0, 18.5, -337.0),
    Go(0.0, 18.5, -350.0),
    Resume,
    Go(4.0, 20.0, -366.0),
    Go(-2.0, 21.5, -378.0),
    Go(3.0, 23.0, -391.0),
    Go(-1.0, 24.0, -404.0),
    Go(0.0, 25.0, -424.0),
];

/// The bot gives up when one step takes longer than this (seconds of game time).
const STEP_LIMIT: f64 = 20.0;
/// Most deaths allowed on the way.
const MAX_DEATHS: u32 = 6;
/// The run must end within this much game time.
const TIME_LIMIT: f64 = 400.0;

fn vec3(w: &World, id: InstanceId, prop: &str) -> Vec3 {
    match w.get(id, prop) {
        Ok(Value::Vector3(v)) => v,
        _ => Vec3::default(),
    }
}

fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

fn parts_of(w: &World, model: InstanceId) -> Vec<InstanceId> {
    let mut out = Vec::new();
    let mut stack = vec![model];
    while let Some(id) = stack.pop() {
        if w.is_a(id, "BasePart") {
            out.push(id);
        }
        stack.extend_from_slice(w.children(id));
    }
    out
}

fn hud_text(w: &World) -> String {
    let path = [
        "Players",
        "Player1",
        "PlayerGui",
        "HUD",
        "Banner",
        "Message",
    ];
    let mut cur = w.root(path[0]);
    for name in &path[1..] {
        cur = cur.and_then(|c| w.find_first_child(c, name));
    }
    match cur.map(|m| w.get(m, "Text")) {
        Some(Ok(Value::String(s))) => s,
        _ => String::new(),
    }
}

struct Bot {
    game: Game,
    /// Static copy of the level for "is there ground ahead" rays.
    probe: PhysicsWorld,
    step: usize,
    resume: usize,
    hop: bool,
    deaths: u32,
    step_time: f64,
    seen: Vec<String>,
}

impl Bot {
    fn world(&self) -> Rc<std::cell::RefCell<World>> {
        self.game.world()
    }

    fn root(&self) -> (Vec3, Vec<InstanceId>) {
        let world = self.world();
        let w = world.borrow();
        let model = self.game.character().expect("character");
        let root = w
            .find_first_child(model, "HumanoidRootPart")
            .expect("root part");
        (vec3(&w, root, "Position"), parts_of(&w, model))
    }

    fn mover_z(&self) -> f64 {
        let world = self.world();
        let w = world.borrow();
        let ws = w.root("Workspace").expect("workspace");
        let mover = w.find_first_child(ws, "MovingPlatform").expect("mover");
        vec3(&w, mover, "Position").z
    }

    /// True when a ray just ahead of the feet finds a floor no more than 0.7 studs down.
    fn ground_ahead(&self, root: Vec3, dir: Vec3, ignore: &[InstanceId]) -> bool {
        let feet = root.y - 3.0;
        let from = root + dir * 1.3;
        let origin = Vec3::new(from.x, feet + 0.5, from.z);
        self.probe
            .raycast(origin, Vec3::new(0.0, -1.2, 0.0), ignore)
            .is_some()
    }

    fn tick(&mut self, input: FrameInput) -> Option<HumanoidState> {
        let out = self.game.frame(&input, TICK);
        self.step_time += TICK;
        let text = hud_text(&self.world().borrow());
        if self.seen.last() != Some(&text) {
            self.seen.push(text);
        }
        out.state
    }

    /// Input that moves toward `target` (x, feet height, z); `None` once standing there.
    fn steer(&self, target: Vec3, state: Option<HumanoidState>) -> Option<FrameInput> {
        let (root, own) = self.root();
        let to = flat(target - root);
        let grounded = state == Some(HumanoidState::Running);
        let feet = root.y - 3.0;
        if to.magnitude() < 1.0 && grounded && (feet - target.y).abs() < 0.6 {
            return None;
        }
        let dir = to.unit();
        let jump = grounded && (self.hop || !self.ground_ahead(root, dir, &own));
        // Camera yaw stays 0, so "forward" is -Z and "right" is +X.
        let speed = if to.magnitude() < 1.0 { 0.0 } else { 1.0 };
        Some(FrameInput {
            movement: [(dir.x * speed) as f32, (-dir.z * speed) as f32],
            jump,
            run: true,
            ..FrameInput::default()
        })
    }

    fn run(&mut self) -> f64 {
        let mut state = self.tick(FrameInput::default());
        let mut last_character = self.game.character();
        while self.step < ROUTE.len() {
            assert!(
                self.game.time() < TIME_LIMIT,
                "out of time at step {}",
                self.step
            );
            assert!(
                self.step_time < STEP_LIMIT,
                "stuck at step {} {:?}, root {:?}",
                self.step,
                ROUTE[self.step],
                self.root().0
            );
            if state == Some(HumanoidState::Dead) {
                println!(
                    "death {} at step {} {:?}, root {:?}, t = {:.1}",
                    self.deaths + 1,
                    self.step,
                    ROUTE[self.step],
                    self.root().0,
                    self.game.time()
                );
                while self.game.character() == last_character {
                    self.tick(FrameInput::default());
                }
                last_character = self.game.character();
                self.deaths += 1;
                assert!(self.deaths <= MAX_DEATHS, "{} deaths", self.deaths);
                self.step = self.resume;
                self.step_time = 0.0;
                self.hop = false;
                self.probe.sync_from(&self.world().borrow());
                state = self.tick(FrameInput::default());
                continue;
            }
            let input = match ROUTE[self.step] {
                Go(x, y, z) => self.steer(Vec3::new(x, y, z), state),
                Resume => {
                    self.resume = self.step + 1;
                    None
                }
                Hop(on) => {
                    self.hop = on;
                    None
                }
                MoverAtLeast(z) => (self.mover_z() < z).then(FrameInput::default),
                Mover => {
                    let target = Vec3::new(0.0, 12.0, self.mover_z());
                    let (root, _) = self.root();
                    let on = flat(target - root).magnitude() < 2.5
                        && state == Some(HumanoidState::Running)
                        && (root.y - 15.0).abs() < 0.6;
                    if on {
                        None
                    } else {
                        self.steer(target, state).or(Some(FrameInput::default()))
                    }
                }
                MoverAtMost(z) => (self.mover_z() > z).then(FrameInput::default),
            };
            match input {
                Some(i) => state = self.tick(i),
                None => {
                    self.step += 1;
                    self.step_time = 0.0;
                }
            }
        }
        // Let the Finish script and the HUD react.
        for _ in 0..30 {
            self.tick(FrameInput::default());
        }
        self.game.time()
    }
}

#[test]
fn a_bot_using_only_player_input_finishes_the_course() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../templates/parkour");
    let mut game = Game::open(&dir, None, Rc::new(StdoutSink)).expect("load Parkour");
    game.start().expect("start");
    let mut probe = PhysicsWorld::new();
    probe.sync_from(&game.world().borrow());
    let mut bot = Bot {
        game,
        probe,
        step: 0,
        resume: 0,
        hop: false,
        deaths: 0,
        step_time: 0.0,
        seen: Vec::new(),
    };
    let time = bot.run();
    println!(
        "bot finished in {time:.1} s of game time with {} deaths; HUD: {:?}",
        bot.deaths, bot.seen
    );
    for expected in ["Checkpoint 1/3", "Checkpoint 2/3", "Checkpoint 3/3"] {
        assert!(bot.seen.iter().any(|t| t == expected), "{:?}", bot.seen);
    }
    let last = bot.seen.last().cloned().unwrap_or_default();
    assert!(
        last.starts_with("Finished in ") && last.ends_with(" s"),
        "HUD at the end: {last:?}"
    );
    let seconds: f64 = last["Finished in ".len()..last.len() - 2]
        .parse()
        .expect("a number");
    assert!(
        last.split('.').nth(1).map(str::len) == Some(3),
        "one decimal: {last}"
    );
    assert!(seconds > 20.0 && seconds <= time, "{seconds} vs {time}");
}
