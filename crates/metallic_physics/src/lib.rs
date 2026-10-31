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
//! linear and angular velocity derived from the move, so parts standing on it are carried along
//! and parts in its way are pushed, by rotating movers too.
//!
//! Characters: [`spawn_character`] builds a Roblox-style character Model (a `HumanoidRootPart`
//! and 15 invisible R15 body parts that the runtime draws the skinned avatar over) for a Player
//! and [`CharacterController`] moves it each frame (walk, jump, slopes, steps, moving platforms,
//! death and respawn, checkpoints, touches). Character parts are anchored, so the physics moves
//! them as kinematic bodies. The `HumanoidRootPart` is solid (it pushes loose parts); the body
//! parts have CanCollide off, so they are sensors that never collide with or push anything. They
//! hold the bone-attached touch volumes ([`BodyPart`], [`BodyPose`]), which are queries of the
//! [`CharacterController`]. Nothing is special-cased by name: an NPC Model with R15-named solid
//! parts keeps its collisions. [`PhysicsWorld::drain_touched`]
//! skips every pair involving a character part, and the runtime gets character touches from
//! [`CharacterEvent::Touched`] and [`CharacterEvent::TouchEnded`] instead.
#![warn(missing_docs)]

#[cfg(test)]
mod alloc_count;
mod avatar;
mod body;
mod character;
mod limbs;
mod sync;
mod touched;

pub use avatar::VIOLET;
pub use body::{BODY_PART_COUNT, BodyPart, BodyPose, MAX_TOUCH_CANDIDATES, TouchQueryCost};
pub use character::{
    CharacterController, CharacterEvent, CharacterState, HumanoidState, MAX_PUSH, MAX_STEP_HEIGHT,
    MoveInput, RESPAWN_DELAY, ROOT_HEIGHT, RUN_MULTIPLIER, SupportContact, WORLD_LIMIT,
    place_character, spawn_character,
};
pub use limbs::MAX_CHARACTER_PAIRS;
pub use sync::{MaterialProps, material_props};

use metallic_core::{InstanceId, Value, Vec3, World};
use rapier3d::prelude::{
    ActiveCollisionTypes, ActiveEvents, ColliderHandle, Group, InteractionGroups,
    InteractionTestMode, PhysicsWorld as RapierWorld, Pose, QueryFilter, QueryPipeline, Ray,
    RigidBodyBuilder, RigidBodyHandle, RigidBodyType, Rotation, Vector,
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

/// Collision group of character parts; they interact with everything except each other.
const CHARACTER_GROUP: Group = Group::GROUP_1;

/// Result of a successful [`PhysicsWorld::raycast`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    /// The part that was hit.
    pub instance: InstanceId,
    /// World position of the hit.
    pub position: Vec3,
    /// Surface normal at the hit (unit length, pointing away from the surface).
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
    /// Angular velocity (radians/s) derived from the last kinematic move (anchored parts only).
    derived_angular_velocity: Vec3,
}

