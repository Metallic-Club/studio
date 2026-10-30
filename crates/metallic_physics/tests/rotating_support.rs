//! Platform motion is sampled with the runtime's controller-then-physics ordering.

use metallic_core::{InstanceId, Value, Vec3, World};
use metallic_physics::{
    CharacterController, HumanoidState, MoveInput, PhysicsWorld, ROOT_HEIGHT, spawn_character,
};

const DT: f64 = 1.0 / 60.0;

struct Sim {
    world: World,
    physics: PhysicsWorld,
    controller: CharacterController,
    player: InstanceId,
    platform: InstanceId,
}

impl Sim {
    fn new() -> Self {
        let mut world = World::new_with_services();
        let platform = world.create("Part", world.root("Workspace")).unwrap();
        for (name, value) in [
            ("Anchored", Value::Bool(true)),
            ("Position", Value::Vector3(Vec3::new(0.0, -1.0, 0.0))),
            ("Size", Value::Vector3(Vec3::new(40.0, 2.0, 40.0))),
        ] {
            world.set(platform, name, value).unwrap();
        }
        let player = world.create("Player", world.root("Players")).unwrap();
        let character = spawn_character(&mut world, player).unwrap();
        let root = world
            .find_first_child(character, "HumanoidRootPart")
            .unwrap();
        world
            .set(
                root,
                "Position",
                Value::Vector3(Vec3::new(8.0, ROOT_HEIGHT, 0.0)),
            )
            .unwrap();
        let mut physics = PhysicsWorld::new();
        physics.sync_from(&world);
        let mut sim = Self {
            world,
            physics,
            controller: CharacterController::new(),
            player,
            platform,
        };
        for _ in 0..30 {
            sim.frame(MoveInput::default());
        }
        sim.assert_standing();
        sim
    }

    fn character(&self) -> InstanceId {
        let Value::Ref(Some(character)) = self.world.get(self.player, "Character").unwrap() else {
            panic!("missing character");
        };
        character
    }

    fn root(&self) -> InstanceId {
        self.world
            .find_first_child(self.character(), "HumanoidRootPart")
            .unwrap()
    }

    fn vector(&self, property: &str) -> Vec3 {
        let Value::Vector3(value) = self.world.get(self.root(), property).unwrap() else {
            panic!("missing root vector");
        };
        value
    }

    fn position(&self) -> Vec3 {
        self.vector("Position")
    }

    fn velocity(&self) -> Vec3 {
        self.controller.velocity(self.character()).unwrap()
    }

    fn rotate(&mut self, orientation: Vec3, input: MoveInput) {
        self.world
            .set(self.platform, "Orientation", Value::Vector3(orientation))
            .unwrap();
        self.frame(input);
    }

    fn frame(&mut self, input: MoveInput) {
        let character = self.character();
        self.controller
            .step(&mut self.world, &self.physics, character, input, DT);
        self.controller.drain_events();
        self.physics.step(&mut self.world, DT);
    }

    fn stop(&mut self) {
        // Rates come from the last completed physics step; allow that sample to clear.
        self.frame(MoveInput::default());
        self.frame(MoveInput::default());
    }

    fn assert_standing(&self) {
        assert_eq!(
            self.controller.state(self.character()),
            Some(HumanoidState::Running),
            "at {:?}",
            self.position()
        );
    }
}

fn rotate_y(point: Vec3, degrees: f64) -> Vec3 {
    let (sine, cosine) = degrees.to_radians().sin_cos();
    Vec3::new(
        point.x * cosine + point.z * sine,
        point.y,
        -point.x * sine + point.z * cosine,
    )
}

fn yaw_error(actual: f64, expected: f64) -> f64 {
    ((actual - expected + 180.0).rem_euclid(360.0) - 180.0).abs()
}

#[test]
fn off_center_rider_follows_positive_and_negative_quarter_turns() {
    for sign in [1.0, -1.0] {
        let mut sim = Sim::new();
        let initial = sim.position();
        for frame in 1..=60 {
            sim.rotate(
                Vec3::new(0.0, sign * 90.0 * frame as f64 / 60.0, 0.0),
                MoveInput::default(),
            );
            sim.assert_standing();
        }
        sim.stop();
        let expected = rotate_y(initial, sign * 90.0);
        assert!(
            (sim.position() - expected).magnitude() < 1.0,
            "rider {:?}, expected {expected:?}",
            sim.position()
        );
    }
}

