//! Rigid-body physics for the Metallic Engine: Rapier 3D behind our own interface.
//!
//! A [`PhysicsWorld`] mirrors the BaseParts of a [`metallic_core::World`]:
//! [`PhysicsWorld::sync_from`] creates, updates and removes Rapier bodies to match the tree,
//! [`PhysicsWorld::step`] simulates and writes `Position`/`Orientation` of unanchored parts back,
//! [`PhysicsWorld::raycast`] answers scene queries and [`PhysicsWorld::drain_touched`] reports
//! new contacts. Units are studs and seconds; gravity is 196.2 studs/s2 downward.
//!
//! Mapping: `Anchored` parts are fixed, others dynamic; `Shape` picks the collider;
//! `CanCollide = false` makes a sensor (reports touches, never blocks); `Material` picks density,
//! friction and restitution (see [`material_props`]). An anchored part whose `Position` or
//! `Orientation` was changed since the last step is moved as a kinematic body for that step, with
//! velocity derived from the move, so parts standing on it are carried along.
//! A move faster than [`TELEPORT_SPEED`] is a teleport instead: plain reposition, zero velocity.
#![warn(missing_docs)]

mod sync;
mod touched;

pub use sync::{MaterialProps, material_props};

use metallic_core::{InstanceId, Value, Vec3, World};
use rapier3d::prelude::{
    ActiveCollisionTypes, ActiveEvents, ColliderHandle, PhysicsWorld as RapierWorld, Pose,
    QueryFilter, Ray, RigidBodyBuilder, RigidBodyHandle, RigidBodyType, Rotation, Vector,
};
use std::collections::{HashMap, HashSet};
use sync::{PartState, Signature};
use touched::{EventCollector, TouchTracker};

/// Gravity in studs per second squared (downward).
pub const GRAVITY: f64 = 196.2;
/// Longest `dt` one [`PhysicsWorld::step`] call simulates, in seconds; larger values are clamped.
pub const MAX_STEP: f64 = 0.25;
/// Length of one fixed simulation substep in seconds.
pub const SUBSTEP: f64 = 1.0 / 240.0;
/// Speed in studs/s above which a move of an anchored part counts as a teleport.
///
/// [`PhysicsWorld::step`] derives an anchored part's velocity as distance / simulated time.
/// Above this limit (e.g. a script resetting a platform 50 studs in one frame) the part is
/// repositioned with zero derived velocity instead of being swept as a kinematic body, so parts
/// touching it are not launched.
pub const TELEPORT_SPEED: f64 = 500.0;

/// Result of a successful [`PhysicsWorld::raycast`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    /// The part that was hit.
    pub instance: InstanceId,
    /// World position of the hit.
    pub position: Vec3,
    /// Surface normal at the hit (unit length, pointing away from the surface). A ray that
    /// starts inside a part hits it at distance 0; the normal is then the reversed ray direction.
    pub normal: Vec3,
    /// Distance from the ray origin to the hit, in studs.
    pub distance: f64,
}

/// Rapier handles and bookkeeping for one part.
struct Entry {
    body: RigidBodyHandle,
    collider: ColliderHandle,
    signature: Signature,
    /// Position and orientation as last seen in (or written to) the Instance tree.
    last_position: Vec3,
    last_orientation: Vec3,
    /// Velocity derived from the last kinematic move (anchored parts only).
    derived_velocity: Vec3,
}

/// The physics simulation for one [`World`].
pub struct PhysicsWorld {
    rapier: RapierWorld,
    entries: HashMap<InstanceId, Entry>,
    owners: HashMap<ColliderHandle, InstanceId>,
    collector: EventCollector,
    touches: TouchTracker,
    accumulator: f64,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}

fn to_vector(v: Vec3) -> Vector {
    Vector::new(v.x as f32, v.y as f32, v.z as f32)
}

fn from_vector(v: Vector) -> Vec3 {
    Vec3::new(f64::from(v.x), f64::from(v.y), f64::from(v.z))
}

fn to_pose(position: Vec3, orientation: Vec3) -> Pose {
    let [x, y, z, w] = sync::quat_from_orientation(orientation);
    Pose::from_parts(
        to_vector(position),
        Rotation::from_xyzw(x as f32, y as f32, z as f32, w as f32),
    )
}

fn close(a: Vec3, b: Vec3) -> bool {
    (a - b).magnitude() < 1e-6
}

impl PhysicsWorld {
    /// Creates an empty simulation with studs-scale gravity and a fixed 1/240 s substep.
    pub fn new() -> Self {
        let mut rapier = RapierWorld::new();
        rapier.gravity = Vector::new(0.0, -(GRAVITY as f32), 0.0);
        rapier.integration_parameters.dt = SUBSTEP as f32;
        PhysicsWorld {
            rapier,
            entries: HashMap::new(),
            owners: HashMap::new(),
            collector: EventCollector::default(),
            touches: TouchTracker::default(),
            accumulator: 0.0,
        }
    }

