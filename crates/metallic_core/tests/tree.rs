use metallic_core::*;

fn world() -> World {
    World::new_with_services()
}

fn ws(w: &World) -> InstanceId {
    w.root("Workspace").unwrap()
}

#[test]
fn reparent_refuses_cycle() {
    let mut w = world();
    let a = w.create("Model", Some(ws(&w))).unwrap();
    let b = w.create("Model", Some(a)).unwrap();
    assert_eq!(w.set_parent(a, Some(b)), Err(CoreError::Cycle));
    assert_eq!(w.set_parent(a, Some(a)), Err(CoreError::Cycle));
    assert_eq!(w.children(a), &[b]);
}

#[test]
fn destroy_removes_subtree_and_invalidates_ids() {
    let mut w = world();
    let a = w.create("Model", Some(ws(&w))).unwrap();
    let b = w.create("Part", Some(a)).unwrap();
    w.destroy(a);
    assert!(!w.is_alive(a));
    assert!(!w.is_alive(b));
    assert!(w.children(ws(&w)).is_empty());
    assert!(matches!(w.get(b, "Name"), Err(CoreError::InvalidInstance)));
    assert_eq!(
        w.set_parent(b, Some(ws(&w))),
        Err(CoreError::InvalidInstance)
    );
    // A reused slot must not resurrect the old id.
    let c = w.create("Part", Some(ws(&w))).unwrap();
    assert_ne!(c, b);
    assert!(!w.is_alive(b));
}

#[test]
fn full_name_is_dotted_path() {
    let mut w = world();
    let lava = w.create("Model", Some(ws(&w))).unwrap();
    w.set(lava, "Name", Value::String("Lava".into())).unwrap();
    let s = w.create("Script", Some(lava)).unwrap();
    assert_eq!(w.full_name(s), "Workspace.Lava.Script");
    assert_eq!(w.find_first_child(lava, "Script"), Some(s));
    assert_eq!(w.find_first_child(lava, "Nope"), None);
}

#[test]
fn set_rejects_wrong_type() {
    let mut w = world();
    let p = w.create("Part", Some(ws(&w))).unwrap();
    let err = w.set(p, "Size", Value::String("x".into())).unwrap_err();
    assert!(matches!(err, CoreError::TypeMismatch { .. }), "{err:?}");
    assert!(matches!(
        w.set(p, "Nope", Value::Nil),
        Err(CoreError::UnknownProperty { .. })
    ));
    assert!(matches!(
        w.set(p, "Material", Value::String("Cheese".into())),
        Err(CoreError::InvalidValue { .. })
    ));
    assert!(matches!(
        w.create("Nope", None),
        Err(CoreError::UnknownClass(_))
    ));
}

#[test]
fn defaults_match_registry() {
    let mut w = world();
    let p = w.create("Part", Some(ws(&w))).unwrap();
    assert_eq!(
        w.get(p, "Size").unwrap(),
        Value::Vector3(Vec3::new(4.0, 1.0, 2.0))
    );
    assert_eq!(
        w.get(p, "Color").unwrap(),
        Value::Color3(Color3::new(163.0, 162.0, 165.0))
    );
    assert_eq!(
        w.get(p, "Material").unwrap(),
        Value::String("Plastic".into())
    );
    assert_eq!(w.get(p, "Name").unwrap(), Value::String("Part".into()));
    assert_eq!(
        w.get(ws(&w), "FallenPartsDestroyHeight").unwrap(),
        Value::Number(-500.0)
    );
    let h = w.create("Humanoid", None).unwrap();
    assert_eq!(w.get(h, "MaxSlopeAngle").unwrap(), Value::Number(89.0));
    let pl = w.create("Player", None).unwrap();
    assert_eq!(w.get(pl, "RespawnLocation").unwrap(), Value::Ref(None));
}

#[test]
fn inheritance() {
    let mut w = world();
    let sp = w.create("SpawnLocation", None).unwrap();
    assert!(w.is_a(sp, "Part") && w.is_a(sp, "BasePart") && w.is_a(sp, "Instance"));
    assert!(!w.is_a(sp, "Script"));
    let ls = w.create("LocalScript", None).unwrap();
    assert!(w.is_a(ls, "Script"));
    w.set(sp, "Anchored", Value::Bool(true)).unwrap();
}

#[test]
fn services_exist() {
    let w = world();
    for s in [
        "Workspace",
        "Players",
        "ReplicatedStorage",
        "ServerScriptService",
        "StarterPlayer",
        "StarterGui",
        "Lighting",
        "RunService",
    ] {
        assert!(w.root(s).is_some(), "{s}");
    }
    let sp = w.root("StarterPlayer").unwrap();
    assert!(w.find_first_child(sp, "StarterPlayerScripts").is_some());
}

