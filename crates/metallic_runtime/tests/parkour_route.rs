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
    /// Stand still until the named moving platform's Z is at least this.
    MoverAtLeast(&'static str, f64),
    /// Walk onto the centre of the named moving platform, whose top is at this height.
    Mover(&'static str, f64),
    /// Ride the named moving platform until its Z is at most this.
    MoverAtMost(&'static str, f64),
    /// Keep jumping (true) or stop (false) during the following `Go` steps.
    Hop(bool),
}

use Step::*;

/// Where `examples/build_parkour/level.rs` places sections 2, 3 and 4: (Y, Z) added to the
/// section coordinates used below.
const SLATE_AT: (f64, f64) = (0.0, -61.0);
const MOSS_AT: (f64, f64) = (0.0, -115.0);
const ICE_AT: (f64, f64) = (10.0, -115.0);

/// The tower climb as built by the level (section 3 coordinates): centre (x, z), radius, first
/// angle and step (degrees, 0 = +Z), first top, rise, pad count.
const TOWER: ((f64, f64), f64, f64, f64, f64, f64, usize) =
    ((0.0, -450.0), 8.0, 250.0, 50.0, 22.0, 2.5, 14);

/// Moves a section's steps from section coordinates into the world.
fn placed(at: (f64, f64), steps: Vec<Step>) -> Vec<Step> {
    let (dy, dz) = at;
    steps
        .into_iter()
        .map(|s| match s {
            Go(x, y, z) => Go(x, y + dy, z + dz),
            MoverAtLeast(n, z) => MoverAtLeast(n, z + dz),
            Mover(n, top) => Mover(n, top + dy),
            MoverAtMost(n, z) => MoverAtMost(n, z + dz),
            other => other,
        })
        .collect()
}

/// A lava walk starting at `z0`: through each safe lane, turning only on the floor tiles between
/// strips. `side` is 1 for the first walk and -1 for the mirrored second one.
fn lava_walk(z0: f64, side: f64) -> Vec<Step> {
    [
        (5.0, 5.0),
        (5.0, 14.0),
        (-5.0, 18.0),
        (-5.0, 26.0),
        (0.0, 30.0),
        (0.0, 38.0),
        (5.25, 42.0),
        (5.25, 51.5),
        (0.0, 52.5),
    ]
    .into_iter()
    .map(|(x, d)| Go(x * side, 13.5, z0 - d))
    .collect()
}

fn route() -> Vec<Step> {
    let mut r = vec![
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
        Go(-5.0, 10.5, -118.0),
        Go(3.0, 12.0, -129.0),
        Go(-3.0, 12.0, -141.0),
        Go(2.0, 13.5, -152.0),
        Go(0.0, 13.5, -158.0),
        Go(0.0, 13.5, -182.0),
        Go(-4.0, 15.0, -190.0),
        Go(3.0, 15.0, -201.0),
        Go(-2.0, 13.5, -212.0),
        Go(0.0, 13.5, -228.0),
        Resume,
    ];

    let mut slate = lava_walk(-175.0, 1.0);
    slate.extend([
        MoverAtLeast("MovingPlatform", -234.0),
        Mover("MovingPlatform", 13.5),
        MoverAtMost("MovingPlatform", -247.0),
        Go(0.0, 13.5, -258.0),
        // The beams over the lava river.
        Go(-2.0, 13.5, -265.0),
        Go(-2.0, 13.5, -278.0),
        Go(0.0, 13.5, -281.0),
        Go(2.0, 13.5, -285.0),
        Go(2.0, 13.5, -299.0),
    ]);
    slate.extend(lava_walk(-300.0, -1.0));
    slate.extend([Go(0.0, 13.5, -362.0), Resume]);
    r.extend(placed(SLATE_AT, slate));

    let mut moss = vec![
        Go(4.0, 13.5, -314.0),
        Hop(true),
        Go(4.0, 13.5, -346.0),
        Hop(false),
        Go(4.0, 15.0, -356.0),
        Go(-3.0, 16.5, -367.0),
        Go(-3.0, 18.0, -377.0),
        Go(-3.0, 18.0, -393.0),
        Go(-3.0, 18.0, -403.0),
        Go(-3.0, 20.0, -413.0),
        Go(2.0, 20.0, -419.0),
        Go(-2.0, 20.0, -425.0),
        Go(3.0, 20.0, -431.0),
        Go(-8.0, 20.0, -441.0),
        Hop(true),
    ];
    let ((cx, cz), radius, first, step, top, rise, pads) = TOWER;
    for i in 0..pads {
        let a = (first + step * i as f64).to_radians();
        moss.push(Go(
            cx + radius * a.sin(),
            top + rise * i as f64,
            cz + radius * a.cos(),
        ));
    }
    moss.extend([Hop(false), Go(0.0, 54.5, -474.0), Resume]);
    r.extend(placed(MOSS_AT, moss));

    r.extend(placed(
        ICE_AT,
        vec![
            Go(4.0, 46.0, -490.0),
            Go(-2.0, 47.5, -502.0),
            Go(3.0, 49.0, -515.0),
            Go(-1.0, 50.0, -528.0),
            Go(0.0, 50.0, -540.0),
            Go(0.0, 50.0, -543.5),
            MoverAtLeast("MovingPlatform2", -550.0),
            Mover("MovingPlatform2", 50.0),
            MoverAtMost("MovingPlatform2", -563.0),
            Go(0.0, 50.0, -573.0),
            Go(4.0, 51.0, -585.0),
            Go(-4.0, 52.0, -594.0),
            Go(4.0, 53.0, -603.0),
            Go(-4.0, 54.0, -612.0),
            Go(0.0, 55.0, -621.0),
            Go(0.0, 56.0, -638.0),
        ],
    ));
    r
}

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
    route: Vec<Step>,
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

    fn mover_z(&self, name: &str) -> f64 {
        let world = self.world();
        let w = world.borrow();
        let ws = w.root("Workspace").expect("workspace");
        let mover = w.find_first_child(ws, name).expect("mover");
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
            println!("{:6.1} s  HUD: {text}", self.game.time());
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
        while self.step < self.route.len() {
            assert!(
                self.game.time() < TIME_LIMIT,
                "out of time at step {}",
                self.step
            );
            assert!(
                self.step_time < STEP_LIMIT,
                "stuck at step {} {:?}, root {:?}",
                self.step,
                self.route[self.step],
                self.root().0
            );
            if state == Some(HumanoidState::Dead) {
                println!(
                    "death {} at step {} {:?}, root {:?}, t = {:.1}",
                    self.deaths + 1,
                    self.step,
                    self.route[self.step],
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
            let input = match self.route[self.step] {
                Go(x, y, z) => self.steer(Vec3::new(x, y, z), state),
                Resume => {
                    self.resume = self.step + 1;
                    None
                }
                Hop(on) => {
                    self.hop = on;
                    None
                }
                MoverAtLeast(name, z) => (self.mover_z(name) < z).then(FrameInput::default),
                Mover(name, top) => {
                    let target = Vec3::new(0.0, top, self.mover_z(name));
                    let (root, _) = self.root();
                    let on = flat(target - root).magnitude() < 2.5
                        && state == Some(HumanoidState::Running)
                        && (root.y - 3.0 - top).abs() < 0.6;
                    if on {
                        None
                    } else {
                        self.steer(target, state).or(Some(FrameInput::default()))
                    }
                }
                MoverAtMost(name, z) => (self.mover_z(name) > z).then(FrameInput::default),
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
        route: route(),
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
