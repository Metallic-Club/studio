//! Runtime-owned signature/dependency cache; the physics wrapper owns its own synchronization.

use metallic_core::{ChangeCursor, Changes, InstanceId, Value, World};
use metallic_physics::PhysicsWorld;
use std::collections::{HashMap, HashSet};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq)]
struct PartShape {
    size: Value,
    shape: Value,
    material: Value,
    anchored: Value,
    can_collide: Value,
    character: bool,
}

/// Decides when physics needs synchronization without rebuilding a full scene snapshot per tick.
#[derive(Default)]
pub(crate) struct PhysicsSyncCache {
    cursor: Option<ChangeCursor>,
    parts: HashMap<InstanceId, PartShape>,
    /// Only direct Humanoid children of eligible Models under Workspace are indexed.
    humanoids: HashMap<InstanceId, InstanceId>,
    model_humanoids: HashMap<InstanceId, usize>,
    #[cfg(test)]
    work: SyncWork,
}

#[cfg(test)]
#[derive(Debug, Default, PartialEq, Eq)]
struct SyncWork {
    full_scans: usize,
    full_instances: usize,
    subtree_instances: usize,
    part_reads: usize,
    physics_syncs: usize,
}

impl PhysicsSyncCache {
    /// Explicit start/respawn sync remains query-ready; valid cache history still updates sparsely.
    pub(crate) fn synchronize(&mut self, physics: &mut PhysicsWorld, world: &World, force: bool) {
        let changed = self.refresh(world);
        if changed || force {
            #[cfg(test)]
            {
                self.work.physics_syncs += 1;
            }
            physics.sync_from(world);
        }
    }

    fn refresh(&mut self, world: &World) -> bool {
        #[cfg(test)]
        {
            self.work = SyncWork::default();
        }
        let now = world.change_cursor();
        if self.cursor == Some(now) {
            return false;
        }
        let changes = self
            .cursor
            .map_or(Changes::Overflow, |cursor| world.changes_since(cursor).0);
        let changed = match changes {
            Changes::Overflow => {
                self.reconcile(world);
                true
            }
            Changes::Ids(ids) => self.refresh_ids(world, ids),
        };
        self.cursor = Some(now);
        changed
    }

    fn read_shape(&mut self, world: &World, id: InstanceId, character: bool) -> PartShape {
        #[cfg(test)]
        {
            self.work.part_reads += 1;
        }
        let get = |property| world.get(id, property).unwrap_or(Value::Nil);
        PartShape {
            size: get("Size"),
            shape: get("Shape"),
            material: get("Material"),
            anchored: get("Anchored"),
            can_collide: get("CanCollide"),
            character,
        }
    }

    fn reconcile(&mut self, world: &World) {
        #[cfg(test)]
        {
            self.work.full_scans += 1;
        }
        self.parts.clear();
        self.humanoids.clear();
        self.model_humanoids.clear();
        let mut stack: Vec<_> = world
            .root("Workspace")
            .map(|id| (id, false))
            .into_iter()
            .collect();
        while let Some((id, inherited_character)) = stack.pop() {
            #[cfg(test)]
            {
                self.work.full_instances += 1;
            }
            let mut count = 0;
            if world.is_a(id, "Model") {
                for &child in world.children(id) {
                    if world.is_a(child, "Humanoid") {
                        self.humanoids.insert(child, id);
                        count += 1;
                    }
                }
            }
            if count > 0 {
                self.model_humanoids.insert(id, count);
            }
            if world.is_a(id, "BasePart") {
                let shape = self.read_shape(world, id, inherited_character);
                self.parts.insert(id, shape);
            }
            let character = inherited_character || count > 0;
            stack.extend(world.children(id).iter().map(|&child| (child, character)));
        }
    }

    fn refresh_ids(&mut self, world: &World, ids: Vec<InstanceId>) -> bool {
        // Update all direct memberships before comparing net Model classification. Journal IDs
        // describe final state, so replacing/reparenting Humanoids in one batch is order-independent.
        let mut before = HashMap::new();
        for &id in &ids {
            let old = self.humanoids.get(&id).copied();
            let new = humanoid_model(world, id);
            if old == new {
                continue;
            }
            if let Some(model) = old {
                before
                    .entry(model)
                    .or_insert_with(|| self.model_humanoids.contains_key(&model));
                let count = self
                    .model_humanoids
                    .get_mut(&model)
                    .expect("indexed Humanoid count");
                *count -= 1;
                if *count == 0 {
                    self.model_humanoids.remove(&model);
                }
                self.humanoids.remove(&id);
            }
            if let Some(model) = new {
                before
                    .entry(model)
                    .or_insert_with(|| self.model_humanoids.contains_key(&model));
                *self.model_humanoids.entry(model).or_default() += 1;
                self.humanoids.insert(id, model);
            }
        }
        let mut candidates: HashSet<_> = ids
            .into_iter()
            .filter(|id| self.parts.contains_key(id) || world.is_a(*id, "BasePart"))
            .collect();
        for (model, was_character) in before {
            if was_character == self.model_humanoids.contains_key(&model) {
                continue;
            }
            let mut stack = world.children(model).to_vec();
            while let Some(id) = stack.pop() {
                #[cfg(test)]
                {
                    self.work.subtree_instances += 1;
                }
                if world.is_a(id, "BasePart") {
                    candidates.insert(id);
                }
                stack.extend_from_slice(world.children(id));
            }
        }
        let mut changed = false;
        for id in candidates {
            let shape = self
                .membership(world, id)
                .map(|character| self.read_shape(world, id, character));
            if self.parts.get(&id) == shape.as_ref() {
                continue;
            }
            changed = true;
            if let Some(shape) = shape {
                self.parts.insert(id, shape);
            } else {
                self.parts.remove(&id);
            }
        }
        changed
    }

    fn membership(&self, world: &World, id: InstanceId) -> Option<bool> {
        if !world.is_a(id, "BasePart") {
            return None;
        }
        let workspace = world.root("Workspace")?;
        let mut current = world.parent(id);
        let mut character = false;
        while let Some(ancestor) = current {
            if ancestor == workspace {
                return Some(character);
            }
            character |= self.model_humanoids.contains_key(&ancestor);
            current = world.parent(ancestor);
        }
        None
    }
}

fn humanoid_model(world: &World, id: InstanceId) -> Option<InstanceId> {
    if !world.is_a(id, "Humanoid") {
        return None;
    }
    let model = world.parent(id)?;
    if !world.is_a(model, "Model") {
        return None;
    }
    let workspace = world.root("Workspace")?;
    let mut current = Some(model);
    while let Some(ancestor) = current {
        if ancestor == workspace {
            return Some(model);
        }
        current = world.parent(ancestor);
    }
    None
}