#[test]
fn deep_clone_is_independent() {
    let mut w = world();
    let p = w.create("Part", Some(ws(&w))).unwrap();
    let w2 = w.deep_clone();
    w.set(p, "Anchored", Value::Bool(true)).unwrap();
    w.destroy(p);
    assert!(w2.is_alive(p));
    assert_eq!(w2.get(p, "Anchored").unwrap(), Value::Bool(false));
}

#[test]
fn clone_subtree_copies_and_remaps_refs() {
    let mut w = world();
    let m = w.create("Model", Some(ws(&w))).unwrap();
    let pl = w.create("Player", Some(m)).unwrap();
    let sp = w.create("SpawnLocation", Some(m)).unwrap();
    w.set(pl, "RespawnLocation", Value::Ref(Some(sp))).unwrap();
    let c = w.clone_subtree(m).unwrap();
    assert_ne!(c, m);
    assert_eq!(w.parent(c), None);
    let kids = w.children(c).to_vec();
    assert_eq!(kids.len(), 2);
    assert_eq!(
        w.get(kids[0], "RespawnLocation").unwrap(),
        Value::Ref(Some(kids[1]))
    );
}

#[test]
fn signals_fire() {
    use std::sync::{Arc, Mutex};
    let mut w = world();
    let added = Arc::new(Mutex::new(Vec::new()));
    let a2 = added.clone();
    let conn = w
        .child_added
        .connect(move |&(parent, child)| a2.lock().unwrap().push((parent, child)));
    let p = w.create("Part", Some(ws(&w))).unwrap();
    assert_eq!(added.lock().unwrap().as_slice(), &[(ws(&w), p)]);
    let changed = Arc::new(Mutex::new(Vec::new()));
    let c2 = changed.clone();
    w.property_changed(p, "Anchored")
        .connect(move |v| c2.lock().unwrap().push(v.clone()));
    w.set(p, "Anchored", Value::Bool(true)).unwrap();
    w.set(p, "CanCollide", Value::Bool(false)).unwrap();
    assert_eq!(changed.lock().unwrap().as_slice(), &[Value::Bool(true)]);
    w.child_added.disconnect(conn);
    w.create("Part", Some(ws(&w))).unwrap();
    assert_eq!(added.lock().unwrap().len(), 1);
}

#[test]
fn vec3_math() {
    let a = Vec3::new(3.0, 0.0, 4.0);
    assert_eq!(a.magnitude(), 5.0);
    assert_eq!(a.unit(), Vec3::new(0.6, 0.0, 0.8));
    assert_eq!(a + Vec3::new(1.0, 1.0, 1.0), Vec3::new(4.0, 1.0, 5.0));
    assert_eq!(a - a, Vec3::new(0.0, 0.0, 0.0));
    assert_eq!(a * 2.0, Vec3::new(6.0, 0.0, 8.0));
    assert_eq!(Vec3::new(0.0, 0.0, 0.0).unit(), Vec3::new(0.0, 0.0, 0.0));
}

#[test]
fn clone_subtree_of_destroyed_id_errors() {
    let mut w = world();
    let p = w.create("Part", Some(ws(&w))).unwrap();
    w.destroy(p);
    assert_eq!(w.clone_subtree(p), Err(CoreError::InvalidInstance));
}

#[test]
fn world_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<World>();
}

/// Records every value `prop` of `id` fires with.
fn record(
    w: &mut World,
    id: InstanceId,
    prop: &str,
) -> std::sync::Arc<std::sync::Mutex<Vec<Value>>> {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let s2 = seen.clone();
    w.property_changed(id, prop)
        .connect(move |v| s2.lock().unwrap().push(v.clone()));
    seen
}

#[test]
fn destroy_ignores_out_of_range_signal_id() {
    let mut w = World::new_empty();
    let part = w.create("Part", None).unwrap();
    let survivor = w.create("Part", None).unwrap();
    let seen = record(&mut w, survivor, "Anchored");
    let forged: InstanceId = ron::from_str("(index: 4294967295, generation: 0)").unwrap();
    w.property_changed(forged, "Name").connect(|_| {});
    w.destroy(part);
    assert!(!w.is_alive(part));
    w.set(survivor, "Anchored", Value::Bool(true)).unwrap();
    assert_eq!(seen.lock().unwrap().as_slice(), &[Value::Bool(true)]);
}

#[test]
fn destroy_ignores_stale_generation_signal_id() {
    let mut w = World::new_empty();
    let old = w.create("Part", None).unwrap();
    w.destroy(old);
    let survivor = w.create("Part", None).unwrap();
    let seen = record(&mut w, survivor, "Anchored");
    w.property_changed(old, "Name").connect(|_| {});
    let part = w.create("Part", None).unwrap();
    w.destroy(part);
    w.set(survivor, "Anchored", Value::Bool(true)).unwrap();
    assert_eq!(seen.lock().unwrap().as_slice(), &[Value::Bool(true)]);
}

