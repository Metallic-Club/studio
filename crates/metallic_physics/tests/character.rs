use metallic_core::CFrame;
use metallic_core::{Color3, InstanceId, Value, Vec3, World};
use metallic_physics::{
    BodyPart, BodyPose, CharacterController, CharacterEvent, CharacterState, HumanoidState,
    MAX_PUSH, MAX_TOUCH_CANDIDATES, MoveInput, PhysicsWorld, RESPAWN_DELAY, WORLD_LIMIT,
    spawn_character,
};

const DT: f64 = 1.0 / 60.0;
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
    /// Parts that kill on touch.
    lava: Vec<InstanceId>,
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
            lava: Vec::new(),
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
        // The lava script: `Humanoid.Health = 0` when lava is touched.
        for e in &new {
            if let CharacterEvent::Touched {
                character, part, ..
            } = *e
                && self.lava.contains(&part)
                && let Some(h) = self.world.find_first_child(character, "Humanoid")
            {
                self.world.set(h, "Health", Value::Number(0.0)).unwrap();
            }
        }
        self.events.extend(new);
        self.phys.step(&mut self.world, DT);
    }

    /// Touches of `part` by the movement capsule (reported as the `HumanoidRootPart`).
    fn touched_count(&self, part: InstanceId) -> usize {
        self.touched_by(part, "HumanoidRootPart")
    }

    /// Touches of `part` by the character part named `hit`.
    fn touched_by(&self, part: InstanceId, hit: &str) -> usize {
        self.count(part, hit, true)
    }

    /// TouchEnded events of `part` by the character part named `hit`.
    fn ended_by(&self, part: InstanceId, hit: &str) -> usize {
        self.count(part, hit, false)
    }

    fn count(&self, part: InstanceId, hit_name: &str, start: bool) -> usize {
        self.events
            .iter()
            .filter(|e| {
                let (p, h) = match **e {
                    CharacterEvent::Touched { part, hit, .. } if start => (part, hit),
                    CharacterEvent::TouchEnded { part, hit, .. } if !start => (part, hit),
                    _ => return false,
                };
                p == part && self.world.get(h, "Name").ok() == Some(Value::String(hit_name.into()))
            })
            .count()
    }

    /// Names of the character parts that touched `part`, sorted, without repeats.
    fn hits(&self, part: InstanceId) -> Vec<String> {
        let mut names: Vec<String> = self
            .events
            .iter()
            .filter_map(|e| match *e {
                CharacterEvent::Touched { part: p, hit, .. } if p == part => {
                    match self.world.get(hit, "Name") {
                        Ok(Value::String(n)) => Some(n),
                        _ => None,
                    }
                }
                _ => None,
            })
            .collect();
        names.sort();
        names.dedup();
        names
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
        ..MoveInput::default()
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

    for body in BodyPart::ALL {
        let name = body.name();
        let p = world
            .find_first_child(ch, name)
            .unwrap_or_else(|| panic!("missing {name}"));
        assert_eq!(v3(&world, p, "Size"), body.size(), "{name}");
        assert!(!world.get(p, "CanCollide").unwrap().eq(&Value::Bool(true)));
    }
    // The body parts hold the touch volumes; the runtime draws the skinned avatar.
    let parts: Vec<InstanceId> = world
        .children(ch)
        .iter()
        .copied()
        .filter(|&c| world.is_a(c, "BasePart"))
        .collect();
    assert_eq!(parts.len(), 16);
    for p in parts {
        assert_eq!(num(&world, p, "Transparency"), 1.0);
        assert!(world.children(p).is_empty());
    }
    // feet on the ground: the foot boxes end 3 studs below the root
    let foot = world.find_first_child(ch, "LeftFoot").unwrap();
    let bottom = v3(&world, foot, "Position").y - v3(&world, foot, "Size").y / 2.0;
    assert!(bottom.abs() < 1e-9, "{bottom}");
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
        .find_first_child(sim.character(), "UpperTorso")
        .unwrap();
    assert!((v3(&sim.world, torso, "Orientation").y + 90.0).abs() < 1e-6);
}

#[test]
fn running_is_one_and_a_half_times_walk_speed_without_changing_it() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    let start = sim.root_pos();
    sim.run(
        1.0,
        MoveInput {
            run: true,
            ..walk(1.0, 0.0)
        },
    );
    let d = sim.root_pos().x - start.x;
    assert!((d - 24.0).abs() <= 24.0 * 0.05, "ran {d}");
    let hum = sim
        .world
        .find_first_child(sim.character(), "Humanoid")
        .unwrap();
    assert_eq!(num(&sim.world, hum, "WalkSpeed"), 16.0);
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
        ..MoveInput::default()
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
    // Carried at 8 studs/s while standing still: no own motion (the avatar stays idle).
    let ch = sim.character();
    assert!((sim.ctl.velocity(ch).unwrap().x - 8.0).abs() < 0.5);
    assert!(sim.ctl.relative_velocity(ch).unwrap().magnitude() < 0.5);
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
    sim.lava.push(lava);
    sim.run(0.5, idle());
    sim.run(1.0, walk(1.0, 0.0));
    // The feet reach the strip before the rounded bottom of the capsule does.
    assert!(sim.touched_by(lava, "LeftFoot") + sim.touched_by(lava, "RightFoot") >= 1);
    assert_eq!(sim.died_count(), 1);
}

