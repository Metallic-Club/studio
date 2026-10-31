//! `workspace:Raycast` through a running `Game`: real physics, real scripts.

use metallic_core::{InstanceId, Value, Vec3, World};
use metallic_runtime::{FrameInput, Game, TICK};
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

fn part(world: &mut World, parent: InstanceId, name: &str, pos: Vec3, size: Vec3) -> InstanceId {
    let id = world.create("Part", Some(parent)).unwrap();
    for (property, value) in [
        ("Name", Value::String(name.to_owned())),
        ("Position", Value::Vector3(pos)),
        ("Size", Value::Vector3(size)),
        ("Anchored", Value::Bool(true)),
    ] {
        world.set(id, property, value).unwrap();
    }
    id
}

/// Runs `source` as a server script over a small scene and returns what it printed.
fn run(source: &str) -> Vec<String> {
    let mut world = World::new_with_services();
    let workspace = world.root("Workspace").unwrap();
    // A wall 19 studs ahead, a second one behind it, a part without collision in front.
    part(
        &mut world,
        workspace,
        "Wall",
        Vec3::new(0.0, 10.0, -20.0),
        Vec3::new(10.0, 10.0, 2.0),
    );
    part(
        &mut world,
        workspace,
        "Far",
        Vec3::new(0.0, 10.0, -40.0),
        Vec3::new(10.0, 10.0, 2.0),
    );
    let ghost = part(
        &mut world,
        workspace,
        "Ghost",
        Vec3::new(0.0, 10.0, -10.0),
        Vec3::new(4.0, 4.0, 1.0),
    );
    world.set(ghost, "CanCollide", Value::Bool(false)).unwrap();
    let group = world.create("Model", Some(workspace)).unwrap();
    world
        .set(group, "Name", Value::String("Group".into()))
        .unwrap();
    part(
        &mut world,
        group,
        "Side",
        Vec3::new(30.0, 10.0, -20.0),
        Vec3::new(4.0, 4.0, 4.0),
    );
    world
        .set(
            world.find_first_child(group, "Side").unwrap(),
            "Material",
            Value::String("Glass".into()),
        )
        .unwrap();
    let script = world
        .create("Script", world.root("ServerScriptService"))
        .unwrap();
    world
        .set(script, "Source", Value::String(source.to_owned()))
        .unwrap();
    let output = Rc::new(Captured::default());
    let mut game = Game::from_world("raycast-api".to_owned(), world, output.clone());
    game.start().unwrap();
    for _ in 0..3 {
        assert_eq!(game.frame(&FrameInput::default(), TICK).ticks, 1);
    }
    output.0.borrow().clone()
}

const HEAD: &str = "
local ORIGIN = Vector3.new(0, 10, 0)
local FORWARD = Vector3.new(0, 0, -100)
local function near(a, b) return math.abs(a - b) < 0.01 end
";

#[test]
fn the_nearest_part_comes_back_with_instance_position_normal_material_and_distance() {
    let lines = run(&format!(
        "{HEAD}
         local params = RaycastParams.new()
         params.RespectCanCollide = true
         local r = workspace:Raycast(ORIGIN, FORWARD, params)
         print(r.Instance == workspace.Wall, near(r.Distance, 19), near(r.Position.Z, -19))
         print(near(r.Normal.Z, 1) and near(r.Normal.Magnitude, 1), r.Material == Enum.Material.Plastic)
         print(tostring(r), typeof(r))"
    ));
    assert_eq!(
        lines,
        ["true true true", "true true", "RaycastResult RaycastResult"],
        "{lines:?}"
    );
}

#[test]
fn parts_without_collision_are_hit_unless_the_params_respect_can_collide() {
    let lines = run(&format!(
        "{HEAD}
         print(workspace:Raycast(ORIGIN, FORWARD).Instance.Name)
         local p = RaycastParams.new()
         p.RespectCanCollide = true
         print(workspace:Raycast(ORIGIN, FORWARD, p).Instance.Name)"
    ));
    assert_eq!(lines, ["Ghost", "Wall"], "{lines:?}");
}

#[test]
fn exclude_skips_the_listed_instances_and_their_descendants() {
    let lines = run(&format!(
        "{HEAD}
         local p = RaycastParams.new()
         p.RespectCanCollide = true
         p.FilterType = Enum.RaycastFilterType.Exclude
         p.FilterDescendantsInstances = {{ workspace.Wall }}
         print(workspace:Raycast(ORIGIN, FORWARD, p).Instance.Name)
         p:AddToFilter(workspace.Far)
         print(workspace:Raycast(ORIGIN, FORWARD, p))
         -- A model excludes the parts inside it.
         local side = RaycastParams.new()
         side.FilterDescendantsInstances = {{ workspace.Group }}
         local o = Vector3.new(30, 10, 0)
         print(workspace:Raycast(o, Vector3.new(0, 0, -100), side))
         print(workspace:Raycast(o, Vector3.new(0, 0, -100)).Instance.Name)"
    ));
    assert_eq!(lines, ["Far", "nil", "nil", "Side"], "{lines:?}");
}

#[test]
fn include_hits_only_the_listed_instances_and_their_descendants() {
    let lines = run(&format!(
        "{HEAD}
         local p = RaycastParams.new()
         p.FilterType = Enum.RaycastFilterType.Include
         print(workspace:Raycast(ORIGIN, FORWARD, p))   -- nothing listed: nothing to hit
         p.FilterDescendantsInstances = {{ workspace.Far }}
         print(workspace:Raycast(ORIGIN, FORWARD, p).Instance.Name)
         p.FilterDescendantsInstances = {{ workspace.Group }}
         print(workspace:Raycast(Vector3.new(30, 10, 0), FORWARD, p).Instance.Name)
         print(workspace:Raycast(ORIGIN, FORWARD, p))"
    ));
    assert_eq!(lines, ["nil", "Far", "Side", "nil"], "{lines:?}");
}

