//! Creation rules, re-parenting, searches, names and destruction.

use super::classes::{game, registry_with};
use super::*;
use crate::ClassId;
use crate::dm::DmError;

#[test]
fn root_and_service_rules() {
    let registry = registry_with(&[]);
    let class = |name: &str| registry.class_id(name).expect("class");
    let mut dm = DataModel::new(registry.clone());
    assert!(dm.is_empty());
    assert_eq!(dm.create_service(class("Workspace")), Err(DmError::NoRoot));
    assert_eq!(
        dm.create_root(class("Workspace")),
        Err(DmError::IsService { class: "Workspace" })
    );
    let root = dm.create_root(class("Game")).expect("root");
    assert_eq!(dm.create_root(class("Game")), Err(DmError::RootExists));
    assert_eq!(dm.root(), Some(root));
    assert_eq!(dm.parent(root), Ok(None));

    let ws = dm.create_service(class("Workspace")).expect("service");
    assert_eq!(dm.parent(ws), Ok(Some(root)));
    assert_eq!(dm.service(class("Workspace")), Some(ws));
    assert_eq!(dm.service(class("Lighting")), None);
    assert_eq!(
        dm.create_service(class("Workspace")),
        Err(DmError::DuplicateService { class: "Workspace" })
    );
    assert_eq!(
        dm.create_service(class("Folder")),
        Err(DmError::NotAService { class: "Folder" })
    );
    assert_eq!(dm.len(), 2);
}

#[test]
fn create_refuses_services_and_non_creatable_classes() {
    let (mut dm, ids) = game();
    assert_eq!(
        dm.create(ids.workspace_class),
        Err(DmError::IsService { class: "Workspace" })
    );
    assert_eq!(dm.create(ids.game_class), Err(DmError::NotCreatable { class: "Game" }));
    assert_eq!(
        dm.create(ids.instance),
        Err(DmError::NotCreatable { class: "Instance" })
    );
    assert_eq!(dm.create(ClassId::new(999)), Err(DmError::UnknownClass(999)));
    let p = dm.create(ids.part).expect("part");
    assert_eq!(dm.parent(p), Ok(None));
    assert_eq!(dm.is_a(p, ids.instance), Ok(true));
    assert_eq!(dm.is_a(p, ids.folder), Ok(false));
}

#[test]
fn set_parent_refuses_cycles_root_services_and_stale_ids() {
    let (mut dm, ids) = game();
    let a = folder_in(&mut dm, &ids, ids.workspace);
    let b = folder_in(&mut dm, &ids, a);
    let c = folder_in(&mut dm, &ids, b);
    let err = dm.set_parent(a, Some(c)).expect_err("cycle");
    assert_eq!(
        err.to_string(),
        "cannot set the parent of Folder: the new parent is Folder itself or one of its descendants"
    );
    assert!(matches!(
        dm.set_parent(a, Some(a)),
        Err(DmError::WouldCreateCycle { .. })
    ));
    assert_eq!(dm.set_parent(ids.game, Some(a)), Err(DmError::IsRoot { class: "Game" }));
    assert_eq!(
        dm.set_parent(ids.workspace, Some(a)),
        Err(DmError::IsService { class: "Workspace" })
    );
    assert_eq!(
        dm.set_parent(ids.lighting, None),
        Err(DmError::IsService { class: "Lighting" })
    );
    let gone = dm.create(ids.folder).expect("folder");
    dm.destroy(gone).expect("destroy");
    assert_eq!(dm.set_parent(a, Some(gone)), Err(DmError::NoInstance(gone)));
    assert_eq!(dm.set_parent(gone, Some(a)), Err(DmError::NoInstance(gone)));
    // Nothing moved.
    assert_eq!(dm.parent(a), Ok(Some(ids.workspace)));
    assert!(kids(&dm, c).is_empty());
}

