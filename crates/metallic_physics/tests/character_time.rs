//! Invalid frame-time samples must not change character state or future motion.

use metallic_core::{Changes, InstanceId, Value, Vec3, World};
use metallic_physics::{
    CharacterController, CharacterEvent, HumanoidState, MAX_STEP, MoveInput, PhysicsWorld,
    RESPAWN_DELAY, spawn_character,
};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DT: f64 = 1.0 / 60.0;

struct Sim {
    world: World,
    physics: PhysicsWorld,
    controller: CharacterController,
    player: InstanceId,
}

impl Sim {
    fn new() -> Self {
        let mut world = World::new_with_services();
        let workspace = world.root("Workspace").unwrap();
        let floor = world.create("Part", Some(workspace)).unwrap();
        for (name, value) in [
            ("Anchored", Value::Bool(true)),
            ("Position", Value::Vector3(Vec3::new(0.0, -2.0, 0.0))),
            ("Size", Value::Vector3(Vec3::new(512.0, 4.0, 512.0))),
        ] {
            world.set(floor, name, value).unwrap();
        }
        let player = world.create("Player", world.root("Players")).unwrap();
        spawn_character(&mut world, player).unwrap();
        let mut physics = PhysicsWorld::new();
        physics.sync_from(&world);
        Self {
            world,
            physics,
            controller: CharacterController::new(),
            player,
        }
    }

    fn character(&self) -> InstanceId {
        let Value::Ref(Some(character)) = self.world.get(self.player, "Character").unwrap() else {
            panic!("missing character");
        };
        character
    }

    fn position(&self) -> Vec3 {
        let root = self
            .world
            .find_first_child(self.character(), "HumanoidRootPart")
            .unwrap();
        let Value::Vector3(position) = self.world.get(root, "Position").unwrap() else {
            panic!("missing root position");
        };
        position
    }

    fn frame(&mut self, input: MoveInput, dt: f64) -> Vec<CharacterEvent> {
        let character = self.character();
        self.controller
            .step(&mut self.world, &self.physics, character, input, dt);
        let events = self.controller.drain_events();
        if events
            .iter()
            .any(|event| matches!(event, CharacterEvent::Respawned { .. }))
        {
            self.physics.sync_from(&self.world);
        }
        self.physics.step(&mut self.world, dt);
        events
    }

    fn settle(&mut self) {
        for _ in 0..10 {
            self.frame(MoveInput::default(), DT);
        }
        assert_eq!(
            self.controller.state(self.character()),
            Some(HumanoidState::Running)
        );
    }

    fn invalid_sample(&mut self, dt: f64) {
        let character = self.character();
        let state = self.controller.state(character);
        let velocity = self.controller.velocity(character);
        let position = self.position();
        let cursor = self.world.change_cursor();
        self.controller.step(
            &mut self.world,
            &self.physics,
            character,
            MoveInput {
                dir: Vec3::new(1.0, 0.0, 0.0),
                jump: true,
                ..MoveInput::default()
            },
            dt,
        );
        assert_eq!(self.world.changes_since(cursor).0, Changes::Ids(vec![]));
        assert_eq!(self.character(), character);
        assert_eq!(self.position(), position);
        assert_eq!(self.controller.state(character), state);
        assert_eq!(self.controller.velocity(character), velocity);
        assert!(self.controller.drain_events().is_empty());
    }
}

fn same_motion(actual: &Sim, reference: &Sim) {
    let actual_position = actual.position();
    assert!(
        actual_position.x.is_finite()
            && actual_position.y.is_finite()
            && actual_position.z.is_finite()
    );
    assert!(
        (actual_position - reference.position()).magnitude() < 1e-6,
        "actual {actual_position:?}, reference {:?}",
        reference.position()
    );
    assert_eq!(
        actual.controller.state(actual.character()),
        reference.controller.state(reference.character())
    );
    assert_eq!(
        actual.controller.velocity(actual.character()),
        reference.controller.velocity(reference.character())
    );
}

#[test]
fn invalid_samples_before_first_step_do_not_create_motion_or_apply_input() {
    for dt in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut sim = Sim::new();
        sim.invalid_sample(dt);
        assert!(sim.controller.state(sim.character()).is_none());
        assert!(sim.controller.velocity(sim.character()).is_none());
        let mut reference = Sim::new();
        sim.frame(MoveInput::default(), DT);
        reference.frame(MoveInput::default(), DT);
        same_motion(&sim, &reference);
    }
}

#[test]
fn invalid_grounded_jump_is_not_applied_to_later_frames() {
    for dt in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut sim = Sim::new();
        let mut reference = Sim::new();
        sim.settle();
        reference.settle();
        sim.invalid_sample(dt);
        // Ignored jump input must not prime a velocity for the following idle frame.
        sim.frame(MoveInput::default(), DT);
        reference.frame(MoveInput::default(), DT);
        same_motion(&sim, &reference);
        let jump = MoveInput {
            jump: true,
            ..MoveInput::default()
        };
        sim.frame(jump, DT);
        reference.frame(jump, DT);
        same_motion(&sim, &reference);
        assert_eq!(
            sim.controller.state(sim.character()),
            Some(HumanoidState::Jumping)
        );
    }
}

#[test]
fn nonfinite_airborne_time_does_not_poison_gravity_and_later_motion() {
    for dt in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        let mut sim = Sim::new();
        let mut reference = Sim::new();
        sim.settle();
        reference.settle();
        let jump = MoveInput {
            jump: true,
            ..MoveInput::default()
        };
        sim.frame(jump, DT);
        reference.frame(jump, DT);
        sim.invalid_sample(dt);
        for _ in 0..60 {
            let movement = MoveInput {
                dir: Vec3::new(0.0, 0.0, -1.0),
                ..MoveInput::default()
            };
            sim.frame(movement, DT);
            reference.frame(movement, DT);
            same_motion(&sim, &reference);
        }
        assert_eq!(
            sim.controller.state(sim.character()),
            Some(HumanoidState::Running)
        );
    }
}