    /// Makes the simulation match the BaseParts currently in `world`.
    ///
    /// New parts get a body, parts whose Size/Shape/Material/Anchored/CanCollide changed are
    /// rebuilt, and parts that were destroyed or are no longer BaseParts are removed. Call it
    /// after adding or removing parts or changing those properties; moving parts needs no sync.
    pub fn sync_from(&mut self, world: &World) {
        let parts = sync::collect_parts(world);
        let alive: HashSet<InstanceId> = parts.iter().copied().collect();
        let gone: Vec<InstanceId> = self
            .entries
            .keys()
            .filter(|id| !alive.contains(id))
            .copied()
            .collect();
        for id in gone {
            self.remove_entry(id);
        }
        for id in parts {
            let Some(state) = sync::read_part(world, id) else {
                continue;
            };
            let stale = self
                .entries
                .get(&id)
                .is_some_and(|e| e.signature != state.signature());
            if stale {
                self.remove_entry(id);
            }
            if !self.entries.contains_key(&id) {
                self.add_entry(id, &state);
            }
        }
        // Refresh the broad phase so raycasts work before the first step.
        self.rapier.detect_collisions(&(), &self.collector);
        self.collect_touches();
    }

    fn add_entry(&mut self, id: InstanceId, s: &PartState) {
        let mat = sync::material_props(&s.material);
        let pose = to_pose(s.position, s.orientation);
        let builder = if s.anchored {
            RigidBodyBuilder::fixed()
        } else {
            RigidBodyBuilder::dynamic()
        }
        .pose(pose);
        let collider = sync::build_collider(&s.shape, s.size)
            .density(mat.density)
            .friction(mat.friction)
            .restitution(mat.restitution)
            .sensor(!s.can_collide)
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .active_collision_types(
                ActiveCollisionTypes::DYNAMIC_DYNAMIC
                    | ActiveCollisionTypes::DYNAMIC_KINEMATIC
                    | ActiveCollisionTypes::DYNAMIC_FIXED
                    | ActiveCollisionTypes::KINEMATIC_KINEMATIC
                    | ActiveCollisionTypes::KINEMATIC_FIXED,
            );
        let (body, collider) = self.rapier.insert(builder, collider);
        self.owners.insert(collider, id);
        self.entries.insert(
            id,
            Entry {
                body,
                collider,
                signature: s.signature(),
                last_position: s.position,
                last_orientation: s.orientation,
                derived_velocity: Vec3::default(),
            },
        );
    }

    fn remove_entry(&mut self, id: InstanceId) {
        if let Some(e) = self.entries.remove(&id) {
            self.owners.remove(&e.collider);
            self.rapier.remove_body(e.body);
            self.touches.forget(id);
        }
    }

    fn collect_touches(&mut self) {
        let events = self.collector.take();
        self.touches.apply(events, &self.owners);
    }

