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
    let c = w.clone_subtree(m);
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
    use std::cell::RefCell;
    use std::rc::Rc;
    let mut w = world();
    let added = Rc::new(RefCell::new(Vec::new()));
    let a2 = added.clone();
    let conn = w
        .child_added
        .connect(move |&(parent, child)| a2.borrow_mut().push((parent, child)));
    let p = w.create("Part", Some(ws(&w))).unwrap();
    assert_eq!(added.borrow().as_slice(), &[(ws(&w), p)]);
    let changed = Rc::new(RefCell::new(Vec::new()));
    let c2 = changed.clone();
    w.property_changed(p, "Anchored")
        .connect(move |v| c2.borrow_mut().push(v.clone()));
    w.set(p, "Anchored", Value::Bool(true)).unwrap();
    w.set(p, "CanCollide", Value::Bool(false)).unwrap();
    assert_eq!(changed.borrow().as_slice(), &[Value::Bool(true)]);
    w.child_added.disconnect(conn);
    w.create("Part", Some(ws(&w))).unwrap();
    assert_eq!(added.borrow().len(), 1);
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