#[test]
fn set_parent_updates_both_sides_and_keeps_child_order() {
    let (mut dm, ids) = game();
    let ws = ids.workspace;
    let a = part_in(&mut dm, &ids, ws, "A");
    let b = part_in(&mut dm, &ids, ws, "B");
    let c = part_in(&mut dm, &ids, ws, "C");
    assert_eq!(kids(&dm, ws), [a, b, c]);
    let folder = folder_in(&mut dm, &ids, ws);
    dm.set_parent(b, Some(folder)).expect("move");
    assert_eq!(kids(&dm, ws), [a, c, folder]);
    assert_eq!(kids(&dm, folder), [b]);
    assert_eq!(dm.parent(b), Ok(Some(folder)));
    dm.set_parent(b, Some(ws)).expect("move back");
    assert_eq!(kids(&dm, ws), [a, c, folder, b]);
    // Sibling positions were renumbered: the walk visits every child once, in order.
    let walked: Vec<Id> = dm.descendants(ws).expect("walk").collect();
    assert_eq!(walked, vec![a, c, folder, b]);
    dm.set_parent(a, None).expect("unparent");
    assert_eq!(dm.parent(a), Ok(None));
    assert_eq!(kids(&dm, ws), [c, folder, b]);

    take_notices(&mut dm);
    dm.set_parent(c, Some(ws)).expect("same parent");
    assert!(take_notices(&mut dm).is_empty(), "moving to the same parent is a no-op");
    dm.set_parent(c, Some(folder)).expect("move");
    assert_eq!(
        take_notices(&mut dm),
        vec![Notice::ParentChanged {
            id: c,
            old: Some(ws),
            new: Some(folder)
        }]
    );
}

#[test]
fn descendants_walk_depth_first_in_order() {
    let (mut dm, ids) = game();
    let ws = ids.workspace;
    let model = folder_in(&mut dm, &ids, ws);
    let p1 = part_in(&mut dm, &ids, model, "P1");
    let inner = folder_in(&mut dm, &ids, model);
    let p2 = part_in(&mut dm, &ids, inner, "P2");
    let p3 = part_in(&mut dm, &ids, model, "P3");
    let p4 = part_in(&mut dm, &ids, ws, "P4");
    let walked: Vec<Id> = dm.descendants(ws).expect("walk").collect();
    assert_eq!(walked, vec![model, p1, inner, p2, p3, p4]);
    let walked: Vec<Id> = dm.descendants(model).expect("walk").collect();
    assert_eq!(walked, vec![p1, inner, p2, p3]);
    assert_eq!(dm.descendants(p4).expect("leaf").count(), 0);
    let all: Vec<Id> = dm.descendants(ids.game).expect("walk").collect();
    assert_eq!(all.len(), 8);
}

#[test]
fn searches_names_and_full_names() {
    let (mut dm, ids) = game();
    let ws = ids.workspace;
    let model = folder_in(&mut dm, &ids, ws);
    dm.set_name(model, "Model".into()).expect("name");
    let door = part_in(&mut dm, &ids, model, "Door");
    let knob = part_in(&mut dm, &ids, door, "Knob");

    assert_eq!(dm.find_first_child(ws, "Door", false), Ok(None));
    assert_eq!(dm.find_first_child(ws, "Door", true), Ok(Some(door)));
    assert_eq!(dm.find_first_child(model, "Door", false), Ok(Some(door)));
    assert_eq!(dm.find_first_child_of_class(ws, ids.part, false), Ok(None));
    assert_eq!(dm.find_first_child_of_class(ws, ids.part, true), Ok(Some(door)));
    assert_eq!(dm.find_first_child_of_class(ws, ids.instance, true), Ok(None));
    assert_eq!(dm.find_first_child_which_is_a(ws, ids.instance, false), Ok(Some(model)));
    assert_eq!(dm.find_first_child_which_is_a(door, ids.part, false), Ok(Some(knob)));

    assert_eq!(dm.is_descendant_of(knob, ws), Ok(true));
    assert_eq!(dm.is_descendant_of(knob, ids.game), Ok(true));
    assert_eq!(dm.is_descendant_of(ws, knob), Ok(false));
    assert_eq!(dm.is_descendant_of(knob, knob), Ok(false));

    assert_eq!(dm.full_name(knob).as_deref(), Ok("Workspace.Model.Door.Knob"));
    assert_eq!(dm.full_name(ws).as_deref(), Ok("Workspace"));
    assert_eq!(dm.full_name(ids.game).as_deref(), Ok("Game"));
    dm.set_parent(door, None).expect("detach");
    assert_eq!(dm.full_name(knob).as_deref(), Ok("Door.Knob"));

    take_notices(&mut dm);
    dm.set_name(door, "Door".into()).expect("same name");
    assert!(take_notices(&mut dm).is_empty());
    dm.set_name(door, "Gate".into()).expect("rename");
    assert_eq!(dm.shared_name(door).as_deref(), Ok("Gate"));
    assert_eq!(take_notices(&mut dm), vec![Notice::NameChanged { id: door }]);
    assert!(dm.slots.body(door).is_some_and(|b| b.name.is_some()));
    dm.set_name(door, "Part".into()).expect("back to the class name");
    assert_eq!(dm.name(door), Ok("Part"));
    assert!(
        dm.slots.body(door).is_some_and(|b| b.name.is_none()),
        "class name is not stored"
    );
}