#[test]
fn standing_still_on_lava_dies() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    assert_eq!(sim.died_count(), 0);
    // lava appears right under the standing character, top flush with the floor
    let lava = part(
        &mut sim.world,
        "Part",
        Vec3::new(0.0, -0.5, 0.0),
        Vec3::new(4.0, 1.0, 4.0),
    );
    sim.phys.sync_from(&sim.world);
    sim.lava.push(lava);
    sim.run(0.25, idle());
    assert_eq!(sim.touched_count(lava), 1);
    assert_eq!(sim.died_count(), 1);
    // dead characters report no further touches
    sim.run(1.0, idle());
    assert_eq!(sim.touched_count(lava), 1);
    assert_eq!(sim.died_count(), 1);
}

#[test]
fn arm_brushing_checkpoint_registers() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    // a zone beside the path: only the hanging arm (out to 1.46 with the margin) reaches it, the
    // capsule (radius 1.1 with the margin) does not
    let cp = part(
        &mut world,
        "Checkpoint",
        Vec3::new(2.3, 3.0, 10.0),
        Vec3::new(2.0, 6.0, 4.0),
    );
    world.set(cp, "CanCollide", Value::Bool(false)).unwrap();
    world.set(cp, "Order", Value::Number(1.0)).unwrap();
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    sim.run(1.0, walk(0.0, 1.0));
    let p = sim.root_pos();
    assert!(p.z > 14.0 && p.x.abs() < 0.05, "path {p:?}");
    assert_eq!(sim.touched_count(cp), 0, "the capsule stays clear");
    // Walking toward +Z, the character's left side faces +X.
    assert!(
        sim.hits(cp).iter().all(|h| h.starts_with("Left")),
        "{:?}",
        sim.hits(cp)
    );
    assert_eq!(sim.touched_by(cp, "LeftHand"), 1);
    assert_eq!(
        reference(&sim.world, sim.player, "RespawnLocation"),
        Some(cp)
    );
}

#[test]
fn touched_fires_once_until_separation() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let pad = part(
        &mut world,
        "Part",
        Vec3::new(10.0, -0.5, 0.0),
        Vec3::new(4.0, 1.0, 8.0),
    );
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    sim.run(0.625, walk(1.0, 0.0));
    sim.run(1.0, idle());
    assert_eq!(sim.touched_count(pad), 1, "standing on it");
    sim.run(1.0, walk(1.0, 0.0));
    assert_eq!(sim.touched_count(pad), 1, "walked off");
    sim.run(1.0, walk(-1.0, 0.0));
    assert!((sim.root_pos().x - 10.0).abs() < 1.0);
    assert_eq!(sim.touched_count(pad), 2, "came back");
}

#[test]
fn physics_never_reports_character_parts() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let zone = part(
        &mut world,
        "Part",
        Vec3::new(8.0, 3.0, 8.0),
        Vec3::new(4.0, 6.0, 4.0),
    );
    world.set(zone, "CanCollide", Value::Bool(false)).unwrap();
    let mut sim = Sim::start(world);
    sim.run(1.0, walk(1.0, 1.0));
    assert_eq!(sim.touched_count(zone), 1);
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
        assert!(!inside(a) && !inside(b), "physics touch {a:?} {b:?}");
    }
}

#[test]
fn push_out_is_capped_per_step() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    // a wall appears 1.5 studs deep inside the character (its +X face at x = 0.5)
    part(
        &mut sim.world,
        "Part",
        Vec3::new(-2.25, 3.0, 0.0),
        Vec3::new(5.5, 6.0, 10.0),
    );
    sim.phys.sync_from(&sim.world);
    let x0 = sim.root_pos().x;
    sim.frame(idle());
    let d = sim.root_pos().x - x0;
    assert!(d > 0.0 && d <= MAX_PUSH + 1e-9, "pushed {d} in one step");
    sim.run(0.5, idle());
    assert!(sim.root_pos().x >= 1.49, "not out: {:?}", sim.root_pos());
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
            ..MoveInput::default()
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

#[test]
fn respawn_on_non_colliding_checkpoint_stands_on_top() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    // a tall zone whose top is at y = 6
    let zone = part(
        &mut world,
        "Checkpoint",
        Vec3::new(10.0, 3.0, 0.0),
        Vec3::new(4.0, 6.0, 4.0),
    );
    world.set(zone, "CanCollide", Value::Bool(false)).unwrap();
    // a pad lying on its side: Size.X is its height once rolled 90 degrees, top at y = 3
    let rolled = part(
        &mut world,
        "Checkpoint",
        Vec3::new(-10.0, 0.0, 0.0),
        Vec3::new(6.0, 2.0, 4.0),
    );
    world
        .set(
            rolled,
            "Orientation",
            Value::Vector3(Vec3::new(0.0, 0.0, 90.0)),
        )
        .unwrap();
    let player = add_player(&mut world);
    world
        .set(player, "RespawnLocation", Value::Ref(Some(zone)))
        .unwrap();
    let ch = spawn_character(&mut world, player).unwrap();
    let root = world.find_first_child(ch, "HumanoidRootPart").unwrap();
    let p = v3(&world, root, "Position");
    assert!((p - Vec3::new(10.0, 9.0, 0.0)).magnitude() < 1e-9, "{p:?}");
    world
        .set(player, "RespawnLocation", Value::Ref(Some(rolled)))
        .unwrap();
    let ch = spawn_character(&mut world, player).unwrap();
    let root = world.find_first_child(ch, "HumanoidRootPart").unwrap();
    let p = v3(&world, root, "Position");
    assert!((p - Vec3::new(-10.0, 6.0, 0.0)).magnitude() < 1e-9, "{p:?}");
}