#[test]
fn the_direction_length_is_the_maximum_distance_and_misses_are_nil() {
    let lines = run(&format!(
        "{HEAD}
         local p = RaycastParams.new()
         p.RespectCanCollide = true
         print(workspace:Raycast(ORIGIN, Vector3.new(0, 0, -18), p))
         print(workspace:Raycast(ORIGIN, Vector3.new(0, 0, -20), p) ~= nil)
         print(workspace:Raycast(ORIGIN, Vector3.new(0, 0, 0), p))
         print(workspace:Raycast(ORIGIN, Vector3.new(0, 100, 0), p))
         print(workspace:Raycast(ORIGIN, Vector3.new(0, 0, 1e9), p))"
    ));
    assert_eq!(lines, ["nil", "true", "nil", "nil", "nil"], "{lines:?}");
}

#[test]
fn a_ray_that_starts_inside_a_part_goes_on_to_the_next_with_a_unit_normal() {
    let lines = run(&format!(
        "{HEAD}
         local p = RaycastParams.new()
         p.RespectCanCollide = true
         local r = workspace:Raycast(Vector3.new(0, 10, -20), FORWARD, p)
         print(r.Instance.Name, near(r.Distance, 19), near(r.Normal.Magnitude, 1), near(r.Normal.Z, 1))
         -- slanted rays: the normal stays unit length
         for _, d in {{ Vector3.new(3, 1, -50), Vector3.new(-4, -2, -50), Vector3.new(0.1, 0, -50) }} do
             local h = workspace:Raycast(Vector3.new(0, 10, -1), d, p)
             assert(h and near(h.Normal.Magnitude, 1))
         end
         print('unit')"
    ));
    assert_eq!(lines, ["Far true true true", "unit"], "{lines:?}");
}

#[test]
fn hit_material_works_as_an_enum_and_can_be_assigned_to_a_part() {
    let lines = run(&format!(
        "{HEAD}
         local r = workspace:Raycast(Vector3.new(30, 10, 0), FORWARD)
         print(r.Material == Enum.Material.Glass, r.Material.Name)
         workspace.Wall.Material = r.Material
         print(workspace.Wall.Material)"
    ));
    assert_eq!(lines, ["true Glass", "Glass"], "{lines:?}");
}

#[test]
fn raycast_params_behave_like_roblox() {
    let lines = run(
        "local p = RaycastParams.new()
         print(p.FilterType == Enum.RaycastFilterType.Exclude, p.IgnoreWater, #p.FilterDescendantsInstances)
         p.IgnoreWater = true
         p.CollisionGroup = 'Enemies'
         p:AddToFilter({ workspace.Wall, workspace.Far })
         p:AddToFilter(workspace.Group)
         print(p.IgnoreWater, p.CollisionGroup, #p.FilterDescendantsInstances)
         p.FilterDescendantsInstances = nil
         print(#p.FilterDescendantsInstances, tostring(p), typeof(p))
         print(pcall(function() p.FilterType = 5 end))
         print(pcall(function() p.FilterDescendantsInstances = { 1, 2 } end))
         print(pcall(function() p:AddToFilter('x') end))",
    );
    assert_eq!(lines[0], "true false 0", "{lines:?}");
    assert_eq!(lines[1], "true Enemies 3", "{lines:?}");
    assert_eq!(lines[2], "0 RaycastParams RaycastParams", "{lines:?}");
    for l in &lines[3..] {
        assert!(l.starts_with("false"), "{lines:?}");
    }
}

#[test]
fn bad_arguments_raise_clear_errors() {
    let lines = run(
        "local function try(...) local ok, e = pcall(workspace.Raycast, workspace, ...) print(ok, e) end
         try(1, Vector3.new(0, 0, -1))
         try(Vector3.new(0, 0, 0), 'x')
         try(Vector3.new(0, 0, 0), Vector3.new(0, 0, -1), 5)
         try(Vector3.new(0/0, 0, 0), Vector3.new(0, 0, -1))
         try(Vector3.new(0, 0, 0), Vector3.new(math.huge, 0, 0))
         print(pcall(function() return workspace.Wall:Raycast(Vector3.new(0, 0, 0), Vector3.new(0, 0, -1)) end))",
    );
    assert_eq!(lines.len(), 6, "{lines:?}");
    assert!(lines[0].contains("Vector3 expected"), "{lines:?}");
    assert!(lines[1].contains("Vector3 expected"));
    assert!(lines[2].contains("RaycastParams expected"));
    assert!(lines[3].contains("finite"));
    assert!(lines[4].contains("finite"));
    assert!(
        lines[5].starts_with("false"),
        "Raycast is a Workspace method"
    );
}

#[test]
fn duplicate_filter_entries_are_fine() {
    let lines = run(
        "local m = Instance.new('Model', workspace)
         for i = 1, 3 do Instance.new('Part', m) end
         local p = RaycastParams.new()
         p.FilterDescendantsInstances = { m, m, workspace.Wall }
         print(workspace:Raycast(Vector3.new(0, 10, 0), Vector3.new(0, 0, -100), p).Instance.Name)",
    );
    // Duplicates in the filter are fine; the nearest unfiltered part is the ghost.
    assert_eq!(lines, ["Ghost"], "{lines:?}");
}
