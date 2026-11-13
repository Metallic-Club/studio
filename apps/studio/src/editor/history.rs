//! Undo and redo (#348): a history of steps per scene, as Forge's (`state/history.rs`).
//!
//! - **What is recorded.** Studio changes a scene only through the scene's edits ([`Scene::set`](super::Scene::set),
//!   [`Scene::move_to`](super::Scene::move_to)), which note each change here. Nothing else changes the editor's scene
//!   (no script runs in it), so the data model needs no journal of its own; the one exception is the editor's view,
//!   written into the current camera as it moves and on purpose no step (#576). A running game has a history of its
//!   own, dropped at Stop: it holds the panels' edits of the game, not what the game's scripts do.
//! - **Steps.** The app ends the action under way once per frame ([`History::commit`]), but not while a mouse button
//!   is held: a drag is one step, however many frames it takes. Edits of the same properties less than
//!   [`MERGE_WITHIN`] apart join the step before (typing a value, then typing more). A step also holds the selection
//!   before and after it when the selection changed; a click that only selects is a step of its own, as in Forge, so
//!   undo puts back what was selected too.
//! - **Deleting keeps the instance.** A delete takes the instance out of the tree, so undo puts the same instance back
//!   where it was among its siblings (#346), with everything inside it and the same id. Instances out of the tree are
//!   destroyed once no step can bring them back (the redo steps a new action drops, the steps past [`LIMIT`]).
//! - **Saved or not.** Each step that changes the scene leaves it at a new revision; the scene has unsaved changes
//!   while its revision is not the one last saved ([`History::is_saved`]), so undoing back to it clears the mark.

use std::time::{Duration, Instant};

use metallic_engine::metallic_core::{DataModel, DmError, Id, PropId, Value};

use super::Selection;

/// How many steps are kept: the oldest go first.
pub const LIMIT: usize = 1000;
/// Edits of the same properties closer together than this are one step.
pub const MERGE_WITHIN: Duration = Duration::from_millis(600);

/// Where an instance is in the tree: its parent and the sibling it comes before. `parent` is `None` for an instance
/// out of the tree (not created yet, or deleted).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    /// Its parent.
    pub parent: Option<Id>,
    /// The sibling after it, `None` when it is the last.
    pub before: Option<Id>,
}

impl Place {
    /// Where `id` is now.
    pub fn of(dm: &DataModel, id: Id) -> Result<Self, DmError> {
        Ok(Self {
            parent: dm.parent(id)?,
            before: dm.next_sibling(id)?,
        })
    }

    /// Puts `id` here: under the parent, before the sibling while it is still the parent's child (else last).
    fn restore(self, dm: &mut DataModel, id: Id) -> Result<(), DmError> {
        let Some(parent) = self.parent else {
            return dm.set_parent(id, None);
        };
        let before = self
            .before
            .filter(|&before| before != id && dm.contains(before) && dm.parent(before).ok().flatten() == Some(parent));
        dm.set_parent_before(id, parent, before)
    }
}

/// One change a step made, with what it was before.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// Property `prop` of `id` went from `old` to `new`.
    Prop {
        /// The instance.
        id: Id,
        /// The property.
        prop: PropId,
        /// Its value before.
        old: Value,
        /// Its value after.
        new: Value,
    },
    /// `id` moved from `old` to `new`: a new instance comes from no parent, a deleted one goes to none.
    Parent {
        /// The instance.
        id: Id,
        /// Where it was.
        old: Place,
        /// Where it went.
        new: Place,
    },
}

impl Change {
    /// The instance it changed.
    pub fn id(&self) -> Id {
        match self {
            Change::Prop { id, .. } | Change::Parent { id, .. } => *id,
        }
    }

    /// Whether it changed nothing in the end (a value set and set back).
    fn is_noop(&self) -> bool {
        match self {
            Change::Prop { old, new, .. } => old == new,
            Change::Parent { old, new, .. } => old == new,
        }
    }