/// A level with steps, a slope-free wall and a pit edge, so replays touch every code path.
fn replay_level() -> Sim {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    for i in 0..4 {
        let h = 1.0 + f64::from(i);
        part(
            &mut world,
            "Part",
            Vec3::new(10.0 + f64::from(i) * 6.0, h / 2.0, 0.0),
            Vec3::new(6.0, h, 20.0),
        );
    }
    part(
        &mut world,
        "Part",
        Vec3::new(-15.0, 5.0, 0.0),
        Vec3::new(2.0, 10.0, 40.0),
    );
    Sim::start(world)
}

/// Seeded inputs (a linear congruential generator) with jumps and runs, held for a few frames.
fn seeded_inputs(count: usize) -> Vec<MoveInput> {
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as f64 / f64::from(1u32 << 31)
    };
    let mut out = Vec::with_capacity(count);
    let mut cur = MoveInput::default();
    while out.len() < count {
        if out.len() % 12 == 0 {
            let a = next() * std::f64::consts::TAU;
            cur = MoveInput {
                dir: Vec3::new(a.cos(), 0.0, a.sin()) * (0.4 + next()),
                jump: next() < 0.3,
                run: next() < 0.4,
            };
        }
        out.push(cur);
    }
    out
}

fn replay(sim: &mut Sim, inputs: &[MoveInput]) -> Vec<(Vec3, HumanoidState)> {
    inputs
        .iter()
        .map(|&i| {
            sim.frame(i);
            let ch = sim.character();
            (sim.root_pos(), sim.ctl.state(ch).unwrap())
        })
        .collect()
}

#[test]
fn replay_600_inputs_from_saved_state_matches() {
    let mut sim = replay_level();
    let inputs = seeded_inputs(600);
    let ch = sim.character();
    let saved = sim.ctl.save_state(&sim.world, ch).expect("saved");
    // First pass; also save mid-run, airborne, once past 150 inputs.
    let mut mid: Option<(usize, CharacterState)> = None;
    let mut first = Vec::new();
    for (i, &input) in inputs.iter().enumerate() {
        sim.frame(input);
        first.push((sim.root_pos(), sim.ctl.state(ch).unwrap()));
        if mid.is_none() && i >= 150 {
            let s = sim.ctl.save_state(&sim.world, ch).unwrap();
            if s.vertical != 0.0 {
                mid = Some((i + 1, s));
            }
        }
    }
    let moved = first
        .iter()
        .any(|(p, _)| (*p - saved.position).magnitude() > 5.0);
    assert!(moved, "the inputs must actually move the character");
    let (start, mid_state) = mid.expect("an airborne tick after 150 inputs");
    assert_ne!(mid_state.vertical, 0.0);

    for (from, state) in [(0, saved), (start, mid_state)] {
        sim.ctl.restore_state(&mut sim.world, ch, &state);
        let second = replay(&mut sim, &inputs[from..]);
        for (i, (a, b)) in first[from..].iter().zip(&second).enumerate() {
            let tick = from + i;
            assert!(
                (a.0 - b.0).magnitude() < 1e-4,
                "tick {tick}: {:?} vs {:?}",
                a.0,
                b.0
            );
            assert_eq!(a.1, b.1, "tick {tick}");
        }
    }
}

#[test]
fn save_then_restore_returns_the_same_state() {
    let mut sim = replay_level();
    let ch = sim.character();
    // Settle on the ground, then jump and save a few frames into the air.
    sim.run(0.5, MoveInput::default());
    sim.run(
        0.1,
        MoveInput {
            dir: Vec3::new(1.0, 0.0, 0.0),
            jump: true,
            run: false,
        },
    );
    let saved = sim.ctl.save_state(&sim.world, ch).unwrap();
    assert_ne!(saved.vertical, 0.0, "saved while airborne");
    sim.run(
        1.0,
        MoveInput {
            dir: Vec3::new(0.0, 0.0, 1.0),
            jump: false,
            run: true,
        },
    );
    sim.ctl.restore_state(&mut sim.world, ch, &saved);
    assert_eq!(sim.ctl.save_state(&sim.world, ch), Some(saved));
}

#[test]
fn restore_moves_root_and_limbs() {
    let mut sim = replay_level();
    let ch = sim.character();
    let mut state = sim.ctl.save_state(&sim.world, ch).unwrap();
    state.position = Vec3::new(40.0, 3.0, -20.0);
    state.yaw = 90.0;
    sim.ctl.restore_state(&mut sim.world, ch, &state);
    assert!((sim.root_pos() - state.position).magnitude() < 1e-9);
    // Yaw 90 turns an offset (x, y, z) into (z, y, -x).
    let turned = |p: BodyPart| {
        let c = p.rest_centre();
        state.position + Vec3::new(c.z, c.y, -c.x)
    };
    for p in [BodyPart::Head, BodyPart::RightUpperArm, BodyPart::LeftFoot] {
        let id = sim.world.find_first_child(ch, p.name()).unwrap();
        let at = v3(&sim.world, id, "Position");
        assert!((at - turned(p)).magnitude() < 1e-9, "{p:?} at {at:?}");
        assert!((v3(&sim.world, id, "Orientation").y - 90.0).abs() < 1e-9);
    }
}

#[test]
fn place_character_poses_the_limbs() {
    let mut sim = replay_level();
    let ch = sim.character();
    metallic_physics::place_character(&mut sim.world, ch, Vec3::new(1.0, 2.0, 3.0), 0.0);
    assert!((sim.root_pos() - Vec3::new(1.0, 2.0, 3.0)).magnitude() < 1e-9);
    let foot = sim.world.find_first_child(ch, "LeftFoot").unwrap();
    let expect = Vec3::new(1.0, 2.0, 3.0) + BodyPart::LeftFoot.rest_centre();
    assert!((v3(&sim.world, foot, "Position") - expect).magnitude() < 1e-9);
}