    /// Advances the simulation by `dt` seconds in fixed 1/240 s substeps (the remainder is
    /// carried over to the next call) and writes `Position`/`Orientation` of unanchored parts
    /// back into `world`.
    ///
    /// Before simulating, a changed `Position`/`Orientation` is applied: an anchored part is moved
    /// as a kinematic body (velocity = distance / simulated time, see [`Self::velocity`]), or
    /// just repositioned with zero velocity when that exceeds [`TELEPORT_SPEED`]; an unanchored
    /// part is teleported and its velocity cleared. `dt` is clamped to [`MAX_STEP`]
    /// (0.25 s) so a long frame cannot spiral.
    ///
    /// Callers must call [`Self::sync_from`] after adding, removing or reconfiguring parts
    /// (Size, Shape, Material, Anchored, CanCollide, reparenting); until then `step` keeps using
    /// the Anchored value the body was last synced with. Parts not yet synced are ignored.
    pub fn step(&mut self, world: &mut World, dt: f64) {
        self.accumulator += dt.clamp(0.0, MAX_STEP);
        let substeps = ((self.accumulator / SUBSTEP) + 1e-6).floor() as usize;
        if substeps == 0 {
            return;
        }
        self.accumulator = (self.accumulator - substeps as f64 * SUBSTEP).max(0.0);
        let sim_time = substeps as f64 * SUBSTEP;

        let ids: Vec<InstanceId> = self.entries.keys().copied().collect();
        // (part, target pose) of anchored parts moving this step.
        let mut movers: Vec<(InstanceId, Pose)> = Vec::new();
        for &id in &ids {
            let Some(state) = sync::read_part(world, id) else {
                continue;
            };
            let Some(e) = self.entries.get_mut(&id) else {
                continue;
            };
            let moved = !close(state.position, e.last_position)
                || !close(state.orientation, e.last_orientation);
            let Some(body) = self.rapier.bodies.get_mut(e.body) else {
                continue;
            };
            if e.signature.anchored {
                e.derived_velocity = Vec3::default();
                let v = (state.position - e.last_position) * (1.0 / sim_time);
                if moved && v.magnitude() > TELEPORT_SPEED {
                    // Teleport: plain reposition, no derived velocity (#34).
                    body.set_position(to_pose(state.position, state.orientation), true);
                } else if moved {
                    e.derived_velocity = v;
                    body.set_body_type(RigidBodyType::KinematicVelocityBased, true);
                    body.set_linvel(to_vector(v), true);
                    body.set_rotation(to_pose(state.position, state.orientation).rotation, true);
                    movers.push((id, to_pose(state.position, state.orientation)));
                }
            } else if moved {
                body.set_position(to_pose(state.position, state.orientation), true);
                body.set_linvel(Vector::ZERO, true);
                body.set_angvel(Vector::ZERO, true);
            }
            e.last_position = state.position;
            e.last_orientation = state.orientation;
        }

        for _ in 0..substeps {
            self.rapier.step_with_events(&(), &self.collector);
        }

        // Land movers exactly on their target and make them fixed again.
        for (id, pose) in movers {
            if let Some(body) = self
                .entries
                .get(&id)
                .and_then(|e| self.rapier.bodies.get_mut(e.body))
            {
                body.set_linvel(Vector::ZERO, false);
                body.set_body_type(RigidBodyType::Fixed, false);
                body.set_position(pose, false);
            }
        }

        // Write dynamic bodies back to the Instance tree.
        for &id in &ids {
            let Some(e) = self.entries.get_mut(&id) else {
                continue;
            };
            let Some(body) = self.rapier.bodies.get(e.body) else {
                continue;
            };
            if !body.is_dynamic() || body.is_sleeping() {
                continue;
            }
            let t = from_vector(body.translation());
            let q = body.rotation();
            let o = sync::orientation_from_quat([
                f64::from(q.x),
                f64::from(q.y),
                f64::from(q.z),
                f64::from(q.w),
            ]);
            if world.set(id, "Position", Value::Vector3(t)).is_ok()
                && world.set(id, "Orientation", Value::Vector3(o)).is_ok()
            {
                e.last_position = t;
                e.last_orientation = o;
            }
        }
        self.collect_touches();
    }

    /// Current velocity of a part in studs per second: the body velocity for unanchored parts,
    /// the velocity derived from the last move for anchored ones. `None` for unknown parts.
    pub fn velocity(&self, id: InstanceId) -> Option<Vec3> {
        let e = self.entries.get(&id)?;
        let body = self.rapier.bodies.get(e.body)?;
        Some(if body.is_dynamic() {
            from_vector(body.linvel())
        } else {
            e.derived_velocity
        })
    }

    /// Casts a ray and returns the nearest solid (CanCollide) part hit, skipping `ignore`.
    ///
    /// Like Roblox, the length of `dir` is the maximum distance in studs. Returns `None` when
    /// nothing is hit within that distance or `dir` is zero. Reflects the state of the last
    /// [`Self::sync_from`] or [`Self::step`].
    pub fn raycast(&self, origin: Vec3, dir: Vec3, ignore: &[InstanceId]) -> Option<RayHit> {
        let max = dir.magnitude();
        if max <= 0.0 || !max.is_finite() {
            return None;
        }
        let unit = dir.unit();
        let ray = Ray::new(to_vector(origin), to_vector(unit));
        let skip: Vec<ColliderHandle> = ignore
            .iter()
            .filter_map(|id| self.entries.get(id).map(|e| e.collider))
            .collect();
        let predicate = |h: ColliderHandle, _: &rapier3d::prelude::Collider| !skip.contains(&h);
        let filter = QueryFilter::default()
            .exclude_sensors()
            .predicate(&predicate);
        let (handle, hit) = self
            .rapier
            .cast_ray_and_get_normal(&ray, max as f32, true, filter)?;
        let instance = *self.owners.get(&handle)?;
        let distance = f64::from(hit.time_of_impact);
        // A ray starting inside a collider hits at distance 0 with a zero normal (#34).
        let n = from_vector(hit.normal);
        let len = n.magnitude();
        let normal = if len > 1e-6 && len.is_finite() {
            n * (1.0 / len)
        } else {
            unit * -1.0
        };
        Some(RayHit {
            instance,
            position: origin + unit * distance,
            normal,
            distance,
        })
    }

    /// Returns the part pairs that started touching since the last call. Each contact is
    /// reported once and again only after the parts separated and touched anew. Sensors
    /// (`CanCollide = false`) report touches with moving parts.
    pub fn drain_touched(&mut self) -> Vec<(InstanceId, InstanceId)> {
        self.touches.drain()
    }
}
