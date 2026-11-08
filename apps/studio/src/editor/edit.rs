//! Changing a scene (#348): every edit the panels and tools make goes through [`Scene`]'s methods here, which note
//! it in the scene's history (see [`super::history`]), and the app ends the action under way once a frame.

use std::time::Instant;

use metallic_engine::metallic_core::{DmError, Id, PropId, Value};

use super::Scene;
use super::history::{Change, Place};

impl Scene<'_> {
    /// Writes `prop` of `id`, as part of the action under way. The value noted is the one stored (clamped to the
    /// property's range).
    pub fn set(&mut self, id: Id, prop: PropId, value: Value) -> Result<(), DmError> {
        let old = self.dm.get(id, prop)?;
        self.dm.set(id, prop, value)?;
        let new = self.dm.get(id, prop)?;
        if old != new {
            self.history.note(Change::Prop { id, prop, old, new });
        }
        Ok(())
    }

    /// Writes the property named `name` of `id` (see [`Scene::set`]).
    pub fn set_by_name(&mut self, id: Id, name: &str, value: Value) -> Result<(), DmError> {
        let prop = self.dm.prop_id(id, name)?;
        self.set(id, prop, value)
    }

    /// Moves `id` under `parent`, before its child `before` (`None`: last), or out of the tree with no `parent`: a
    /// delete, which undo takes back to the same instance. As part of the action under way.
    pub fn move_to(&mut self, id: Id, parent: Option<Id>, before: Option<Id>) -> Result<(), DmError> {
        let old = Place::of(self.dm, id)?;
        match parent {
            Some(parent) => self.dm.set_parent_before(id, parent, before)?,
            None => self.dm.set_parent(id, None)?,
        }
        let new = Place::of(self.dm, id)?;
        if old != new {
            self.history.note(Change::Parent { id, old, new });
        }
        Ok(())
    }

    /// Ends the action under way as one undo step, unless `holding` (a mouse button is down: a drag goes on).
    pub fn commit(&mut self, now: Instant, holding: bool) {
        self.history.commit(self.dm, self.selection, now, holding);
    }

    /// Undoes the last step, ending the action under way first, and puts back the selection it had. Returns whether
    /// there was one.
    pub fn undo(&mut self, now: Instant) -> bool {
        self.commit(now, false);
        let Some(applied) = self.history.undo(self.dm) else {
            return false;
        };
        self.applied(applied);
        true
    }

    /// Redoes the last step undone. Returns whether there was one.
    pub fn redo(&mut self, now: Instant) -> bool {
        self.commit(now, false);
        let Some(applied) = self.history.redo(self.dm) else {
            return false;
        };
        self.applied(applied);
        true
    }

    /// After an undo or redo: its selection, shown in the Explorer; what could not be replayed, logged.
    fn applied(&mut self, applied: super::history::Applied) {
        for problem in &applied.problems {
            log::warn!("history: a change could not be replayed: {problem}");
        }
        if let Some(selection) = applied.selection {
            *self.selection = selection;
        }
        if let Some(active) = self.selection.active() {
            self.explorer.reveal(self.dm, active);
        }
    }
}
