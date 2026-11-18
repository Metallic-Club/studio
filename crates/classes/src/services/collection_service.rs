//! `CollectionService` (#809): instances found by their tags, as Roblox's.
//!
//! Tags live on the instances themselves (`metallic_core`'s `DataModel::set_tag`, saved with the scene); the
//! service's methods only read and write them. `GetTagged` and `GetAllTags` look at the game's tree, so an instance
//! outside it (with no parent, or destroyed) is not found.
//!
//! # The added and removed signals
//!
//! `GetInstanceAddedSignal(tag)` and `GetInstanceRemovedSignal(tag)` return the signals of a hidden
//! [`TAG_SIGNALS`] instance made for that tag the first time it is asked for, under the service (and so never saved).
//! The engine fires them through [`TagEvents::step`] once per frame, before the frame's scripts: for each instance
//! that gained the tag while in the game, or came into the game with it, `InstanceAdded`; for each that lost it or
//! left the game, `InstanceRemoved`. Like Roblox's deferred events, they fire after the change, not inside it. What
//! was in the game when it started fires nothing (`GetTagged` finds it).

use std::collections::HashMap;
use std::sync::Arc;

use metallic_core::{
    ClassDesc, ConsumerId, DataModel, DirtyBits, Id, MethodCall, MethodDesc, Param, ScriptError, ScriptValue,
    Ty,
};

/// The hidden class whose instances carry one tag's signals.
pub const TAG_SIGNALS: &str = "CollectionTag";

/// Finds instances by their tags.
pub static CLASS: ClassDesc = ClassDesc::new("CollectionService", Some("Instance"))
    .describe("Finds instances by the tags they were given with AddTag, and says when tagged instances come and go.")
    .service(13)
    .hidden_in_explorer()
    .methods(METHODS);

static METHODS: &[MethodDesc] = &[
    MethodDesc::new("AddTag", add_tag)
        .params(&[Param::new("instance", Ty::Class("Instance")), Param::new("tag", Ty::String)])
        .returns_nothing()
        .describe("Gives the instance the tag. Adding one it has does nothing."),
    MethodDesc::new("RemoveTag", remove_tag)
        .params(&[Param::new("instance", Ty::Class("Instance")), Param::new("tag", Ty::String)])
        .returns_nothing()
        .describe("Takes the tag away from the instance. Removing one it lacks does nothing."),
    MethodDesc::new("HasTag", has_tag)
        .params(&[Param::new("instance", Ty::Class("Instance")), Param::new("tag", Ty::String)])
        .returns(Ty::Bool)
        .describe("Returns whether the instance has the tag."),
    MethodDesc::new("GetTags", get_tags)
        .params(&[Param::new("instance", Ty::Class("Instance"))])
        .returns(Ty::Array(&Ty::String))
        .describe("Returns the instance's tags, in the order they were added."),
    MethodDesc::new("GetTagged", get_tagged)
        .params(&[Param::new("tag", Ty::String)])
        .returns(Ty::Array(&Ty::Class("Instance")))
        .describe("Returns every instance in the game with the tag, in tree order."),
    MethodDesc::new("GetAllTags", get_all_tags)
        .params(&[])
        .returns(Ty::Array(&Ty::String))
        .describe("Returns every tag an instance in the game has, each once."),
    MethodDesc::new("GetInstanceAddedSignal", get_added_signal)
        .params(&[Param::new("tag", Ty::String)])
        .returns(Ty::Signal(&Ty::Class("Instance")))
        .describe(
            "Returns a signal that fires with each instance that gains the tag while in the game, or comes into the \
             game with it.",
        ),
    MethodDesc::new("GetInstanceRemovedSignal", get_removed_signal)
        .params(&[Param::new("tag", Ty::String)])
        .returns(Ty::Signal(&Ty::Class("Instance")))
        .describe("Returns a signal that fires with each instance that loses the tag, or leaves the game with it."),
];

fn add_tag(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let instance = call.args.instance(0)?;
    let tag = call.args.string(1)?;
    call.dm.set_tag(instance, &tag, true)?;
    Ok(ScriptValue::Nil)
}

fn remove_tag(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let instance = call.args.instance(0)?;
    let tag = call.args.string(1)?;
    call.dm.set_tag(instance, &tag, false)?;
    Ok(ScriptValue::Nil)
}

