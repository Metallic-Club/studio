use metallic_core::{Color3, InstanceId, Value, Vec3, World};
use metallic_physics::{
    CharacterController, CharacterEvent, HumanoidState, MoveInput, PhysicsWorld, RESPAWN_DELAY,
    spawn_character,
};

const DT: f64 = 1.0 / 60.0;
const TITANIUM: Color3 = Color3::new(142.0, 145.0, 150.0);
const VIOLET: Color3 = Color3::new(155.0, 130.0, 212.0);

fn part(world: &mut World, class: &str, pos: Vec3, size: Vec3) -> InstanceId {
    let ws = world.root("Workspace").unwrap();
    let id = world.create(class, Some(ws)).unwrap();
    world.set(id, "Position", Value::Vector3(pos)).unwrap();
    world.set(id, "Size", Value::Vector3(size)).unwrap();
    world.set(id, "Anchored", Value::Bool(true)).unwrap();
    id
}

/// Baseplate whose top surface is at y = 0.
fn baseplate(world: &mut World) -> InstanceId {
    part(
        world,
        "Part",
        Vec3::new(0.0, -2.0, 0.0),
        Vec3::new(512.0, 4.0, 512.0),
    )
}

fn v3(world: &World, id: InstanceId, prop: &str) -> Vec3 {
    match world.get(id, prop).unwrap() {
        Value::Vector3(v) => v,
        v => panic!("{prop} is {v:?}"),
    }
}

fn num(world: &World, id: InstanceId, prop: &str) -> f64 {
    match world.get(id, prop).unwrap() {
        Value::Number(v) => v,
        v => panic!("{prop} is {v:?}"),
    }
}

fn reference(world: &World, id: InstanceId, prop: &str) -> Option<InstanceId> {
    match world.get(id, prop).unwrap() {
        Value::Ref(r) => r,
        v => panic!("{prop} is {v:?}"),
    }
}

fn add_player(world: &mut World) -> InstanceId {
    let players = world.root("Players").unwrap();
    let p = world.create("Player", Some(players)).unwrap();
    world
        .set(p, "Name", Value::String("Player1".into()))
        .unwrap();
    p
}

struct Sim {
    world: World,
    phys: PhysicsWorld,
    ctl: CharacterController,
    player: InstanceId,
    events: Vec<CharacterEvent>,
}

impl Sim {
    /// Adds a player, spawns its character and syncs physics (call after building the scene).
    fn start(mut world: World) -> Sim {
        let player = add_player(&mut world);
        spawn_character(&mut world, player).unwrap();
        let mut phys = PhysicsWorld::new();
        phys.sync_from(&world);
        Sim {
            world,
            phys,
            ctl: CharacterController::new(),
            player,
            events: Vec::new(),
        }
    }

    fn character(&self) -> InstanceId {
        reference(&self.world, self.player, "Character").expect("player has a character")
    }

    fn root(&self) -> InstanceId {
        self.world
            .find_first_child(self.character(), "HumanoidRootPart")
            .unwrap()
    }

    fn humanoid(&self) -> InstanceId {
        self.world
            .find_first_child(self.character(), "Humanoid")
            .unwrap()
    }

    fn root_pos(&self) -> Vec3 {
        v3(&self.world, self.root(), "Position")
    }

    /// One game frame: controller, then physics (synced again after a respawn).
    fn frame(&mut self, input: MoveInput) {
        let ch = self.character();
        self.ctl.step(&mut self.world, &self.phys, ch, input, DT);
        let new = self.ctl.drain_events();
        if new
            .iter()
            .any(|e| matches!(e, CharacterEvent::Respawned { .. }))
        {
            self.phys.sync_from(&self.world);
        }
        self.events.extend(new);
        self.phys.step(&mut self.world, DT);
    }

    fn run(&mut self, seconds: f64, input: MoveInput) {
        for _ in 0..(seconds * 60.0).round() as usize {
            self.frame(input);
        }
    }

    fn died_count(&self) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e, CharacterEvent::Died { .. }))
            .count()
    }
}

fn idle() -> MoveInput {
    MoveInput::default()
}

fn walk(x: f64, z: f64) -> MoveInput {
    MoveInput {
        dir: Vec3::new(x, 0.0, z),
        jump: false,
    }
}