/// The physics simulation for one [`World`].
pub struct PhysicsWorld {
    rapier: RapierWorld,
    entries: HashMap<InstanceId, Entry>,
    owners: HashMap<ColliderHandle, InstanceId>,
    collector: EventCollector,
    touches: TouchTracker,
    /// Synced parts that belong to a character; [`Self::drain_touched`] never reports them.
    characters: HashSet<InstanceId>,
    accumulator: f64,
    /// Replica mode: no body is ever integrated (see [`PhysicsWorld::new_replica`]).
    replica: bool,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn to_vector(v: Vec3) -> Vector {
    Vector::new(v.x as f32, v.y as f32, v.z as f32)
}

pub(crate) fn from_vector(v: Vector) -> Vec3 {
    Vec3::new(f64::from(v.x), f64::from(v.y), f64::from(v.z))
}

pub(crate) fn to_pose(position: Vec3, orientation: Vec3) -> Pose {
    let [x, y, z, w] = sync::quat_from_orientation(orientation);
    Pose::from_parts(
        to_vector(position),
        Rotation::from_xyzw(x as f32, y as f32, z as f32, w as f32),
    )
}

/// Angular velocity in radians per second (world axis scaled by the rate) that turns orientation
/// `from` into `to` (Roblox Euler degrees) in `time` seconds, along the shortest arc.
fn angular_velocity_between(from: Vec3, to: Vec3, time: f64) -> Vec3 {
    let [ax, ay, az, aw] = sync::quat_from_orientation(from);
    let [bx, by, bz, bw] = sync::quat_from_orientation(to);
    // delta = to * conj(from); the vector part is aw*b - bw*a - b x a.
    let mut w = bw * aw + bx * ax + by * ay + bz * az;
    let mut v = Vec3::new(
        aw * bx - bw * ax - (by * az - bz * ay),
        aw * by - bw * ay - (bz * ax - bx * az),
        aw * bz - bw * az - (bx * ay - by * ax),
    );
    if w < 0.0 {
        w = -w;
        v = v * -1.0;
    }
    let s = v.magnitude();
    if s < 1e-12 || time <= 0.0 {
        return Vec3::default();
    }
    v * (2.0 * s.atan2(w) / (s * time))
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
            characters: HashSet::new(),
            accumulator: 0.0,
            replica: false,
        }
    }

    /// Creates a simulation for clients, which never simulate loose parts: every part becomes a
    /// fixed collider (kinematic while it moves) that follows its `Position`/`Orientation` in
    /// the World, so raycasts and character sweeps see it where the server put it. `step` never
    /// integrates or writes back bodies.
    pub fn new_replica() -> Self {
        PhysicsWorld {
            replica: true,
            ..Self::new()
        }
    }

    /// Makes the simulation match the BaseParts currently in `world`.
    ///
    /// New parts get a body, parts whose Size/Shape/Material/Anchored/CanCollide changed are
    /// rebuilt, and parts that were destroyed or are no longer BaseParts are removed. Call it
    /// after adding or removing parts or changing those properties; moving parts needs no sync.
    pub fn sync_from(&mut self, world: &World) {
        let parts = sync::collect_parts(world);
        let alive: HashSet<InstanceId> = parts.iter().map(|&(id, _)| id).collect();
        let gone: Vec<InstanceId> = self
            .entries
            .keys()
            .filter(|id| !alive.contains(id))
            .copied()
            .collect();
        for id in gone {
            self.remove_entry(id, touched::Removed::Gone);
        }
        for (id, character) in parts {
            let Some(state) = sync::read_part(world, id) else {
                continue;
            };
            let stale = self
                .entries
                .get(&id)
                .is_some_and(|e| e.signature != state.signature(character));
            if stale {
                self.remove_entry(id, touched::Removed::Rebuilt);
            }
            if !self.entries.contains_key(&id) {
                self.add_entry(id, &state, character);
            }
        }
        // Refresh the broad phase so raycasts work before the first step.
        self.rapier.detect_collisions(&(), &self.collector);
        self.collect_touches();
    }

    fn add_entry(&mut self, id: InstanceId, s: &PartState, character: bool) {
        let mat = sync::material_props(&s.material);
        let pose = to_pose(s.position, s.orientation);
        let builder = if s.anchored || self.replica {
            RigidBodyBuilder::fixed()
        } else {
            RigidBodyBuilder::dynamic()
        }
        .pose(pose);
        let groups = if character {
            InteractionGroups::new(
                CHARACTER_GROUP,
                Group::ALL.difference(CHARACTER_GROUP),
                InteractionTestMode::And,
            )
        } else {
            InteractionGroups::all()
        };
        let collider = sync::build_collider(&s.shape, s.size)
            .collision_groups(groups)
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
                signature: s.signature(character),
                last_position: s.position,
                last_orientation: s.orientation,
                derived_velocity: Vec3::default(),
                derived_angular_velocity: Vec3::default(),
            },
        );
        if character {
            self.characters.insert(id);
        }
    }

    fn remove_entry(&mut self, id: InstanceId, why: touched::Removed) {
        if let Some(e) = self.entries.remove(&id) {
            self.owners.remove(&e.collider);
            self.rapier.remove_body(e.body);
            self.touches.forget(id, why);
            self.characters.remove(&id);
        }
    }

    fn collect_touches(&mut self) {
        let events = self.collector.take();
        self.touches.apply(events, &self.owners, &self.characters);
    }

    /// Advances the simulation by `dt` seconds in fixed 1/240 s substeps (the remainder is
    /// carried over to the next call) and writes `Position`/`Orientation` of unanchored parts
    /// back into `world`.
    ///
    /// Before simulating, a changed `Position`/`Orientation` is applied: an anchored part is moved
    /// as a kinematic body (velocity = distance / simulated time, see [`Self::velocity`];
    /// angular velocity = shortest-arc rotation / simulated time, see [`Self::angular_velocity`]),
    /// so it sweeps through the move and pushes or carries loose parts; an unanchored part is
    /// teleported and its velocity cleared. `dt` is clamped to [`MAX_STEP`] (0.25 s) so a long
    /// frame cannot spiral; a negative or non-finite `dt` (NaN, infinity) counts as 0, so that
    /// call simulates nothing and later calls work normally.
    ///
    /// Callers must call [`Self::sync_from`] after adding, removing or reconfiguring parts
    /// (Size, Shape, Material, Anchored, CanCollide, reparenting); until then `step` keeps using
    /// the Anchored value the body was last synced with. Parts not yet synced are ignored.
    pub fn step(&mut self, world: &mut World, dt: f64) {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, MAX_STEP)
        } else {
            0.0
        };
        self.accumulator += dt;
        if !self.accumulator.is_finite() {
            self.accumulator = 0.0;
        }
        let substeps = ((self.accumulator / SUBSTEP) + 1e-6).floor() as usize;
        if substeps == 0 {
            return;
        }
        self.accumulator = (self.accumulator - substeps as f64 * SUBSTEP).max(0.0);
        let sim_time = substeps as f64 * SUBSTEP;

        let replica = self.replica;
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
            if e.signature.anchored || replica {
                e.derived_velocity = Vec3::default();
                e.derived_angular_velocity = Vec3::default();
                if moved {
                    let v = (state.position - e.last_position) * (1.0 / sim_time);
                    let w =
                        angular_velocity_between(e.last_orientation, state.orientation, sim_time);
                    e.derived_velocity = v;
                    e.derived_angular_velocity = w;
                    body.set_body_type(RigidBodyType::KinematicVelocityBased, true);
                    body.set_linvel(to_vector(v), true);
                    body.set_angvel(to_vector(w), true);
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
                body.set_angvel(Vector::ZERO, false);
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
        // Rebuilt parts' pairs that did not touch again in this step end now.
        self.touches.settle();
    }

    /// Current velocity of a part in studs per second: the body velocity for unanchored parts,
    /// the velocity derived from the last move for anchored ones (in a replica world every part
    /// reports the derived, kinematic velocity). `None` for unknown parts.
    pub fn velocity(&self, id: InstanceId) -> Option<Vec3> {
        let e = self.entries.get(&id)?;
        let body = self.rapier.bodies.get(e.body)?;
        Some(if body.is_dynamic() {
            from_vector(body.linvel())
        } else {
            e.derived_velocity
        })
    }

    /// Current angular velocity of a part in radians per second (world rotation axis scaled by
    /// the rate): the body's for unanchored parts, the one derived from the last orientation
    /// change for anchored ones (in a replica world every part reports the derived value). A point `p` on the part moves at
    /// `velocity + angular_velocity × (p - Position)`. `None` for unknown parts.
    pub fn angular_velocity(&self, id: InstanceId) -> Option<Vec3> {
        let e = self.entries.get(&id)?;
        let body = self.rapier.bodies.get(e.body)?;
        Some(if body.is_dynamic() {
            from_vector(body.angvel())
        } else {
            e.derived_angular_velocity
        })
    }

    /// The pose (`Position`, `Orientation`) the next [`Self::step`] gives a part. For an anchored
    /// part (every part in a replica world) that is its pose in `world` now, which a script may
    /// have changed since the last step; for a simulated part it is the current body pose. `None`
    /// for unknown parts. The character controller carries riders by the difference between this
    /// and the body pose, so a rider stays at most one tick behind a part a script moves every
    /// tick (#374). A future teleport rule (#34) has to apply here as well as in `step`.
    pub fn scripted_pose(&self, world: &World, id: InstanceId) -> Option<(Vec3, Vec3)> {
        let e = self.entries.get(&id)?;
        let body = self.rapier.bodies.get(e.body)?;
        if !(e.signature.anchored || self.replica) {
            let q = body.rotation();
            let o = sync::orientation_from_quat([
                f64::from(q.x),
                f64::from(q.y),
                f64::from(q.z),
                f64::from(q.w),
            ]);
            return Some((from_vector(body.translation()), o));
        }
        Some(match sync::read_part(world, id) {
            // The same test `step` uses to decide whether the part moved.
            Some(state)
                if !close(state.position, e.last_position)
                    || !close(state.orientation, e.last_orientation) =>
            {
                (state.position, state.orientation)
            }
            _ => (e.last_position, e.last_orientation),
        })
    }

    /// [`Self::scripted_pose`] as a body pose: exactly the current body pose when the part will
    /// not move, so a still support never shifts its rider.
    pub(crate) fn scripted_body_pose(&self, world: &World, id: InstanceId) -> Option<Pose> {
        let e = self.entries.get(&id)?;
        let body = self.rapier.bodies.get(e.body)?;
        let (position, orientation) = self.scripted_pose(world, id)?;
        if !(e.signature.anchored || self.replica)
            || (close(position, e.last_position) && close(orientation, e.last_orientation))
        {
            return Some(*body.position());
        }
        Some(to_pose(position, orientation))
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
        Some(RayHit {
            instance,
            position: origin + unit * distance,
            normal: from_vector(hit.normal),
            distance,
        })
    }

    /// The collider of a synced part.
    pub(crate) fn collider_of(&self, id: InstanceId) -> Option<ColliderHandle> {
        self.entries.get(&id).map(|e| e.collider)
    }

    /// The part that owns a collider.
    pub(crate) fn owner_of(&self, handle: ColliderHandle) -> Option<InstanceId> {
        self.owners.get(&handle).copied()
    }

    /// Rapier scene queries over the current colliders, restricted by `filter`.
    pub(crate) fn queries<'a>(&'a self, filter: QueryFilter<'a>) -> QueryPipeline<'a> {
        self.rapier.query_pipeline_with_filter(filter)
    }

    /// Returns the part pairs that started touching since the last call. Each contact is
    /// reported once and again only after the parts separated and touched anew. Sensors
    /// (`CanCollide = false`) report touches with moving parts.
    /// Pairs involving a character part are never reported; character touches come from
    /// [`CharacterEvent::Touched`].
    pub fn drain_touched(&mut self) -> Vec<(InstanceId, InstanceId)> {
        self.touches.drain()
    }

    /// Returns the part pairs that stopped touching since the last call: one entry for each pair
    /// that [`Self::drain_touched`] reported, when its parts separate or one of them is removed
    /// (destroyed, or out of the Workspace; the runtime fires `TouchEnded` on both, which reaches
    /// scripts on the part that still exists). A part rebuilt in place (Size, Shape or
    /// CanCollide changed) that still touches reports neither an end nor a new start. Pairs
    /// involving a character part are never reported (see [`CharacterEvent::TouchEnded`]).
    /// Drain [`Self::drain_touched`] first: a pair that started and stopped since the last
    /// drain is in both.
    pub fn drain_touch_ended(&mut self) -> Vec<(InstanceId, InstanceId)> {
        self.touches.drain_ended()
    }
}
