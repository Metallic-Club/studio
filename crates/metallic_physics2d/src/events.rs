//! Contact bookkeeping: turns backend collision events into "new touch" pairs and sensor
//! enter/exit events, and implements one-way platforms as a contact hook.
use crate::desc::BodyId;
use rapier2d::prelude::{
    ColliderHandle, ColliderSet, CollisionEvent, ContactModificationContext, ContactPair,
    EventHandler, PhysicsHooks, Real, RigidBodySet, SoftBodySet, SoftBodyTearEvent, Vector,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

/// Event handler handed to the backend that only records start/stop events.
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

/// Whether a body entered or left a sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensorEventKind {
    /// The body started overlapping the sensor.
    Enter,
    /// The body stopped overlapping the sensor (or one of them was removed).
    Exit,
}

/// A body entering or leaving a sensor (trigger) collider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SensorEvent {
    /// The body that owns the sensor collider.
    pub sensor: BodyId,
    /// The body that entered or left.
    pub other: BodyId,
    /// Enter or exit.
    pub kind: SensorEventKind,
}

fn key(a: BodyId, b: BodyId) -> (BodyId, BodyId) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Per body pair: how many collider pairs the backend says touch, and whether a character
/// controller reported touching it this tick.
#[derive(Default, Clone, Copy)]
struct TouchCount {
    backend: u32,
    character: bool,
}

impl TouchCount {
    fn any(self) -> bool {
        self.backend > 0 || self.character
    }
}

/// Tracks which body pairs touch and which bodies overlap which sensors.
#[derive(Default)]
pub(crate) struct ContactTracker {
    touching: BTreeMap<(BodyId, BodyId), TouchCount>,
    touched_queue: Vec<(BodyId, BodyId)>,
    /// (sensor body, other body) -> number of overlapping collider pairs.
    overlaps: BTreeMap<(BodyId, BodyId), u32>,
    sensor_queue: Vec<SensorEvent>,
}

impl ContactTracker {
    fn touch_delta(&mut self, a: BodyId, b: BodyId, f: impl FnOnce(&mut TouchCount)) {
        let k = key(a, b);
        let entry = self.touching.entry(k).or_default();
        let before = entry.any();
        f(entry);
        let after = entry.any();
        if !before && after {
            self.touched_queue.push((a, b));
        }
        if !after {
            self.touching.remove(&k);
        }
    }

    fn overlap_delta(&mut self, sensor: BodyId, other: BodyId, started: bool) {
        let count = self.overlaps.entry((sensor, other)).or_insert(0);
        if started {
            *count += 1;
            if *count == 1 {
                self.sensor_queue.push(SensorEvent {
                    sensor,
                    other,
                    kind: SensorEventKind::Enter,
                });
            }
        } else if *count > 0 {
            *count -= 1;
            if *count == 0 {
                self.overlaps.remove(&(sensor, other));
                self.sensor_queue.push(SensorEvent {
                    sensor,
                    other,
                    kind: SensorEventKind::Exit,
                });
            }
        }
    }

    /// Applies backend events. Events for colliders we no longer know are ignored.
    pub(crate) fn apply(
        &mut self,
        events: Vec<CollisionEvent>,
        owners: &HashMap<ColliderHandle, BodyId>,
        sensors: &HashSet<ColliderHandle>,
    ) {
        for ev in events {
            let (h1, h2, started) = match ev {
                CollisionEvent::Started(a, b, _) => (a, b, true),
                CollisionEvent::Stopped(a, b, _) => (a, b, false),
            };
            let (Some(&a), Some(&b)) = (owners.get(&h1), owners.get(&h2)) else {
                continue;
            };
            if a == b {
                continue;
            }
            let s1 = sensors.contains(&h1);
            let s2 = sensors.contains(&h2);
            if s1 || s2 {
                if s1 {
                    self.overlap_delta(a, b, started);
                }
                if s2 {
                    self.overlap_delta(b, a, started);
                }
            }
            self.touch_delta(a, b, |c| {
                if started {
                    c.backend += 1;
                } else {
                    c.backend = c.backend.saturating_sub(1);
                }
            });
        }
    }

    /// Replaces the set of bodies a character touched this tick.
    pub(crate) fn set_character_touches(
        &mut self,
        character: BodyId,
        previous: &[BodyId],
        current: &[BodyId],
    ) {
        for &other in previous {
            if !current.contains(&other) {
                self.touch_delta(character, other, |c| c.character = false);
            }
        }
        for &other in current {
            if !previous.contains(&other) {
                self.touch_delta(character, other, |c| c.character = true);
            }
        }
    }

