//! `workspace:Raycast` plumbing without a physics: a recording `SceneQuery`.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::Rig;
use metallic_core::{Value, Vec3};
use metallic_script::{SceneHit, SceneQuery, SceneRay};

#[derive(Default)]
struct Recorder {
    rays: RefCell<Vec<SceneRay>>,
    answer: RefCell<Option<SceneHit>>,
}

impl SceneQuery for Recorder {
    fn raycast(&self, ray: &SceneRay) -> Option<SceneHit> {
        self.rays.borrow_mut().push(ray.clone());
        *self.answer.borrow()
    }
}

fn run(rig: &mut Rig, source: &str) -> Vec<String> {
    rig.server_script("Main", source);
    rig.host.start().unwrap();
    rig.run(0.0, 3);
    rig.lines()
}

#[test]
fn without_a_scene_query_raycast_finds_nothing() {
    let mut rig = Rig::new();
    let lines = run(
        &mut rig,
        "print(workspace:Raycast(Vector3.new(0, 0, 0), Vector3.new(0, -10, 0)))",
    );
    assert_eq!(lines, ["print:nil"], "{lines:?}");
}

#[test]
fn the_ray_reaches_the_query_with_the_expanded_filter() {
    let mut rig = Rig::new();
    let ws = rig.root("Workspace");
    let model = rig.add("Model", "M", ws);
    let a = rig.add("Part", "A", model);
    let b = rig.add("Part", "B", model);
    let c = rig.add("Part", "C", ws);
    let query = Rc::new(Recorder::default());
    rig.host.set_scene_query(query.clone());
    run(
        &mut rig,
        "local p = RaycastParams.new()
         p.FilterType = Enum.RaycastFilterType.Include
         p.RespectCanCollide = true
         p.FilterDescendantsInstances = { workspace.M, workspace.C }
         workspace:Raycast(Vector3.new(1, 2, 3), Vector3.new(0, -50, 0), p)
         workspace:Raycast(Vector3.new(0, 0, 0), Vector3.new(0, 0, -20000))",
    );
    let rays = query.rays.borrow();
    assert_eq!(rays.len(), 2);
    let first = &rays[0];
    assert!(first.include && first.respect_can_collide);
    assert_eq!(first.origin, Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(first.direction, Vec3::new(0.0, -50.0, 0.0));
    let mut parts = first.parts.clone();
    parts.sort();
    let mut want = vec![model, a, b, c];
    want.sort();
    assert_eq!(parts, want);
    // Defaults: exclude nothing, do not respect CanCollide; a long ray is shortened to 15000.
    let second = &rays[1];
    assert!(!second.include && !second.respect_can_collide && second.parts.is_empty());
    assert!((second.direction.magnitude() - 15_000.0).abs() < 1e-6);
}

#[test]
fn the_answer_becomes_a_raycast_result() {
    let mut rig = Rig::new();
    let ws = rig.root("Workspace");
    let part = rig.add("Part", "Target", ws);
    rig.world
        .borrow_mut()
        .set(part, "Material", Value::String("Metal".into()))
        .unwrap();
    let query = Rc::new(Recorder::default());
    *query.answer.borrow_mut() = Some(SceneHit {
        instance: part,
        position: Vec3::new(1.0, 2.0, 3.0),
        normal: Vec3::new(0.0, 1.0, 0.0),
        distance: 7.5,
    });
    rig.host.set_scene_query(query);
    let lines = run(
        &mut rig,
        "local r = workspace:Raycast(Vector3.new(0, 0, 0), Vector3.new(0, 0, -10))
         print(r.Instance == workspace.Target, r.Position, r.Normal, r.Distance, r.Material)
         print(pcall(function() r.Distance = 1 end))",
    );
    assert_eq!(
        lines[0], "print:true 1, 2, 3 0, 1, 0 7.5 Enum.Material.Metal",
        "{lines:?}"
    );
    assert!(lines[1].starts_with("print:false"), "{lines:?}");
}