/// A pose whose right arm points straight forward at shoulder height (facing -Z), the rest
/// unchanged.
fn pointing_pose() -> BodyPose {
    let mut pose = BodyPose::rest(0.0);
    // Turn each right-arm part 90 degrees about X around the shoulder: down becomes forward.
    let shoulder = BodyPart::RightUpperArm.rest_ends().0;
    let turn = CFrame::from_position(shoulder)
        * CFrame::from_euler_xyz(std::f64::consts::FRAC_PI_2, 0.0, 0.0)
        * CFrame::from_position(shoulder * -1.0);
    for p in [
        BodyPart::RightUpperArm,
        BodyPart::RightLowerArm,
        BodyPart::RightHand,
    ] {
        pose.parts[p.index()] = turn * pose.parts[p.index()];
    }
    pose
}

#[test]
fn pointing_hand_touches_a_part_the_idle_body_does_not() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    // A small sensor ahead of the right shoulder, beyond the capsule (1.1 with the margin).
    let target = part(
        &mut world,
        "Part",
        Vec3::new(0.75, 3.6, -2.0),
        Vec3::new(0.6, 0.6, 0.6),
    );
    world.set(target, "CanCollide", Value::Bool(false)).unwrap();
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    assert!(sim.hits(target).is_empty(), "idle body touched it");
    let ch = sim.character();
    let pose = pointing_pose();
    let (_, tip) = pose.world_ends(BodyPart::RightHand, sim.root_pos());
    assert!(tip.z < -1.5, "the hand points forward: {tip:?}");
    sim.ctl.set_body_pose(ch, &pose);
    sim.run(0.1, idle());
    assert_eq!(sim.touched_by(target, "RightHand"), 1);
    assert_eq!(
        sim.touched_count(target),
        0,
        "the capsule does not reach it"
    );
    let touching = sim.ctl.touching(ch, target);
    assert!(
        touching.contains(&Some(BodyPart::RightHand)) && !touching.contains(&None),
        "{touching:?}"
    );
    // The hand box in the world moved with the volume.
    let hand = sim.world.find_first_child(ch, "RightHand").unwrap();
    let centre = sim.root_pos() + pose.parts[BodyPart::RightHand.index()].position;
    assert!((v3(&sim.world, hand, "Position") - centre).magnitude() < 1e-9);
    // Lowering the arm again ends the touch.
    sim.ctl.clear_body_pose(ch);
    sim.run(0.1, idle());
    assert_eq!(sim.ended_by(target, "RightHand"), 1);
    assert_eq!(sim.touched_by(target, "RightHand"), 1);
}

#[test]
fn body_volumes_never_push_or_block() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    // A loose post standing where the lowered pointing hand will reach.
    let crate_part = part(
        &mut world,
        "Part",
        Vec3::new(0.75, 1.25, -2.0),
        Vec3::new(1.0, 2.5, 1.0),
    );
    world
        .set(crate_part, "Anchored", Value::Bool(false))
        .unwrap();
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    let rest = v3(&sim.world, crate_part, "Position");
    let ch = sim.character();
    // Point the arm down-forward into the crate: rotate the pointing pose down 45 degrees.
    let mut pose = pointing_pose();
    let shoulder = BodyPart::RightUpperArm.rest_ends().0;
    let down = CFrame::from_position(shoulder)
        * CFrame::from_euler_xyz(-0.9, 0.0, 0.0)
        * CFrame::from_position(shoulder * -1.0);
    for p in [
        BodyPart::RightUpperArm,
        BodyPart::RightLowerArm,
        BodyPart::RightHand,
    ] {
        pose.parts[p.index()] = down * pose.parts[p.index()];
    }
    sim.ctl.set_body_pose(ch, &pose);
    sim.run(0.5, idle());
    assert_eq!(
        sim.touched_by(crate_part, "RightHand"),
        1,
        "{:?}",
        sim.hits(crate_part)
    );
    let after = v3(&sim.world, crate_part, "Position");
    assert!(
        (after - rest).magnitude() < 1e-3,
        "pushed from {rest:?} to {after:?}"
    );
    // Rays pass through the body parts: a ray at the hand only meets the crate behind it.
    let hand = sim.world.find_first_child(ch, "RightHand").unwrap();
    let at = v3(&sim.world, hand, "Position");
    let hit = sim.phys.raycast(
        at + Vec3::new(5.0, 0.0, 0.0),
        Vec3::new(-10.0, 0.0, 0.0),
        &[],
    );
    assert!(hit.is_none_or(|h| h.instance != hand));
}

#[test]
fn touch_ended_pairs_every_touched() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let pad = part(
        &mut world,
        "Part",
        Vec3::new(10.0, -0.5, 0.0),
        Vec3::new(4.0, 1.0, 8.0),
    );
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    sim.run(2.0, walk(1.0, 0.0));
    assert!(sim.root_pos().x > 20.0);
    let hits = sim.hits(pad);
    assert!(hits.contains(&"LeftFoot".to_string()), "{hits:?}");
    assert!(hits.contains(&"HumanoidRootPart".to_string()), "{hits:?}");
    for h in &hits {
        assert_eq!(sim.touched_by(pad, h), sim.ended_by(pad, h), "{h}");
    }
}

