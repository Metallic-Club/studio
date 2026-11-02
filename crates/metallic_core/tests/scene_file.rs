use metallic_core::*;

mod common;
use common::TestDir;

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
    let dir = TestDir::new("scene_round_trip_is_equal");
    let (w, _) = sample();
    let a = dir.file("rt_a.ron");
    let b = dir.file("rt_b.ron");
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
    let dir = TestDir::new("scene_file_is_diff_friendly");
    let (mut w, p) = sample();
    let a = dir.file("df_a.ron");
    let b = dir.file("df_b.ron");
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
    let dir = TestDir::new("load_errors_are_reported");
    let f = dir.file("bad.ron");
    std::fs::write(&f, "(version: 1, roots: [(class: \"Nope\")])").unwrap();
    assert!(load_scene(&f).is_err());
    std::fs::write(&f, "garbage").unwrap();
    assert!(matches!(load_scene(&f), Err(CoreError::Scene(_))));
    assert!(matches!(
        load_scene(&dir.file("missing_zzz.ron")),
        Err(CoreError::Io(_))
    ));
}

#[test]
fn pivots_and_scale_survive_a_save() {
    let dir = TestDir::new("pivots_and_scale_survive_a_save");
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    let m = w.create("Model", Some(ws)).unwrap();
    let p = w.create("Part", Some(m)).unwrap();
    let offset =
        CFrame::from_position(Vec3::new(0.0, -1.0, 0.0)) * CFrame::from_euler_xyz(0.1, 0.2, 0.3);
    w.set(p, "PivotOffset", Value::CFrame(Box::new(offset)))
        .unwrap();
    w.set(m, "PrimaryPart", Value::Ref(Some(p))).unwrap();
    transform::set_world_pivot(&mut w, m, CFrame::from_position(Vec3::new(4.0, 5.0, 6.0))).unwrap();
    transform::scale_to(&mut w, m, 2.0).unwrap();
    let path = dir.file("pivots.ron");
    save_scene(&w, &path).unwrap();
    let w2 = load_scene(&path).unwrap();
    let m2 = w2.children(w2.root("Workspace").unwrap())[0];
    let p2 = w2.children(m2)[0];
    assert_eq!(transform::get_pivot(&w2, m2), transform::get_pivot(&w, m));
    assert_eq!(
        transform::world_pivot(&w2, m2),
        transform::world_pivot(&w, m)
    );
    assert_eq!(transform::model_scale(&w2, m2), 2.0);
    assert_eq!(w2.get(m2, "PrimaryPart").unwrap(), Value::Ref(Some(p2)));
}

#[test]
fn module_script_source_round_trips_and_is_not_a_script() {
    let dir = TestDir::new("module_script_round_trip");
    let mut w = World::new_with_services();
    let rs = w.root("ReplicatedStorage").unwrap();
    let m = w.create("ModuleScript", Some(rs)).unwrap();
    w.set(m, "Name", Value::String("Config".into())).unwrap();
    w.set(
        m,
        "Source",
        Value::String("local M = {}\nreturn M -- \"quoted\"".into()),
    )
    .unwrap();
    assert!(!w.is_a(m, "Script"), "a ModuleScript never runs by itself");
    assert!(w.set(m, "Enabled", Value::Bool(false)).is_err());
    let path = dir.file("module.ron");
    save_scene(&w, &path).unwrap();
    let w2 = load_scene(&path).unwrap();
    let rs2 = w2.root("ReplicatedStorage").unwrap();
    let m2 = w2.find_first_child(rs2, "Config").unwrap();
    assert_eq!(w2.class_name(m2), Some("ModuleScript"));
    assert_eq!(
        w2.get(m2, "Source").unwrap(),
        Value::String("local M = {}\nreturn M -- \"quoted\"".into())
    );
}