#[test]
fn destroy_removes_the_subtree_and_stales_its_ids() {
    let (mut dm, ids) = game();
    let ws = ids.workspace;
    let keep = part_in(&mut dm, &ids, ws, "Keep");
    let model = folder_in(&mut dm, &ids, ws);
    let a = part_in(&mut dm, &ids, model, "A");
    let b = part_in(&mut dm, &ids, a, "B");
    dm.set(keep, ids.target, Value::Ref(Some(b))).expect("ref");
    let before = dm.len();
    take_notices(&mut dm);

    dm.destroy(model).expect("destroy");
    assert_eq!(dm.len(), before - 3);
    for id in [model, a, b] {
        assert!(!dm.contains(id));
        assert_eq!(dm.parent(id), Err(DmError::NoInstance(id)));
    }
    assert_eq!(kids(&dm, ws), [keep]);
    assert_eq!(dm.reference(keep, ids.target), Ok(None));
    assert_eq!(
        take_notices(&mut dm),
        vec![
            Notice::Destroying { id: model },
            Notice::Destroying { id: a },
            Notice::Destroying { id: b },
            Notice::ParentChanged {
                id: model,
                old: Some(ws),
                new: None
            },
        ]
    );
    assert_eq!(dm.destroy(model), Err(DmError::NoInstance(model)));

    // A reused slot gets a new id; the old one stays stale.
    let fresh = part(&mut dm, &ids);
    assert!([model, a, b].iter().any(|old| old.index() == fresh.index()));
    assert!(![model, a, b].contains(&fresh));
}

#[test]
fn destroy_refuses_root_and_services() {
    let (mut dm, ids) = game();
    assert_eq!(dm.destroy(ids.game), Err(DmError::IsRoot { class: "Game" }));
    assert_eq!(
        dm.destroy(ids.workspace),
        Err(DmError::IsService { class: "Workspace" })
    );
}

#[test]
fn destroy_children_clears_everything_but_services() {
    let (mut dm, ids) = game();
    let ws = ids.workspace;
    let model = folder_in(&mut dm, &ids, ws);
    let parts: Vec<Id> = (0..5)
        .map(|i| part_in(&mut dm, &ids, model, &format!("P{i}")))
        .collect();
    let loose = folder_in(&mut dm, &ids, ids.game);
    dm.destroy_children(model).expect("clear");
    assert!(kids(&dm, model).is_empty());
    assert!(parts.iter().all(|p| !dm.contains(*p)));
    dm.destroy_children(ids.game).expect("clear root");
    assert!(!dm.contains(loose));
    assert!(dm.contains(model), "inside a kept service");
    assert_eq!(kids(&dm, ids.game), [ids.workspace, ids.lighting]);
}