#[test]
fn touch_query_cost_is_bounded() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    // 4000 parts far away and a dense cloud of 1000 small sensors around the character.
    for i in 0..4000 {
        let (x, z) = ((i % 64) as f64 * 3.0 + 100.0, (i / 64) as f64 * 3.0 + 100.0);
        part(
            &mut world,
            "Part",
            Vec3::new(x, 0.5, z),
            Vec3::new(1.0, 1.0, 1.0),
        );
    }
    let mut sim = Sim::start(world);
    sim.run(0.25, idle());
    let ch = sim.character();
    let cost = sim.ctl.touch_query_cost(ch);
    assert!(cost.parts <= 2, "far parts were looked at: {cost:?}");
    assert!(cost.tests <= 32, "{cost:?}");
    assert_eq!(cost.capped_queries, 0);
    for i in 0..1000 {
        let f = i as f64;
        let at = Vec3::new(
            (f * 0.37).sin() * 1.2,
            3.0 + (f * 0.11).cos() * 2.4,
            (f * 0.53).sin() * 1.2,
        );
        let id = part(&mut sim.world, "Part", at, Vec3::new(0.2, 0.2, 0.2));
        sim.world.set(id, "CanCollide", Value::Bool(false)).unwrap();
    }
    sim.phys.sync_from(&sim.world);
    sim.run(0.1, idle());
    let cost = sim.ctl.touch_query_cost(ch);
    assert_eq!(cost.parts, MAX_TOUCH_CANDIDATES);
    assert!(cost.tests <= MAX_TOUCH_CANDIDATES * 16, "{cost:?}");
    assert!(cost.dropped > 0 && cost.capped_queries > 0, "{cost:?}");
}

#[test]
fn contacts_survive_the_cap_and_the_nearest_parts_win() {
    let mut world = World::new_with_services();
    let floor = baseplate(&mut world);
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    assert!(sim.touched_count(floor) == 1, "standing on the floor");
    let root = sim.root_pos();
    // 1200 specks in the corners of the body's bounds: inside the broad-phase box, touching
    // nothing, and created before the part that matters.
    for i in 0..1200 {
        let f = i as f64;
        let (sx, sz) = [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)][i % 4];
        let at = root + Vec3::new(sx, -2.8 + (f / 1200.0) * 5.3, sz);
        let id = part(&mut sim.world, "Part", at, Vec3::new(0.05, 0.05, 0.05));
        sim.world.set(id, "CanCollide", Value::Bool(false)).unwrap();
    }
    // Then a sensor at the chest.
    let target = part(
        &mut sim.world,
        "Part",
        root + Vec3::new(0.0, 0.3, -0.3),
        Vec3::new(0.2, 0.2, 0.2),
    );
    sim.world
        .set(target, "CanCollide", Value::Bool(false))
        .unwrap();
    sim.phys.sync_from(&sim.world);
    sim.run(0.5, idle());
    let cost = sim.ctl.touch_query_cost(sim.character());
    assert!(cost.dropped > 0, "the cap was reached: {cost:?}");
    assert!(!sim.hits(target).is_empty(), "the nearest part was tested");
    for h in sim.hits(floor) {
        assert_eq!(sim.ended_by(floor, &h), 0, "{h} lost the floor at the cap");
    }
}

#[test]
fn destroyed_or_replaced_characters_end_their_contacts() {
    let mut world = World::new_with_services();
    let floor = baseplate(&mut world);
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    let first = sim.character();
    let touching = sim.hits(floor);
    assert!(!touching.is_empty());
    // Replaced without dying: a new character for the same player.
    let second = spawn_character(&mut sim.world, sim.player).unwrap();
    sim.phys.sync_from(&sim.world);
    sim.run(0.5, idle());
    let ended = |sim: &Sim, character: InstanceId| {
        sim.events
            .iter()
            .filter(|e| {
                matches!(e, CharacterEvent::TouchEnded { character: c, part, .. }
                    if *c == character && *part == floor)
            })
            .count()
    };
    let touched = |sim: &Sim, character: InstanceId| {
        sim.events
            .iter()
            .filter(|e| {
                matches!(e, CharacterEvent::Touched { character: c, part, .. }
                    if *c == character && *part == floor)
            })
            .count()
    };
    assert_eq!(ended(&sim, first), touched(&sim, first), "replaced");
    assert!(touched(&sim, second) > 0);
    // Destroyed outright: the next step of any character ends its contacts.
    sim.world.destroy(second);
    sim.ctl.step(&mut sim.world, &sim.phys, second, idle(), DT);
    sim.events.extend(sim.ctl.drain_events());
    assert_eq!(ended(&sim, second), touched(&sim, second), "destroyed");
}

#[test]
fn npc_with_r15_named_parts_keeps_solid_physics() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let ws = world.root("Workspace").unwrap();
    let npc = world.create("Model", Some(ws)).unwrap();
    world.create("Humanoid", Some(npc)).unwrap();
    let mut solid = |name: &str, pos: Vec3, size: Vec3| {
        let id = world.create("Part", Some(npc)).unwrap();
        world.set(id, "Name", Value::String(name.into())).unwrap();
        world.set(id, "Position", Value::Vector3(pos)).unwrap();
        world.set(id, "Size", Value::Vector3(size)).unwrap();
        world.set(id, "Anchored", Value::Bool(true)).unwrap();
        id
    };
    let torso = solid(
        "UpperTorso",
        Vec3::new(20.0, 3.0, 0.0),
        Vec3::new(2.0, 2.0, 1.0),
    );
    let head = solid("Head", Vec3::new(20.0, 4.5, 0.0), Vec3::new(1.0, 1.0, 1.0));
    // A loose ball dropped on the NPC's head.
    let ball = part(
        &mut world,
        "Part",
        Vec3::new(20.0, 8.0, 0.0),
        Vec3::new(1.0, 1.0, 1.0),
    );
    world.set(ball, "Anchored", Value::Bool(false)).unwrap();
    let mut sim = Sim::start(world);
    sim.run(1.5, idle());
    let y = v3(&sim.world, ball, "Position").y;
    assert!((y - 5.5).abs() < 0.1, "the ball rests on the head, at {y}");
    let ray = sim
        .phys
        .raycast(Vec3::new(30.0, 3.0, 0.0), Vec3::new(-20.0, 0.0, 0.0), &[]);
    assert_eq!(
        ray.map(|h| h.instance),
        Some(torso),
        "rays hit the NPC torso"
    );
    // Renaming changes nothing: there is no name-based rule to flip.
    sim.world
        .set(head, "Name", Value::String("Hat".into()))
        .unwrap();
    sim.world
        .set(torso, "Name", Value::String("LeftHand".into()))
        .unwrap();
    sim.phys.sync_from(&sim.world);
    sim.run(0.5, idle());
    let y = v3(&sim.world, ball, "Position").y;
    assert!((y - 5.5).abs() < 0.1, "still resting, at {y}");
    let ray = sim
        .phys
        .raycast(Vec3::new(30.0, 3.0, 0.0), Vec3::new(-20.0, 0.0, 0.0), &[]);
    assert_eq!(ray.map(|h| h.instance), Some(torso));
}

