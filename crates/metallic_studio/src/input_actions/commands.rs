//! Undo-ready edits of the instance tree.
//!
//! Every change the panel makes goes through a [`StepBuilder`]: it applies each edit to the
//! [`World`] at once (so a later edit can use an instance an earlier one created) and records
//! it. [`StepBuilder::finish`] returns one [`EditStep`], which is one undo step whatever it
//! contains: a rename, a preset that replaces twenty bindings, a touch control moved with the
//! mouse. The panel hands finished steps to a [`CommandSink`]; [`History`] is the sink used
//! until the editor's shared undo stack (#10) exists, and that stack only needs to implement
//! the same trait.
//!
//! Destroying an instance cannot be undone in place (ids are generational and never come back),
//! so a destroyed subtree is kept as a [`Snapshot`] and rebuilt with new ids. Every rebuild
//! returns an [`IdMap`] from the old ids to the new ones, and [`History`] rewrites the ids in all
//! of its steps with it, so a step that renamed an instance before it was deleted still undoes
//! after the delete was undone.

use std::collections::HashMap;

use metallic_core::classes::{all_properties, default_value_ref};
use metallic_core::{CoreError, InstanceId, Value, World};

/// Old instance id to new instance id, after a destroyed subtree was rebuilt.
pub type IdMap = HashMap<InstanceId, InstanceId>;

/// A subtree kept as data: class, the properties that differ from the class default, children.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// Class name.
    pub class: String,
    /// Properties to set after creating the instance, in order.
    pub props: Vec<(String, Value)>,
    /// Children, in tree order.
    pub children: Vec<Snapshot>,
}

impl Snapshot {
    /// An instance of `class` with only its defaults.
    pub fn new(class: &str) -> Self {
        Self {
            class: class.to_string(),
            props: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Sets a property (replacing an earlier value of the same property).
    pub fn with(mut self, prop: &str, value: Value) -> Self {
        self.props.retain(|(p, _)| p != prop);
        self.props.push((prop.to_string(), value));
        self
    }

    /// Adds a child.
    pub fn child(mut self, child: Snapshot) -> Self {
        self.children.push(child);
        self
    }

    /// The value this snapshot sets for `prop`, if any.
    pub fn prop(&self, prop: &str) -> Option<&Value> {
        self.props.iter().find(|(p, _)| p == prop).map(|(_, v)| v)
    }

    /// Reads an instance and its descendants. `None` for a destroyed id.
    pub fn capture(world: &World, id: InstanceId) -> Option<Snapshot> {
        let class = world.class_name(id)?;
        let mut snap = Snapshot::new(class);
        for def in all_properties(class) {
            let Ok(value) = world.get_ref(id, def.name) else {
                continue;
            };
            if default_value_ref(class, def.name) != Some(value) {
                snap.props.push((def.name.to_string(), value.clone()));
            }
        }
        for &child in world.children(id) {
            if let Some(c) = Snapshot::capture(world, child) {
                snap.children.push(c);
            }
        }
        Some(snap)
    }

    /// Creates the subtree under `parent` (as its last child). Returns the new ids in pre-order.
    fn build(&self, world: &mut World, parent: InstanceId) -> Result<Vec<InstanceId>, CoreError> {
        let mut ids = Vec::new();
        if let Err(e) = self.build_into(world, parent, &mut ids) {
            if let Some(&root) = ids.first() {
                world.destroy(root);
            }
            return Err(e);
        }
        Ok(ids)
    }

    fn build_into(
        &self,
        world: &mut World,
        parent: InstanceId,
        ids: &mut Vec<InstanceId>,
    ) -> Result<(), CoreError> {
        let id = world.create(&self.class, Some(parent))?;
        ids.push(id);
        for (prop, value) in &self.props {
            world.set(id, prop, value.clone())?;
        }
        for child in &self.children {
            child.build_into(world, id, ids)?;
        }
        Ok(())
    }
}

/// Ids of a live subtree in pre-order (the order [`Snapshot::build`] creates them in).
fn preorder(world: &World, root: InstanceId) -> Vec<InstanceId> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        out.push(id);
        stack.extend(world.children(id).iter().rev());
    }
    out
}

/// Moves the last child of `parent` to `index`.
fn move_last_to(world: &mut World, parent: InstanceId, index: usize) -> Result<(), CoreError> {
    let Some(&last) = world.children(parent).last() else {
        return Ok(());
    };
    world.set_parent_at(last, Some(parent), index)
}