fn has_tag(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let instance = call.args.instance(0)?;
    let tag = call.args.string(1)?;
    Ok(ScriptValue::Bool(call.dm.has_tag(instance, &tag)?))
}

fn get_tags(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let instance = call.args.instance(0)?;
    crate::base::instance::tag_list(call.dm, instance)
}

/// Every instance in the game's tree, in tree order.
fn in_game(dm: &DataModel) -> impl Iterator<Item = Id> + '_ {
    dm.root()
        .and_then(|root| dm.descendants(root).ok())
        .into_iter()
        .flatten()
}

fn get_tagged(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let tag = call.args.string(0)?;
    let dm = &*call.dm;
    Ok(ScriptValue::List(
        in_game(dm)
            .filter(|&id| dm.has_tag(id, &tag).unwrap_or(false))
            .map(ScriptValue::Instance)
            .collect(),
    ))
}

fn get_all_tags(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let dm = &*call.dm;
    let mut all: Vec<Arc<str>> = Vec::new();
    for id in in_game(dm) {
        for tag in dm.tags(id).unwrap_or_default() {
            if !all.contains(tag) {
                all.push(Arc::clone(tag));
            }
        }
    }
    Ok(ScriptValue::List(all.into_iter().map(ScriptValue::String).collect()))
}

fn get_added_signal(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    tag_signal(call, "InstanceAdded")
}

fn get_removed_signal(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    tag_signal(call, "InstanceRemoved")
}

/// The signal `name` of the tag passed first, making the tag's [`TAG_SIGNALS`] instance when there is none.
fn tag_signal(call: &mut MethodCall<'_>, name: &str) -> Result<ScriptValue, ScriptError> {
    let tag = call.args.string(0)?;
    let dm = &mut *call.dm;
    let holder = match holder(dm, call.this, &tag) {
        Some(holder) => holder,
        None => {
            let class = dm
                .registry()
                .class_id(TAG_SIGNALS)
                .ok_or_else(|| ScriptError::runtime(format!("there is no {TAG_SIGNALS} class")))?;
            let holder = dm.create_internal(class)?;
            dm.set_name(holder, Arc::clone(&tag))?;
            dm.set_parent(holder, Some(call.this))?;
            holder
        }
    };
    Ok(ScriptValue::Signal(holder, dm.signal_id(holder, name)?))
}

/// The [`TAG_SIGNALS`] instance of `tag` under `service`, if a script asked for its signals.
fn holder(dm: &DataModel, service: Id, tag: &str) -> Option<Id> {
    let class = dm.registry().class_id(TAG_SIGNALS)?;
    dm.children(service)
        .ok()?
        .find(|&child| dm.class_of(child).ok() == Some(class) && dm.name(child) == Ok(tag))
}

/// What fires the tags' added and removed signals (see the [module documentation](self)): the engine starts one with
/// the game and steps it once a frame.
#[derive(Debug, Default)]
pub struct TagEvents {
    /// Watching the tree and the tags, while the game runs.
    consumer: Option<ConsumerId>,
    /// The tags of every tagged instance in the game, as last seen.
    known: HashMap<Id, Vec<Arc<str>>>,
    changes: Vec<(Id, DirtyBits)>,
    removed: Vec<Id>,
}

impl TagEvents {
    /// Watching nothing until [`TagEvents::start`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts watching the game, as it starts: what is tagged in it now was there before.
    pub fn start(&mut self, dm: &mut DataModel) -> Result<(), metallic_core::DmError> {
        self.stop(dm);
        self.consumer = Some(dm.register_consumer(DirtyBits::STRUCTURE | DirtyBits::TAGS)?);
        self.known = in_game(dm)
            .filter_map(|id| {
                let tags = dm.tags(id).ok()?;
                (!tags.is_empty()).then(|| (id, tags.to_vec()))
            })
            .collect();
        Ok(())
    }

