//! Collects Rapier contact events and turns them into "each new pair once" touch pairs.
use metallic_core::InstanceId;
use rapier3d::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// Event handler handed to Rapier that just records start/stop events for later processing.
#[derive(Default)]
pub(crate) struct EventCollector {
    events: Mutex<Vec<CollisionEvent>>,
}

impl EventCollector {
    /// Takes all events recorded since the last call.
    pub(crate) fn take(&self) -> Vec<CollisionEvent> {
        match self.events.lock() {
            Ok(mut g) => std::mem::take(&mut *g),
            Err(_) => Vec::new(),
        }
    }
}

impl EventHandler for EventCollector {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        event: CollisionEvent,
        _contact_pair: Option<&ContactPair>,
    ) {
        if let Ok(mut g) = self.events.lock() {
            g.push(event);
        }
    }

    fn handle_contact_force_event(
        &self,
        _dt: Real,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        _contact_pair: &ContactPair,
        _total_force_magnitude: Real,
    ) {
    }

    fn handle_soft_body_tear_event(&self, _soft_bodies: &SoftBodySet, _event: &SoftBodyTearEvent) {}
}

/// Tracks which instance pairs are currently touching and queues new ones.
#[derive(Default)]
pub(crate) struct TouchTracker {
    touching: HashSet<(InstanceId, InstanceId)>,
    queue: Vec<(InstanceId, InstanceId)>,
}

fn key(a: InstanceId, b: InstanceId) -> (InstanceId, InstanceId) {
    if a <= b { (a, b) } else { (b, a) }
}

impl TouchTracker {
    /// Applies Rapier events; a pair is queued only when it was not already touching. Pairs
    /// involving an `ignored` instance (character parts) are dropped.
    pub(crate) fn apply(
        &mut self,
        events: Vec<CollisionEvent>,
        owners: &HashMap<ColliderHandle, InstanceId>,
        ignored: &HashSet<InstanceId>,
    ) {
        for ev in events {
            let (h1, h2) = match ev {
                CollisionEvent::Started(a, b, _) | CollisionEvent::Stopped(a, b, _) => (a, b),
            };
            let (Some(&a), Some(&b)) = (owners.get(&h1), owners.get(&h2)) else {
                continue;
            };
            if ignored.contains(&a) || ignored.contains(&b) {
                continue;
            }
            let k = key(a, b);
            if ev.started() {
                if self.touching.insert(k) {
                    self.queue.push((a, b));
                }
            } else {
                self.touching.remove(&k);
            }
        }
    }

    /// Forgets every pair involving `id` (the part was removed).
    pub(crate) fn forget(&mut self, id: InstanceId) {
        self.touching.retain(|&(a, b)| a != id && b != id);
    }

    /// Takes the queued new pairs.
    pub(crate) fn drain(&mut self) -> Vec<(InstanceId, InstanceId)> {
        std::mem::take(&mut self.queue)
    }
}
