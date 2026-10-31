//! Humanoid tuning is sanitised at the write boundary (#172, #272): huge values are clamped to
//! the documented maximums and non-finite values are refused, through `World::set` and the
//! scene loader alike.
use metallic_core::classes::{
    MAX_HUMANOID_HEALTH, MAX_JUMP_POWER, MAX_SLOPE_ANGLE, MAX_WALK_SPEED,
};
use metallic_core::*;

mod common;
use common::TestDir;

fn number(w: &World, id: InstanceId, prop: &str) -> f64 {
    match w.get(id, prop).unwrap() {
        Value::Number(n) => n,
        v => panic!("{prop} is {v:?}"),
    }
}

const LIMITS: [(&str, f64); 5] = [
    ("Health", MAX_HUMANOID_HEALTH),
    ("MaxHealth", MAX_HUMANOID_HEALTH),
    ("WalkSpeed", MAX_WALK_SPEED),
    ("JumpPower", MAX_JUMP_POWER),
    ("MaxSlopeAngle", MAX_SLOPE_ANGLE),
];

#[test]
fn huge_and_negative_values_are_clamped() {
    let mut w = World::new_with_services();
    let h = w.create("Humanoid", None).unwrap();
    for (prop, max) in LIMITS {
        w.set(h, prop, Value::Number(1e300)).unwrap();
        assert_eq!(number(&w, h, prop), max, "{prop} high");
        w.set(h, prop, Value::Number(-5.0)).unwrap();
        assert_eq!(number(&w, h, prop), 0.0, "{prop} low");
        // In-range values are kept as written.
        w.set(h, prop, Value::Number(42.5)).unwrap();
        assert_eq!(number(&w, h, prop), 42.5, "{prop} in range");
    }
}

#[test]
fn non_finite_values_are_refused_and_the_old_value_stays() {
    let mut w = World::new_with_services();
    let h = w.create("Humanoid", None).unwrap();
    for (prop, _) in LIMITS {
        let before = number(&w, h, prop);
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(
                w.set(h, prop, Value::Number(bad)).is_err(),
                "{prop} = {bad}"
            );
            assert_eq!(number(&w, h, prop), before, "{prop} kept");
        }
    }
}

#[test]
fn defaults_are_inside_the_limits() {
    let mut w = World::new_with_services();
    let h = w.create("Humanoid", None).unwrap();
    assert_eq!(number(&w, h, "Health"), 100.0);
    assert_eq!(number(&w, h, "MaxHealth"), 100.0);
    assert_eq!(number(&w, h, "WalkSpeed"), 16.0);
    assert_eq!(number(&w, h, "JumpPower"), 50.0);
    assert_eq!(number(&w, h, "MaxSlopeAngle"), 89.0);
}

#[test]
fn scene_files_are_clamped_on_load() {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    let model = w.create("Model", Some(ws)).unwrap();
    let h = w.create("Humanoid", Some(model)).unwrap();
    w.set(h, "WalkSpeed", Value::Number(20.0)).unwrap();
    let dir = TestDir::new("scene_files_are_clamped_on_load");
    let path = dir.file("huge.mscene");
    save_scene(&w, &path).unwrap();
    // Edit the file by hand as a creator might.
    let text = std::fs::read_to_string(&path).unwrap();
    let line = "(\"WalkSpeed\", Number(20.0))";
    assert!(text.contains(line), "scene holds the WalkSpeed:\n{text}");
    std::fs::write(&path, text.replace(line, "(\"WalkSpeed\", Number(1e300))")).unwrap();
    let loaded = load_scene(&path).unwrap();
    let ws2 = loaded.root("Workspace").unwrap();
    let model2 = loaded.children(ws2)[0];
    let h2 = loaded.find_first_child(model2, "Humanoid").unwrap();
    assert_eq!(number(&loaded, h2, "WalkSpeed"), MAX_WALK_SPEED);
}
