//! `LookOverride` (#222): writes, resolution through Models, the journal-driven cache, and
//! scene files.
use metallic_core::look_override::{self, LookOverrides, resolve};
use metallic_core::*;

mod common;
use common::TestDir;

fn s(v: &str) -> Value {
    Value::String(v.into())
}

fn set_look(w: &mut World, id: InstanceId, v: &str) {
    w.set(id, "LookOverride", s(v)).unwrap();
}

fn look(w: &World, id: InstanceId) -> String {
    match w.get(id, "LookOverride").unwrap() {
        Value::String(v) => v,
        other => panic!("{other:?}"),
    }
}

/// A cache kept current from the journal, checked against a fresh rebuild and against
/// [`resolve`] for every part after each step.
struct Tracked {
    cache: LookOverrides,
    cursor: ChangeCursor,
}

impl Tracked {
    fn new(w: &World) -> Tracked {
        let mut cache = LookOverrides::new();
        cache.rebuild(w);
        Tracked {
            cache,
            cursor: w.change_cursor(),
        }
    }

    /// Applies the journal; returns the parts reported as changed.
    fn sync(&mut self, w: &World) -> Vec<InstanceId> {
        let (changes, cursor) = w.changes_since(self.cursor);
        self.cursor = cursor;
        let mut changed = self.cache.apply(w, &changes);
        changed.sort_unstable();
        changed.dedup();
        let mut fresh = LookOverrides::new();
        fresh.rebuild(w);
        for part in parts(w) {
            assert_eq!(self.cache.get(part), resolve(w, part), "cache vs resolve");
            assert_eq!(self.cache.get(part), fresh.get(part), "cache vs rebuild");
        }
        assert_eq!(self.cache.overridden_parts(), fresh.overridden_parts());
        changed
    }
}

fn parts(w: &World) -> Vec<InstanceId> {
    let mut out = Vec::new();
    let mut stack: Vec<InstanceId> = w.roots().to_vec();
    while let Some(id) = stack.pop() {
        if w.is_a(id, "BasePart") {
            out.push(id);
        }
        stack.extend_from_slice(w.children(id));
    }
    out
}

#[test]
fn display_names_are_stored_as_ids_and_unknown_styles_are_refused() {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    let p = w.create("Part", Some(ws)).unwrap();
    let m = w.create("Model", Some(ws)).unwrap();
    assert_eq!(look(&w, p), "inherit");
    assert_eq!(look(&w, m), "inherit");
    set_look(&mut w, p, "Retro / PS1");
    assert_eq!(look(&w, p), "retro-32");
    set_look(&mut w, m, "Pixel 3D");
    assert_eq!(look(&w, m), "pixel-3d");
    set_look(&mut w, p, "toon");
    assert_eq!(look(&w, p), "toon");
    // Refused writes leave the value alone.
    for bad in ["comic", "", "looks/MyToon"] {
        let err = w.set(p, "LookOverride", s(bad)).unwrap_err();
        assert!(
            matches!(err, CoreError::InvalidValue { .. }),
            "{bad}: {err:?}"
        );
        assert_eq!(look(&w, p), "toon");
    }
    assert!(matches!(
        w.set(p, "LookOverride", Value::Number(1.0)),
        Err(CoreError::TypeMismatch { .. })
    ));
    let f = w.create("Folder", Some(ws)).unwrap();
    assert!(matches!(
        w.set(f, "LookOverride", s("toon")),
        Err(CoreError::UnknownProperty { .. })
    ));
    for sub in ["Part", "SpawnLocation", "Checkpoint"] {
        let id = w.create(sub, Some(ws)).unwrap();
        set_look(&mut w, id, "voxel");
        assert_eq!(look(&w, id), "voxel", "{sub}");
    }
    assert_eq!(look_override::display_name("retro-32"), Some("Retro / PS1"));
}

