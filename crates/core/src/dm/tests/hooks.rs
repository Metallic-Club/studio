//! Class behaviour hooks: each is called once per class in the chain, root class first, with the right arguments.

use std::cell::RefCell;

use super::classes::{game_with, registry_with};
use super::*;
use crate::{ClassBehavior, ClassDesc, PropId};

thread_local! {
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn log(entry: String) {
    LOG.with(|l| l.borrow_mut().push(entry));
}

fn take_log() -> Vec<String> {
    LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

/// Records every hook as `"<who> <hook> <args>"`.
struct Recorder(&'static str);

impl ClassBehavior for Recorder {
    fn on_created(&self, dm: &mut DataModel, id: Id) {
        assert_eq!(dm.parent(id), Ok(None), "created before it has a parent");
        log(format!("{} created {}", self.0, id.index()));
    }

    fn on_prop_changed(&self, dm: &mut DataModel, id: Id, prop: PropId) {
        log(format!(
            "{} prop {} {}",
            self.0,
            id.index(),
            dm.registry().props(dm.class_of(id).expect("class"))[prop.index()]
                .meta
                .name
        ));
        // Hooks may write: mirror Transparency into Reflectance (which re-enters `set` once, then is a no-op).
        if self.0 == "leaf"
            && let (Ok(t), Ok(r)) = (dm.prop_id(id, "Transparency"), dm.prop_id(id, "Reflectance"))
            && prop == t
        {
            let value = dm.get(id, t).expect("read");
            dm.set(id, r, value).expect("mirror");
        }
    }

    fn on_parent_changed(&self, _: &mut DataModel, id: Id, old: Option<Id>, new: Option<Id>) {
        let index = |x: Option<Id>| x.map_or("none".to_owned(), |x| x.index().to_string());
        log(format!(
            "{} parent {} {} -> {}",
            self.0,
            id.index(),
            index(old),
            index(new)
        ));
    }

    fn on_destroying(&self, dm: &mut DataModel, id: Id) {
        assert!(dm.contains(id), "still intact");
        log(format!("{} destroying {}", self.0, id.index()));
    }
}

static BASE: Recorder = Recorder("base");
static LEAF: Recorder = Recorder("leaf");

static HOOKED_BASE: ClassDesc = ClassDesc::new("HookedBase", Some("Part")).behavior(&BASE);
static HOOKED: ClassDesc = ClassDesc::new("Hooked", Some("HookedBase")).creatable().behavior(&LEAF);

#[test]
fn every_hook_runs_once_per_class_in_the_chain() {
    let registry = registry_with(&[&HOOKED_BASE, &HOOKED]);
    let hooked = registry.class_id("Hooked").expect("class");
    let (mut dm, ids) = game_with(registry);
    take_log();

    let h = dm.create(hooked).expect("create");
    let n = h.index();
    assert_eq!(
        take_log(),
        vec![format!("base created {n}"), format!("leaf created {n}")]
    );

    dm.set(h, ids.anchored, Value::Bool(true)).expect("set");
    assert_eq!(
        take_log(),
        vec![format!("base prop {n} Anchored"), format!("leaf prop {n} Anchored")]
    );
    dm.set(h, ids.anchored, Value::Bool(true)).expect("no-op");
    assert!(take_log().is_empty(), "equal writes run no hooks");

    dm.set(h, ids.transparency, number(0.5)).expect("set");
    assert_eq!(
        take_log(),
        vec![
            format!("base prop {n} Transparency"),
            format!("leaf prop {n} Transparency"),
            format!("base prop {n} Reflectance"),
            format!("leaf prop {n} Reflectance"),
        ]
    );
    assert_eq!(dm.number(h, ids.reflectance), Ok(0.5));

    let ws = ids.workspace.index();
    dm.set_parent(h, Some(ids.workspace)).expect("parent");
    assert_eq!(
        take_log(),
        vec![
            format!("base parent {n} none -> {ws}"),
            format!("leaf parent {n} none -> {ws}")
        ]
    );

    let child = dm.create(hooked).expect("create");
    dm.set_parent(child, Some(h)).expect("parent");
    take_log();
    dm.destroy(h).expect("destroy");
    let c = child.index();
    assert_eq!(
        take_log(),
        vec![
            format!("base destroying {n}"),
            format!("leaf destroying {n}"),
            format!("base destroying {c}"),
            format!("leaf destroying {c}"),
        ]
    );

    // Classes without behaviours run nothing.
    let p = part(&mut dm, &ids);
    dm.set(p, ids.anchored, Value::Bool(true)).expect("set");
    dm.destroy(p).expect("destroy");
    assert!(take_log().is_empty());
}

#[test]
fn clones_run_on_created_parent_first() {
    let registry = registry_with(&[&HOOKED_BASE, &HOOKED]);
    let hooked = registry.class_id("Hooked").expect("class");
    let (mut dm, _ids) = game_with(registry);
    let a = dm.create(hooked).expect("create");
    let b = dm.create(hooked).expect("create");
    dm.set_parent(b, Some(a)).expect("parent");
    take_log();
    let copy = dm.clone_tree(a).expect("clone").expect("archivable");
    let copy_child = kids(&dm, copy)[0];
    assert_eq!(
        take_log(),
        vec![
            format!("base created {}", copy.index()),
            format!("leaf created {}", copy.index()),
            format!("base created {}", copy_child.index()),
            format!("leaf created {}", copy_child.index()),
        ]
    );
}
