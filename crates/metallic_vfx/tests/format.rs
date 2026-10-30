//! Effect file format, presets and restyling.

use metallic_vfx::{ArtStyle, Effect, Simulation, VfxError, presets};
use std::path::PathBuf;

fn presets_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("presets")
}

#[test]
fn at_least_twenty_presets_with_unique_names() {
    let all = presets::all();
    assert!(all.len() >= 20, "only {} presets", all.len());
    let mut names = presets::names();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), all.len());
    for required in [
        "Fire",
        "Smoke",
        "Sparks",
        "Magic",
        "Explosion",
        "Dust puff",
        "Footstep",
        "Splash",
        "Rain",
        "Snow",
        "Falling leaves",
        "Confetti",
        "Hit impact",
        "Muzzle flash",
        "Heal",
        "Portal",
        "Shockwave",
        "Toon smear",
        "Pixel spark",
        "Lava bubbles",
    ] {
        assert!(presets::by_name(required).is_ok(), "missing {required}");
    }
    assert_eq!(
        presets::by_name("nope"),
        Err(VfxError::NoSuchPreset("nope".into()))
    );
}

#[test]
fn every_preset_round_trips_through_ron() {
    for e in presets::all() {
        e.validate().unwrap_or_else(|err| panic!("{}: {err}", e.name));
        let text = e.to_ron().unwrap_or_else(|err| panic!("{err}"));
        let back = Effect::from_ron(&text).unwrap_or_else(|err| panic!("{}: {err}\n{text}", e.name));
        assert_eq!(back, e, "{} changed in a round trip", e.name);
        assert_eq!(back.to_ron().ok(), Some(text), "{} output unstable", e.name);
    }
}

#[test]
fn every_restyled_preset_round_trips_and_simulates() {
    for e in presets::all() {
        for style in ArtStyle::ALL {
            let r = e.restyle(style);
            assert_eq!(r.style, style);
            let text = r.to_ron().unwrap_or_else(|err| panic!("{err}"));
            let back = Effect::from_ron(&text).unwrap_or_else(|err| panic!("{err}"));
            assert_eq!(back, r, "{} as {style:?}", e.name);
            let mut sim = Simulation::new(r, 3).unwrap_or_else(|err| panic!("{err}"));
            for _ in 0..30 {
                sim.step(1.0 / 60.0);
            }
        }
    }
}

#[test]
fn restyle_to_own_style_is_identity() {
    for e in presets::all() {
        assert_eq!(e.restyle(e.style), e);
    }
}

#[test]
fn format_is_readable_and_minimal() {
    let e = presets::by_name("Fire").unwrap_or_default();
    let text = e.to_ron().unwrap_or_default();
    assert!(text.contains("name: \"Fire\""));
    assert!(text.contains("\"#FFB238\""), "colours are hex");
    assert!(!text.contains("disabled"), "default flags are left out");
    assert!(text.lines().all(|l| l.len() < 120), "lines stay short for diffs");
}

#[test]
fn files_written_by_hand_load_with_defaults() {
    let text = r##"(
  name: "Tiny",
  emitters: [
    (
      name: "Dots",
      capacity: 10,
      spawn: [Rate(per_second: 5.0)],
      init: [Lifetime(seconds: (1.0, 2.0))],
      render: (kind: Billboard, texture: Dot),
    ),
  ],
)"##;
    let e = Effect::from_ron(text).unwrap_or_else(|err| panic!("{err}"));
    assert!(e.looping);
    assert_eq!(e.format, metallic_vfx::effect::FORMAT_VERSION);
    assert_eq!(e.emitters[0].time_scale, 1.0);
}

#[test]
fn parse_errors_and_newer_versions_are_reported() {
    assert!(matches!(Effect::from_ron("(name: 3)"), Err(VfxError::Parse(_))));
    let text = "(format: 99, name: \"Future\")";
    assert_eq!(Effect::from_ron(text), Err(VfxError::UnsupportedVersion(99)));
    let bad = r##"(name: "X", emitters: [(name: "E", capacity: 0)])"##;
    assert!(matches!(Effect::from_ron(bad), Err(VfxError::Invalid { .. })));
}

/// The `.vfx.ron` files under `presets/` are generated from code. Run with `METALLIC_VFX_BLESS=1` to
/// rewrite them after changing a preset.
#[test]
fn preset_files_match_code() {
    let dir = presets_dir();
    let bless = std::env::var_os("METALLIC_VFX_BLESS").is_some();
    for e in presets::all() {
        let path = dir.join(presets::file_name(&e.name));
        let text = e.to_ron().unwrap_or_else(|err| panic!("{err}"));
        if bless {
            std::fs::write(&path, &text).unwrap_or_else(|err| panic!("{err}"));
            continue;
        }
        let on_disk = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("missing {}; run with METALLIC_VFX_BLESS=1", path.display()));
        assert_eq!(
            on_disk.replace("\r\n", "\n"),
            text,
            "{} is stale; run with METALLIC_VFX_BLESS=1",
            path.display()
        );
    }
}