/// One change of the tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// A property changed from `old` to `new`.
    Set {
        /// The instance.
        id: InstanceId,
        /// The property.
        prop: String,
        /// Value before.
        old: Value,
        /// Value after.
        new: Value,
    },
    /// A subtree was created at `index` among the children of `parent`.
    Create {
        /// The parent.
        parent: InstanceId,
        /// Position among the parent's children.
        index: usize,
        /// What was created.
        snapshot: Snapshot,
        /// The ids of the created subtree in pre-order, while it exists.
        ids: Vec<InstanceId>,
    },
    /// A subtree was destroyed; it sat at `index` among the children of `parent`.
    Destroy {
        /// The parent.
        parent: InstanceId,
        /// Position among the parent's children.
        index: usize,
        /// What was destroyed.
        snapshot: Snapshot,
        /// The ids the subtree had in pre-order (valid again after an undo).
        ids: Vec<InstanceId>,
    },
}

impl Edit {
    fn create(
        world: &mut World,
        parent: InstanceId,
        index: usize,
        snapshot: &Snapshot,
    ) -> Result<Vec<InstanceId>, CoreError> {
        let ids = snapshot.build(world, parent)?;
        move_last_to(world, parent, index)?;
        Ok(ids)
    }

    /// Does the edit (again). New ids made by rebuilding a subtree go into `map`.
    fn redo(&mut self, world: &mut World, map: &mut IdMap) -> Result<(), CoreError> {
        match self {
            Edit::Set { id, prop, new, .. } => world.set(*id, prop, new.clone()),
            Edit::Create {
                parent,
                index,
                snapshot,
                ids,
            } => {
                let fresh = Self::create(world, *parent, *index, snapshot)?;
                map.extend(ids.iter().copied().zip(fresh.iter().copied()));
                *ids = fresh;
                Ok(())
            }
            Edit::Destroy { ids, .. } => {
                let root = *ids.first().ok_or(CoreError::InvalidInstance)?;
                if !world.is_alive(root) {
                    return Err(CoreError::InvalidInstance);
                }
                world.destroy(root);
                Ok(())
            }
        }
    }

    /// Takes the edit back. New ids made by rebuilding a subtree go into `map`.
    fn undo(&mut self, world: &mut World, map: &mut IdMap) -> Result<(), CoreError> {
        match self {
            Edit::Set { id, prop, old, .. } => world.set(*id, prop, old.clone()),
            Edit::Create { ids, .. } => {
                let root = *ids.first().ok_or(CoreError::InvalidInstance)?;
                if !world.is_alive(root) {
                    return Err(CoreError::InvalidInstance);
                }
                world.destroy(root);
                Ok(())
            }
            Edit::Destroy {
                parent,
                index,
                snapshot,
                ids,
            } => {
                let fresh = Self::create(world, *parent, *index, snapshot)?;
                map.extend(ids.iter().copied().zip(fresh.iter().copied()));
                *ids = fresh;
                Ok(())
            }
        }
    }

    fn remap(&mut self, map: &IdMap) {
        let fix = |id: &mut InstanceId| {
            if let Some(&n) = map.get(id) {
                *id = n;
            }
        };
        match self {
            Edit::Set { id, .. } => fix(id),
            Edit::Create { parent, ids, .. } | Edit::Destroy { parent, ids, .. } => {
                fix(parent);
                ids.iter_mut().for_each(fix);
            }
        }
    }
}

/// One undo step: a label for the Edit menu and the edits in the order they were made.
#[derive(Debug, Clone, PartialEq)]
pub struct EditStep {
    /// What the step did, for "Undo <label>".
    pub label: String,
    /// The edits, in order.
    pub edits: Vec<Edit>,
    /// Consecutive steps with the same key merge into one (a value dragged over many frames).
    pub merge_key: Option<String>,
}

impl EditStep {
    /// Takes the step back (edits in reverse order). New ids go into the returned map.
    pub fn undo(&mut self, world: &mut World) -> Result<IdMap, CoreError> {
        let mut map = IdMap::new();
        for i in (0..self.edits.len()).rev() {
            let (done, rest) = self.edits.split_at_mut(i);
            rest[0].undo(world, &mut map)?;
            remap_all(done, &map);
        }
        Ok(map)
    }

    /// Does the step again. New ids go into the returned map.
    pub fn redo(&mut self, world: &mut World) -> Result<IdMap, CoreError> {
        let mut map = IdMap::new();
        for i in 0..self.edits.len() {
            let (_, rest) = self.edits.split_at_mut(i);
            let (cur, later) = rest.split_at_mut(1);
            cur[0].redo(world, &mut map)?;
            remap_all(later, &map);
        }
        Ok(map)
    }