#[test]
fn spawns_at_origin_without_spawn_location() {
    let mut world = World::new_with_services();
    let player = add_player(&mut world);
    let ch = spawn_character(&mut world, player).unwrap();
    assert_eq!(reference(&world, player, "Character"), Some(ch));
    assert_eq!(world.class_name(ch), Some("Model"));
    assert_eq!(
        world.get(ch, "Name").unwrap(),
        Value::String("Player1".into())
    );
    let ws = world.root("Workspace").unwrap();
    assert_eq!(world.parent(ch), Some(ws));

    let root = world.find_first_child(ch, "HumanoidRootPart").unwrap();
    assert!((v3(&world, root, "Position") - Vec3::new(0.0, 3.0, 0.0)).magnitude() < 1e-9);
    assert_eq!(v3(&world, root, "Size"), Vec3::new(2.0, 2.0, 1.0));
    assert_eq!(num(&world, root, "Transparency"), 1.0);

    let hum = world.find_first_child(ch, "Humanoid").unwrap();
    assert_eq!(world.class_name(hum), Some("Humanoid"));
    assert_eq!(num(&world, hum, "Health"), 100.0);
    assert_eq!(num(&world, hum, "WalkSpeed"), 16.0);

    let limbs = [
        ("Head", Vec3::new(2.0, 1.0, 1.0)),
        ("Torso", Vec3::new(2.0, 2.0, 1.0)),
        ("Left Arm", Vec3::new(1.0, 2.0, 1.0)),
        ("Right Arm", Vec3::new(1.0, 2.0, 1.0)),
        ("Left Leg", Vec3::new(1.0, 2.0, 1.0)),
        ("Right Leg", Vec3::new(1.0, 2.0, 1.0)),
    ];
    for (name, size) in limbs {
        let p = world
            .find_first_child(ch, name)
            .unwrap_or_else(|| panic!("missing {name}"));
        assert_eq!(v3(&world, p, "Size"), size, "{name}");
        assert_eq!(num(&world, p, "Transparency"), 0.0, "{name}");
        assert_eq!(world.get(p, "Color").unwrap(), Value::Color3(TITANIUM));
    }
    let visible = world
        .children(ch)
        .iter()
        .filter(|&&c| world.is_a(c, "BasePart") && num(&world, c, "Transparency") < 1.0)
        .count();
    assert_eq!(visible, 6);
    let torso = world.find_first_child(ch, "Torso").unwrap();
    let stripe = world.find_first_child(torso, "Stripe").unwrap();
    assert_eq!(world.get(stripe, "Color").unwrap(), Value::Color3(VIOLET));
    // feet on the ground: legs end 3 studs below the root
    let leg = world.find_first_child(ch, "Left Leg").unwrap();
    assert!((v3(&world, leg, "Position").y - 1.0).abs() < 1e-9);
}

#[test]
fn spawns_on_first_spawn_location() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    part(
        &mut world,
        "SpawnLocation",
        Vec3::new(10.0, 0.5, -4.0),
        Vec3::new(6.0, 1.0, 6.0),
    );
    let sim = Sim::start(world);
    assert!((sim.root_pos() - Vec3::new(10.0, 4.0, -4.0)).magnitude() < 1e-9);
}

#[test]
fn walk_speed_16_studs_per_second() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    let start = sim.root_pos();
    assert!((start.y - 3.0).abs() < 0.1, "rests at {start:?}");
    assert_eq!(sim.ctl.state(sim.character()), Some(HumanoidState::Running));
    sim.run(1.0, walk(1.0, 0.0));
    let end = sim.root_pos();
    let d = end.x - start.x;
    assert!((d - 16.0).abs() <= 16.0 * 0.05, "walked {d}");
    assert!(
        (end.y - start.y).abs() < 0.05,
        "y drifted {}",
        end.y - start.y
    );
    // the avatar turned to face +X (look vector -Z rotated by -90 degrees around Y)
    let torso = sim
        .world
        .find_first_child(sim.character(), "Torso")
        .unwrap();
    assert!((v3(&sim.world, torso, "Orientation").y + 90.0).abs() < 1e-6);
}

#[test]
fn jump_apex_about_6_4_studs() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    let y0 = sim.root_pos().y;
    sim.frame(MoveInput {
        dir: Vec3::default(),
        jump: true,
    });
    assert_eq!(sim.ctl.state(sim.character()), Some(HumanoidState::Jumping));
    let mut apex = sim.root_pos().y;
    for _ in 0..90 {
        sim.frame(idle());
        apex = apex.max(sim.root_pos().y);
    }
    let h = apex - y0;
    assert!((h - 6.37).abs() <= 0.3, "apex {h}");
    // landed again
    assert!((sim.root_pos().y - y0).abs() < 0.05);
    assert_eq!(sim.ctl.state(sim.character()), Some(HumanoidState::Running));
}

