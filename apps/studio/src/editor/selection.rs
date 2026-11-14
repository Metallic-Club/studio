//! What is selected (#347): several instances, in the order they were picked, and the **active** one, the last
//! picked. The active instance is what a panel that shows one thing leads with (Properties' header, the camera's
//! orbit, the gizmo's local axes); everything that acts on the selection acts on all of it.
//!
//! The editor's scene and a running game's each have their own (ADR 0010 §5). Instances that are no longer in the
//! game (destroyed, or taken out of the tree) drop out ([`Selection::retain_in_game`]) before each frame's panels.

use std::collections::HashSet;

use metallic_engine::metallic_core::{DataModel, Id};

/// The selected instances, the active one last.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// In the order they were picked; the last is the active one. Never twice the same.
    items: Vec<Id>,
    /// The same instances, to answer "is it selected" without a walk (#597): the Explorer asks for every row it draws.
    chosen: HashSet<Id>,
}

impl Selection {
    /// Nothing selected.
    pub fn new() -> Self {
        Self::default()
    }

    /// `id` alone, or nothing.
    pub fn only(id: Option<Id>) -> Self {
        Self::of(id)
    }

    /// `ids`, in that order, the last active (each once).
    pub fn of(ids: impl IntoIterator<Item = Id>) -> Self {
        let mut selection = Self::new();
        selection.extend(ids);
        selection
    }

    /// Whether nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// How many instances are selected.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether `id` is selected.
    pub fn contains(&self, id: Id) -> bool {
        self.chosen.contains(&id)
    }

    /// The active instance: the last picked.
    pub fn active(&self) -> Option<Id> {
        self.items.last().copied()
    }

    /// Every selected instance, the active one last.
    pub fn ids(&self) -> &[Id] {
        &self.items
    }

    /// Selects `id` alone, or nothing.
    pub fn set_only(&mut self, id: Option<Id>) {
        *self = Self::only(id);
    }

    /// Selects nothing.
    pub fn clear(&mut self) {
        self.items.clear();
        self.chosen.clear();
    }

    /// Adds `id` and makes it the active one (moving it last if it was selected already).
    pub fn add(&mut self, id: Id) {
        if !self.chosen.insert(id) {
            if self.items.last() == Some(&id) {
                return;
            }
            self.items.retain(|&item| item != id);
        }
        self.items.push(id);
    }

    /// Adds each of `ids` in turn, the last becoming the active one: linear in how many there are, as long as few of
    /// them were selected already.
    pub fn extend(&mut self, ids: impl IntoIterator<Item = Id>) {
        let mut again = HashSet::new();
        for id in ids {
            if self.chosen.insert(id) {
                self.items.push(id);
            } else {
                // Moved last: taken out below, all at once, then put back in order.
                again.insert(id);
                self.items.push(id);
            }
        }
        if !again.is_empty() {
            // Keep each one's last place only.
            let mut seen = HashSet::new();
            let mut kept: Vec<Id> = self.items.iter().rev().copied().filter(|id| seen.insert(*id)).collect();
            kept.reverse();
            self.items = kept;
        }
    }

    /// Takes `id` out; the one picked before it becomes active if it was.
    pub fn remove(&mut self, id: Id) {
        if self.chosen.remove(&id) {
            self.items.retain(|&item| item != id);
        }
    }

    /// Takes each of `ids` out, in one pass over the selection.
    pub fn remove_all(&mut self, ids: impl IntoIterator<Item = Id>) {
        let gone: HashSet<Id> = ids.into_iter().filter(|id| self.chosen.remove(id)).collect();
        if !gone.is_empty() {
            self.items.retain(|item| !gone.contains(item));
        }
    }

    /// Ctrl-click: adds `id` as the active one, or takes it out if it is selected.
    pub fn toggle(&mut self, id: Id) {
        if self.contains(id) {
            self.remove(id);
        } else {
            self.add(id);
        }
    }

    /// Drops what is no longer in `dm`'s tree: destroyed, or out of it (a deleted instance, kept for undo, has no
    /// parent). Returns whether anything dropped out.
    pub fn retain_in_game(&mut self, dm: &DataModel) -> bool {
        let before = self.items.len();
        let root = dm.root();
        self.items
            .retain(|&id| dm.contains(id) && root.is_some_and(|root| dm.is_descendant_of(id, root).unwrap_or(false)));
        if self.items.len() == before {
            return false;
        }
        self.chosen = self.items.iter().copied().collect();
        true
    }

    /// The selected instances that are not inside another selected one, in selection order: what moving, copying and
    /// deleting act on (a child goes with its parent, so it must not go twice).
    pub fn top_level(&self, dm: &DataModel) -> Vec<Id> {
        self.items
            .iter()
            .copied()
            .filter(|&id| {
                let mut at = dm.parent(id).ok().flatten();
                while let Some(parent) = at {
                    if self.chosen.contains(&parent) {
                        return false;
                    }
                    at = dm.parent(parent).ok().flatten();
                }
                true
            })
            .collect()
    }