    /// Fires the signals of what changed since the last step: only the tags a script asked for signals of.
    pub fn step(&mut self, dm: &mut DataModel) {
        let Some(consumer) = self.consumer else {
            return;
        };
        let Some(service) = dm.registry().class_id(CLASS.name).and_then(|class| dm.service(class)) else {
            return;
        };
        dm.drain_changes(consumer, &mut self.changes);
        dm.drain_removed(consumer, &mut self.removed);
        let mut events: Vec<(Arc<str>, &'static str, Id)> = Vec::new();
        for id in self.removed.drain(..) {
            for tag in self.known.remove(&id).unwrap_or_default() {
                events.push((tag, "InstanceRemoved", id));
            }
        }
        let root = dm.root();
        for (id, _) in self.changes.drain(..) {
            let tags = dm.tags(id).unwrap_or_default();
            if tags.is_empty() && !self.known.contains_key(&id) {
                // Almost every moved instance: untagged, and never seen tagged.
                continue;
            }
            let here = root.is_some_and(|root| dm.is_descendant_of(id, root).unwrap_or(false));
            let now: Vec<Arc<str>> = if here { tags.to_vec() } else { Vec::new() };
            let before = self.known.remove(&id).unwrap_or_default();
            for tag in before.iter().filter(|tag| !now.contains(tag)) {
                events.push((Arc::clone(tag), "InstanceRemoved", id));
            }
            for tag in now.iter().filter(|tag| !before.contains(tag)) {
                events.push((Arc::clone(tag), "InstanceAdded", id));
            }
            if !now.is_empty() {
                self.known.insert(id, now);
            }
        }
        for (tag, signal, id) in events {
            let Some(holder) = holder(dm, service, &tag) else {
                continue;
            };
            // Only a destroyed holder fails, and then nothing listens.
            if let Ok(signal) = dm.signal_id(holder, signal) {
                let _ = dm.fire(holder, signal, vec![ScriptValue::Instance(id)]);
            }
        }
    }

    /// Stops watching the game (when it stops).
    pub fn stop(&mut self, dm: &mut DataModel) {
        if let Some(consumer) = self.consumer.take() {
            dm.unregister_consumer(consumer);
        }
        self.known.clear();
    }
}

#[cfg(test)]
mod tests {
    use metallic_core::{ScriptValue, Value};

    use crate::testing::{call, game, new_in, workspace};

    #[test]
    fn tags_are_kept_once_each_and_found_in_the_game() {
        let mut dm = game();
        let ws = workspace(&dm);
        let class = dm.registry().class_id("CollectionService").unwrap();
        let service = dm.service(class).unwrap();
        let part = new_in(&mut dm, "Part", ws, "P");
        let loose = dm.create(dm.registry().class_id("Part").unwrap()).unwrap();
        let tag = |s: &str| ScriptValue::String(s.into());
        for id in [part, loose] {
            call(&mut dm, service, "AddTag", vec![ScriptValue::Instance(id), tag("Lava")]).unwrap();
        }
        call(&mut dm, part, "AddTag", vec![tag("Lava")]).unwrap();
        call(&mut dm, part, "AddTag", vec![tag("Hot")]).unwrap();
        assert_eq!(
            call(&mut dm, part, "GetTags", vec![]),
            Ok(ScriptValue::List(vec![tag("Lava"), tag("Hot")]))
        );
        assert_eq!(
            call(&mut dm, service, "GetTagged", vec![tag("Lava")]),
            Ok(ScriptValue::List(vec![ScriptValue::Instance(part)])),
            "not the instance outside the game"
        );
        assert_eq!(
            call(&mut dm, service, "GetAllTags", vec![]),
            Ok(ScriptValue::List(vec![tag("Lava"), tag("Hot")]))
        );
        call(&mut dm, service, "RemoveTag", vec![ScriptValue::Instance(part), tag("Lava")]).unwrap();
        assert_eq!(
            call(&mut dm, service, "HasTag", vec![ScriptValue::Instance(part), tag("Lava")]),
            Ok(ScriptValue::Bool(false))
        );
        // A tag survives a clone.
        let copy = dm.clone_tree(part).unwrap().unwrap();
        assert!(dm.has_tag(copy, "Hot").unwrap());

        // One holder per tag, under the service, whatever is asked for.
        let added = call(&mut dm, service, "GetInstanceAddedSignal", vec![tag("Lava")]).unwrap();
        let removed = call(&mut dm, service, "GetInstanceRemovedSignal", vec![tag("Lava")]).unwrap();
        let (ScriptValue::Signal(a, _), ScriptValue::Signal(b, _)) = (&added, &removed) else {
            panic!("{added:?} {removed:?}");
        };
        assert_eq!(a, b);
        assert_eq!(dm.get_by_name(*a, "Name"), Ok(Value::String("Lava".into())));
        assert_eq!(dm.child_count(service), Ok(1));
    }
}
