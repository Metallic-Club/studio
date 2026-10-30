//! Behaviour tests for the 2D platformer character controller.
use metallic_physics2d::*;

const DT: f64 = 1.0 / 60.0;
const HERO: BodyId = BodyId(50);

fn floor() -> BodyDesc {
    BodyDesc::new(BodyId(1), BodyKind::Static, Vec2::new(0.0, -2.0))
        .with_collider(ColliderDesc::rect(800.0, 4.0))
}

fn standing_y(cfg: &CharacterConfig, ground_top: f64) -> f64 {
    ground_top + cfg.half_extents.y + cfg.skin
}

struct Sim {
    world: PhysicsWorld2d,
    scene: Scene2d,
    events: Vec<CharacterEvent>,
    touched: Vec<(BodyId, BodyId)>,
    sensor: Vec<SensorEvent>,
}

impl Sim {
    fn new(bodies: Vec<BodyDesc>, start: Vec2, cfg: CharacterConfig) -> Sim {
        let scene = Scene2d {
            bodies,
            joints: vec![],
        };
        let mut world = PhysicsWorld2d::new(WorldConfig::default()).expect("valid config");
        world.sync_from(&scene).expect("valid scene");
        world.add_character(HERO, cfg, start).expect("character");
        Sim {
            world,
            scene,
            events: Vec::new(),
            touched: Vec::new(),
            sensor: Vec::new(),
        }
    }

    fn frame(&mut self, input: CharacterInput) {
        self.world.set_character_input(HERO, input).expect("character");
        self.world.step(&mut self.scene, DT).expect("valid dt");
        self.events.extend(self.world.drain_character_events());
        self.touched.extend(self.world.drain_touched());
        self.sensor.extend(self.world.drain_sensor_events());
    }

    fn frames(&mut self, n: usize, input: CharacterInput) {
        for _ in 0..n {
            self.frame(input);
        }
    }

    fn state(&self) -> CharacterState {
        self.world.character(HERO).expect("character")
    }

    fn took(&mut self, kind: CharacterEventKind) -> bool {
        let found = self.events.iter().any(|e| e.kind == kind);
        self.events.clear();
        found
    }
}

fn idle() -> CharacterInput {
    CharacterInput::default()
}

fn hold(move_x: f64, jump: bool) -> CharacterInput {
    CharacterInput {
        move_x,
        jump,
        drop_through: false,
    }
}

#[test]
fn character_lands_and_stands_on_the_ground() {
    let cfg = CharacterConfig::default();
    let mut sim = Sim::new(vec![floor()], Vec2::new(0.0, 10.0), cfg);
    sim.frames(60, idle());
    let s = sim.state();
    assert!(s.grounded, "{s:?}");
    assert!((s.position.y - standing_y(&cfg, 0.0)).abs() < 0.05, "{s:?}");
    assert_eq!(s.ground_body, Some(BodyId(1)));
    assert!(sim.took(CharacterEventKind::Landed));
    assert!(sim.touched.contains(&(HERO, BodyId(1))), "standing counts as touching");
}

#[test]
fn full_and_tapped_jumps_reach_their_designed_heights() {
    let cfg = CharacterConfig::default();
    let mut sim = Sim::new(vec![floor()], Vec2::new(0.0, 3.0), cfg);
    sim.frames(30, idle());
    let y0 = sim.state().position.y;
    let mut top = y0;
    let mut apex_frame = 0;
    for f in 0..90 {
        sim.frame(hold(0.0, true));
        let y = sim.state().position.y;
        if y > top {
            top = y;
            apex_frame = f + 1;
        }
    }
    let full = top - y0;
    assert!(
        (full - cfg.jump_height).abs() < 0.04 * cfg.jump_height,
        "full jump rose {full:.2}, designed {}",
        cfg.jump_height
    );
    let apex_time = apex_frame as f64 * DT;
    assert!(
        apex_time > cfg.time_to_apex - 0.03 && apex_time < cfg.time_to_apex + 0.12,
        "apex after {apex_time:.3} s"
    );

    // Tap: pressed for one frame only.
    sim.frames(60, idle());
    let y0 = sim.state().position.y;
    sim.frame(hold(0.0, true));
    let mut top = y0;
    for _ in 0..60 {
        sim.frame(idle());
        top = top.max(sim.state().position.y);
    }
    let tap = top - y0;
    assert!(
        tap > cfg.min_jump_height * 0.9 && tap < cfg.min_jump_height + 1.5,
        "tap jump rose {tap:.2}, designed {}",
        cfg.min_jump_height
    );
}

