//! Changing a scene (#348): every edit the panels and tools make goes through [`Scene`]'s methods here, which note
//! it in the scene's history (see [`super::history`]), and the app ends the action under way once a frame. The tree
//! edits (#351: delete, duplicate, cut, copy, paste, rename) act on the selection through them.

use std::time::Instant;

use metallic_engine::metallic_core::glam::Vec3;
use metallic_engine::metallic_core::{CFrame, ClassId, DataModel, DmError, Id, PropId, Value};

use super::history::{Change, Place};
use super::{Scene, Selection};
use crate::viewport::Aim;
use crate::viewport::pick::is_a;
use metallic_engine::metallic_classes::base::world_object;

/// The `CFrame` of `id`, about to go inside `parent` (#412): at `parent`'s pivot when it has one (a light inside a part
/// at the part's middle, its `CFrame` being the part's then); else where the camera looks, `aim`, a part resting on the
/// surface there. `None` without either.
fn placement(dm: &DataModel, id: Id, parent: Id, aim: Option<Aim>) -> Option<CFrame> {
    if let Ok(pivot) = world_object::pivot(dm, parent) {
        if is_a(dm, id, "Light") && is_a(dm, parent, "BasePart") {
            return Some(CFrame::IDENTITY);
        }
        return Some(CFrame::new(pivot.position()));
    }
    let aim = aim?;
    let lift = match (aim.normal, dm.get_by_name(id, "Size")) {
        (Some(normal), Ok(Value::Vector3(size))) => normal * normal.abs().dot(size * 0.5),
        _ => Vec3::ZERO,
    };
    Some(CFrame::new(aim.point + lift))
}

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

    /// Sets the attribute `name` of `id` to `value`, or removes it with `None` (#591), as part of the action under way.
    pub fn set_attribute(&mut self, id: Id, name: &str, value: Option<Value>) -> Result<(), DmError> {
        let old = self.dm.attribute(id, name)?.cloned();
        self.dm.set_attribute(id, name, value)?;
        let new = self.dm.attribute(id, name)?.cloned();
        if old != new {
            self.history.note(Change::Attribute {
                id,
                name: name.to_owned(),
                old,
                new,
            });
        }
        Ok(())
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

    /// Runs `change`, which writes the data model itself (moving a pivot, #355), as part of the action under way:
    /// whichever of the `watched` properties it changed is noted.
    pub fn watching(&mut self, watched: &[(Id, PropId)], change: impl FnOnce(&mut DataModel)) {
        let before: Vec<Option<Value>> = watched.iter().map(|&(id, prop)| self.dm.get(id, prop).ok()).collect();
        change(self.dm);
        for (&(id, prop), old) in watched.iter().zip(before) {
            if let (Some(old), Ok(new)) = (old, self.dm.get(id, prop))
                && old != new
            {
                self.history.note(Change::Prop { id, prop, old, new });
            }
        }
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
        for (id, color) in applied.icon_colors {
            self.explorer.icon_colors.set(id, color);
        }
        if let Some(selection) = applied.selection {
            *self.selection = selection;
        }
        if let Some(active) = self.selection.active() {
            self.explorer.reveal(self.dm, active);
        }
    }
}

/// What was copied or cut (#351): copies of the instances, out of the tree, in the game they came from. Pasting
/// copies them again, so it pastes as often as wanted. A running game's clipboard does not paste into the editor's
/// scene, nor the other way round.
#[derive(Debug, Default)]
pub struct Clipboard {
    /// The game the copies are in ([`DataModel::uid`]).
    game: Option<u64>,
    /// The copies, out of the tree.
    items: Vec<Id>,
}

impl Clipboard {
    /// Whether it holds something that pastes into `dm`.
    pub fn can_paste_into(&self, dm: &DataModel) -> bool {
        self.game == Some(dm.uid()) && self.items.iter().any(|&id| dm.contains(id))
    }
}