    /// Puts back what it was before (`back`) or after.
    fn apply(&self, dm: &mut DataModel, back: bool) -> Result<(), DmError> {
        match self {
            Change::Prop { id, prop, old, new } => dm.set(*id, *prop, if back { old } else { new }.clone()),
            Change::Parent { id, old, new } => if back { old } else { new }.restore(dm, *id),
        }
    }
}

/// One step: a user action.
#[derive(Clone, Debug)]
struct Step {
    /// What it changed, in order.
    changes: Vec<Change>,
    /// The selection before and after, when it changed.
    selection: Option<(Selection, Selection)>,
    /// When it was last added to (typing keeps adding to one step).
    at: Instant,
    /// The scene's revision after it.
    revision: u64,
}

impl Step {
    /// Whether it only writes properties: a step later edits of the same properties can join.
    fn is_property_edit(&self) -> bool {
        self.selection.is_none()
            && !self.changes.is_empty()
            && self.changes.iter().all(|change| matches!(change, Change::Prop { .. }))
    }

    /// Joins `later` (property writes only) to it if it writes exactly the same properties. Returns whether it did.
    fn absorb(&mut self, later: &[Change], at: Instant) -> bool {
        let same = |a: &Change, b: &Change| matches!((a, b), (Change::Prop { id, prop, .. }, Change::Prop { id: i, prop: p, .. }) if id == i && prop == p);
        let same_targets = later.len() == self.changes.len()
            && later
                .iter()
                .all(|change| self.changes.iter().any(|mine| same(mine, change)));
        if !same_targets {
            return false;
        }
        for change in later {
            if let Change::Prop { new, .. } = change
                && let Some(Change::Prop { new: mine, .. }) = self.changes.iter_mut().find(|mine| same(mine, change))
            {
                mine.clone_from(new);
            }
        }
        self.at = at;
        true
    }

    /// Whether it names `id`.
    fn names(&self, id: Id) -> bool {
        self.changes.iter().any(|change| change.id() == id)
    }
}

/// What an undo or a redo did.
#[derive(Clone, Debug, Default)]
pub struct Applied {
    /// The selection to put back, when the step changed it.
    pub selection: Option<Selection>,
    /// Whether it changed the scene (and not only the selection).
    pub edited: bool,
    /// The changes that could not be replayed, and why.
    pub problems: Vec<String>,
}

/// A scene's undo history.
#[derive(Clone, Debug, Default)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// The changes of the action under way.
    pending: Vec<Change>,
    /// The selection when the last step ended.
    selection: Selection,
    /// The scene's revision before the oldest step kept.
    base: u64,
    /// The last revision given out.
    last_revision: u64,
    /// The revision last saved.
    saved: u64,
}

impl History {
    /// A history with nothing in it, `selection` selected.
    pub fn new(selection: Selection) -> Self {
        Self {
            selection,
            ..Self::default()
        }
    }

    /// Notes a change of the action under way. A property written again in the same action keeps its first old value.
    pub fn note(&mut self, change: Change) {
        if let Change::Prop { id, prop, new, .. } = &change
            && let Some(Change::Prop { new: mine, .. }) = self
                .pending
                .iter_mut()
                .find(|mine| matches!(mine, Change::Prop { id: i, prop: p, .. } if i == id && p == prop))
        {
            mine.clone_from(new);
            return;
        }
        self.pending.push(change);
    }

