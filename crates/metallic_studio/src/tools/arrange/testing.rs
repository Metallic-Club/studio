//! Test helpers: a world with a Workspace and quick part builders. Vectors are `DVec3`;
//! assertions accept property vectors and glam vectors alike.

use glam::DVec3;
use metallic_core::{InstanceId, Value, Vec3, World};

use super::math::{to_core, to_glam};
use super::plan::Plan;
use super::scene::{Object, read_objects};

pub fn v(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

pub use v as d;

/// Either vector type, for assertions.
pub trait AsDVec3: Copy + std::fmt::Debug {
    fn as_dvec3(self) -> DVec3;
}

impl AsDVec3 for DVec3 {
    fn as_dvec3(self) -> DVec3 {
        self
    }
}

impl AsDVec3 for Vec3 {
    fn as_dvec3(self) -> DVec3 {
        to_glam(self)
    }
}

#[track_caller]
pub fn assert_vec(a: impl AsDVec3, b: impl AsDVec3) {
    assert!(
        (a.as_dvec3() - b.as_dvec3()).length() < 1e-9,
        "{a:?} != {b:?}"
    );
}

pub use assert_vec as assert_dvec;

pub fn world() -> World {
    World::new_with_services()
}

fn workspace(w: &World) -> InstanceId {
    w.root("Workspace").expect("workspace")
}

pub fn part_under(
    w: &mut World,
    parent: InstanceId,
    pos: DVec3,
    size: DVec3,
    orientation: DVec3,
    shape: &str,
) -> InstanceId {
    let id = w.create("Part", Some(parent)).unwrap();
    w.set(id, "Position", Value::Vector3(to_core(pos))).unwrap();
    w.set(id, "Size", Value::Vector3(to_core(size))).unwrap();
    w.set(id, "Orientation", Value::Vector3(to_core(orientation)))
        .unwrap();
    w.set(id, "Shape", Value::String(shape.into())).unwrap();
    id
}

pub fn part(w: &mut World, pos: DVec3, size: DVec3, orientation: DVec3) -> InstanceId {
    let ws = workspace(w);
    part_under(w, ws, pos, size, orientation, "Block")
}

pub fn model(w: &mut World, parts: &[(DVec3, DVec3)]) -> (InstanceId, Vec<InstanceId>) {
    let ws = workspace(w);
    let m = w.create("Model", Some(ws)).unwrap();
    let kids = parts
        .iter()
        .map(|&(p, s)| part_under(w, m, p, s, DVec3::ZERO, "Block"))
        .collect();
    (m, kids)
}

#[track_caller]
pub fn pos_after(plan: &Plan, id: InstanceId) -> DVec3 {
    match plan.after(id, "Position") {
        Some(Value::Vector3(p)) => to_glam(*p),
        other => panic!("no planned Position for {id:?}: {other:?}"),
    }
}

/// The arrange objects for `ids`.
#[track_caller]
pub fn objs(w: &World, ids: &[InstanceId]) -> Vec<Object> {
    read_objects(w, ids).expect("read objects")
}