#[test]
fn invalid_time_does_not_advance_death_timer_or_respawn_early() {
    for dt in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        let mut sim = Sim::new();
        let mut reference = Sim::new();
        for fixture in [&mut sim, &mut reference] {
            fixture.settle();
            let humanoid = fixture
                .world
                .find_first_child(fixture.character(), "Humanoid")
                .unwrap();
            fixture
                .world
                .set(humanoid, "Health", Value::Number(0.0))
                .unwrap();
            let events = fixture.frame(MoveInput::default(), DT);
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, CharacterEvent::Died { .. }))
            );
        }
        let original = sim.character();
        sim.invalid_sample(dt);
        let mut respawned = false;
        for _ in 0..(RESPAWN_DELAY / DT).ceil() as usize + 2 {
            let actual = sim.frame(MoveInput::default(), DT);
            let expected = reference.frame(MoveInput::default(), DT);
            let is_respawn = |events: &[CharacterEvent]| {
                events
                    .iter()
                    .any(|event| matches!(event, CharacterEvent::Respawned { .. }))
            };
            assert_eq!(is_respawn(&actual), is_respawn(&expected));
            respawned |= is_respawn(&actual);
            same_motion(&sim, &reference);
        }
        assert!(respawned);
        assert_ne!(sim.character(), original);
    }
}

#[test]
fn finite_zero_negative_and_normal_steps_keep_the_existing_contract() {
    let mut zero = Sim::new();
    let mut negative = Sim::new();
    zero.settle();
    negative.settle();
    let input = MoveInput {
        dir: Vec3::new(1.0, 0.0, 0.0),
        jump: true,
        ..MoveInput::default()
    };
    zero.frame(input, 0.0);
    negative.frame(input, -0.5);
    same_motion(&zero, &negative);
    for _ in 0..10 {
        zero.frame(MoveInput::default(), DT);
        negative.frame(MoveInput::default(), DT);
        same_motion(&zero, &negative);
    }
}

#[test]
fn a_half_second_character_step_matches_the_shared_physics_cap() {
    let mut sim = Sim::new();
    let mut reference = Sim::new();
    sim.settle();
    reference.settle();
    let start = sim.position();
    let movement = MoveInput {
        dir: Vec3::new(1.0, 0.0, 0.0),
        ..MoveInput::default()
    };
    sim.frame(movement, 0.5);
    reference.frame(movement, MAX_STEP);
    same_motion(&sim, &reference);
    assert!((sim.position().x - start.x - 16.0 * MAX_STEP).abs() < 1e-3);
}

#[test]
fn finite_long_time_does_not_fast_forward_respawn() {
    for dt in [0.5, f64::MAX] {
        let mut sim = Sim::new();
        let mut reference = Sim::new();
        for fixture in [&mut sim, &mut reference] {
            fixture.settle();
            let humanoid = fixture
                .world
                .find_first_child(fixture.character(), "Humanoid")
                .unwrap();
            fixture
                .world
                .set(humanoid, "Health", Value::Number(0.0))
                .unwrap();
            fixture.frame(MoveInput::default(), DT);
        }
        let original = sim.character();
        sim.frame(MoveInput::default(), dt);
        reference.frame(MoveInput::default(), MAX_STEP);
        assert_eq!(
            sim.character(),
            original,
            "long time must not respawn immediately"
        );
        same_motion(&sim, &reference);
        for _ in 0..(RESPAWN_DELAY / DT).ceil() as usize + 2 {
            let actual = sim.frame(MoveInput::default(), DT);
            let expected = reference.frame(MoveInput::default(), DT);
            let is_respawn = |events: &[CharacterEvent]| {
                events
                    .iter()
                    .any(|event| matches!(event, CharacterEvent::Respawned { .. }))
            };
            assert_eq!(is_respawn(&actual), is_respawn(&expected));
            same_motion(&sim, &reference);
        }
        assert_ne!(sim.character(), original);
    }
}

fn extreme_finite_probe() {
    let mut sim = Sim::new();
    let mut reference = Sim::new();
    sim.settle();
    reference.settle();
    let jump = MoveInput {
        jump: true,
        ..MoveInput::default()
    };
    sim.frame(jump, DT);
    reference.frame(jump, DT);
    sim.frame(
        MoveInput {
            dir: Vec3::new(1.0, 0.0, 0.0),
            ..MoveInput::default()
        },
        f64::MAX,
    );
    reference.frame(
        MoveInput {
            dir: Vec3::new(1.0, 0.0, 0.0),
            ..MoveInput::default()
        },
        MAX_STEP,
    );
    same_motion(&sim, &reference);
    for _ in 0..60 {
        sim.frame(MoveInput::default(), DT);
        reference.frame(MoveInput::default(), DT);
        same_motion(&sim, &reference);
    }
}

#[test]
fn extreme_finite_time_is_bounded_and_recoverable() {
    const CHILD_FLAG: &str = "METALLIC_TEST_CHARACTER_EXTREME_DT_CHILD";
    if std::env::var_os(CHILD_FLAG).is_some() {
        extreme_finite_probe();
        return;
    }
    // An unfixed enormous displacement can enter Rapier with non-finite values.
    // Isolate that baseline probe and bound its complete process lifetime.
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "extreme_finite_time_is_bounded_and_recoverable",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_FLAG, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            result => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("extreme-time probe exceeded its deadline or failed to wait: {result:?}");
            }
        }
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "extreme-time probe failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