#[test]
fn walks_up_45_degree_wedge() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    // slope rises toward +Z from (z=6, y=0) to (z=14, y=8)
    let wedge = part(
        &mut world,
        "Part",
        Vec3::new(0.0, 4.0, 10.0),
        Vec3::new(12.0, 8.0, 8.0),
    );
    world
        .set(wedge, "Shape", Value::String("Wedge".into()))
        .unwrap();
    // landing on top of the slope, top at y = 8
    part(
        &mut world,
        "Part",
        Vec3::new(0.0, 4.0, 24.0),
        Vec3::new(12.0, 8.0, 20.0),
    );
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    sim.run(2.0, walk(0.0, 1.0));
    let p = sim.root_pos();
    assert!(p.z > 16.0, "stuck at {p:?}");
    assert!((p.y - 11.0).abs() < 0.2, "not on top: {p:?}");
}

#[test]
fn steps_up_one_and_a_half_studs_but_not_two_and_a_half() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    // a 1.5 stud step at +X, a 2.5 stud wall at -X
    part(
        &mut world,
        "Part",
        Vec3::new(14.0, 0.75, 0.0),
        Vec3::new(16.0, 1.5, 16.0),
    );
    part(
        &mut world,
        "Part",
        Vec3::new(-14.0, 1.25, 0.0),
        Vec3::new(16.0, 2.5, 16.0),
    );
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    sim.run(1.0, walk(1.0, 0.0));
    let p = sim.root_pos();
    assert!(
        p.x > 10.0 && (p.y - 4.5).abs() < 0.2,
        "did not step up: {p:?}"
    );
    sim.run(2.0, walk(-1.0, 0.0));
    let p = sim.root_pos();
    assert!(p.x > -7.0 && p.x < -4.0, "walked through the wall: {p:?}");
    assert!((p.y - 3.0).abs() < 0.2, "climbed the wall: {p:?}");
}

#[test]
fn carried_by_moving_platform() {
    let mut world = World::new_with_services();
    let plat = part(
        &mut world,
        "Part",
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(20.0, 2.0, 20.0),
    );
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    let x0 = sim.root_pos().x;
    for i in 1..=60 {
        let x = i as f64 * 8.0 / 60.0;
        sim.world
            .set(plat, "Position", Value::Vector3(Vec3::new(x, -1.0, 0.0)))
            .unwrap();
        sim.frame(idle());
    }
    let moved = sim.root_pos().x - x0;
    assert!((moved - 8.0).abs() <= 0.5, "moved {moved}");
    assert_eq!(sim.ctl.state(sim.character()), Some(HumanoidState::Running));
}

#[test]
fn pushed_out_of_moving_part() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let pusher = part(
        &mut world,
        "Part",
        Vec3::new(-4.0, 3.0, 0.0),
        Vec3::new(2.0, 6.0, 10.0),
    );
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    for i in 1..=60 {
        let x = -4.0 + i as f64 * 8.0 / 60.0;
        sim.world
            .set(pusher, "Position", Value::Vector3(Vec3::new(x, 3.0, 0.0)))
            .unwrap();
        sim.frame(idle());
    }
    // the pusher's +X face is at x = 5; the character (radius 1) stays in front of it
    let p = sim.root_pos();
    assert!(p.x > 5.5, "not pushed: {p:?}");
}

#[test]
fn character_touches_anchored_lava() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    // a lava strip inset into the floor, top at y = 0
    let lava = part(
        &mut world,
        "Part",
        Vec3::new(10.0, -0.5, 0.0),
        Vec3::new(4.0, 1.0, 12.0),
    );
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    sim.phys.drain_touched();
    sim.run(1.0, walk(1.0, 0.0));
    let ch = sim.character();
    let hits: Vec<_> = sim
        .phys
        .drain_touched()
        .into_iter()
        .filter(|&(a, b)| a == lava || b == lava)
        .map(|(a, b)| if a == lava { b } else { a })
        .collect();
    assert!(!hits.is_empty(), "lava was never touched");
    assert!(hits.iter().all(|&h| sim.world.parent(h) == Some(ch)));
}

#[test]
fn own_parts_never_touch_each_other() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let mut sim = Sim::start(world);
    sim.run(1.0, walk(1.0, 1.0));
    let ch = sim.character();
    let inside = |id: InstanceId| {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if c == ch {
                return true;
            }
            cur = sim.world.parent(c);
        }
        false
    };
    for (a, b) in sim.phys.drain_touched() {
        assert!(!(inside(a) && inside(b)), "self touch {a:?} {b:?}");
    }
}