    /// Forgets everything about a removed body; sensors it overlapped (or that it owned)
    /// report an exit.
    pub(crate) fn forget(&mut self, id: BodyId) {
        self.touching.retain(|&(a, b), _| a != id && b != id);
        let gone: Vec<(BodyId, BodyId)> = self
            .overlaps
            .keys()
            .filter(|(s, o)| *s == id || *o == id)
            .copied()
            .collect();
        for (sensor, other) in gone {
            self.overlaps.remove(&(sensor, other));
            self.sensor_queue.push(SensorEvent {
                sensor,
                other,
                kind: SensorEventKind::Exit,
            });
        }
    }

    /// Takes the pairs that started touching.
    pub(crate) fn drain_touched(&mut self) -> Vec<(BodyId, BodyId)> {
        std::mem::take(&mut self.touched_queue)
    }

    /// Takes the sensor enter/exit events.
    pub(crate) fn drain_sensor_events(&mut self) -> Vec<SensorEvent> {
        std::mem::take(&mut self.sensor_queue)
    }

    /// True while the two bodies touch (or overlap, for sensors).
    pub(crate) fn is_touching(&self, a: BodyId, b: BodyId) -> bool {
        self.touching.contains_key(&key(a, b))
    }
}

/// Largest angle between a contact normal and a platform's one-way direction for the contact
/// to count as "landing on it" (45 degrees).
pub(crate) const ONE_WAY_MAX_ANGLE: f32 = std::f32::consts::FRAC_PI_4;

const UNKNOWN: u32 = 0;
const ALLOWED: u32 = 1;
const FORBIDDEN: u32 = 2;

/// Contact hook implementing one-way platforms for simulated bodies.
///
/// A contact counts when its normal (pointing out of the platform) is within 45 degrees of the
/// platform's one-way direction. Once a body has started passing through, contacts stay off
/// until it is fully out, so it does not pop up mid-way.
#[derive(Default)]
pub(crate) struct OneWayHooks {
    /// Platform collider -> one-way direction in collider space.
    pub(crate) platforms: HashMap<ColliderHandle, Vector>,
}

impl PhysicsHooks for OneWayHooks {
    fn modify_solver_contacts(&self, ctx: &mut ContactModificationContext) {
        let (platform, local_n, out_sign) = if let Some(n) = self.platforms.get(&ctx.collider1) {
            (ctx.collider1, *n, 1.0)
        } else if let Some(n) = self.platforms.get(&ctx.collider2) {
            (ctx.collider2, *n, -1.0)
        } else {
            return;
        };
        let Some(collider) = ctx.colliders.get(platform) else {
            return;
        };
        let allowed = collider.position().rotation * local_n;
        let Some(rigid) = ctx.rigid_mut() else {
            return;
        };
        let out_normal = *rigid.normal * out_sign;
        let ok = out_normal.dot(allowed) >= ONE_WAY_MAX_ANGLE.cos();
        match *rigid.user_data {
            ALLOWED => {
                if rigid.solver_contacts.is_empty() {
                    *rigid.user_data = UNKNOWN;
                }
            }
            FORBIDDEN => {
                if ok && rigid.solver_contacts.iter().all(|c| c.dist > 0.0) {
                    *rigid.user_data = ALLOWED;
                } else {
                    rigid.solver_contacts.clear();
                }
            }
            _ => {
                if ok {
                    *rigid.user_data = ALLOWED;
                } else {
                    rigid.solver_contacts.clear();
                    if out_normal.length_squared() > 0.1 {
                        *rigid.user_data = FORBIDDEN;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_and_backend_touches_merge() {
        let mut t = ContactTracker::default();
        let (a, b) = (BodyId(1), BodyId(2));
        t.set_character_touches(a, &[], &[b]);
        assert_eq!(t.drain_touched(), vec![(a, b)]);
        // Still touching next tick: no new event.
        t.set_character_touches(a, &[b], &[b]);
        assert!(t.drain_touched().is_empty());
        t.set_character_touches(a, &[b], &[]);
        assert!(!t.is_touching(a, b));
        t.set_character_touches(a, &[], &[b]);
        assert_eq!(t.drain_touched(), vec![(a, b)]);
    }

    #[test]
    fn forgetting_a_body_exits_its_sensors() {
        let mut t = ContactTracker::default();
        t.overlap_delta(BodyId(1), BodyId(2), true);
        assert_eq!(t.drain_sensor_events().len(), 1);
        t.forget(BodyId(2));
        let ev = t.drain_sensor_events();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].kind, SensorEventKind::Exit);
    }
}
