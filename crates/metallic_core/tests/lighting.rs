//! Lighting service properties, effect classes and presets: defaults, round trips and scene files.
use metallic_core::classes::{all_properties, default_value};
use metallic_core::lighting::{
    LIGHTING_CHILD_CLASSES, PRESETS, apply_preset, find_preset, lighting_child, matching_preset,
};
use metallic_core::*;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join("metallic_core_lighting_tests");
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

/// A value of the property's type that differs from `v` (inside its range).
fn other(v: &Value, def: &classes::PropDef) -> Value {
    let allowed = def.allowed;
    match v {
        Value::Number(n) => {
            let up = n + 0.25;
            match def.range {
                Some(r) if up >= r.max => Value::Number(n - 0.25),
                _ => Value::Number(up),
            }
        }
        Value::Bool(b) => Value::Bool(!b),
        Value::Color3(c) => Value::Color3(Color3::new(255.0 - c.r, (c.g + 17.0) % 256.0, 3.0)),
        Value::String(s) => {
            let allowed = allowed.expect("lighting strings are enums");
            let next = allowed
                .iter()
                .find(|a| **a != s.as_str())
                .expect("two values");
            Value::String((*next).to_owned())
        }
        other => panic!("unexpected lighting value {other:?}"),
    }
}

#[test]
fn every_lighting_property_reads_its_default_and_round_trips() {
    let mut w = World::new_with_services();
    let lighting = w.root("Lighting").unwrap();
    let mut ids = vec![("Lighting", lighting)];
    for class in LIGHTING_CHILD_CLASSES {
        ids.push((class, w.create(class, Some(lighting)).unwrap()));
    }
    let mut count = 0;
    for (class, id) in ids {
        for def in all_properties(class) {
            if def.name == "Name" {
                continue;
            }
            let default = default_value(class, def.name).unwrap();
            assert_eq!(
                w.get(id, def.name).unwrap(),
                default,
                "{class}.{}",
                def.name
            );
            let changed = other(&default, def);
            w.set(id, def.name, changed.clone()).unwrap();
            assert_eq!(
                w.get(id, def.name).unwrap(),
                changed,
                "{class}.{}",
                def.name
            );
            count += 1;
        }
    }
    assert!(count >= 40, "only {count} lighting properties");
}

#[test]
fn roblox_names_and_metallic_defaults() {
    let w = World::new_with_services();
    let l = w.root("Lighting").unwrap();
    assert_eq!(w.get(l, "ClockTime").unwrap(), Value::Number(14.5));
    assert_eq!(
        w.get(l, "GeographicLatitude").unwrap(),
        Value::Number(41.73)
    );
    assert_eq!(w.get(l, "GlobalShadows").unwrap(), Value::Bool(true));
    assert_eq!(
        w.get(l, "OutdoorAmbient").unwrap(),
        Value::Color3(Color3::new(0.0, 0.0, 0.0))
    );
    assert_eq!(
        w.get(l, "LightingStyle").unwrap(),
        Value::String("Realistic".into())
    );
}

#[test]
fn enum_properties_reject_unknown_values_and_wrong_types() {
    let mut w = World::new_with_services();
    let l = w.root("Lighting").unwrap();
    assert!(
        w.set(l, "LightingStyle", Value::String("Neon".into()))
            .is_err()
    );
    assert!(w.set(l, "ClockTime", Value::Bool(true)).is_err());
    assert!(w.set(l, "ClockTime", Value::Number(f64::NAN)).is_err());
    let g = w.create("ColorGradingEffect", Some(l)).unwrap();
    assert!(
        w.set(g, "TonemapperPreset", Value::String("Hable".into()))
            .is_err()
    );
    w.set(g, "TonemapperPreset", Value::String("Retro".into()))
        .unwrap();
}

#[test]
fn a_fresh_place_matches_clear_day() {
    let w = World::new_with_services();
    assert_eq!(matching_preset(&w).map(|p| p.id), Some("clear-day"));
}

#[test]
fn applying_a_preset_creates_children_once_and_keeps_other_children() {
    let mut w = World::new_with_services();
    let l = w.root("Lighting").unwrap();
    let mine = w.create("Folder", Some(l)).unwrap();
    for p in PRESETS.iter().chain(PRESETS.iter()) {
        apply_preset(&mut w, p).unwrap();
        assert_eq!(matching_preset(&w).map(|m| m.id), Some(p.id));
    }
    assert_eq!(w.children(l).len(), 1 + LIGHTING_CHILD_CLASSES.len());
    assert!(w.is_alive(mine));
    let atmo = lighting_child(&w, "Atmosphere").unwrap();
    assert_eq!(
        w.get(atmo, "Name").unwrap(),
        Value::String("Atmosphere".into())
    );
}

#[test]
fn an_edited_preset_no_longer_matches() {
    let mut w = World::new_with_services();
    apply_preset(&mut w, find_preset("golden-hour").unwrap()).unwrap();
    let atmo = lighting_child(&w, "Atmosphere").unwrap();
    w.set(atmo, "Haze", Value::Number(2.5)).unwrap();
    assert!(matching_preset(&w).is_none());
}

#[test]
fn every_preset_survives_a_scene_save_and_load() {
    for p in &PRESETS {
        let mut w = World::new_with_services();
        apply_preset(&mut w, p).unwrap();
        let path = tmp(&format!("{}.ron", p.id));
        save_scene(&w, &path).unwrap();
        let loaded = load_scene(&path).unwrap();
        assert_eq!(matching_preset(&loaded).map(|m| m.id), Some(p.id));
        let l = loaded.root("Lighting").unwrap();
        assert_eq!(
            loaded.children(l).len(),
            LIGHTING_CHILD_CLASSES.len(),
            "{}: children duplicated or lost",
            p.id
        );
        // Saving the loaded world again gives the same file.
        let again = tmp(&format!("{}-again.ron", p.id));
        save_scene(&loaded, &again).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            std::fs::read_to_string(&again).unwrap(),
            "{}",
            p.id
        );
    }
}