// ---- Sanitising Humanoid tuning and the controller state (#172, #272) ----

fn finite(v: Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

fn jump() -> MoveInput {
    MoveInput {
        jump: true,
        ..MoveInput::default()
    }
}

fn respawned_count(sim: &Sim) -> usize {
    sim.events
        .iter()
        .filter(|e| matches!(e, CharacterEvent::Respawned { .. }))
        .count()
}

/// #172: a NaN JumpPower is refused, so the character keeps jumping and walking.
#[test]
fn nan_jump_power_does_not_freeze_the_character() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    let h = sim.humanoid();
    assert!(
        sim.world
            .set(h, "JumpPower", Value::Number(f64::NAN))
            .is_err()
    );
    assert_eq!(num(&sim.world, h, "JumpPower"), 50.0);
    sim.run(0.1, jump());
    sim.run(1.5, idle());
    let start = sim.root_pos();
    sim.run(2.0, walk(1.0, 0.0));
    let moved = sim.root_pos().x - start.x;
    assert!((moved - 32.0).abs() < 1.0, "walked {moved} studs in 2 s");
}

/// #172: a non-finite vertical speed in a restored state (the controller's own write boundary)
/// is reset, so the character is not frozen afterwards.
#[test]
fn non_finite_restored_state_is_reset_to_rest() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    let ch = sim.character();
    let mut state = sim.ctl.save_state(&sim.world, ch).unwrap();
    state.vertical = f64::NAN;
    state.inherited = Vec3::new(f64::INFINITY, 0.0, 0.0);
    state.velocity = Vec3::new(f64::NAN, f64::NAN, f64::NAN);
    state.yaw = f64::NAN;
    sim.ctl.restore_state(&mut sim.world, ch, &state);
    let start = sim.root_pos();
    sim.run(2.0, walk(1.0, 0.0));
    let end = sim.root_pos();
    assert!(finite(end));
    assert!((end.x - start.x - 32.0).abs() < 1.0, "walked to {end:?}");
    assert!(finite(sim.ctl.velocity(ch).unwrap()));
    // A NaN move direction is ignored rather than spread into the pose.
    sim.run(
        0.5,
        MoveInput {
            dir: Vec3::new(f64::NAN, 0.0, 1.0),
            ..MoveInput::default()
        },
    );
    assert!(finite(sim.root_pos()));
    assert_eq!(sim.died_count(), 0);
}

/// #172: a NaN Health is refused; Health 0 still kills (the death test is `!(health > 0)`).
#[test]
fn nan_health_is_refused_and_zero_health_dies() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let mut sim = Sim::start(world);
    sim.run(0.25, idle());
    let h = sim.humanoid();
    assert!(sim.world.set(h, "Health", Value::Number(f64::NAN)).is_err());
    assert_eq!(num(&sim.world, h, "Health"), 100.0);
    sim.world.set(h, "Health", Value::Number(-1e300)).unwrap();
    assert_eq!(num(&sim.world, h, "Health"), 0.0);
    sim.run(0.1, idle());
    assert_eq!(sim.died_count(), 1);
    sim.run(RESPAWN_DELAY + 0.1, idle());
    assert_eq!(respawned_count(&sim), 1);
}

/// #272: WalkSpeed 1e300 is clamped to 1000 studs/s; the avatar stays finite.
#[test]
fn huge_walk_speed_is_clamped_and_stays_finite() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    let h = sim.humanoid();
    sim.world.set(h, "WalkSpeed", Value::Number(1e300)).unwrap();
    assert_eq!(num(&sim.world, h, "WalkSpeed"), 1000.0);
    let start = sim.root_pos();
    sim.run(0.1, walk(0.0, -1.0));
    let end = sim.root_pos();
    assert!(finite(end), "root {end:?}");
    let moved = start.z - end.z;
    assert!(moved > 50.0 && moved <= 1000.0 * 0.1 + 1.0, "moved {moved}");
    sim.run(3.0, walk(0.0, -1.0));
    assert!(finite(sim.root_pos()));
}

/// #272: JumpPower 1e300 is clamped to 1000; the jump peaks near 1000^2 / (2 g) and lands.
#[test]
fn huge_jump_power_is_clamped_and_stays_finite() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    let h = sim.humanoid();
    sim.world.set(h, "JumpPower", Value::Number(1e300)).unwrap();
    assert_eq!(num(&sim.world, h, "JumpPower"), 1000.0);
    let ground = sim.root_pos().y;
    let mut peak = ground;
    sim.frame(jump());
    for _ in 0..(12.0 * 60.0) as usize {
        sim.frame(idle());
        assert!(finite(sim.root_pos()));
        peak = peak.max(sim.root_pos().y);
    }
    let apex = 1000.0_f64.powi(2) / (2.0 * 196.2);
    assert!((peak - ground - apex).abs() < 0.02 * apex, "peak {peak}");
    assert!((sim.root_pos().y - ground).abs() < 0.1, "landed");
    assert_eq!(sim.died_count(), 0);
}

