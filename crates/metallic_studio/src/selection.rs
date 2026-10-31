//! The selection: an ordered list of instances shared by the viewport, Explorer, Properties and
//! every tool.
//!
//! # API for tools
//!
//! - [`Selection::ids`] in click order; [`Selection::active`] is the last one clicked (Align and
//!   other reference-based tools use it).
//! - [`Selection::set`], [`Selection::toggle`], [`Selection::add`], [`Selection::clear`] change it;
//!   each bumps [`Selection::revision`], which panels use to refresh cached data.
//! - [`Selection::retain_live`] drops ids that were destroyed or parked by an undo; the editor calls
//!   it after every history change.
//! - [`Selection::parts`] expands Models and folders to the BaseParts under them (what a gizmo moves).
use std::collections::HashSet;

use metallic_core::{InstanceId, World};

use crate::commands::History;

/// Selected instances in click order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Selection {
    ids: Vec<InstanceId>,
    revision: u64,
}

impl Selection {
    /// An empty selection.
    pub fn new() -> Selection {
        Selection::default()
    }

    /// Selected ids, oldest click first.
    pub fn ids(&self) -> &[InstanceId] {
        &self.ids
    }

    /// The active (last clicked) item.
    pub fn active(&self) -> Option<InstanceId> {
        self.ids.last().copied()
    }

    /// Number of selected items.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// True when nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// True when `id` is selected.
    pub fn contains(&self, id: InstanceId) -> bool {
        self.ids.contains(&id)
    }

    /// Changes whenever the selection changes.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Replaces the selection.
    pub fn set(&mut self, ids: impl IntoIterator<Item = InstanceId>) {
        let mut new: Vec<InstanceId> = Vec::new();
        for id in ids {
            if !new.contains(&id) {
                new.push(id);
            }
        }
        if new != self.ids {
            self.ids = new;
            self.revision += 1;
        }
    }

    /// Selects only `id`.
    pub fn select_only(&mut self, id: InstanceId) {
        self.set([id]);
    }

    /// Adds `id` (or makes it active if already selected).
    pub fn add(&mut self, id: InstanceId) {
        self.ids.retain(|&i| i != id);
        self.ids.push(id);
        self.revision += 1;
    }

    /// Adds `id` if absent, removes it if present (Ctrl-click).
    pub fn toggle(&mut self, id: InstanceId) {
        if self.contains(id) {
            self.ids.retain(|&i| i != id);
            self.revision += 1;
        } else {
            self.add(id);
        }
    }

    /// Deselects everything.
    pub fn clear(&mut self) {
        if !self.ids.is_empty() {
            self.ids.clear();
            self.revision += 1;
        }
    }

    /// Drops ids that no longer exist in the tree (destroyed or parked by `history`).
    pub fn retain_live(&mut self, world: &World, history: &History) {
        let before = self.ids.len();
        self.ids
            .retain(|&id| world.is_alive(id) && !is_detached(world, history, id));
        if self.ids.len() != before {
            self.revision += 1;
        }
    }

    /// Every BasePart that the selection moves: selected parts and the parts under selected
    /// Models and folders, each once, in selection order.
    pub fn parts(&self, world: &World) -> Vec<InstanceId> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for &id in &self.ids {
            collect_parts(world, id, &mut out, &mut seen);
        }
        out
    }
}

impl Selection {
    /// The BaseParts in the subtree of `id` (including `id`).
    pub fn parts_of(world: &World, id: InstanceId) -> Vec<InstanceId> {
        let mut out = Vec::new();
        collect_parts(world, id, &mut out, &mut HashSet::new());
        out
    }
}

/// Pushes the BaseParts in the subtree of `id` (including `id`) into `out`, skipping duplicates.
pub fn collect_parts(
    world: &World,
    id: InstanceId,
    out: &mut Vec<InstanceId>,
    seen: &mut HashSet<InstanceId>,
) {
    let mut stack = vec![id];
    while let Some(cur) = stack.pop() {
        if world.is_a(cur, "BasePart") && seen.insert(cur) {
            out.push(cur);
        }
        for &c in world.children(cur).iter().rev() {
            stack.push(c);
        }
    }
}

/// True when `id` or one of its ancestors is parked by `history`.
pub fn is_detached(world: &World, history: &History, id: InstanceId) -> bool {
    let mut cur = Some(id);
    while let Some(c) = cur {
        if history.is_parked(c) {
            return true;
        }
        cur = world.parent(c);
    }
    false
}