#[test]
fn hand_set_lighting_survives_a_scene_save_and_load() {
    let mut w = World::new_with_services();
    let l = w.root("Lighting").unwrap();
    w.set(l, "ClockTime", Value::Number(6.25)).unwrap();
    w.set(l, "Ambient", Value::Color3(Color3::new(12.0, 34.0, 56.0)))
        .unwrap();
    w.set(l, "LightingStyle", Value::String("Soft".into()))
        .unwrap();
    let bloom = w.create("BloomEffect", Some(l)).unwrap();
    w.set(bloom, "Enabled", Value::Bool(false)).unwrap();
    let path = tmp("hand.ron");
    save_scene(&w, &path).unwrap();
    let loaded = load_scene(&path).unwrap();
    let l2 = loaded.root("Lighting").unwrap();
    assert_eq!(loaded.get(l2, "ClockTime").unwrap(), Value::Number(6.25));
    assert_eq!(
        loaded.get(l2, "Ambient").unwrap(),
        Value::Color3(Color3::new(12.0, 34.0, 56.0))
    );
    assert_eq!(
        loaded.get(l2, "LightingStyle").unwrap(),
        Value::String("Soft".into())
    );
    let b2 = lighting_child(&loaded, "BloomEffect").unwrap();
    assert_eq!(loaded.get(b2, "Enabled").unwrap(), Value::Bool(false));
}

/// Every lighting and effect Number and Color3 has a finite range; values outside are clamped
/// (ClockTime wraps), and non-finite values are refused, both on write and on load.
#[test]
fn lighting_values_are_kept_in_range_on_write_and_load() {
    let mut w = World::new_with_services();
    let lighting = w.root("Lighting").unwrap();
    let mut ids = vec![("Lighting", lighting)];
    for class in LIGHTING_CHILD_CLASSES {
        ids.push((class, w.create(class, Some(lighting)).unwrap()));
    }
    for (class, id) in &ids {
        for def in all_properties(class) {
            let is_colour = def.ty == ValueType::Color3;
            if def.ty != ValueType::Number && !is_colour {
                continue;
            }
            let range = def
                .range
                .unwrap_or_else(|| panic!("{class}.{} has no range", def.name));
            assert!(range.min.is_finite() && range.max.is_finite());
            for big in [1e39, -1e39, f64::MAX, f64::MIN] {
                let v = if is_colour {
                    Value::Color3(Color3::new(big, big, big))
                } else {
                    Value::Number(big)
                };
                w.set(*id, def.name, v).unwrap();
                let check = |x: f64| {
                    assert!(
                        x.is_finite() && x >= range.min && x <= range.max,
                        "{class}.{} = {x} after writing {big}",
                        def.name
                    )
                };
                match w.get(*id, def.name).unwrap() {
                    Value::Number(n) => check(n),
                    Value::Color3(c) => [c.r, c.g, c.b].into_iter().for_each(check),
                    other => panic!("{other:?}"),
                }
            }
            for bad in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
                let before = w.get(*id, def.name).unwrap();
                let v = if is_colour {
                    Value::Color3(Color3::new(bad, 0.0, 0.0))
                } else {
                    Value::Number(bad)
                };
                assert!(
                    w.set(*id, def.name, v).is_err(),
                    "{class}.{} took {bad}",
                    def.name
                );
                assert_eq!(w.get(*id, def.name).unwrap(), before);
            }
        }
    }
    // Specific limits.
    w.set(lighting, "ClockTime", Value::Number(25.5)).unwrap();
    assert_eq!(w.get(lighting, "ClockTime").unwrap(), Value::Number(1.5));
    w.set(lighting, "ClockTime", Value::Number(-1.0)).unwrap();
    assert_eq!(w.get(lighting, "ClockTime").unwrap(), Value::Number(23.0));
    w.set(lighting, "Brightness", Value::Number(1e39)).unwrap();
    assert_eq!(w.get(lighting, "Brightness").unwrap(), Value::Number(10.0));
    w.set(lighting, "ExposureCompensation", Value::Number(-1e39))
        .unwrap();
    assert_eq!(
        w.get(lighting, "ExposureCompensation").unwrap(),
        Value::Number(-5.0)
    );

    // A scene file written with out-of-range values (by hand or an older build) loads clamped.
    let mut w = World::new_with_services();
    let l = w.root("Lighting").unwrap();
    w.set(l, "Brightness", Value::Number(7.0)).unwrap();
    w.set(l, "FogColor", Value::Color3(Color3::new(1.0, 2.0, 3.0)))
        .unwrap();
    let path = tmp("out_of_range.ron");
    save_scene(&w, &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let edited = text.replacen("7.0", "1e39", 1).replacen("3.0", "1e300", 1);
    assert_ne!(
        text, edited,
        "scene text has no plain 7.0 / 3.0:
{text}"
    );
    std::fs::write(&path, edited).unwrap();
    let loaded = load_scene(&path).unwrap();
    let l2 = loaded.root("Lighting").unwrap();
    assert_eq!(loaded.get(l2, "Brightness").unwrap(), Value::Number(10.0));
    assert_eq!(
        loaded.get(l2, "FogColor").unwrap(),
        Value::Color3(Color3::new(1.0, 2.0, 255.0))
    );
}