/// A ledge whose top is at y = 0 and right edge at x = 0, over a deep pit.
fn ledge() -> Vec<BodyDesc> {
    vec![
        BodyDesc::new(BodyId(2), BodyKind::Static, Vec2::new(-50.0, -2.0))
            .with_collider(ColliderDesc::rect(100.0, 4.0)),
        BodyDesc::new(BodyId(1), BodyKind::Static, Vec2::new(0.0, -300.0))
            .with_collider(ColliderDesc::rect(800.0, 4.0)),
    ]
}

/// Walks right off the ledge, then presses jump `delay` frames after leaving the ground.
/// Returns whether a jump happened.
fn jump_after_leaving_ledge(delay: usize) -> bool {
    let cfg = CharacterConfig::default();
    let mut sim = Sim::new(ledge(), Vec2::new(-20.0, 3.0), cfg);
    sim.frames(30, idle());
    sim.events.clear();
    let mut airborne = 0;
    for _ in 0..240 {
        sim.frame(hold(1.0, false));
        if !sim.state().grounded {
            airborne += 1;
            if airborne == delay {
                break;
            }
        }
    }
    sim.frame(hold(1.0, true));
    sim.frames(3, hold(1.0, true));
    sim.took(CharacterEventKind::Jumped)
}

#[test]
fn coyote_time_allows_a_late_jump_after_leaving_a_ledge() {
    let cfg = CharacterConfig::default();
    let window = (cfg.coyote_time / DT).floor() as usize; // 6 frames
    assert!(jump_after_leaving_ledge(1), "jump right after the edge");
    assert!(jump_after_leaving_ledge(window - 1), "jump inside coyote time");
    assert!(!jump_after_leaving_ledge(window + 3), "no jump once coyote time is over");
}

/// Drops the character from a height and presses jump `early` frames before the frame it
/// would land. Returns whether it jumped within a few frames of landing.
fn jump_pressed_before_landing(early: usize) -> bool {
    let cfg = CharacterConfig::default();
    // First find the landing frame without pressing anything.
    let mut probe = Sim::new(vec![floor()], Vec2::new(0.0, 30.0), cfg);
    let mut landing = 0;
    for f in 1..=240 {
        probe.frame(idle());
        if probe.state().grounded {
            landing = f;
            break;
        }
    }
    assert!(landing > early + 2, "fall long enough for the test");
    let mut sim = Sim::new(vec![floor()], Vec2::new(0.0, 30.0), cfg);
    for f in 1..=landing + 4 {
        // Press (rising edge) at landing - early, then release right away.
        let jump = f == landing - early;
        sim.frame(hold(0.0, jump));
    }
    sim.took(CharacterEventKind::Jumped)
}

#[test]
fn jump_buffer_remembers_an_early_press() {
    let cfg = CharacterConfig::default();
    let window = (cfg.jump_buffer_time / DT).floor() as usize; // 6 frames
    assert!(jump_pressed_before_landing(1));
    assert!(jump_pressed_before_landing(window - 1), "press inside the buffer jumps on landing");
    assert!(!jump_pressed_before_landing(window + 4), "press long before landing is dropped");
}

#[test]
fn walkable_slopes_are_climbed_and_steep_ones_are_not() {
    let cfg = CharacterConfig::default();
    let rise = 40.0 * 30f64.to_radians().tan();
    let gentle = BodyDesc::new(BodyId(2), BodyKind::Static, Vec2::new(0.0, 0.0)).with_collider(
        ColliderDesc::new(Shape2d::ConvexPolygon {
            points: vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(80.0, 0.0),
                Vec2::new(80.0, rise),
                Vec2::new(40.0, rise),
            ],
        }),
    );
    let mut sim = Sim::new(vec![floor(), gentle], Vec2::new(-10.0, 3.0), cfg);
    sim.frames(20, idle());
    let mut best = 0.0f64;
    for _ in 0..150 {
        sim.frame(hold(1.0, false));
        best = best.max(sim.state().position.y);
    }
    assert!(best > rise + 2.0, "climbed the 30 degree slope: top {best:.2} of {rise:.2}");
    assert!(sim.state().grounded);

    let steep_rise = 20.0 * 70f64.to_radians().tan();
    let steep = BodyDesc::new(BodyId(2), BodyKind::Static, Vec2::new(0.0, 0.0)).with_collider(
        ColliderDesc::new(Shape2d::ConvexPolygon {
            points: vec![Vec2::new(0.0, 0.0), Vec2::new(20.0, 0.0), Vec2::new(20.0, steep_rise)],
        }),
    );
    let mut sim = Sim::new(vec![floor(), steep], Vec2::new(-10.0, 3.0), cfg);
    sim.frames(20, idle());
    let mut best = 0.0f64;
    for _ in 0..150 {
        sim.frame(hold(1.0, false));
        best = best.max(sim.state().position.y);
    }
    assert!(best < standing_y(&cfg, 0.0) + 4.0, "did not climb the 70 degree slope: {best:.2}");
}