impl Scene<'_> {
    /// Whether `id` can be moved, copied or deleted: anything but the root and the services.
    pub fn is_movable(&self, id: Id) -> bool {
        is_movable(self.dm, id)
    }

    /// What the tree edits act on: the selection's instances that are not inside another selected one, without the
    /// services.
    fn movable_selection(&self) -> Vec<Id> {
        self.selection
            .top_level(self.dm)
            .into_iter()
            .filter(|&id| self.is_movable(id))
            .collect()
    }

    /// Delete: takes the selection out of the tree (kept for undo) and selects nothing. Returns how many went.
    pub fn delete(&mut self) -> usize {
        let doomed = self.movable_selection();
        for &id in &doomed {
            if let Err(error) = self.move_to(id, None, None) {
                log::warn!("edit: could not delete: {error}");
            }
        }
        if !doomed.is_empty() {
            self.selection.clear();
        }
        doomed.len()
    }

    /// Duplicate: a copy of each selected instance just after it, and selects the copies. Returns how many.
    pub fn duplicate(&mut self) -> usize {
        let mut copies = Vec::new();
        for id in self.movable_selection() {
            let parent = self.dm.parent(id).ok().flatten();
            let before = self.dm.next_sibling(id).ok().flatten();
            let cloned = self.dm.clone_tree(id);
            if let Ok(Some(copy)) = cloned {
                self.explorer.icon_colors.copy_tree(self.dm, id, copy);
            }
            match cloned {
                Ok(Some(copy)) => match self.move_to(copy, parent, before) {
                    Ok(()) => copies.push(copy),
                    Err(error) => log::warn!("edit: could not duplicate: {error}"),
                },
                // Not archivable: it is not copied, as `Clone` in scripts.
                Ok(None) => {}
                Err(error) => log::warn!("edit: could not duplicate: {error}"),
            }
        }
        if !copies.is_empty() {
            *self.selection = Selection::of(copies.iter().copied());
        }
        copies.len()
    }

    /// Copy: puts copies of the selection on the clipboard, in place of what it held. Returns how many.
    pub fn copy(&mut self) -> usize {
        let originals = self.movable_selection();
        if originals.is_empty() {
            return 0;
        }
        // The old copies were only the clipboard's.
        if self.clipboard.game == Some(self.dm.uid()) {
            for id in std::mem::take(&mut self.clipboard.items) {
                if self.dm.contains(id) && self.dm.parent(id).ok().flatten().is_none() {
                    let _ = self.dm.destroy(id);
                }
            }
        }
        self.clipboard.game = Some(self.dm.uid());
        self.clipboard.items = originals
            .into_iter()
            .filter_map(|id| {
                let copy = self.dm.clone_tree(id).ok().flatten()?;
                self.explorer.icon_colors.copy_tree(self.dm, id, copy);
                Some(copy)
            })
            .collect();
        self.clipboard.items.len()
    }

    /// Cut: copies the selection, then deletes it. Returns how many.
    pub fn cut(&mut self) -> usize {
        if self.copy() == 0 {
            return 0;
        }
        self.delete()
    }

    /// Where a paste goes: inside the active instance (`into`), or beside it (inside a selected service); in
    /// Workspace with nothing selected.
    fn paste_target(&self, into: bool) -> Option<Id> {
        let workspace = self
            .dm
            .registry()
            .class_id("Workspace")
            .and_then(|class| self.dm.service(class));
        match self.selection.active() {
            Some(active) if into || !self.is_movable(active) => Some(active),
            Some(active) => self.dm.parent(active).ok().flatten().or(workspace),
            None => workspace,
        }
    }

    /// Paste: copies of the clipboard's instances beside the active instance (or inside it, `into`), selected.
    /// Returns how many.
    pub fn paste(&mut self, into: bool) -> usize {
        if !self.clipboard.can_paste_into(self.dm) {
            return 0;
        }
        let Some(parent) = self.paste_target(into) else {
            return 0;
        };
        let mut pasted = Vec::new();
        for item in self.clipboard.items.clone() {
            let cloned = self.dm.clone_tree(item);
            if let Ok(Some(copy)) = cloned {
                self.explorer.icon_colors.copy_tree(self.dm, item, copy);
            }
            match cloned {
                Ok(Some(copy)) => match self.move_to(copy, Some(parent), None) {
                    Ok(()) => pasted.push(copy),
                    Err(error) => log::warn!("edit: could not paste: {error}"),
                },
                Ok(None) => {}
                Err(error) => log::warn!("edit: could not paste: {error}"),
            }
        }
        if !pasted.is_empty() {
            *self.selection = Selection::of(pasted.iter().copied());
        }
        pasted.len()
    }

    /// Insert Object (#352): a new `class` inside `parent`, selected and shown in the Explorer, as part of the action
    /// under way. Something with a place in the world (a part, a light, a camera) goes where the camera looks, `aim`,
    /// a part resting on the surface there; inside something with a pivot, at that pivot (#412).
    pub fn insert(&mut self, class: ClassId, parent: Id, aim: Option<Aim>) -> Result<Id, DmError> {
        let id = self.dm.create(class)?;
        if self.dm.prop_id(id, "CFrame").is_ok()
            && let Some(frame) = placement(self.dm, id, parent, aim)
        {
            // Before it joins the tree: the undo step only has to take it out again.
            self.dm.set_by_name(id, "CFrame", Value::CFrame(frame))?;
        }
        if let Err(error) = self.move_to(id, Some(parent), None) {
            let _ = self.dm.destroy(id);
            return Err(error);
        }
        self.selection.set_only(Some(id));
        self.explorer.reveal(self.dm, id);
        Ok(id)
    }

    /// Whether the selection can move under `parent`: something in it moves, and `parent` is neither the root nor
    /// inside (or one of) what moves.
    pub fn can_move_selection(&self, parent: Id) -> bool {
        let moving = self.movable_selection();
        !moving.is_empty()
            && self.dm.root() != Some(parent)
            && moving
                .iter()
                .all(|&id| id != parent && !self.dm.is_descendant_of(parent, id).unwrap_or(true))
    }

    /// Moves the selection under `parent`, before its child `before` (`None`: last), as the Explorer's drops do
    /// (#353). Returns how many moved; nothing when [`Scene::can_move_selection`] refuses.
    pub fn move_selection(&mut self, parent: Id, before: Option<Id>) -> usize {
        if !self.can_move_selection(parent) {
            return 0;
        }
        let moving = self.movable_selection();
        // Before an instance that moves too is before the next one that stays.
        let mut before = before;
        while let Some(id) = before.filter(|id| moving.contains(id)) {
            before = self.dm.next_sibling(id).ok().flatten();
        }
        for &id in &moving {
            if let Err(error) = self.move_to(id, Some(parent), before) {
                log::warn!("edit: could not move: {error}");
            }
        }
        moving.len()
    }

    /// Makes `parent` the selection's parent, as picked from the Parent row (#599): as an Explorer drop does, inside
    /// it last, as part of the action under way. Picking ends when it moved; otherwise it goes on and says why not.
    pub fn pick_parent(&mut self, parent: Id) {
        match self.why_not_parent(parent) {
            Some(reason) => {
                if let Some(pick) = &mut self.explorer.parent_pick {
                    pick.refused = Some(reason);
                }
            }
            None => {
                self.move_selection(parent, None);
                self.explorer.set_expanded(parent, true);
                self.explorer.parent_pick = None;
            }
        }
    }

    /// Why the selection cannot go inside `parent`, if it cannot.
    fn why_not_parent(&self, parent: Id) -> Option<String> {
        let moving = self.movable_selection();
        let name = |id: Id| self.dm.name(id).unwrap_or_default().to_owned();
        if moving.is_empty() {
            return Some("The selection can't move: services stay where they are.".to_owned());
        }
        if self.dm.root() == Some(parent) {
            return Some("Only services go directly in the game.".to_owned());
        }
        if let Some(&inside) = moving
            .iter()
            .find(|&&id| id == parent || self.dm.is_descendant_of(parent, id).unwrap_or(true))
        {
            return Some(format!("{} can't go inside itself.", name(inside)));
        }
        None
    }

    /// Gives `ids` the icon colour `color` in the Explorer (#603), or their class's own back with `None`, as part of
    /// the action under way. Studio's, not the game's: the data model does not change.
    pub fn set_icon_color(&mut self, ids: &[Id], color: Option<[u8; 3]>) {
        for &id in ids {
            let old = self.explorer.icon_colors.get(id);
            if old != color {
                self.explorer.icon_colors.set(id, color);
                self.history.note(Change::IconColor { id, old, new: color });
            }
        }
    }

    /// Renames `id` to `name` (trimmed; an empty name leaves it as it was).
    pub fn rename(&mut self, id: Id, name: &str) -> Result<(), DmError> {
        let name = name.trim();
        if name.is_empty() {
            return Ok(());
        }
        self.set_by_name(id, "Name", Value::String(name.into()))
    }
}