/// The children of `parent` as the iterator gives them, checked against the other ways of reading the list: the
/// count, the reverse walk and each child's parent.
fn checked_kids(dm: &DataModel, parent: Id) -> Vec<Id> {
    let forward = kids(dm, parent);
    let mut backward: Vec<Id> = dm.children(parent).expect("children").rev().collect();
    backward.reverse();
    assert_eq!(forward, backward, "both ends agree");
    assert_eq!(dm.child_count(parent), Ok(forward.len()));
    assert_eq!(dm.children(parent).expect("children").len(), forward.len());
    for &c in &forward {
        assert_eq!(dm.parent(c), Ok(Some(parent)));
    }
    forward
}

#[test]
fn removing_children_keeps_order_at_the_middle_front_and_back() {
    let (mut dm, ids) = game();
    let folder = folder_in(&mut dm, &ids, ids.workspace);
    let p: Vec<Id> = (0..6)
        .map(|i| part_in(&mut dm, &ids, folder, &format!("P{i}")))
        .collect();
    assert_eq!(checked_kids(&dm, folder), p);

    dm.set_parent(p[2], None).expect("middle");
    assert_eq!(checked_kids(&dm, folder), [p[0], p[1], p[3], p[4], p[5]]);
    dm.destroy(p[0]).expect("front");
    assert_eq!(checked_kids(&dm, folder), [p[1], p[3], p[4], p[5]]);
    dm.destroy(p[5]).expect("back");
    assert_eq!(checked_kids(&dm, folder), [p[1], p[3], p[4]]);
    dm.set_parent(p[2], Some(folder)).expect("back in, at the end");
    assert_eq!(checked_kids(&dm, folder), [p[1], p[3], p[4], p[2]]);
    let walked: Vec<Id> = dm.descendants(folder).expect("walk").collect();
    assert_eq!(walked, [p[1], p[3], p[4], p[2]]);

    // Iterating from both ends meets in the middle exactly once.
    let mut both = dm.children(folder).expect("children");
    assert_eq!(both.next(), Some(p[1]));
    assert_eq!(both.next_back(), Some(p[2]));
    assert_eq!(both.len(), 2);
    assert_eq!(both.next_back(), Some(p[4]));
    assert_eq!(both.next(), Some(p[3]));
    assert!(both.is_empty());
    assert_eq!(both.next(), None);
    assert_eq!(both.next_back(), None);
}

#[test]
fn reparenting_moves_whole_subtrees_between_lists() {
    let (mut dm, ids) = game();
    let ws = ids.workspace;
    let a = folder_in(&mut dm, &ids, ws);
    let b = folder_in(&mut dm, &ids, ws);
    let x = part_in(&mut dm, &ids, a, "X");
    let y = part_in(&mut dm, &ids, a, "Y");
    let z = part_in(&mut dm, &ids, a, "Z");
    let inner = part_in(&mut dm, &ids, y, "Inner");

    dm.set_parent(y, Some(b)).expect("move");
    assert_eq!(checked_kids(&dm, a), [x, z]);
    assert_eq!(checked_kids(&dm, b), [y]);
    assert_eq!(checked_kids(&dm, y), [inner]);
    dm.set_parent(x, Some(b)).expect("move");
    dm.set_parent(z, Some(b)).expect("move");
    assert!(checked_kids(&dm, a).is_empty());
    assert_eq!(checked_kids(&dm, b), [y, x, z]);
    // An emptied list fills again from the start.
    dm.set_parent(z, Some(a)).expect("move back");
    assert_eq!(checked_kids(&dm, a), [z]);
    let walked: Vec<Id> = dm.descendants(ws).expect("walk").collect();
    assert_eq!(walked, [a, z, b, y, inner, x]);
}