#[test]
fn moving_platforms_carry_the_character() {
    let cfg = CharacterConfig::default();
    let platform = BodyDesc::new(BodyId(2), BodyKind::Static, Vec2::new(0.0, 10.0))
        .with_collider(ColliderDesc::rect(30.0, 2.0));
    let mut sim = Sim::new(vec![floor(), platform], Vec2::new(0.0, 15.0), cfg);
    sim.frames(30, idle());
    let start = sim.state().position;
    assert!(sim.state().grounded);
    // Right at 10 studs/s for 1 s, then up at 6 studs/s for 1 s.
    for f in 1..=120 {
        let (x, y) = if f <= 60 {
            (f as f64 * DT * 10.0, 10.0)
        } else {
            (10.0, 10.0 + (f - 60) as f64 * DT * 6.0)
        };
        if let Some(p) = sim.scene.body_mut(BodyId(2)) {
            p.position = Vec2::new(x, y);
        }
        sim.frame(idle());
        assert!(sim.state().grounded, "stayed grounded on the platform at frame {f}");
    }
    let end = sim.state().position;
    assert!((end.x - start.x - 10.0).abs() < 0.3, "carried right: {start:?} -> {end:?}");
    assert!((end.y - start.y - 6.0).abs() < 0.3, "carried up: {start:?} -> {end:?}");

    // Jumping off a moving platform keeps its velocity.
    for f in 1..=10 {
        if let Some(p) = sim.scene.body_mut(BodyId(2)) {
            p.position.x = 10.0 + f as f64 * DT * 10.0;
        }
        sim.frame(hold(0.0, f == 10));
    }
    assert!(sim.took(CharacterEventKind::Jumped));
    assert!(sim.state().velocity.x > 8.0, "inherited platform speed: {:?}", sim.state());
}

#[test]
fn one_way_platforms_jump_up_through_and_drop_down() {
    let cfg = CharacterConfig::default();
    let platform = BodyDesc::new(BodyId(2), BodyKind::Static, Vec2::new(0.0, 8.0))
        .with_collider(ColliderDesc::rect(20.0, 1.0).one_way_up());
    let mut sim = Sim::new(vec![floor(), platform], Vec2::new(0.0, 3.0), cfg);
    sim.frames(20, idle());
    assert!((sim.state().position.y - standing_y(&cfg, 0.0)).abs() < 0.05);
    sim.frames(70, hold(0.0, true));
    sim.frames(30, idle());
    let s = sim.state();
    assert!(s.grounded && s.on_one_way, "landed on the platform: {s:?}");
    assert!((s.position.y - standing_y(&cfg, 8.5)).abs() < 0.05, "{s:?}");

    sim.events.clear();
    sim.frame(CharacterInput {
        move_x: 0.0,
        jump: false,
        drop_through: true,
    });
    assert!(sim.took(CharacterEventKind::DroppedThrough));
    sim.frames(60, idle());
    let s = sim.state();
    assert!(s.grounded && !s.on_one_way, "fell back to the floor: {s:?}");
    assert!((s.position.y - standing_y(&cfg, 0.0)).abs() < 0.05, "{s:?}");
}

/// A tall wall whose left face is at x = 9.
fn wall() -> BodyDesc {
    BodyDesc::new(BodyId(3), BodyKind::Static, Vec2::new(10.0, 50.0))
        .with_collider(ColliderDesc::rect(2.0, 100.0))
}

#[test]
fn wall_slide_caps_the_fall_and_wall_jump_pushes_away() {
    let cfg = CharacterConfig::default();
    let mut sim = Sim::new(vec![floor(), wall()], Vec2::new(5.0, 80.0), cfg);
    sim.frames(40, hold(1.0, false));
    let s = sim.state();
    assert_eq!(s.wall, Some(WallSide::Right), "{s:?}");
    assert!(s.wall_sliding, "{s:?}");
    assert!((s.velocity.y + cfg.wall_slide_speed).abs() < 1e-9, "fall capped: {s:?}");

    sim.events.clear();
    sim.frame(hold(1.0, true));
    assert!(sim.took(CharacterEventKind::WallJumped));
    let s = sim.state();
    assert!(s.velocity.x < -30.0 && s.velocity.y > 30.0, "launched away: {s:?}");
    // Input toward the wall is ignored during the lock, so the jump carries away.
    sim.frames(5, hold(1.0, true));
    assert!(sim.state().velocity.x < -30.0);
}

