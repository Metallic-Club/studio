//! Test helpers: a world with a Workspace and quick part builders.

use metallic_core::{InstanceId, Value, Vec3, World};

use super::plan::Plan;

pub fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

pub fn world() -> World {
    World::new_with_services()
}

fn workspace(w: &World) -> InstanceId {
    w.root("Workspace").expect("workspace")
}

pub fn part_under(
    w: &mut World,
    parent: InstanceId,
    pos: Vec3,
    size: Vec3,
    orientation: Vec3,
    shape: &str,
) -> InstanceId {
    let id = w.create("Part", Some(parent)).unwrap();
    w.set(id, "Position", Value::Vector3(pos)).unwrap();
    w.set(id, "Size", Value::Vector3(size)).unwrap();
    w.set(id, "Orientation", Value::Vector3(orientation))
        .unwrap();
    w.set(id, "Shape", Value::String(shape.into())).unwrap();
    id
}

pub fn part(w: &mut World, pos: Vec3, size: Vec3, orientation: Vec3) -> InstanceId {
    let ws = workspace(w);
    part_under(w, ws, pos, size, orientation, "Block")
}

pub fn model(w: &mut World, parts: &[(Vec3, Vec3)]) -> (InstanceId, Vec<InstanceId>) {
    let ws = workspace(w);
    let m = w.create("Model", Some(ws)).unwrap();
    let kids = parts
        .iter()
        .map(|&(p, s)| part_under(w, m, p, s, v(0.0, 0.0, 0.0), "Block"))
        .collect();
    (m, kids)
}

#[track_caller]
pub fn assert_vec(a: Vec3, b: Vec3) {
    assert!((a - b).magnitude() < 1e-9, "{a:?} != {b:?}");
}

#[track_caller]
pub fn pos_after(plan: &Plan, id: InstanceId) -> Vec3 {
    match plan.after(id, "Position") {
        Some(Value::Vector3(p)) => *p,
        other => panic!("no planned Position for {id:?}: {other:?}"),
    }
}