    /// Every ancestor of a selected instance: the rows that hold something selected, which a closed Explorer row is
    /// marked for.
    pub fn holders(&self, dm: &DataModel) -> HashSet<Id> {
        let mut holders = HashSet::new();
        for &id in &self.items {
            let mut at = dm.parent(id).ok().flatten();
            while let Some(parent) = at {
                if !holders.insert(parent) {
                    // Its ancestors are in already.
                    break;
                }
                at = dm.parent(parent).ok().flatten();
            }
        }
        holders
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> (DataModel, Id, Vec<Id>) {
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        let folder = dm.registry().class_id("Folder").unwrap();
        let parts: Vec<Id> = (0..3)
            .map(|_| {
                let id = dm.create(folder).unwrap();
                dm.set_parent(id, Some(workspace)).unwrap();
                id
            })
            .collect();
        (dm, workspace, parts)
    }

    /// #597: adding and taking out many at once keeps the order and the active one, as one by one would.
    #[test]
    fn many_added_or_taken_out_at_once_keep_their_order() {
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let folder = dm.registry().class_id("Folder").unwrap();
        let ids: Vec<Id> = (0..6).map(|_| dm.create(folder).unwrap()).collect();
        let mut selection = Selection::of(ids[..3].iter().copied());
        // Again, with one already in: it moves last, as `add` would move it.
        selection.extend([ids[3], ids[1], ids[4]]);
        let mut one_by_one = Selection::of(ids[..3].iter().copied());
        for id in [ids[3], ids[1], ids[4]] {
            one_by_one.add(id);
        }
        assert_eq!(selection, one_by_one);
        assert_eq!(selection.ids(), [ids[0], ids[2], ids[3], ids[1], ids[4]]);
        assert!(selection.contains(ids[1]) && !selection.contains(ids[5]));
        selection.remove_all([ids[4], ids[0], ids[5]]);
        assert_eq!(selection.ids(), [ids[2], ids[3], ids[1]]);
        assert_eq!(selection.active(), Some(ids[1]));
        assert!(!selection.contains(ids[0]));
        // 20,000 at once, then all taken out: quick, and nothing left.
        let many: Vec<Id> = (0..20_000).map(|_| dm.create(folder).unwrap()).collect();
        let mut big = Selection::of(many.iter().copied());
        assert_eq!(big.len(), 20_000);
        assert_eq!(big.active(), many.last().copied());
        big.remove_all(many.iter().copied());
        assert!(big.is_empty() && !big.contains(many[0]));
    }

    #[test]
    fn the_last_picked_is_active_and_toggling_takes_it_out() {
        let (_, _, ids) = game();
        let mut selection = Selection::only(Some(ids[0]));
        assert_eq!(selection.active(), Some(ids[0]));
        selection.add(ids[1]);
        selection.add(ids[2]);
        assert_eq!(selection.ids(), [ids[0], ids[1], ids[2]]);
        // Adding again moves it last.
        selection.add(ids[0]);
        assert_eq!(selection.ids(), [ids[1], ids[2], ids[0]]);
        selection.toggle(ids[0]);
        assert_eq!(selection.active(), Some(ids[2]), "the one picked before becomes active");
        selection.toggle(ids[0]);
        assert_eq!(selection.active(), Some(ids[0]));
        assert_eq!(selection.len(), 3);
        selection.set_only(Some(ids[1]));
        assert_eq!(selection.ids(), [ids[1]]);
        selection.clear();
        assert!(selection.is_empty() && selection.active().is_none());
        assert_eq!(Selection::of([ids[0], ids[1], ids[0]]).ids(), [ids[1], ids[0]]);
    }

    #[test]
    fn the_top_level_leaves_out_what_is_inside_another_and_holders_are_the_ancestors() {
        let (mut dm, workspace, ids) = game();
        dm.set_parent(ids[1], Some(ids[0])).unwrap();
        dm.set_parent(ids[2], Some(ids[1])).unwrap();
        let selection = Selection::of([ids[2], ids[0]]);
        assert_eq!(
            selection.top_level(&dm),
            [ids[0]],
            "the grandchild goes with its ancestor"
        );
        let holders = Selection::only(Some(ids[2])).holders(&dm);
        assert!(holders.contains(&ids[1]) && holders.contains(&ids[0]) && holders.contains(&workspace));
        assert!(!holders.contains(&ids[2]));

        let mut selection = Selection::of(ids.clone());
        dm.destroy(ids[1]).unwrap();
        assert!(selection.retain_in_game(&dm));
        assert_eq!(
            selection.ids(),
            [ids[0]],
            "the destroyed and what was inside it drop out"
        );
        assert!(!selection.retain_in_game(&dm));
        dm.set_parent(ids[0], None).unwrap();
        assert!(selection.retain_in_game(&dm), "out of the tree is out of the game");
        assert!(selection.is_empty());
    }
}
