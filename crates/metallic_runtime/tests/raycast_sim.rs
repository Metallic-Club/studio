//! `workspace:Raycast` through the CPU-only `Simulation` (no presentation feature needed).
use metallic_core::{Value, Vec3, World};
use metallic_runtime::{Simulation, TICK};
use metallic_script::OutputSink;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
struct Captured(RefCell<Vec<String>>);

impl OutputSink for Captured {
    fn print(&self, line: &str) {
        self.0.borrow_mut().push(line.to_owned());
    }
    fn warn(&self, line: &str) {
        self.0.borrow_mut().push(format!("warning: {line}"));
    }
    fn error(&self, line: &str) {
        self.0.borrow_mut().push(format!("error: {line}"));
    }
}

fn anchored(world: &mut World, name: &str, pos: Vec3, size: Vec3, anchored: bool) {
    let workspace = world.root("Workspace").unwrap();
    let id = world.create("Part", Some(workspace)).unwrap();
    world.set(id, "Name", Value::String(name.into())).unwrap();
    world.set(id, "Position", Value::Vector3(pos)).unwrap();
    world.set(id, "Size", Value::Vector3(size)).unwrap();
    world.set(id, "Anchored", Value::Bool(anchored)).unwrap();
}

/// Runs `source` as a server script over `world` for `ticks` ticks and returns its output.
fn run(mut world: World, source: &str, ticks: usize) -> Vec<String> {
    let script = world
        .create("Script", world.root("ServerScriptService"))
        .unwrap();
    world
        .set(script, "Source", Value::String(source.into()))
        .unwrap();
    let output = Rc::new(Captured::default());
    let mut sim = Simulation::from_world("raycast-sim".to_owned(), world, output.clone());
    sim.start().unwrap();
    for _ in 0..ticks {
        sim.tick_once(&[]);
    }
    let lines = output.0.borrow().clone();
    lines
}

/// A server-only `Simulation` has the same physics behind `workspace:Raycast`.
#[test]
fn a_server_simulation_answers_raycasts_too() {
    let mut world = World::new_with_services();
    anchored(
        &mut world,
        "Wall",
        Vec3::new(0.0, 10.0, -20.0),
        Vec3::new(10.0, 10.0, 2.0),
        true,
    );
    let lines = run(
        world,
        "local r = workspace:Raycast(Vector3.new(0, 10, 0), Vector3.new(0, 0, -100))
         print(r and r.Instance.Name, r and r.Distance)",
        3,
    );
    assert_eq!(
        lines.first().map(String::as_str),
        Some("Wall 19"),
        "{lines:?}"
    );
}

/// A Raycast inside a `Touched` handler works (the physics is not borrowed while handlers run).
#[test]
fn a_raycast_inside_a_touched_handler_hits() {
    let mut world = World::new_with_services();
    anchored(
        &mut world,
        "Floor",
        Vec3::new(0.0, -2.0, 0.0),
        Vec3::new(64.0, 4.0, 64.0),
        true,
    );
    anchored(
        &mut world,
        "Ball",
        Vec3::new(0.0, 1.5, 0.0),
        Vec3::new(2.0, 2.0, 2.0),
        false,
    );
    let lines = run(
        world,
        r#"
        local done = false
        workspace.Floor.Touched:Connect(function(hit)
            if done then return end
            done = true
            local r = workspace:Raycast(Vector3.new(10, 10, 0), Vector3.new(0, -50, 0))
            print("touched", r and r.Instance.Name)
        end)
        "#,
        120,
    );
    assert!(lines.iter().any(|l| l == "touched Floor"), "{lines:?}");
}

/// Parts a script just made or resized are hit at once, before the next physics step.
#[test]
fn parts_created_or_resized_this_tick_are_hit_at_once() {
    let lines = run(
        World::new_with_services(),
        r#"
        local p = Instance.new("Part")
        p.Anchored = true
        p.Size = Vector3.new(4, 4, 4)
        p.Position = Vector3.new(0, 0, -10)
        p.Parent = workspace
        local r = workspace:Raycast(Vector3.new(0, 0, 0), Vector3.new(0, 0, -50))
        print("created", r and r.Instance == p, r and r.Distance)
        p.Size = Vector3.new(4, 4, 10)
        local r2 = workspace:Raycast(Vector3.new(0, 0, 0), Vector3.new(0, 0, -50))
        print("resized", r2 and r2.Distance)
        "#,
        3,
    );
    assert_eq!(
        lines.first().map(String::as_str),
        Some("created true 8"),
        "{lines:?}"
    );
    assert_eq!(
        lines.get(1).map(String::as_str),
        Some("resized 5"),
        "{lines:?}"
    );
}

/// A ray that starts inside many overlapping parts still reaches the next real hit.
#[test]
fn a_ray_inside_many_overlapping_parts_reaches_the_next_hit() {
    let mut world = World::new_with_services();
    for i in 0..40 {
        anchored(
            &mut world,
            &format!("Blob{i}"),
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(4.0, 4.0, 4.0),
            true,
        );
    }
    anchored(
        &mut world,
        "Wall",
        Vec3::new(0.0, 0.0, -20.0),
        Vec3::new(10.0, 10.0, 2.0),
        true,
    );
    let lines = run(
        world,
        "local r = workspace:Raycast(Vector3.new(0, 0, 0), Vector3.new(0, 0, -100))
         print(r and r.Instance.Name)",
        3,
    );
    assert_eq!(lines.first().map(String::as_str), Some("Wall"), "{lines:?}");
}

#[allow(dead_code)]
const _: f64 = TICK;