    /// Whether an action is under way: changes noted and not yet a step.
    pub fn is_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Ends the action under way, at `now`, as a step: what it changed, and the selection if it is not `selection` as
    /// the last step left it. Nothing while `holding` (a mouse button is down: the drag goes on), and nothing when
    /// the action changed nothing. A new step drops what could be redone.
    pub fn commit(&mut self, dm: &mut DataModel, selection: &Selection, now: Instant, holding: bool) {
        if holding {
            return;
        }
        let mut changes = std::mem::take(&mut self.pending);
        changes.retain(|change| !change.is_noop());
        let moved = *selection != self.selection;
        if changes.is_empty() && !moved {
            return;
        }
        let selection = moved.then(|| {
            (
                std::mem::replace(&mut self.selection, selection.clone()),
                selection.clone(),
            )
        });
        let step = Step {
            revision: if changes.is_empty() {
                self.revision()
            } else {
                self.next_revision()
            },
            changes,
            selection,
            at: now,
        };
        if step.is_property_edit()
            && let Some(last) = self.undo.last_mut()
            && last.is_property_edit()
            && now.saturating_duration_since(last.at) < MERGE_WITHIN
            && last.absorb(&step.changes, now)
        {
            last.revision = step.revision;
            if last.changes.iter().all(Change::is_noop) {
                // Typed and typed back: nothing is left to undo.
                self.undo.pop();
            }
            self.drop_redo(dm);
            return;
        }
        self.undo.push(step);
        self.drop_redo(dm);
        if self.undo.len() > LIMIT {
            let oldest = self.undo.remove(0);
            self.base = oldest.revision;
            self.forget(dm, vec![oldest]);
        }
    }

    /// Undoes the last step: its changes backwards. `None` when there is none. End the action under way first.
    pub fn undo(&mut self, dm: &mut DataModel) -> Option<Applied> {
        let step = self.undo.pop()?;
        let mut problems = Vec::new();
        for change in step.changes.iter().rev() {
            if let Err(error) = change.apply(dm, true) {
                problems.push(error.to_string());
            }
        }
        let applied = Applied {
            selection: step.selection.as_ref().map(|(before, _)| before.clone()),
            edited: !step.changes.is_empty(),
            problems,
        };
        if let Some(selection) = &applied.selection {
            self.selection.clone_from(selection);
        }
        self.redo.push(step);
        Some(applied)
    }

    /// Redoes the last step undone: its changes forwards. `None` when there is none.
    pub fn redo(&mut self, dm: &mut DataModel) -> Option<Applied> {
        let step = self.redo.pop()?;
        let mut problems = Vec::new();
        for change in &step.changes {
            if let Err(error) = change.apply(dm, false) {
                problems.push(error.to_string());
            }
        }
        let applied = Applied {
            selection: step.selection.as_ref().map(|(_, after)| after.clone()),
            edited: !step.changes.is_empty(),
            problems,
        };
        if let Some(selection) = &applied.selection {
            self.selection.clone_from(selection);
        }
        self.undo.push(step);
        Some(applied)
    }

    /// Whether there is a step to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether there is a step to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// How many steps can be undone.
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// The scene's revision now: each step that changes it gives a new one.
    pub fn revision(&self) -> u64 {
        self.undo.last().map_or(self.base, |step| step.revision)
    }

    /// Notes that the scene was saved as it is now.
    pub fn mark_saved(&mut self) {
        self.saved = self.revision();
    }

    /// Whether the scene is as it was last saved (or opened): no step since, or undone back to it.
    pub fn is_saved(&self) -> bool {
        self.revision() == self.saved && self.pending.iter().all(Change::is_noop)
    }

    fn next_revision(&mut self) -> u64 {
        self.last_revision += 1;
        self.last_revision
    }

    /// Drops the steps that could be redone.
    fn drop_redo(&mut self, dm: &mut DataModel) {
        let dropped = std::mem::take(&mut self.redo);
        self.forget(dm, dropped);
    }

