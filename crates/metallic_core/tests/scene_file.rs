use metallic_core::*;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join("metallic_core_tests");
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

fn sample() -> (World, InstanceId) {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    let m = w.create("Model", Some(ws)).unwrap();
    w.set(m, "Name", Value::String("Lava".into())).unwrap();
    let p = w.create("Part", Some(m)).unwrap();
    w.set(p, "Position", Value::Vector3(Vec3::new(1.0, 2.5, -3.0)))
        .unwrap();
    w.set(p, "Color", Value::Color3(Color3::new(12.0, 34.5, 56.0)))
        .unwrap();
    w.set(p, "Material", Value::String("Glass".into())).unwrap();
    let q = w.create("Part", Some(m)).unwrap();
    w.set(q, "Anchored", Value::Bool(true)).unwrap();
    let sp = w.create("SpawnLocation", Some(ws)).unwrap();
    let pl = w
        .create("Player", Some(w.root("Players").unwrap()))
        .unwrap();
    w.set(pl, "RespawnLocation", Value::Ref(Some(sp))).unwrap();
    let s = w.create("Script", Some(m)).unwrap();
    w.set(s, "Source", Value::String("print(\"hi\")\nline2".into()))
        .unwrap();
    let g = w
        .create("TextLabel", Some(w.root("StarterGui").unwrap()))
        .unwrap();
    w.set(g, "Size", Value::UDim2(UDim2::new(0.5, 10.0, 0.0, 20.0)))
        .unwrap();
    (w, p)
}

#[test]
fn scene_round_trip_is_equal() {
    let (w, _) = sample();
    let a = tmp("rt_a.ron");
    let b = tmp("rt_b.ron");
    save_scene(&w, &a).unwrap();
    let w2 = load_scene(&a).unwrap();
    save_scene(&w2, &b).unwrap();
    assert_eq!(
        std::fs::read_to_string(&a).unwrap(),
        std::fs::read_to_string(&b).unwrap()
    );
    // Built-in services are not duplicated and refs survive.
    let wsp = w2.root("Workspace").unwrap();
    let players = w2.root("Players").unwrap();
    let sps = w2.root("StarterPlayer").unwrap();
    assert_eq!(w2.children(sps).len(), 1);
    let pl = w2.children(players)[0];
    let sp = w2.find_first_child(wsp, "SpawnLocation").unwrap();
    assert_eq!(w2.get(pl, "RespawnLocation").unwrap(), Value::Ref(Some(sp)));
    let lava = w2.find_first_child(wsp, "Lava").unwrap();
    let script = w2.find_first_child(lava, "Script").unwrap();
    assert_eq!(w2.full_name(script), "Workspace.Lava.Script");
    assert_eq!(
        w2.get(script, "Source").unwrap(),
        Value::String("print(\"hi\")\nline2".into())
    );
}

#[test]
fn scene_file_is_diff_friendly() {
    let (mut w, p) = sample();
    let a = tmp("df_a.ron");
    let b = tmp("df_b.ron");
    save_scene(&w, &a).unwrap();
    w.set(p, "Position", Value::Vector3(Vec3::new(9.0, 2.5, -3.0)))
        .unwrap();
    save_scene(&w, &b).unwrap();
    let ta = std::fs::read_to_string(&a).unwrap();
    let tb = std::fs::read_to_string(&b).unwrap();
    let (la, lb): (Vec<_>, Vec<_>) = (ta.lines().collect(), tb.lines().collect());
    assert_eq!(la.len(), lb.len());
    assert_eq!(la.iter().zip(&lb).filter(|(x, y)| x != y).count(), 1);
    // Only non-default properties are written.
    assert!(!ta.contains("Transparency"));
    assert!(!ta.contains("CanCollide"));
}

#[test]
fn load_errors_are_reported() {
    let f = tmp("bad.ron");
    std::fs::write(&f, "(version: 1, roots: [(class: \"Nope\")])").unwrap();
    assert!(load_scene(&f).is_err());
    std::fs::write(&f, "garbage").unwrap();
    assert!(matches!(load_scene(&f), Err(CoreError::Scene(_))));
    assert!(matches!(
        load_scene(&tmp("missing_zzz.ron")),
        Err(CoreError::Io(_))
    ));
}