#[test]
fn falls_below_height_dies_once_and_respawns_at_checkpoint() {
    let mut world = World::new_with_services();
    let ws = world.root("Workspace").unwrap();
    world
        .set(ws, "FallenPartsDestroyHeight", Value::Number(-50.0))
        .unwrap();
    baseplate(&mut world);
    // a checkpoint pad inset into the floor at x = 20, top at y = 0
    let cp = part(
        &mut world,
        "Checkpoint",
        Vec3::new(20.0, -0.5, 0.0),
        Vec3::new(6.0, 1.0, 6.0),
    );
    world.set(cp, "Order", Value::Number(1.0)).unwrap();
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    sim.run(1.25, walk(1.0, 0.0));
    assert_eq!(
        reference(&sim.world, sim.player, "RespawnLocation"),
        Some(cp)
    );
    assert_eq!(sim.world.get(cp, "Color").unwrap(), Value::Color3(VIOLET));
    assert!(sim.events.iter().any(|e| matches!(
        e,
        CharacterEvent::CheckpointReached { checkpoint, .. } if *checkpoint == cp
    )));

    // a script teleports the character below the fall height
    let old = sim.character();
    let root = sim.root();
    sim.world
        .set(root, "Position", Value::Vector3(Vec3::new(0.0, -60.0, 0.0)))
        .unwrap();
    sim.frame(idle());
    assert_eq!(sim.ctl.state(old), Some(HumanoidState::Dead));
    assert_eq!(sim.died_count(), 1);

    // still dead just before the delay, respawned right after it
    let frames = (RESPAWN_DELAY * 60.0).round() as usize;
    for _ in 0..frames - 3 {
        sim.frame(idle());
    }
    assert_eq!(sim.character(), old);
    for _ in 0..6 {
        sim.frame(idle());
    }
    let new = sim.character();
    assert_ne!(new, old);
    assert!(!sim.world.is_alive(old));
    assert_eq!(sim.died_count(), 1);
    assert!(sim.events.iter().any(|e| matches!(
        e,
        CharacterEvent::Respawned { player, character, .. } if *player == sim.player && *character == new
    )));
    let p = sim.root_pos();
    assert!(
        (p - Vec3::new(20.0, 3.0, 0.0)).magnitude() < 0.2,
        "respawned at {p:?}"
    );
    assert_eq!(sim.ctl.state(new), Some(HumanoidState::Running));
    assert_eq!(num(&sim.world, sim.humanoid(), "Health"), 100.0);
}

#[test]
fn kill_twice_while_dead_fires_died_once() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    let hum = sim.humanoid();
    sim.world.set(hum, "Health", Value::Number(0.0)).unwrap();
    sim.frame(idle());
    assert_eq!(sim.ctl.state(sim.character()), Some(HumanoidState::Dead));
    let x0 = sim.root_pos().x;
    // killed again by a second hazard, and input is ignored while dead
    sim.world.set(hum, "Health", Value::Number(-20.0)).unwrap();
    sim.run(1.0, walk(1.0, 0.0));
    sim.world.set(hum, "Health", Value::Number(0.0)).unwrap();
    sim.run(
        0.5,
        MoveInput {
            dir: Vec3::new(1.0, 0.0, 0.0),
            jump: true,
        },
    );
    assert_eq!(sim.died_count(), 1);
    assert!((sim.root_pos().x - x0).abs() < 1e-6, "moved while dead");
}

#[test]
fn checkpoint_lower_order_does_not_override_higher() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    // tall non-colliding checkpoint zones the character walks through: Order 2 first, then 1
    let second = part(
        &mut world,
        "Checkpoint",
        Vec3::new(10.0, 3.0, 0.0),
        Vec3::new(2.0, 6.0, 8.0),
    );
    world.set(second, "Order", Value::Number(2.0)).unwrap();
    world.set(second, "CanCollide", Value::Bool(false)).unwrap();
    let first = part(
        &mut world,
        "Checkpoint",
        Vec3::new(20.0, 3.0, 0.0),
        Vec3::new(2.0, 6.0, 8.0),
    );
    world.set(first, "Order", Value::Number(1.0)).unwrap();
    world.set(first, "CanCollide", Value::Bool(false)).unwrap();
    let grey = world.get(first, "Color").unwrap();
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    sim.run(2.0, walk(1.0, 0.0));
    assert!(sim.root_pos().x > 22.0, "did not pass both");
    assert_eq!(
        reference(&sim.world, sim.player, "RespawnLocation"),
        Some(second)
    );
    assert_eq!(
        sim.world.get(second, "Color").unwrap(),
        Value::Color3(VIOLET)
    );
    assert_eq!(sim.world.get(first, "Color").unwrap(), grey);
    let reached: Vec<_> = sim
        .events
        .iter()
        .filter(|e| matches!(e, CharacterEvent::CheckpointReached { .. }))
        .collect();
    assert_eq!(reached.len(), 1, "{reached:?}");
}