#[test]
fn standing_facing_tracks_platform_yaw_in_both_directions() {
    for sign in [1.0, -1.0] {
        let mut sim = Sim::new();
        for frame in 1..=60 {
            sim.rotate(
                Vec3::new(0.0, sign * 90.0 * frame as f64 / 60.0, 0.0),
                MoveInput::default(),
            );
        }
        sim.stop();
        let orientation = sim.vector("Orientation");
        assert!(
            yaw_error(orientation.y, sign * 90.0) < 0.1,
            "{orientation:?}"
        );
        assert_eq!(orientation.x, 0.0);
        assert_eq!(orientation.z, 0.0);
    }
}

#[test]
fn repeated_rotations_do_not_accumulate_radial_drift() {
    let mut sim = Sim::new();
    let initial = sim.position();
    for frame in 1..=480 {
        let angle = frame as f64 * 1.5;
        sim.rotate(Vec3::new(0.0, angle, 0.0), MoveInput::default());
        let expected = rotate_y(initial, angle);
        assert!(
            (sim.position() - expected).magnitude() < 1.0,
            "frame {frame}: rider {:?}, expected {expected:?}",
            sim.position()
        );
        sim.assert_standing();
    }
    sim.stop();
    let position = sim.position();
    for _ in 0..30 {
        sim.frame(MoveInput::default());
    }
    assert!((sim.position() - position).magnitude() < 1e-4);
}

#[test]
fn stationary_and_translating_support_do_not_gain_rotation_or_duplicate_translation() {
    let mut sim = Sim::new();
    let initial = sim.position();
    for _ in 0..30 {
        sim.frame(MoveInput::default());
    }
    assert!((sim.position() - initial).magnitude() < 1e-4);
    for frame in 1..=60 {
        sim.world
            .set(
                sim.platform,
                "Position",
                Value::Vector3(Vec3::new(frame as f64 * 8.0 / 60.0, -1.0, 0.0)),
            )
            .unwrap();
        sim.frame(MoveInput::default());
    }
    sim.stop();
    assert!((sim.position().x - initial.x - 8.0).abs() < 0.05);
    assert!(yaw_error(sim.vector("Orientation").y, 0.0) < 0.1);
    sim.assert_standing();
    let stopped = sim.position();
    for _ in 0..30 {
        sim.frame(MoveInput::default());
    }
    assert!((sim.position() - stopped).magnitude() < 1e-4);
}

#[test]
fn jump_keeps_tangential_momentum_without_airborne_platform_steering() {
    let mut sim = Sim::new();
    for frame in 1..=30 {
        sim.rotate(
            Vec3::new(0.0, frame as f64 * 1.5, 0.0),
            MoveInput::default(),
        );
    }
    let before = sim.position();
    sim.rotate(
        Vec3::new(0.0, 46.5, 0.0),
        MoveInput {
            jump: true,
            ..MoveInput::default()
        },
    );
    let inherited = sim.velocity();
    let omega = std::f64::consts::FRAC_PI_2;
    assert!((inherited.x - omega * before.z).abs() < 0.5);
    assert!((inherited.z + omega * before.x).abs() < 0.5);
    assert_eq!(
        sim.controller.state(sim.character()),
        Some(HumanoidState::Jumping)
    );
    let yaw = sim.vector("Orientation").y;
    for frame in 32..=37 {
        sim.rotate(
            Vec3::new(0.0, frame as f64 * 1.5, 0.0),
            MoveInput::default(),
        );
        assert!((sim.velocity().x - inherited.x).abs() < 1e-3);
        assert!((sim.velocity().z - inherited.z).abs() < 1e-3);
        assert!(yaw_error(sim.vector("Orientation").y, yaw) < 0.1);
    }
    for _ in 0..60 {
        sim.frame(MoveInput::default());
    }
    sim.assert_standing();
    assert!(sim.velocity().x.abs() < 1e-3 && sim.velocity().z.abs() < 1e-3);
}

#[test]
fn explicit_movement_keeps_world_direction_and_facing() {
    let mut sim = Sim::new();
    for frame in 1..=30 {
        sim.rotate(
            Vec3::new(0.0, frame as f64 * 1.5, 0.0),
            MoveInput::default(),
        );
    }
    let omega = std::f64::consts::FRAC_PI_2;
    for frame in 31..=40 {
        let before = sim.position();
        sim.rotate(
            Vec3::new(0.0, frame as f64 * 1.5, 0.0),
            MoveInput {
                dir: Vec3::new(1.0, 0.0, 0.0),
                ..MoveInput::default()
            },
        );
        assert!(yaw_error(sim.vector("Orientation").y, -90.0) < 1e-5);
        assert!((sim.velocity().x - (16.0 + omega * before.z)).abs() < 0.75);
        assert!((sim.velocity().z + omega * before.x).abs() < 0.75);
    }
}