    fn remap(&mut self, map: &IdMap) {
        remap_all(&mut self.edits, map);
    }
}

fn remap_all(edits: &mut [Edit], map: &IdMap) {
    if map.is_empty() {
        return;
    }
    for e in edits {
        e.remap(map);
    }
}

/// Applies edits to a world and records them as one step.
pub struct StepBuilder<'w> {
    world: &'w mut World,
    edits: Vec<Edit>,
}

impl<'w> StepBuilder<'w> {
    /// Starts an empty step on `world`.
    pub fn new(world: &'w mut World) -> Self {
        Self {
            world,
            edits: Vec::new(),
        }
    }

    /// Read access to the world while building.
    pub fn world(&self) -> &World {
        self.world
    }

    /// Sets a property. Setting the value it already has records nothing.
    pub fn set(&mut self, id: InstanceId, prop: &str, value: Value) -> Result<(), CoreError> {
        let old = self.world.get(id, prop)?;
        if old == value {
            return Ok(());
        }
        self.world.set(id, prop, value.clone())?;
        // Two sets of one property in one step keep the first old value.
        if let Some(Edit::Set { new, .. }) = self
            .edits
            .iter_mut()
            .rev()
            .find(|e| matches!(e, Edit::Set { id: i, prop: p, .. } if *i == id && p == prop))
        {
            *new = value;
            return Ok(());
        }
        self.edits.push(Edit::Set {
            id,
            prop: prop.to_string(),
            old,
            new: value,
        });
        Ok(())
    }

    /// Creates `snapshot` as the last child of `parent`; returns the root of the new subtree.
    pub fn create(
        &mut self,
        parent: InstanceId,
        snapshot: Snapshot,
    ) -> Result<InstanceId, CoreError> {
        let index = self.world.children(parent).len();
        self.create_at(parent, index, snapshot)
    }

    /// Creates `snapshot` at `index` among the children of `parent`.
    pub fn create_at(
        &mut self,
        parent: InstanceId,
        index: usize,
        snapshot: Snapshot,
    ) -> Result<InstanceId, CoreError> {
        if !self.world.is_alive(parent) {
            return Err(CoreError::InvalidInstance);
        }
        let index = index.min(self.world.children(parent).len());
        let ids = Edit::create(self.world, parent, index, &snapshot)?;
        let root = ids[0];
        self.edits.push(Edit::Create {
            parent,
            index,
            snapshot,
            ids,
        });
        Ok(root)
    }

    /// Destroys an instance and its descendants. Built-in services and roots are refused.
    pub fn destroy(&mut self, id: InstanceId) -> Result<(), CoreError> {
        let parent = self.world.parent(id).ok_or(CoreError::InvalidInstance)?;
        if self.world.is_builtin(id) {
            return Err(CoreError::InvalidInstance);
        }
        let snapshot = Snapshot::capture(self.world, id).ok_or(CoreError::InvalidInstance)?;
        let index = self
            .world
            .children(parent)
            .iter()
            .position(|&c| c == id)
            .unwrap_or(0);
        let ids = preorder(self.world, id);
        self.world.destroy(id);
        self.edits.push(Edit::Destroy {
            parent,
            index,
            snapshot,
            ids,
        });
        Ok(())
    }

    /// True when nothing was changed yet.
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// Takes back everything this builder did (for a gesture that was cancelled or failed half way).
    pub fn abort(self) {
        let mut step = EditStep {
            label: String::new(),
            edits: self.edits,
            merge_key: None,
        };
        let _ = step.undo(self.world);
    }

    /// The finished step; `None` when nothing changed.
    pub fn finish(self, label: impl Into<String>) -> Option<EditStep> {
        (!self.edits.is_empty()).then(|| EditStep {
            label: label.into(),
            edits: self.edits,
            merge_key: None,
        })
    }

    /// Like [`Self::finish`], with a merge key: the sink folds it into the previous step when
    /// that step has the same key and nothing sealed it.
    pub fn finish_merging(
        self,
        label: impl Into<String>,
        key: impl Into<String>,
    ) -> Option<EditStep> {
        let key = key.into();
        self.finish(label).map(|mut s| {
            s.merge_key = Some(key);
            s
        })
    }
}

/// Where finished steps go: the panel's own [`History`] now, the editor's undo stack after #10.
pub trait CommandSink {
    /// Takes a step that is already applied to the world.
    fn record(&mut self, step: EditStep);
    /// Ends a gesture: the next step never merges into the last one.
    fn seal(&mut self) {}
}