/// #272: a character outside the world bounds dies and respawns, including when the position
/// is huge (beyond [`WORLD_LIMIT`]) rather than below `FallenPartsDestroyHeight`.
#[test]
fn leaving_the_world_kills_and_respawns() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let mut sim = Sim::start(world);
    sim.run(0.5, idle());
    let old = sim.character();
    let mut state = sim.ctl.save_state(&sim.world, old).unwrap();
    state.position = Vec3::new(0.0, 3.0, -1e300);
    sim.ctl.restore_state(&mut sim.world, old, &state);
    sim.run(0.1, idle());
    assert_eq!(sim.died_count(), 1);
    sim.run(RESPAWN_DELAY + 0.1, idle());
    assert_eq!(respawned_count(&sim), 1);
    assert_ne!(sim.character(), old);
    let p = sim.root_pos();
    assert!(finite(p) && p.z.abs() < 1.0, "respawned at {p:?}");

    // Just past the limit on X, set by a script.
    let root = sim.root();
    sim.world
        .set(
            root,
            "Position",
            Value::Vector3(Vec3::new(WORLD_LIMIT * 1.5, 3.0, 0.0)),
        )
        .unwrap();
    sim.run(0.1, idle());
    assert_eq!(sim.died_count(), 2);

    // A non-finite restored position is never written, and the character keeps living.
    sim.run(RESPAWN_DELAY + 0.1, idle());
    let ch = sim.character();
    let before = sim.root_pos();
    let mut s = sim.ctl.save_state(&sim.world, ch).unwrap();
    s.position = Vec3::new(f64::NAN, f64::INFINITY, 0.0);
    sim.ctl.restore_state(&mut sim.world, ch, &s);
    assert_eq!(sim.root_pos(), before);
    sim.run(0.1, idle());
    assert_eq!(sim.died_count(), 2);
}

/// #345: a SpawnLocation beyond `WORLD_LIMIT` is skipped, so the character spawns at the default
/// point and lives instead of dying and respawning in a loop.
#[test]
fn spawn_location_outside_the_world_is_skipped() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    part(
        &mut world,
        "SpawnLocation",
        Vec3::new(WORLD_LIMIT * 2.0, 0.5, 0.0),
        Vec3::new(6.0, 1.0, 6.0),
    );
    let mut sim = Sim::start(world);
    assert!((sim.root_pos() - Vec3::new(0.0, 3.0, 0.0)).magnitude() < 1e-9);
    sim.run(RESPAWN_DELAY + 1.0, idle());
    assert_eq!(sim.died_count(), 0);
    assert_eq!(respawned_count(&sim), 0);
}

/// B's spot: a little ahead of A's right shoulder, facing A.
const FACING: Vec3 = Vec3::new(0.75, 3.0, -2.4);

/// A (`Player1`) at the origin facing -Z with its right arm pointing forward, and B at
/// [`FACING`]. With `stepped`, both are stepped once far apart first (a character the
/// controller has not stepped yet is an ordinary scene part for one step).
fn facing_pair(stepped: bool) -> (Sim, InstanceId, InstanceId) {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let mut sim = Sim::start(world);
    let a = sim.character();
    sim.ctl.set_body_pose(a, &pointing_pose());
    let b = if stepped {
        let b = second_character(&mut sim, "B", Vec3::new(0.75, 3.0, -40.0), 180.0);
        step_all(&mut sim, &[a, b], 2);
        metallic_physics::place_character(&mut sim.world, b, FACING, 180.0);
        b
    } else {
        second_character(&mut sim, "B", FACING, 180.0)
    };
    (sim, a, b)
}

fn second_character(sim: &mut Sim, name: &str, at: Vec3, yaw: f64) -> InstanceId {
    let players = sim.world.root("Players").unwrap();
    let p = sim.world.create("Player", Some(players)).unwrap();
    sim.world
        .set(p, "Name", Value::String(name.into()))
        .unwrap();
    let c = spawn_character(&mut sim.world, p).unwrap();
    metallic_physics::place_character(&mut sim.world, c, at, yaw);
    sim.phys.sync_from(&sim.world);
    c
}

/// Steps every character in `characters` once per frame, then the physics.
fn step_all(sim: &mut Sim, characters: &[InstanceId], frames: usize) {
    for _ in 0..frames {
        for &c in characters {
            sim.ctl
                .step(&mut sim.world, &sim.phys, c, MoveInput::default(), DT);
        }
        sim.events.extend(sim.ctl.drain_events());
        sim.phys.step(&mut sim.world, DT);
    }
}

/// Every (part, hit) pair of the Touched (or TouchEnded) events, as an unordered pair.
fn touch_pairs(sim: &Sim, start: bool) -> Vec<(InstanceId, InstanceId)> {
    sim.events
        .iter()
        .filter_map(|e| match (*e, start) {
            (CharacterEvent::Touched { part, hit, .. }, true)
            | (CharacterEvent::TouchEnded { part, hit, .. }, false) => {
                Some(if part.to_bits() < hit.to_bits() {
                    (part, hit)
                } else {
                    (hit, part)
                })
            }
            _ => None,
        })
        .collect()
}