#[test]
fn small_tilts_use_the_foot_contact_lever_arm_and_keep_the_capsule_upright() {
    for sign in [1.0, -1.0] {
        let mut sim = Sim::new();
        let initial = sim.position();
        let rate = sign * 10.0;
        for frame in 1..=60 {
            sim.rotate(
                Vec3::new(0.0, 0.0, rate * frame as f64 / 60.0),
                MoveInput::default(),
            );
            sim.assert_standing();
            if frame == 2 {
                // The support contact is one stud above the part origin. The root
                // is four studs above it and would produce the wrong lever arm.
                assert!(
                    (sim.velocity().x + rate.to_radians()).abs() < 0.15,
                    "contact velocity {:?}",
                    sim.velocity()
                );
            }
        }
        sim.stop();
        let (sine, cosine) = rate.to_radians().sin_cos();
        let expected = Vec3::new(
            initial.x * cosine - sine,
            initial.y - 1.0 + initial.x * sine + cosine,
            initial.z,
        );
        assert!((sim.position() - expected).magnitude() < 1.0);
        let orientation = sim.vector("Orientation");
        assert_eq!(orientation.x, 0.0);
        assert_eq!(orientation.z, 0.0);
    }
}

#[test]
fn replacing_the_support_body_discards_its_old_contact() {
    let mut sim = Sim::new();
    for frame in 1..=30 {
        sim.rotate(
            Vec3::new(0.0, frame as f64 * 1.5, 0.0),
            MoveInput::default(),
        );
    }
    sim.stop();
    let before = sim.position();
    sim.world
        .set(
            sim.platform,
            "Size",
            Value::Vector3(Vec3::new(44.0, 2.0, 44.0)),
        )
        .unwrap();
    sim.physics.sync_from(&sim.world);
    sim.frame(MoveInput::default());
    assert!((sim.position() - before).magnitude() < 0.05);
    for frame in 1..=30 {
        sim.rotate(
            Vec3::new(0.0, 45.0 + frame as f64 * 1.5, 0.0),
            MoveInput::default(),
        );
    }
    sim.stop();
    assert!((sim.position() - rotate_y(before, 45.0)).magnitude() < 0.1);
    sim.assert_standing();
}

#[test]
fn script_repositioning_does_not_reuse_the_old_off_center_jump_contact() {
    let mut sim = Sim::new();
    for frame in 1..=30 {
        sim.rotate(
            Vec3::new(0.0, frame as f64 * 1.5, 0.0),
            MoveInput::default(),
        );
    }
    sim.world
        .set(
            sim.root(),
            "Position",
            Value::Vector3(Vec3::new(0.0, ROOT_HEIGHT, 0.0)),
        )
        .unwrap();
    sim.frame(MoveInput {
        jump: true,
        ..MoveInput::default()
    });
    assert_eq!(
        sim.controller.state(sim.character()),
        Some(HumanoidState::Jumping)
    );
    assert!(sim.velocity().x.abs() < 0.05 && sim.velocity().z.abs() < 0.05);
}

#[test]
fn falling_dynamic_support_keeps_its_rider_when_stopped_by_a_floor() {
    let mut sim = Sim::new();
    let floor = sim
        .world
        .create("Part", sim.world.root("Workspace"))
        .unwrap();
    for (name, value) in [
        ("Anchored", Value::Bool(true)),
        ("Position", Value::Vector3(Vec3::new(0.0, -9.0, 0.0))),
        ("Size", Value::Vector3(Vec3::new(100.0, 2.0, 100.0))),
    ] {
        sim.world.set(floor, name, value).unwrap();
    }
    sim.world
        .set(sim.platform, "Anchored", Value::Bool(false))
        .unwrap();
    sim.physics.sync_from(&sim.world);
    // Establish the replacement body's contact before it begins falling.
    let character = sim.character();
    sim.controller.step(
        &mut sim.world,
        &sim.physics,
        character,
        MoveInput::default(),
        DT,
    );
    for frame in 1..=90 {
        // The controller consumes the *previous* completed physics step.
        let Value::Vector3(position) = sim.world.get(sim.platform, "Position").unwrap() else {
            panic!("platform position");
        };
        sim.frame(MoveInput::default());
        assert!(
            (sim.position().y - position.y - 1.0 - ROOT_HEIGHT).abs() < 0.1,
            "frame {frame}: rider {:?}, platform {position:?}",
            sim.position()
        );
        sim.assert_standing();
    }
}