/// Undo and redo stacks for the panel.
#[derive(Debug, Default)]
pub struct History {
    undo: Vec<EditStep>,
    redo: Vec<EditStep>,
    sealed: bool,
    last_map: IdMap,
}

/// Most steps kept; the oldest are dropped first.
pub const HISTORY_LIMIT: usize = 200;

impl History {
    /// Empty stacks.
    pub fn new() -> Self {
        Self::default()
    }

    /// True when there is a step to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// True when there is a step to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Label of the step [`Self::undo`] would take back.
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|s| s.label.as_str())
    }

    /// Label of the step [`Self::redo`] would do again.
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|s| s.label.as_str())
    }

    /// Number of undo steps.
    pub fn len(&self) -> usize {
        self.undo.len()
    }

    /// True when there is nothing to undo.
    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }

    /// Ids that the last undo or redo rebuilt (old id to new id), for selections to follow.
    pub fn last_remap(&self) -> &IdMap {
        &self.last_map
    }

    fn remap_stacks(&mut self, map: &IdMap) {
        for s in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            s.remap(map);
        }
    }

    /// Takes back the last step. Returns false when there was none or it no longer applies.
    pub fn undo(&mut self, world: &mut World) -> bool {
        let Some(mut step) = self.undo.pop() else {
            return false;
        };
        self.sealed = true;
        match step.undo(world) {
            Ok(map) => {
                self.remap_stacks(&map);
                self.last_map = map;
                self.redo.push(step);
                true
            }
            Err(e) => {
                log::warn!("undo of `{}` failed: {e}", step.label);
                // A half-applied step cannot be trusted; drop both stacks rather than corrupt more.
                self.undo.clear();
                self.redo.clear();
                false
            }
        }
    }

    /// Does the last undone step again.
    pub fn redo(&mut self, world: &mut World) -> bool {
        let Some(mut step) = self.redo.pop() else {
            return false;
        };
        self.sealed = true;
        match step.redo(world) {
            Ok(map) => {
                self.remap_stacks(&map);
                self.last_map = map;
                self.undo.push(step);
                true
            }
            Err(e) => {
                log::warn!("redo of `{}` failed: {e}", step.label);
                self.undo.clear();
                self.redo.clear();
                false
            }
        }
    }
}

impl CommandSink for History {
    fn record(&mut self, step: EditStep) {
        self.redo.clear();
        if !self.sealed
            && step.merge_key.is_some()
            && let Some(top) = self.undo.last_mut()
            && top.merge_key == step.merge_key
        {
            for edit in step.edits {
                // Keep the first old value of a property and the last new one.
                if let Edit::Set { id, prop, new, .. } = &edit
                    && let Some(Edit::Set { new: top_new, .. }) = top.edits.iter_mut().find(
                        |e| matches!(e, Edit::Set { id: i, prop: p, .. } if i == id && p == prop),
                    )
                {
                    *top_new = new.clone();
                    continue;
                }
                top.edits.push(edit);
            }
            return;
        }
        self.undo.push(step);
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.sealed = false;
    }