    /// Destroys the instances out of the tree that `dropped` could have brought back and no step kept names.
    fn forget(&self, dm: &mut DataModel, dropped: Vec<Step>) {
        for step in dropped {
            for change in step.changes {
                let Change::Parent { id, .. } = change else {
                    continue;
                };
                let out = dm.contains(id) && dm.parent(id).ok().flatten().is_none();
                let named = self.undo.iter().chain(&self.redo).any(|kept| kept.names(id));
                if out && !named {
                    // Instances out of the tree never fail to destroy; a failure only leaves one in memory.
                    if let Err(error) = dm.destroy(id) {
                        log::debug!("history: could not free an instance out of the tree: {error}");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scene {
        dm: DataModel,
        history: History,
        selection: Selection,
        workspace: Id,
        now: Instant,
    }

    impl Scene {
        fn new() -> Self {
            let dm = metallic_engine::metallic_classes::new_game().unwrap();
            let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
            Self {
                dm,
                history: History::default(),
                selection: Selection::new(),
                workspace,
                now: Instant::now(),
            }
        }

        fn part(&mut self, name: &str) -> Id {
            let part = self.dm.create(self.dm.registry().class_id("Part").unwrap()).unwrap();
            self.dm.set_name(part, name.into()).unwrap();
            self.dm.set_parent(part, Some(self.workspace)).unwrap();
            part
        }

        fn set(&mut self, id: Id, name: &str, value: Value) {
            let prop = self.dm.prop_id(id, name).unwrap();
            let old = self.dm.get(id, prop).unwrap();
            self.dm.set(id, prop, value).unwrap();
            let new = self.dm.get(id, prop).unwrap();
            self.history.note(Change::Prop { id, prop, old, new });
        }

        fn move_to(&mut self, id: Id, parent: Option<Id>) {
            let old = Place::of(&self.dm, id).unwrap();
            self.dm.set_parent(id, parent).unwrap();
            let new = Place::of(&self.dm, id).unwrap();
            self.history.note(Change::Parent { id, old, new });
        }

        /// Ends the action, `after` later than the last.
        fn commit(&mut self, after: Duration) {
            self.now += after;
            self.history.commit(&mut self.dm, &self.selection, self.now, false);
        }

        fn undo(&mut self) -> Applied {
            let applied = self.history.undo(&mut self.dm).expect("a step to undo");
            if let Some(selection) = &applied.selection {
                self.selection.clone_from(selection);
            }
            applied
        }

        fn redo(&mut self) -> Applied {
            let applied = self.history.redo(&mut self.dm).expect("a step to redo");
            if let Some(selection) = &applied.selection {
                self.selection.clone_from(selection);
            }
            applied
        }

        fn number(&self, id: Id, name: &str) -> f64 {
            match self.dm.get_by_name(id, name).unwrap() {
                Value::Number(n) => n,
                other => panic!("{other:?}"),
            }
        }

        fn kids(&self) -> Vec<String> {
            self.dm
                .children(self.workspace)
                .unwrap()
                .map(|id| self.dm.name(id).unwrap().to_owned())
                .collect()
        }
    }

    const LATER: Duration = Duration::from_secs(5);

    #[test]
    fn a_property_edit_undoes_and_redoes() {
        let mut s = Scene::new();
        let part = s.part("Wall");
        s.set(part, "Transparency", Value::Number(0.5));
        s.commit(LATER);
        assert!(s.history.can_undo() && !s.history.can_redo());
        let applied = s.undo();
        assert!(applied.edited && applied.problems.is_empty());
        assert_eq!(s.number(part, "Transparency"), 0.0);
        assert!(s.history.can_redo());
        s.redo();
        assert_eq!(s.number(part, "Transparency"), 0.5);
    }

    #[test]
    fn edits_close_together_are_one_step_and_a_held_button_is_one_gesture() {
        let mut s = Scene::new();
        let part = s.part("Wall");
        // A drag: nothing is a step while the button is held.
        for value in [0.1, 0.2, 0.3] {
            s.set(part, "Transparency", Value::Number(value));
            s.history.commit(&mut s.dm, &s.selection, s.now, true);
        }
        assert!(!s.history.can_undo() && s.history.is_pending());
        s.commit(LATER);
        // Typing: a write per frame, close together, join that step.
        for value in [0.4, 0.5, 0.6] {
            s.set(part, "Transparency", Value::Number(value));
            s.commit(Duration::from_millis(100));
        }
        assert_eq!(s.history.undo_len(), 1);
        s.undo();
        assert_eq!(s.number(part, "Transparency"), 0.0);
        s.redo();
        assert_eq!(s.number(part, "Transparency"), 0.6);
        // Far apart, or another property: steps of their own.
        s.set(part, "Transparency", Value::Number(0.7));
        s.commit(LATER);
        s.set(part, "Anchored", Value::Bool(true));
        s.commit(Duration::from_millis(100));
        assert_eq!(s.history.undo_len(), 3);
        // Typed and typed back: nothing left to undo.
        s.set(part, "Anchored", Value::Bool(false));
        s.commit(Duration::from_millis(100));
        assert_eq!(s.history.undo_len(), 2);
    }

    #[test]
    fn a_delete_undoes_to_the_same_instance_in_its_place_and_selected() {
        let mut s = Scene::new();
        let parts: Vec<Id> = ["A", "B", "C"].iter().map(|name| s.part(name)).collect();
        let child = s.dm.create(s.dm.registry().class_id("Folder").unwrap()).unwrap();
        s.dm.set_parent(child, Some(parts[1])).unwrap();
        s.selection = Selection::only(Some(parts[1]));
        s.commit(LATER);
        // Delete B: out of the tree, the selection cleared.
        s.move_to(parts[1], None);
        s.selection.clear();
        s.commit(LATER);
        assert_eq!(s.kids(), ["Camera", "A", "C"]);
        s.undo();
        assert_eq!(s.kids(), ["Camera", "A", "B", "C"], "back where it was");
        assert_eq!(s.dm.parent(child).unwrap(), Some(parts[1]), "with what was inside it");
        assert_eq!(s.selection.ids(), [parts[1]], "and selected again");
        s.redo();
        assert_eq!(s.kids(), ["Camera", "A", "C"]);
        assert!(s.selection.is_empty());
        // A click that only selects is a step of its own.
        s.selection = Selection::only(Some(parts[0]));
        s.commit(LATER);
        let applied = s.undo();
        assert!(!applied.edited);
        assert!(s.selection.is_empty());
    }

    #[test]
    fn the_saved_mark_follows_the_revision() {
        let mut s = Scene::new();
        let part = s.part("Wall");
        assert!(s.history.is_saved());
        s.set(part, "Transparency", Value::Number(0.5));
        assert!(!s.history.is_saved(), "an action under way is a change");
        s.commit(LATER);
        assert!(!s.history.is_saved());
        s.history.mark_saved();
        assert!(s.history.is_saved());
        s.selection = Selection::only(Some(part));
        s.commit(LATER);
        assert!(s.history.is_saved(), "selecting changes nothing to save");
        s.undo();
        s.undo();
        assert!(!s.history.is_saved(), "undone past the save");
        s.redo();
        assert!(s.history.is_saved(), "back at the save");
    }

    #[test]
    fn instances_no_step_can_bring_back_are_destroyed() {
        let mut s = Scene::new();
        // A new instance, added and undone: out of the tree, but redo can bring it back.
        let new = s.dm.create(s.dm.registry().class_id("Part").unwrap()).unwrap();
        s.move_to(new, Some(s.workspace));
        s.commit(LATER);
        s.undo();
        assert!(s.dm.contains(new) && s.dm.parent(new).unwrap().is_none());
        // A new action drops the redo, and the instance with it.
        let part = s.part("Wall");
        s.set(part, "Transparency", Value::Number(0.5));
        s.commit(LATER);
        assert!(!s.history.can_redo());
        assert!(!s.dm.contains(new));
        // A deleted instance a later step names is kept while that step is.
        s.move_to(part, None);
        s.commit(LATER);
        assert!(s.dm.contains(part));
    }

    #[test]
    fn the_history_keeps_its_last_steps() {
        let mut s = Scene::new();
        let part = s.part("Wall");
        for step in 0..LIMIT + 5 {
            s.set(
                part,
                "Transparency",
                Value::Number(if step % 2 == 0 { 0.5 } else { 0.25 }),
            );
            s.commit(LATER);
        }
        assert_eq!(s.history.undo_len(), LIMIT);
        s.history.mark_saved();
        while s.history.can_undo() {
            s.undo();
        }
        assert!(!s.history.is_saved());
        assert_eq!(
            s.number(part, "Transparency"),
            0.5,
            "the oldest kept step's value before it"
        );
    }
}