#[test]
fn the_nearest_model_wins_and_parts_do_not_pass_styles_on() {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    let outer = w.create("Model", Some(ws)).unwrap();
    let middle = w.create("Model", Some(outer)).unwrap();
    let inner = w.create("Model", Some(middle)).unwrap();
    let folder = w.create("Folder", Some(inner)).unwrap();
    let deep = w.create("Part", Some(folder)).unwrap();
    let child_of_part = w.create("Part", Some(deep)).unwrap();
    let loose = w.create("Part", Some(ws)).unwrap();
    let mut t = Tracked::new(&w);

    assert_eq!(resolve(&w, deep), None, "no override: the place Look");
    set_look(&mut w, outer, "toon");
    t.sync(&w);
    assert_eq!(
        resolve(&w, deep),
        Some("toon"),
        "through Models and a Folder"
    );
    set_look(&mut w, inner, "retro-32");
    t.sync(&w);
    assert_eq!(
        resolve(&w, deep),
        Some("retro-32"),
        "the nearest Model wins"
    );
    // `middle` stays inherit, so the nearest non-inherit Model above `deep` is `inner`.
    set_look(&mut w, deep, "voxel");
    t.sync(&w);
    assert_eq!(
        resolve(&w, deep),
        Some("voxel"),
        "the part's own value wins"
    );
    assert_eq!(
        resolve(&w, child_of_part),
        Some("retro-32"),
        "a part does not pass its style to a child part"
    );
    assert_eq!(resolve(&w, loose), None);
    assert_eq!(t.cache.get(deep), Some("voxel"));
}

#[test]
fn clearing_falls_back_step_by_step() {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    let outer = w.create("Model", Some(ws)).unwrap();
    let inner = w.create("Model", Some(outer)).unwrap();
    let p = w.create("Part", Some(inner)).unwrap();
    set_look(&mut w, outer, "soft");
    set_look(&mut w, inner, "low-poly");
    set_look(&mut w, p, "pixel-3d");
    let mut t = Tracked::new(&w);
    assert_eq!(t.cache.get(p), Some("pixel-3d"));

    set_look(&mut w, p, "inherit");
    assert_eq!(t.sync(&w), vec![p]);
    assert_eq!(t.cache.get(p), Some("low-poly"));
    set_look(&mut w, inner, "Inherit");
    assert_eq!(t.sync(&w), vec![p]);
    assert_eq!(t.cache.get(p), Some("soft"));
    set_look(&mut w, outer, "inherit");
    assert_eq!(t.sync(&w), vec![p]);
    assert_eq!(t.cache.get(p), None, "back to the place Look");
    assert_eq!(t.cache.overridden_parts(), 0);
}

#[test]
fn reparenting_updates_the_resolved_style() {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    let red = w.create("Model", Some(ws)).unwrap();
    let blue = w.create("Model", Some(ws)).unwrap();
    set_look(&mut w, red, "toon");
    set_look(&mut w, blue, "retro-32");
    let group = w.create("Model", Some(red)).unwrap();
    let a = w.create("Part", Some(group)).unwrap();
    let b = w.create("Part", Some(group)).unwrap();
    let mut t = Tracked::new(&w);
    assert_eq!(t.cache.get(a), Some("toon"));

    // A part moves between Models.
    w.set_parent(a, Some(blue)).unwrap();
    assert_eq!(t.sync(&w), vec![a]);
    assert_eq!(t.cache.get(a), Some("retro-32"));
    // A whole group moves: its parts follow.
    w.set_parent(group, Some(blue)).unwrap();
    assert_eq!(t.sync(&w), vec![b]);
    assert_eq!(t.cache.get(b), Some("retro-32"));
    // Out of every Model.
    w.set_parent(group, Some(ws)).unwrap();
    t.sync(&w);
    assert_eq!(t.cache.get(b), None);
    // Clones carry their own values and resolve under their new parent.
    set_look(&mut w, group, "voxel");
    let copy = w.clone_subtree(group).unwrap();
    w.set_parent(copy, Some(red)).unwrap();
    t.sync(&w);
    let copied_b = w.children(copy)[0];
    assert_eq!(t.cache.get(copied_b), Some("voxel"));
    // Destroying drops cached entries.
    w.destroy(blue);
    t.sync(&w);
    assert_eq!(t.cache.get(a), None);
}