    fn seal(&mut self) {
        self.sealed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_with_context() -> (World, InstanceId, InstanceId) {
        let mut w = World::new_with_services();
        let rs = w.root("ReplicatedStorage").unwrap();
        let ctx = w.create("InputContext", Some(rs)).unwrap();
        w.set(ctx, "Name", Value::String("Gameplay".into()))
            .unwrap();
        (w, rs, ctx)
    }

    fn name(w: &World, id: InstanceId) -> String {
        match w.get(id, "Name") {
            Ok(Value::String(s)) => s,
            _ => String::new(),
        }
    }

    #[test]
    fn set_undo_redo() {
        let (mut w, _, ctx) = world_with_context();
        let mut h = History::new();
        let mut b = StepBuilder::new(&mut w);
        b.set(ctx, "Priority", Value::Number(5.0)).unwrap();
        b.set(ctx, "Priority", Value::Number(7.0)).unwrap();
        let step = b.finish("Priority").unwrap();
        assert_eq!(
            step.edits.len(),
            1,
            "two sets of one property fold into one edit"
        );
        h.record(step);
        assert_eq!(w.get(ctx, "Priority").unwrap(), Value::Number(7.0));
        assert!(h.undo(&mut w));
        assert_eq!(w.get(ctx, "Priority").unwrap(), Value::Number(0.0));
        assert!(h.redo(&mut w));
        assert_eq!(w.get(ctx, "Priority").unwrap(), Value::Number(7.0));
    }

    #[test]
    fn no_op_set_records_nothing() {
        let (mut w, _, ctx) = world_with_context();
        let mut b = StepBuilder::new(&mut w);
        b.set(ctx, "Priority", Value::Number(0.0)).unwrap();
        assert!(b.finish("x").is_none());
    }

    #[test]
    fn delete_then_undo_restores_order_and_follows_ids() {
        let (mut w, _, ctx) = world_with_context();
        let mut ids = Vec::new();
        for n in ["A", "B", "C"] {
            let a = w.create("InputAction", Some(ctx)).unwrap();
            w.set(a, "Name", Value::String(n.into())).unwrap();
            let bind = w.create("InputBinding", Some(a)).unwrap();
            w.set(bind, "KeyCode", Value::String("E".into())).unwrap();
            ids.push(a);
        }
        let mut h = History::new();
        // Rename B, then delete it: undoing both must give back B with its old name.
        let mut b = StepBuilder::new(&mut w);
        b.set(ids[1], "Name", Value::String("Renamed".into()))
            .unwrap();
        h.record(b.finish("Rename").unwrap());
        let mut b = StepBuilder::new(&mut w);
        b.destroy(ids[1]).unwrap();
        h.record(b.finish("Delete").unwrap());
        assert_eq!(w.children(ctx).len(), 2);

        assert!(h.undo(&mut w));
        let kids = w.children(ctx).to_vec();
        let names: Vec<String> = kids.iter().map(|&k| name(&w, k)).collect();
        assert_eq!(names, ["A", "Renamed", "C"]);
        let restored = kids[1];
        assert_eq!(w.children(restored).len(), 1, "binding came back too");
        assert_eq!(h.last_remap().get(&ids[1]), Some(&restored));

        assert!(h.undo(&mut w));
        assert_eq!(name(&w, restored), "B");
        assert!(h.redo(&mut w));
        assert!(h.redo(&mut w));
        assert_eq!(w.children(ctx).len(), 2);
        assert!(!w.is_alive(restored));
    }

    #[test]
    fn create_undo_redo() {
        let (mut w, _, ctx) = world_with_context();
        let mut h = History::new();
        let mut b = StepBuilder::new(&mut w);
        let a = b
            .create(
                ctx,
                Snapshot::new("InputAction")
                    .with("Name", Value::String("Dash".into()))
                    .child(
                        Snapshot::new("InputBinding").with("KeyCode", Value::String("Q".into())),
                    ),
            )
            .unwrap();
        h.record(b.finish("Add action").unwrap());
        assert_eq!(name(&w, a), "Dash");
        assert!(h.undo(&mut w));
        assert!(w.children(ctx).is_empty());
        assert!(h.redo(&mut w));
        let a2 = w.children(ctx)[0];
        assert_eq!(name(&w, a2), "Dash");
        assert_eq!(
            w.get(w.children(a2)[0], "KeyCode").unwrap(),
            Value::String("Q".into())
        );
    }

    #[test]
    fn merging_steps_until_sealed() {
        let (mut w, _, ctx) = world_with_context();
        let mut h = History::new();
        for v in [1.0, 2.0, 3.0] {
            let mut b = StepBuilder::new(&mut w);
            b.set(ctx, "Priority", Value::Number(v)).unwrap();
            h.record(b.finish_merging("Priority", "p").unwrap());
        }
        assert_eq!(h.len(), 1);
        h.seal();
        let mut b = StepBuilder::new(&mut w);
        b.set(ctx, "Priority", Value::Number(9.0)).unwrap();
        h.record(b.finish_merging("Priority", "p").unwrap());
        assert_eq!(h.len(), 2);
        h.undo(&mut w);
        assert_eq!(w.get(ctx, "Priority").unwrap(), Value::Number(3.0));
        h.undo(&mut w);
        assert_eq!(w.get(ctx, "Priority").unwrap(), Value::Number(0.0));
    }

    #[test]
    fn abort_takes_everything_back() {
        let (mut w, _, ctx) = world_with_context();
        let mut b = StepBuilder::new(&mut w);
        b.set(ctx, "Sink", Value::Bool(true)).unwrap();
        b.create(ctx, Snapshot::new("InputAction")).unwrap();
        b.abort();
        assert_eq!(w.get(ctx, "Sink").unwrap(), Value::Bool(false));
        assert!(w.children(ctx).is_empty());
    }
}
