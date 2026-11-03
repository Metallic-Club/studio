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
    assert_eq!(dm.children(c), Ok(&[][..]));
}

#[test]
fn set_parent_updates_both_sides_and_keeps_child_order() {
    let (mut dm, ids) = game();
    let ws = ids.workspace;
    let a = part_in(&mut dm, &ids, ws, "A");
    let b = part_in(&mut dm, &ids, ws, "B");
    let c = part_in(&mut dm, &ids, ws, "C");
    assert_eq!(dm.children(ws), Ok(&[a, b, c][..]));
    let folder = folder_in(&mut dm, &ids, ws);
    dm.set_parent(b, Some(folder)).expect("move");
    assert_eq!(dm.children(ws), Ok(&[a, c, folder][..]));
    assert_eq!(dm.children(folder), Ok(&[b][..]));
    assert_eq!(dm.parent(b), Ok(Some(folder)));
    dm.set_parent(b, Some(ws)).expect("move back");
    assert_eq!(dm.children(ws), Ok(&[a, c, folder, b][..]));
    // Sibling positions were renumbered: the walk visits every child once, in order.
    let walked: Vec<Id> = dm.descendants(ws).expect("walk").collect();
    assert_eq!(walked, vec![a, c, folder, b]);
    dm.set_parent(a, None).expect("unparent");
    assert_eq!(dm.parent(a), Ok(None));
    assert_eq!(dm.children(ws), Ok(&[c, folder, b][..]));

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
    assert_eq!(dm.children(ws), Ok(&[keep][..]));
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
    assert_eq!(dm.children(model), Ok(&[][..]));
    assert!(parts.iter().all(|p| !dm.contains(*p)));
    dm.destroy_children(ids.game).expect("clear root");
    assert!(!dm.contains(loose));
    assert!(dm.contains(model), "inside a kept service");
    assert_eq!(dm.children(ids.game), Ok(&[ids.workspace, ids.lighting][..]));
}