#[test]
fn wall_slide_and_wall_jump_can_be_turned_off() {
    let cfg = CharacterConfig {
        wall_slide: false,
        wall_jump: false,
        ..CharacterConfig::default()
    };
    let mut sim = Sim::new(vec![floor(), wall()], Vec2::new(5.0, 80.0), cfg);
    sim.frames(40, hold(1.0, false));
    let s = sim.state();
    assert_eq!(s.wall, Some(WallSide::Right), "the wall is still reported: {s:?}");
    assert!(!s.wall_sliding && s.velocity.y < -cfg.wall_slide_speed * 2.0, "{s:?}");
    sim.events.clear();
    sim.frame(hold(1.0, true));
    assert!(!sim.took(CharacterEventKind::WallJumped));
}

#[test]
fn character_touches_bricks_and_triggers_sensors() {
    let cfg = CharacterConfig::default();
    let brick = BodyDesc::new(BodyId(4), BodyKind::Static, Vec2::new(20.0, 3.0))
        .with_collider(ColliderDesc::rect(2.0, 6.0));
    let zone = BodyDesc::new(BodyId(5), BodyKind::Static, Vec2::new(8.0, 3.0))
        .with_collider(ColliderDesc::rect(4.0, 6.0).sensor());
    let mut sim = Sim::new(vec![floor(), brick, zone], Vec2::new(0.0, 3.0), cfg);
    sim.frames(90, hold(1.0, false));
    assert!(sim.touched.contains(&(HERO, BodyId(4))), "{:?}", sim.touched);
    assert!(sim.state().position.x < 19.0 - cfg.half_extents.x + 0.1, "brick blocks");
    let kinds: Vec<SensorEventKind> = sim
        .sensor
        .iter()
        .filter(|e| e.sensor == BodyId(5) && e.other == HERO)
        .map(|e| e.kind)
        .collect();
    assert_eq!(kinds, vec![SensorEventKind::Enter, SensorEventKind::Exit]);
}

#[test]
fn character_pushes_crates() {
    let cfg = CharacterConfig::default();
    let crate_box = BodyDesc::new(BodyId(6), BodyKind::Dynamic, Vec2::new(8.0, 1.5))
        .with_collider(ColliderDesc::rect(3.0, 3.0));
    let mut sim = Sim::new(vec![floor(), crate_box], Vec2::new(0.0, 3.0), cfg);
    sim.frames(120, hold(1.0, false));
    let x = sim.scene.body(BodyId(6)).expect("crate").position.x;
    assert!(x > 12.0, "crate was pushed: {x}");
}

/// Runs a fixed input script at a given frame rate; returns the character's final state.
fn run_at(frame_dt: f64) -> (Vec2, Vec2) {
    let cfg = CharacterConfig::default();
    let platform = BodyDesc::new(BodyId(2), BodyKind::Static, Vec2::new(20.0, 8.0))
        .with_collider(ColliderDesc::rect(20.0, 1.0).one_way_up());
    let scene = Scene2d {
        bodies: vec![floor(), platform],
        joints: vec![],
    };
    let mut scene = scene;
    let mut world = PhysicsWorld2d::new(WorldConfig::default()).expect("valid config");
    world.sync_from(&scene).expect("valid scene");
    world.add_character(HERO, cfg, Vec2::new(0.0, 3.0)).expect("character");
    let frames = (3.0 / frame_dt).round() as usize;
    for f in 0..frames {
        // Input changes only on 1/30 s boundaries so every frame rate sees the same script.
        let t = (f as f64 * frame_dt * 30.0 + 1e-9).floor() / 30.0;
        let input = CharacterInput {
            move_x: if t < 1.5 { 1.0 } else { -1.0 },
            jump: (0.5..0.9).contains(&t) || (2.0..2.1).contains(&t),
            drop_through: false,
        };
        world.set_character_input(HERO, input).expect("character");
        world.step(&mut scene, frame_dt).expect("valid dt");
    }
    let s = world.character(HERO).expect("character");
    (s.position, s.velocity)
}

#[test]
fn controller_feel_does_not_depend_on_frame_rate() {
    let at30 = run_at(1.0 / 30.0);
    let at60 = run_at(1.0 / 60.0);
    let at120 = run_at(1.0 / 120.0);
    assert_eq!(at30, at60);
    assert_eq!(at60, at120);
}