#[test]
fn destroying_under_a_large_parent_keeps_order() {
    const N: usize = 2_000;
    let (mut dm, ids) = game();
    let folder = folder_in(&mut dm, &ids, ids.workspace);
    let parts: Vec<Id> = (0..N)
        .map(|_| {
            let p = part(&mut dm, &ids);
            dm.set_parent(p, Some(folder)).expect("parent");
            p
        })
        .collect();
    // Every other part from the front, then every third remaining one from the back.
    for &p in parts.iter().step_by(2) {
        dm.destroy(p).expect("destroy");
    }
    let mut left: Vec<Id> = parts.iter().copied().skip(1).step_by(2).collect();
    assert_eq!(checked_kids(&dm, folder), left);
    let doomed: Vec<Id> = left.iter().rev().copied().step_by(3).collect();
    for &p in &doomed {
        dm.destroy(p).expect("destroy");
    }
    left.retain(|p| !doomed.contains(p));
    assert_eq!(checked_kids(&dm, folder), left);
    assert_eq!(dm.descendants(folder).expect("walk").collect::<Vec<_>>(), left);
    dm.destroy_children(folder).expect("clear");
    assert!(checked_kids(&dm, folder).is_empty());
    assert!(parts.iter().all(|&p| !dm.contains(p)));
}

#[test]
fn children_stay_right_after_many_removals_and_inserts() {
    let (mut dm, ids) = game();
    let ws = ids.workspace;
    let a = folder_in(&mut dm, &ids, ws);
    let b = folder_in(&mut dm, &ids, ws);
    let mut in_a: Vec<Id> = Vec::new();
    let mut in_b: Vec<Id> = Vec::new();
    // A fixed linear congruential sequence: deterministic, and it hits every position.
    let mut seed: u32 = 12_345;
    let mut roll = |n: usize| {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        (seed >> 16) as usize % n.max(1)
    };
    for step in 0..3_000 {
        match roll(4) {
            0 | 1 => {
                let p = part(&mut dm, &ids);
                dm.set_parent(p, Some(a)).expect("insert");
                in_a.push(p);
            }
            2 if !in_a.is_empty() => {
                let p = in_a.remove(roll(in_a.len()));
                if step % 2 == 0 {
                    dm.destroy(p).expect("destroy");
                } else {
                    dm.set_parent(p, Some(b)).expect("move");
                    in_b.push(p);
                }
            }
            // Less often than moves out, so both lists grow.
            3 if !in_b.is_empty() && step % 3 == 0 => {
                let p = in_b.remove(roll(in_b.len()));
                dm.set_parent(p, Some(a)).expect("move back");
                in_a.push(p);
            }
            _ => {}
        }
        if step % 500 == 0 {
            assert_eq!(checked_kids(&dm, a), in_a);
            assert_eq!(checked_kids(&dm, b), in_b);
        }
    }
    assert!(in_a.len() > 100 && in_b.len() > 10, "the sequence exercised both lists");
    assert_eq!(checked_kids(&dm, a), in_a);
    assert_eq!(checked_kids(&dm, b), in_b);
    let mut walked = vec![a];
    walked.extend(&in_a);
    walked.push(b);
    walked.extend(&in_b);
    assert_eq!(dm.descendants(ws).expect("walk").collect::<Vec<_>>(), walked);
}

#[test]
fn child_lists_refuse_stale_ids_and_never_name_them() {
    let (mut dm, ids) = game();
    let folder = folder_in(&mut dm, &ids, ids.workspace);
    let a = part_in(&mut dm, &ids, folder, "A");
    let b = part_in(&mut dm, &ids, folder, "B");
    dm.destroy(a).expect("destroy");
    // The freed slot is reused by a new instance outside the folder.
    let fresh = part(&mut dm, &ids);
    assert_eq!(fresh.index(), a.index());
    assert_eq!(checked_kids(&dm, folder), [b]);
    assert!(matches!(dm.children(a), Err(DmError::NoInstance(x)) if x == a));
    assert_eq!(dm.child_count(a), Err(DmError::NoInstance(a)));
    assert!(dm.descendants(a).is_err());
    assert_eq!(dm.destroy_children(a), Err(DmError::NoInstance(a)));
    assert_eq!(dm.parent(fresh), Ok(None));

    dm.destroy(folder).expect("destroy the parent");
    assert!(dm.children(folder).is_err());
    assert!(!dm.contains(b));
    assert!(checked_kids(&dm, ids.workspace).is_empty());
}