fn in_model(world: &World, part: InstanceId, model: InstanceId) -> bool {
    world.parent(part) == Some(model)
}

fn name_of(world: &World, id: InstanceId) -> String {
    match world.get(id, "Name") {
        Ok(Value::String(s)) => s,
        _ => String::new(),
    }
}

#[test]
fn stepped_characters_touch_limb_to_limb_once_per_contact() {
    let (mut sim, a, b) = facing_pair(true);
    step_all(&mut sim, &[a, b], 30);
    let started = touch_pairs(&sim, true);
    let between: Vec<_> = started
        .iter()
        .filter(|&&(x, y)| {
            (in_model(&sim.world, x, a) && in_model(&sim.world, y, b))
                || (in_model(&sim.world, x, b) && in_model(&sim.world, y, a))
        })
        .copied()
        .collect();
    let names: Vec<String> = between
        .iter()
        .map(|&(x, y)| format!("{}-{}", name_of(&sim.world, x), name_of(&sim.world, y)))
        .collect();
    // The pointing hand meets B's limbs, not only B's root box.
    let hand = sim.world.find_first_child(a, "RightHand").unwrap();
    assert!(
        between.iter().any(|&(x, y)| {
            let other = if x == hand {
                y
            } else if y == hand {
                x
            } else {
                return false;
            };
            name_of(&sim.world, other) != "HumanoidRootPart"
        }),
        "{names:?}"
    );
    // Each contact is reported once, though both characters step.
    let mut unique = between.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), between.len(), "duplicates: {names:?}");
    assert_eq!(sim.ctl.touching_characters(a).len(), between.len());

    // B walks off (placed away): every contact ends once.
    metallic_physics::place_character(&mut sim.world, b, Vec3::new(30.0, 3.0, 0.0), 0.0);
    step_all(&mut sim, &[a, b], 5);
    let mut ended: Vec<_> = touch_pairs(&sim, false)
        .into_iter()
        .filter(|p| between.contains(p))
        .collect();
    ended.sort_unstable();
    assert_eq!(ended, unique, "{names:?}");
    assert!(sim.ctl.touching_characters(a).is_empty());
}

#[test]
fn a_dying_character_ends_its_limb_contacts() {
    let (mut sim, a, b) = facing_pair(true);
    step_all(&mut sim, &[a, b], 20);
    let touching = sim.ctl.touching_characters(b).len();
    assert!(touching > 0);
    let humanoid = sim.world.find_first_child(b, "Humanoid").unwrap();
    sim.world
        .set(humanoid, "Health", Value::Number(0.0))
        .unwrap();
    let before = sim.events.len();
    step_all(&mut sim, &[a, b], 10);
    let ends = sim.events[before..]
        .iter()
        .filter(|e| {
            matches!(e, CharacterEvent::TouchEnded { part, hit, .. }
            if in_model(&sim.world, *part, a) || in_model(&sim.world, *hit, a))
        })
        .count();
    assert_eq!(ends, touching);
    // Dead, it touches nobody until it respawns.
    let restarted = sim.events[before..]
        .iter()
        .filter(|e| {
            matches!(e, CharacterEvent::Touched { part, hit, .. }
            if in_model(&sim.world, *part, b) || in_model(&sim.world, *hit, b))
        })
        .count();
    assert_eq!(restarted, 0);
    assert!(sim.ctl.touching_characters(a).is_empty());
}

#[test]
fn a_destroyed_character_ends_its_limb_contacts() {
    let (mut sim, a, b) = facing_pair(true);
    step_all(&mut sim, &[a, b], 20);
    let touching = sim.ctl.touching_characters(a).len();
    assert!(touching > 0);
    sim.world.destroy(b);
    let before = sim.events.len();
    step_all(&mut sim, &[a], 2);
    let ends = sim.events[before..]
        .iter()
        .filter(|e| matches!(e, CharacterEvent::TouchEnded { character, .. } if *character == a))
        .count();
    assert_eq!(ends, touching);
}

#[test]
fn touches_between_characters_have_a_bounded_cost() {
    let mut world = World::new_with_services();
    baseplate(&mut world);
    let mut sim = Sim::start(world);
    let mut all = vec![sim.character()];
    // A crowd packed into 3 x 3 studs: every body overlaps every other.
    for k in 0..40 {
        let f = f64::from(k);
        let at = Vec3::new((f * 0.37).sin() * 1.5, 3.0, (f * 0.53).cos() * 1.5);
        all.push(second_character(&mut sim, &format!("P{k}"), at, f * 9.0));
    }
    step_all(&mut sim, &all, 3);
    let mut dropped = 0;
    for &c in &all {
        let cost = sim.ctl.touch_query_cost(c);
        assert!(
            cost.characters <= metallic_physics::MAX_CHARACTER_PAIRS,
            "{cost:?}"
        );
        assert!(cost.character_tests <= cost.characters * 256, "{cost:?}");
        dropped += cost.characters_dropped;
    }
    assert!(dropped > 0, "the crowd should reach the cap");
}

#[test]
fn an_unstepped_character_is_touched_like_scene_parts() {
    // B is never stepped (an NPC, or a remote character on a client): its parts are ordinary
    // scene parts, so A's hand still touches them.
    let (mut sim, a, b) = facing_pair(false);
    step_all(&mut sim, &[a], 20);
    let hand = sim.world.find_first_child(a, "RightHand").unwrap();
    assert!(
        sim.events
            .iter()
            .any(|e| matches!(e, CharacterEvent::Touched { part, hit, .. }
            if *hit == hand && in_model(&sim.world, *part, b))),
        "{:?}",
        sim.events
    );
    assert!(sim.ctl.touching_characters(a).is_empty());
}