#[test]
fn unrelated_model_edits_do_not_touch_its_parts() {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    let m = w.create("Model", Some(ws)).unwrap();
    set_look(&mut w, m, "toon");
    for _ in 0..50 {
        w.create("Part", Some(m)).unwrap();
    }
    let mut t = Tracked::new(&w);
    w.set(m, "Name", s("Renamed")).unwrap();
    assert!(t.sync(&w).is_empty());
    set_look(&mut w, m, "Toon");
    assert!(
        t.sync(&w).is_empty(),
        "the same style again changes nothing"
    );
    set_look(&mut w, m, "soft");
    assert_eq!(t.sync(&w).len(), 50);
}

#[test]
fn journal_overflow_rebuilds_the_cache() {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    let m = w.create("Model", Some(ws)).unwrap();
    let p = w.create("Part", Some(m)).unwrap();
    let mut t = Tracked::new(&w);
    set_look(&mut w, m, "toon");
    // Far more journal entries than the journal keeps.
    for i in 0..40_000 {
        w.set(p, "Transparency", Value::Number(f64::from(i % 2) * 0.5))
            .unwrap();
    }
    let (changes, cursor) = w.changes_since(t.cursor);
    assert_eq!(changes, Changes::Overflow);
    t.cursor = cursor;
    let changed = t.cache.apply(&w, &changes);
    assert_eq!(changed, vec![p]);
    assert_eq!(t.cache.get(p), Some("toon"));
}

#[test]
fn seeded_edits_keep_the_cache_equal_to_a_rebuild() {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    let mut models = vec![ws];
    let mut all: Vec<InstanceId> = Vec::new();
    let mut seed = 0x2222_u64;
    let mut next = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let mut t = Tracked::new(&w);
    for step in 0..400 {
        match next(6) {
            0 => {
                let parent = models[next(models.len())];
                if w.is_alive(parent) {
                    let m = w.create("Model", Some(parent)).unwrap();
                    models.push(m);
                    all.push(m);
                }
            }
            1 | 2 => {
                let parent = models[next(models.len())];
                if w.is_alive(parent) {
                    all.push(w.create("Part", Some(parent)).unwrap());
                }
            }
            3 if !all.is_empty() => {
                let id = all[next(all.len())];
                if w.is_alive(id) {
                    let v = look_override::LOOK_OVERRIDES[next(4)];
                    set_look(&mut w, id, v);
                }
            }
            4 if !all.is_empty() => {
                let id = all[next(all.len())];
                let parent = models[next(models.len())];
                if w.is_alive(id) && w.is_alive(parent) {
                    let _ = w.set_parent(id, Some(parent)); // cycles are refused
                }
            }
            5 if step % 7 == 0 && !all.is_empty() => {
                let id = all[next(all.len())];
                w.destroy(id);
            }
            _ => {}
        }
        t.sync(&w);
    }
    assert!(t.cache.overridden_parts() > 0, "the run set some overrides");
}

#[test]
fn scene_files_keep_overrides_and_omit_inherit() {
    let dir = TestDir::new("look_override_scene");
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").unwrap();
    let m = w.create("Model", Some(ws)).unwrap();
    set_look(&mut w, m, "Retro / PS1");
    let styled = w.create("Part", Some(m)).unwrap();
    set_look(&mut w, styled, "toon");
    let plain = w.create("Part", Some(m)).unwrap();
    let cleared = w.create("Part", Some(ws)).unwrap();
    set_look(&mut w, cleared, "voxel");
    set_look(&mut w, cleared, "inherit");
    let path = dir.file("looks.ron");
    save_scene(&w, &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.matches("LookOverride").count(), 2, "{text}");
    assert!(!text.contains("inherit"), "{text}");

    let back = load_scene(&path).unwrap();
    let ws2 = back.root("Workspace").unwrap();
    let m2 = back.children(ws2)[0];
    let kids = back.children(m2);
    assert_eq!(look(&back, m2), "retro-32");
    assert_eq!(look(&back, kids[0]), "toon");
    assert_eq!(resolve(&back, kids[0]), Some("toon"));
    assert_eq!(resolve(&back, kids[1]), Some("retro-32"));
    let _ = (plain, styled);
}