/// An edit of the tree the Edit menu, the keymap and the Explorer's menu ask for (#351).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TreeEdit {
    /// Copy the selection, then delete it.
    Cut,
    /// Copy the selection.
    Copy,
    /// Paste beside the active instance.
    Paste,
    /// Paste inside the active instance.
    PasteInto,
    /// Copy the selection beside itself.
    Duplicate,
    /// Rename the active instance in the Explorer.
    Rename,
    /// Delete the selection.
    Delete,
}

impl TreeEdit {
    /// Whether it pastes (and so needs the clipboard rather than a selection).
    pub fn pastes(self) -> bool {
        matches!(self, TreeEdit::Paste | TreeEdit::PasteInto)
    }
}

/// Whether `id` can be moved, copied or deleted: anything but the root and the services.
pub fn is_movable(dm: &DataModel, id: Id) -> bool {
    let Ok(class) = dm.class_of(id) else {
        return false;
    };
    dm.root() != Some(id)
        && dm
            .registry()
            .class(class)
            .is_some_and(|info| info.service_order().is_none())
}

impl Scene<'_> {
    /// Does `edit`. Rename starts renaming the active instance in its Explorer row.
    pub fn tree_edit(&mut self, edit: TreeEdit) {
        match edit {
            TreeEdit::Cut => {
                self.cut();
            }
            TreeEdit::Copy => {
                self.copy();
            }
            TreeEdit::Paste => {
                self.paste(false);
            }
            TreeEdit::PasteInto => {
                self.paste(true);
            }
            TreeEdit::Duplicate => {
                self.duplicate();
            }
            TreeEdit::Rename => {
                if let Some(active) = self.selection.active().filter(|&id| self.is_movable(id)) {
                    self.explorer.reveal(self.dm, active);
                    self.explorer.start_rename(active);
                }
            }
            TreeEdit::Delete => {
                self.delete();
            }
        }
    }
}
