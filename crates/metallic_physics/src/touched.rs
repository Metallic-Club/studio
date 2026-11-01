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
    /// Stops not yet drained, each with the [`TouchTracker::drain`] count when it happened.
    ended: Vec<((InstanceId, InstanceId), u64)>,
    /// Pairs of a part rebuilt in place, waiting for the next contact pass to tell whether they
    /// still touch.
    rebuilt: Vec<(InstanceId, InstanceId)>,
    /// How many times the new pairs were drained.
    drains: u64,
}

/// Why a part's collider went away.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Removed {
    /// Destroyed or out of the Workspace: its pairs end now.
    Gone,
    /// Rebuilt with a new collider (Size, Shape or CanCollide changed): its pairs end only if the
    /// new collider does not touch in the next contact pass.
    Rebuilt,
}

fn key(a: InstanceId, b: InstanceId) -> (InstanceId, InstanceId) {
    if a <= b { (a, b) } else { (b, a) }
}

impl TouchTracker {
    /// Applies Rapier events; a pair is queued only when it was not already touching, and a
    /// stop is queued only for a pair that was touching, so every `Touched` is followed by one
    /// `TouchEnded`. A pair that starts again before anyone drained the new pairs since its stop
    /// cancels that stop (it separated and met again within one drain, so `Touched` cannot
    /// arrive before its own `TouchEnded`), and a rebuilt part's pair that starts again was
    /// never apart. Pairs involving an `ignored` instance (character parts) are dropped.
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
                if let Some(i) = self.rebuilt.iter().position(|&r| r == k) {
                    self.rebuilt.remove(i);
                    self.touching.insert(k);
                } else if let Some(i) = self
                    .ended
                    .iter()
                    .position(|&(e, at)| e == k && at == self.drains)
                {
                    self.ended.remove(i);
                    self.touching.insert(k);
                } else if self.touching.insert(k) {
                    self.queue.push((a, b));
                }
            } else if self.touching.remove(&k) {
                self.ended.push((k, self.drains));
            }
        }
    }

    /// The collider of `id` was removed. Its pairs end now if the part is gone, so the part it
    /// touched hears `TouchEnded`; a rebuilt part's pairs wait for [`TouchTracker::settle`].
    pub(crate) fn forget(&mut self, id: InstanceId, why: Removed) {
        let mut pairs: Vec<(InstanceId, InstanceId)> = self
            .touching
            .iter()
            .filter(|&&(a, b)| a == id || b == id)
            .copied()
            .collect();
        pairs.sort_unstable();
        for k in &pairs {
            self.touching.remove(k);
        }
        match why {
            Removed::Gone => {
                let at = self.drains;
                self.ended.extend(pairs.into_iter().map(|k| (k, at)));
                // A rebuilt pair whose part is now gone ends too.
                let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.rebuilt)
                    .into_iter()
                    .partition(|&(a, b)| a == id || b == id);
                self.ended.extend(gone.into_iter().map(|k| (k, at)));
                self.rebuilt = kept;
            }
            Removed::Rebuilt => self.rebuilt.extend(pairs),
        }
    }

    /// Call after a contact pass that covered rebuilt colliders (a physics step): rebuilt pairs
    /// that did not touch again end now.
    pub(crate) fn settle(&mut self) {
        let at = self.drains;
        self.ended.extend(
            std::mem::take(&mut self.rebuilt)
                .into_iter()
                .map(|k| (k, at)),
        );
    }

    /// Takes the pairs that stopped touching since the last call.
    pub(crate) fn drain_ended(&mut self) -> Vec<(InstanceId, InstanceId)> {
        std::mem::take(&mut self.ended)
            .into_iter()
            .map(|(k, _)| k)
            .collect()
    }

    /// Takes the queued new pairs.
    pub(crate) fn drain(&mut self) -> Vec<(InstanceId, InstanceId)> {
        self.drains += 1;
        std::mem::take(&mut self.queue)
    }
}
