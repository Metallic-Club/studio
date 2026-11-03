//! Data model tests, on the class set shared with the integration tests and benches.

mod changes;
mod clone;
mod hooks;
mod pins;
mod props;
mod signals;
mod tree;

#[path = "../../../tests/support/classes.rs"]
mod classes;

use super::{DataModel, Notice};
use crate::{Id, Value};
use classes::Ids;

/// A new part with no parent.
fn part(dm: &mut DataModel, ids: &Ids) -> Id {
    dm.create(ids.part).expect("create part")
}

/// A new folder under `parent`.
fn folder_in(dm: &mut DataModel, ids: &Ids, parent: Id) -> Id {
    let folder = dm.create(ids.folder).expect("create folder");
    dm.set_parent(folder, Some(parent)).expect("parent folder");
    folder
}

/// A new part under `parent`, named `name`.
fn part_in(dm: &mut DataModel, ids: &Ids, parent: Id, name: &str) -> Id {
    let p = part(dm, ids);
    dm.set_name(p, name.into()).expect("name");
    dm.set_parent(p, Some(parent)).expect("parent part");
    p
}

/// The instance's children, in order.
fn kids(dm: &DataModel, id: Id) -> Vec<Id> {
    dm.children(id).expect("children").collect()
}

/// The notices recorded since the last call.
fn take_notices(dm: &mut DataModel) -> Vec<Notice> {
    std::mem::take(&mut dm.notices)
}

/// How many non-default values the instance stores.
fn stored_count(dm: &DataModel, id: Id) -> usize {
    dm.slots.body(id).map_or(0, |b| b.props.len())
}

fn number(n: f64) -> Value {
    Value::Number(n)
}