#[test]
fn player_gui_is_a_registered_class_with_unique_bits() {
    let mut w = world();
    let player = w
        .create("Player", Some(w.root("Players").unwrap()))
        .unwrap();
    let gui = w.create("PlayerGui", Some(player)).unwrap();
    assert_eq!(w.class_name(gui), Some("PlayerGui"));
    assert!(w.is_a(gui, "Instance"));
    assert_ne!(gui.to_bits(), player.to_bits());
    w.destroy(gui);
    let again = w.create("PlayerGui", Some(player)).unwrap();
    assert_ne!(again.to_bits(), gui.to_bits());
}

/// Every built-in: the root services plus `StarterPlayerScripts` under `StarterPlayer`.
fn builtins(w: &World) -> Vec<InstanceId> {
    let mut out = w.roots().to_vec();
    let sp = w.root("StarterPlayer").unwrap();
    out.extend_from_slice(w.children(sp));
    assert!(out.iter().all(|&b| w.is_builtin(b)));
    assert_eq!(out.len(), 9);
    out
}

/// The whole tree as (parent, children) pairs, for "unchanged" checks.
fn shape(w: &World) -> Vec<(Option<InstanceId>, Vec<InstanceId>)> {
    let mut out = vec![(None, w.roots().to_vec())];
    let mut stack = w.roots().to_vec();
    while let Some(id) = stack.pop() {
        out.push((Some(id), w.children(id).to_vec()));
        stack.extend_from_slice(w.children(id));
    }
    out
}

#[test]
fn builtin_services_cannot_be_reparented() {
    let mut w = world();
    let folder = w
        .create("Folder", Some(w.root("Lighting").unwrap()))
        .unwrap();
    let before = shape(&w);
    for b in builtins(&w) {
        let class = w.class_name(b).unwrap().to_string();
        let locked = Err(CoreError::Locked(class.clone()));
        assert_eq!(w.set_parent(b, Some(folder)), locked, "{class}");
        assert_eq!(w.set_parent(b, Some(ws(&w))), locked, "{class}");
        assert_eq!(w.set_parent(b, None), locked, "{class}");
        assert_eq!(shape(&w), before, "{class}");
    }
    assert!(w.root("Workspace").is_some());
    assert_eq!(w.full_name(ws(&w)), "Workspace");
}

#[test]
fn destroying_a_builtin_service_is_a_no_op() {
    let mut w = world();
    let p = w.create("Part", Some(ws(&w))).unwrap();
    let before = shape(&w);
    let cur = w.change_cursor();
    for b in builtins(&w) {
        w.destroy(b);
        assert!(w.is_alive(b));
    }
    assert!(w.is_alive(p));
    assert_eq!(shape(&w), before);
    assert_eq!(w.changes_since(cur).0, Changes::Ids(Vec::new()));
}

#[test]
fn builtin_services_cannot_be_cloned() {
    let mut w = world();
    let before = shape(&w);
    for b in builtins(&w) {
        let class = w.class_name(b).unwrap().to_string();
        assert_eq!(w.clone_subtree(b), Err(CoreError::Locked(class)));
    }
    assert_eq!(shape(&w), before);
}

#[test]
fn services_and_abstract_classes_are_not_creatable() {
    let mut w = world();
    let before = shape(&w);
    for class in [
        "Workspace",
        "Instance",
        "BasePart",
        "Players",
        "ReplicatedStorage",
        "ServerScriptService",
        "StarterPlayer",
        "StarterPlayerScripts",
        "StarterGui",
        "Lighting",
        "RunService",
    ] {
        let refused = Err(CoreError::NotCreatable(class.to_string()));
        assert_eq!(w.create(class, None), refused, "{class}");
        assert_eq!(w.create(class, Some(ws(&w))), refused, "{class}");
        assert!(!classes::class_def(class).unwrap().creatable, "{class}");
    }
    assert_eq!(shape(&w), before);
}

#[test]
fn concrete_classes_are_still_creatable() {
    let mut w = world();
    for class in ["Part", "Folder", "Model", "SpawnLocation", "Script"] {
        let id = w.create(class, Some(ws(&w))).unwrap();
        assert_eq!(w.class_name(id), Some(class));
        assert!(!w.is_builtin(id));
        assert!(classes::class_def(class).unwrap().creatable, "{class}");
    }
    // Plain instances can still move and be destroyed.
    let f = w.find_first_child(ws(&w), "Folder").unwrap();
    let p = w.find_first_child(ws(&w), "Part").unwrap();
    w.set_parent(p, Some(f)).unwrap();
    assert_eq!(w.parent(p), Some(f));
    w.destroy(f);
    assert!(!w.is_alive(p));
}

#[test]
fn deep_clone_keeps_builtins_locked() {
    let mut w = world().deep_clone();
    let b = ws(&w);
    assert!(w.is_builtin(b));
    w.destroy(b);
    assert_eq!(
        w.set_parent(b, None),
        Err(CoreError::Locked("Workspace".into()))
    );
    assert_eq!(w.root("Workspace"), Some(b));
}
