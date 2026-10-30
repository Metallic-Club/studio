//! [`PhysicsWorld2d`]: the 2D simulation behind our own interface.
use crate::character::{
    Character, CharacterConfig, CharacterEvent, CharacterInput, CharacterState, TickContext,
    to_groups,
};
use crate::desc::{BodyDesc, BodyId, BodyKind, CollisionGroups, JointDesc, JointId, JointKind, Scene2d};
use crate::error::Physics2dError;
use crate::events::{ContactTracker, EventCollector, OneWayHooks, SensorEvent};
use crate::math::{Pose2, Vec2};
use crate::shapes::{self, DebugGeometry, from_pose, from_v, pose, v};
use rapier2d::prelude::{
    ActiveCollisionTypes, ActiveEvents, ActiveHooks, ColliderHandle, FixedJointBuilder,
    GenericJoint, GenericJointBuilder, ImpulseJointHandle, JointAxesMask, JointAxis,
    PhysicsWorld as RapierWorld, PrismaticJointBuilder, QueryFilter, Ray, RevoluteJointBuilder,
    RigidBodyBuilder, RigidBodyHandle, RigidBodyType, Vector,
};
use std::collections::{BTreeMap, HashMap, HashSet};

/// World-wide simulation settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldConfig {
    /// Gravity in studs/s^2. Default `(0, -196.2)`, the same as 3D.
    pub gravity: Vec2,
    /// Length of one fixed simulation tick in seconds. Default 1/60.
    pub tick: f64,
    /// Solver substeps per tick (Rapier's soft-constraint substepping). Default 4, which
    /// solves at 240 Hz while collision detection runs once per tick (the #115 proposal).
    pub solver_substeps: usize,
    /// Longest frame time one [`PhysicsWorld2d::step`] simulates; more is dropped so a hitch
    /// cannot spiral. Default 0.25 s.
    pub max_frame_time: f64,
    /// How many studs make one "metre" for the solver's internal tolerances. Default 4
    /// (a 4-stud box is the typical object size).
    pub length_unit: f64,
}

impl Default for WorldConfig {
    fn default() -> Self {
        WorldConfig {
            gravity: Vec2::new(0.0, -crate::GRAVITY),
            tick: 1.0 / 60.0,
            solver_substeps: 4,
            max_frame_time: 0.25,
            length_unit: 4.0,
        }
    }
}

/// Result of a successful [`PhysicsWorld2d::raycast`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit2d {
    /// The body that was hit.
    pub body: BodyId,
    /// World position of the hit.
    pub point: Vec2,
    /// Surface normal at the hit (unit length, pointing away from the surface).
    pub normal: Vec2,
    /// Distance from the ray origin to the hit, in studs.
    pub distance: f64,
}

/// Which colliders a [`PhysicsWorld2d::raycast`] considers.
#[derive(Debug, Clone, Copy, Default)]
pub struct RayFilter<'a> {
    /// Bodies (or characters) to skip.
    pub ignore: &'a [BodyId],
    /// Only colliders whose groups interact with these.
    pub groups: Option<CollisionGroups>,
    /// Also hit sensors (off by default, like 3D raycasts).
    pub include_sensors: bool,
}

/// State of a body after the last step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyState {
    /// Position and rotation.
    pub pose: Pose2,
    /// Linear velocity in studs/s (derived from the move for static movers).
    pub linear_velocity: Vec2,
    /// Angular velocity in radians/s.
    pub angular_velocity: f64,
    /// Whether the body is asleep (dynamic bodies at rest).
    pub sleeping: bool,
}

/// Kind of thing a [`DebugShape2d`] belongs to, for colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugBodyKind {
    /// A static body.
    Static,
    /// A dynamic body.
    Dynamic,
    /// A kinematic body, or a static body moving this step.
    Kinematic,
    /// A character controller.
    Character,
}

/// One collider outline for the physics debug view.
#[derive(Debug, Clone, PartialEq)]
pub struct DebugShape2d {
    /// Owner of the collider.
    pub body: BodyId,
    /// What kind of body it is.
    pub kind: DebugBodyKind,
    /// Sensor (trigger) collider.
    pub sensor: bool,
    /// One-way platform.
    pub one_way: bool,
    /// Body is asleep.
    pub sleeping: bool,
    /// World-space geometry.
    pub geometry: DebugGeometry,
}

/// A pending move of a static or kinematic body during the current step.
#[derive(Debug, Clone, Copy)]
struct Mover {
    from: Pose2,
    to: Pose2,
}

struct BodyEntry {
    handle: RigidBodyHandle,
    colliders: Vec<ColliderHandle>,
    desc: BodyDesc,
    /// Pose as last seen in (or written to) the scene.
    last_pose: Pose2,
    /// Pose before the last tick, for render interpolation.
    prev_pose: Pose2,
    mover: Option<Mover>,
    derived_velocity: Vec2,
}

struct JointEntry {
    handle: ImpulseJointHandle,
    desc: JointDesc,
    bodies: (RigidBodyHandle, RigidBodyHandle),
}

/// The 2D physics simulation for one scene.
///
/// Usage per frame: [`Self::sync_from`] after structural edits, then
/// [`Self::step`] with the frame time, then read events with [`Self::drain_touched`],
/// [`Self::drain_sensor_events`] and [`Self::drain_character_events`].
pub struct PhysicsWorld2d {
    config: WorldConfig,
    rapier: RapierWorld,
    bodies: BTreeMap<BodyId, BodyEntry>,
    joints: BTreeMap<JointId, JointEntry>,
    characters: BTreeMap<BodyId, Character>,
    owners: HashMap<ColliderHandle, BodyId>,
    sensors: HashSet<ColliderHandle>,
    hooks: OneWayHooks,
    collector: EventCollector,
    tracker: ContactTracker,
    character_events: Vec<CharacterEvent>,
    accumulator: f64,
    ticks: u64,
}

impl std::fmt::Debug for PhysicsWorld2d {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhysicsWorld2d")
            .field("config", &self.config)
            .field("bodies", &self.bodies.len())
            .field("joints", &self.joints.len())
            .field("characters", &self.characters.len())
            .field("ticks", &self.ticks)
            .finish()
    }
}

fn close(a: Pose2, b: Pose2) -> bool {
    (a.position - b.position).length() < 1e-9 && (a.angle - b.angle).abs() < 1e-9
}

impl PhysicsWorld2d {
    /// Creates an empty world.
    pub fn new(config: WorldConfig) -> Result<Self, Physics2dError> {
        let ok = config.gravity.is_finite()
            && config.tick.is_finite()
            && config.tick > 0.0
            && config.tick <= 0.1
            && config.solver_substeps >= 1
            && config.solver_substeps <= 64
            && config.max_frame_time.is_finite()
            && config.max_frame_time >= config.tick
            && config.length_unit.is_finite()
            && config.length_unit > 0.0;
        if !ok {
            return Err(Physics2dError::InvalidConfig(
                "world config: finite gravity, 0 < tick <= 0.1, 1..=64 substeps, \
                 max_frame_time >= tick, length_unit > 0",
            ));
        }
        let mut rapier = RapierWorld::new();
        rapier.gravity = v(config.gravity);
        rapier.integration_parameters.dt = config.tick as f32;
        rapier.integration_parameters.num_solver_iterations = config.solver_substeps;
        rapier.integration_parameters.length_unit = config.length_unit as f32;
        Ok(PhysicsWorld2d {
            config,
            rapier,
            bodies: BTreeMap::new(),
            joints: BTreeMap::new(),
            characters: BTreeMap::new(),
            owners: HashMap::new(),
            sensors: HashSet::new(),
            hooks: OneWayHooks::default(),
            collector: EventCollector::default(),
            tracker: ContactTracker::default(),
            character_events: Vec::new(),
            accumulator: 0.0,
            ticks: 0,
        })
    }

    /// The configuration this world was created with.
    pub fn config(&self) -> &WorldConfig {
        &self.config
    }

    /// Number of fixed ticks simulated so far.
    pub fn tick_count(&self) -> u64 {
        self.ticks
    }

    /// Fraction (0..1) of a tick left over in the accumulator: blend factor between the
    /// previous and current tick for smooth rendering at any frame rate.
    pub fn interpolation_alpha(&self) -> f64 {
        (self.accumulator / self.config.tick).clamp(0.0, 1.0)
    }

    /// Makes the simulation match the structure of `scene`.
    ///
    /// New bodies and joints are created, ones whose description changed (anything except
    /// the pose) are rebuilt, and ones no longer in the scene are removed. Poses are not read
    /// here: [`Self::step`] applies them. The scene is validated first; on error nothing changes.
    pub fn sync_from(&mut self, scene: &Scene2d) -> Result<(), Physics2dError> {
        self.validate_scene(scene)?;
        let alive: HashSet<BodyId> = scene.bodies.iter().map(|b| b.id).collect();
        let gone: Vec<BodyId> = self
            .bodies
            .keys()
            .filter(|id| !alive.contains(id))
            .copied()
            .collect();
        for id in gone {
            self.remove_body(id);
        }
        for desc in &scene.bodies {
            let stale = self
                .bodies
                .get(&desc.id)
                .is_some_and(|e| !e.desc.structure_eq(desc));
            if stale {
                self.remove_body(desc.id);
            }
            if !self.bodies.contains_key(&desc.id) {
                self.add_body(desc)?;
            }
        }
        self.sync_joints(scene)?;
        // Refresh the broad phase so queries work before the first step.
        self.rapier.detect_collisions(&self.hooks, &self.collector);
        self.collect_events();
        Ok(())
    }

    fn validate_scene(&self, scene: &Scene2d) -> Result<(), Physics2dError> {
        let mut ids = HashSet::new();
        for b in &scene.bodies {
            b.validate()?;
            if !ids.insert(b.id) || self.characters.contains_key(&b.id) {
                return Err(Physics2dError::DuplicateBody(b.id));
            }
            for c in &b.colliders {
                let _ = shapes::build_shape(b.id, c)?;
            }
        }
        let mut joint_ids = HashSet::new();
        for j in &scene.joints {
            j.validate()?;
            if !joint_ids.insert(j.id) {
                return Err(Physics2dError::DuplicateJoint(j.id));
            }
            if !ids.contains(&j.body_a) || !ids.contains(&j.body_b) {
                return Err(Physics2dError::InvalidJoint {
                    joint: j.id,
                    reason: "joint names a body that is not in the scene",
                });
            }
        }
        Ok(())
    }

    fn add_body(&mut self, desc: &BodyDesc) -> Result<(), Physics2dError> {
        let builder = match desc.kind {
            BodyKind::Static => RigidBodyBuilder::fixed(),
            BodyKind::Dynamic => RigidBodyBuilder::dynamic(),
            // Kinematic bodies never sleep (see `Character::new`).
            BodyKind::Kinematic => RigidBodyBuilder::kinematic_position_based().can_sleep(false),
        }
        .pose(pose(Pose2::new(desc.position, desc.angle)))
        .gravity_scale(desc.gravity_scale as f32)
        .linear_damping(desc.linear_damping as f32)
        .angular_damping(desc.angular_damping as f32)
        .ccd_enabled(desc.bullet);
        let builder = if desc.fixed_rotation {
            builder.lock_rotations()
        } else {
            builder
        };
        let handle = self.rapier.insert_body(builder);
        let mut colliders = Vec::with_capacity(desc.colliders.len());
        for c in &desc.colliders {
            let mut builder = shapes::build_shape(desc.id, c)?
                .density(c.material.density as f32)
                .friction(c.material.friction as f32)
                .restitution(c.material.restitution as f32)
                .sensor(c.sensor)
                .collision_groups(to_groups(c.groups))
                .active_events(ActiveEvents::COLLISION_EVENTS)
                .active_collision_types(
                    ActiveCollisionTypes::DYNAMIC_DYNAMIC
                        | ActiveCollisionTypes::DYNAMIC_KINEMATIC
                        | ActiveCollisionTypes::DYNAMIC_FIXED
                        | ActiveCollisionTypes::KINEMATIC_KINEMATIC
                        | ActiveCollisionTypes::KINEMATIC_FIXED,
                );
            if c.one_way.is_some() {
                builder = builder.active_hooks(ActiveHooks::MODIFY_SOLVER_CONTACTS);
            }
            let ch = self.rapier.insert_collider(builder, Some(handle));
            self.owners.insert(ch, desc.id);
            if c.sensor {
                self.sensors.insert(ch);
            }
            if let Some(n) = c.one_way.and_then(Vec2::normalized) {
                // Store the direction in collider space (body space rotated back by the
                // collider's own angle).
                self.hooks.platforms.insert(ch, v(n.rotated(-c.angle)));
            }
            colliders.push(ch);
        }
        let p = Pose2::new(desc.position, desc.angle);
        self.bodies.insert(
            desc.id,
            BodyEntry {
                handle,
                colliders,
                desc: desc.clone(),
                last_pose: p,
                prev_pose: p,
                mover: None,
                derived_velocity: Vec2::ZERO,
            },
        );
        Ok(())
    }

    fn remove_body(&mut self, id: BodyId) {
        if let Some(e) = self.bodies.remove(&id) {
            for c in &e.colliders {
                self.owners.remove(c);
                self.sensors.remove(c);
                self.hooks.platforms.remove(c);
            }
            self.rapier.remove_body(e.handle);
            self.tracker.forget(id);
            // The backend removed the body's joints with it.
            self.joints
                .retain(|_, j| j.bodies.0 != e.handle && j.bodies.1 != e.handle);
        }
    }

    fn sync_joints(&mut self, scene: &Scene2d) -> Result<(), Physics2dError> {
        let wanted: HashMap<JointId, &JointDesc> = scene.joints.iter().map(|j| (j.id, j)).collect();
        let stale: Vec<JointId> = self
            .joints
            .iter()
            .filter(|(id, j)| {
                let handles = self.body_handles(j.desc.body_a, j.desc.body_b);
                wanted.get(id).is_none_or(|w| **w != j.desc) || handles != Some(j.bodies)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            if let Some(j) = self.joints.remove(&id) {
                self.rapier.remove_impulse_joint(j.handle);
            }
        }
        for j in &scene.joints {
            if self.joints.contains_key(&j.id) {
                continue;
            }
            let Some(bodies) = self.body_handles(j.body_a, j.body_b) else {
                return Err(Physics2dError::InvalidJoint {
                    joint: j.id,
                    reason: "joint names a body that is not in the scene",
                });
            };
            let angle_a = self.bodies.get(&j.body_a).map_or(0.0, |e| e.last_pose.angle);
            let angle_b = self.bodies.get(&j.body_b).map_or(0.0, |e| e.last_pose.angle);
            let data = build_joint(j, angle_a - angle_b);
            let handle = self.rapier.insert_impulse_joint(bodies.0, bodies.1, data);
            self.joints.insert(
                j.id,
                JointEntry {
                    handle,
                    desc: j.clone(),
                    bodies,
                },
            );
        }
        Ok(())
    }

    fn body_handles(&self, a: BodyId, b: BodyId) -> Option<(RigidBodyHandle, RigidBodyHandle)> {
        Some((self.bodies.get(&a)?.handle, self.bodies.get(&b)?.handle))
    }

    fn collect_events(&mut self) {
        let events = self.collector.take();
        self.tracker.apply(events, &self.owners, &self.sensors);
    }

    /// Advances the simulation by `dt` seconds in fixed ticks (the remainder is carried to the
    /// next call) and writes the pose of dynamic and kinematic bodies back into `scene`.
    ///
    /// Before simulating, poses changed in `scene` since the last step are applied: a dynamic
    /// body is teleported (velocity cleared); a static or kinematic body becomes a mover that
    /// travels to the new pose over the ticks of this step, carrying what stands on it.
    /// Bodies not yet synced with [`Self::sync_from`] are ignored.
    ///
    /// `dt` must be finite and not negative; otherwise [`Physics2dError::InvalidDt`] is
    /// returned and nothing changes (one bad frame time cannot freeze the world, see #33).
    /// Returns the number of ticks simulated.
    pub fn step(&mut self, scene: &mut Scene2d, dt: f64) -> Result<u32, Physics2dError> {
        if !dt.is_finite() || dt < 0.0 {
            return Err(Physics2dError::InvalidDt(dt));
        }
        for b in &scene.bodies {
            if !b.position.is_finite() || !b.angle.is_finite() {
                return Err(Physics2dError::NonFinite {
                    body: b.id,
                    field: "position",
                });
            }
        }
        self.apply_scene_poses(scene);

        self.accumulator += dt.min(self.config.max_frame_time);
        let ticks = ((self.accumulator / self.config.tick) + 1e-9).floor() as u32;
        if ticks == 0 {
            return Ok(0);
        }
        self.accumulator = (self.accumulator - f64::from(ticks) * self.config.tick).max(0.0);

        let movers: Vec<(BodyId, RigidBodyHandle, Mover)> = self
            .bodies
            .iter()
            .filter_map(|(id, e)| e.mover.map(|m| (*id, e.handle, m)))
            .collect();
        for (_, handle, _) in &movers {
            if let Some(b) = self.rapier.bodies.get_mut(*handle)
                && b.is_fixed()
            {
                b.set_body_type(RigidBodyType::KinematicPositionBased, false);
            }
            // Through the island manager, so bodies resting on the mover wake consistently.
            self.rapier.wake_up(*handle, true);
        }

        for k in 1..=ticks {
            if k == ticks {
                self.remember_previous_poses();
            }
            let t = f64::from(k) / f64::from(ticks);
            for (_, handle, m) in &movers {
                if let Some(b) = self.rapier.bodies.get_mut(*handle) {
                    b.set_next_kinematic_position(pose(m.from.lerp(m.to, t)));
                }
            }
            for c in self.characters.values_mut() {
                c.before_world_step(&self.rapier);
            }
            self.rapier.step_with_events(&self.hooks, &self.collector);
            self.tick_characters();
            self.collect_events();
            self.ticks += 1;
        }

        // Static movers become static again, exactly at their target.
        let sim_time = f64::from(ticks) * self.config.tick;
        for e in self.bodies.values_mut() {
            e.derived_velocity = Vec2::ZERO;
        }
        for (id, handle, m) in movers {
            if let Some(e) = self.bodies.get_mut(&id) {
                e.mover = None;
                e.derived_velocity = (m.to.position - m.from.position) * (1.0 / sim_time);
                e.last_pose = m.to;
                if e.desc.kind == BodyKind::Static
                    && let Some(b) = self.rapier.bodies.get_mut(handle)
                {
                    b.set_body_type(RigidBodyType::Fixed, false);
                    b.set_position(pose(m.to), false);
                }
            }
        }
        self.write_back(scene);
        Ok(ticks)
    }

    fn apply_scene_poses(&mut self, scene: &Scene2d) {
        for b in &scene.bodies {
            let Some(e) = self.bodies.get_mut(&b.id) else {
                continue;
            };
            let target = Pose2::new(b.position, b.angle);
            if close(target, e.last_pose) {
                continue;
            }
            match e.desc.kind {
                BodyKind::Dynamic => {
                    if let Some(body) = self.rapier.bodies.get_mut(e.handle) {
                        body.set_position(pose(target), false);
                        body.set_linvel(Vector::ZERO, false);
                        body.set_angvel(0.0, false);
                    }
                    self.rapier.wake_up(e.handle, true);
                    e.last_pose = target;
                    e.prev_pose = target;
                }
                BodyKind::Static | BodyKind::Kinematic => {
                    let from = e.mover.map_or(e.last_pose, |m| m.from);
                    e.mover = Some(Mover { from, to: target });
                }
            }
        }
    }

    fn remember_previous_poses(&mut self) {
        for e in self.bodies.values_mut() {
            if let Some(b) = self.rapier.bodies.get(e.handle) {
                e.prev_pose = from_pose(b.position());
            }
        }
        for c in self.characters.values_mut() {
            c.prev_position = c.position;
        }
    }

    fn tick_characters(&mut self) {
        let mut ctx = TickContext {
            rapier: &mut self.rapier,
            one_way: &self.hooks.platforms,
            owners: &self.owners,
            dt: self.config.tick,
        };
        for c in self.characters.values_mut() {
            let previous = std::mem::take(&mut c.touches);
            c.tick(&mut ctx, &mut self.character_events);
            self.tracker.set_character_touches(c.id, &previous, &c.touches);
        }
    }

    fn write_back(&mut self, scene: &mut Scene2d) {
        for b in &mut scene.bodies {
            let Some(e) = self.bodies.get_mut(&b.id) else {
                continue;
            };
            if e.desc.kind == BodyKind::Static {
                continue;
            }
            let Some(body) = self.rapier.bodies.get(e.handle) else {
                continue;
            };
            let p = from_pose(body.position());
            b.position = p.position;
            b.angle = p.angle;
            e.last_pose = p;
        }
    }

    /// Position, velocity and sleep state of a body or character. `None` for unknown ids.
    pub fn body_state(&self, id: BodyId) -> Option<BodyState> {
        if let Some(c) = self.characters.get(&id) {
            let s = c.state(&self.owners);
            return Some(BodyState {
                pose: Pose2::new(s.position, 0.0),
                linear_velocity: s.velocity,
                angular_velocity: 0.0,
                sleeping: false,
            });
        }
        let e = self.bodies.get(&id)?;
        let b = self.rapier.bodies.get(e.handle)?;
        let dynamic = b.is_dynamic();
        Some(BodyState {
            pose: from_pose(b.position()),
            linear_velocity: if dynamic {
                from_v(b.linvel())
            } else {
                e.derived_velocity
            },
            angular_velocity: if dynamic { f64::from(b.angvel()) } else { 0.0 },
            sleeping: b.is_sleeping(),
        })
    }

    /// Pose to draw this frame: blended between the last two ticks by
    /// [`Self::interpolation_alpha`], so motion is smooth on any refresh rate.
    pub fn render_pose(&self, id: BodyId) -> Option<Pose2> {
        let alpha = self.interpolation_alpha();
        if let Some(c) = self.characters.get(&id) {
            return Some(Pose2::new(c.prev_position.lerp(c.position, alpha), 0.0));
        }
        let e = self.bodies.get(&id)?;
        let b = self.rapier.bodies.get(e.handle)?;
        Some(e.prev_pose.lerp(from_pose(b.position()), alpha))
    }

    /// Sets the velocity of a dynamic body and wakes it.
    pub fn set_velocity(&mut self, id: BodyId, linear: Vec2, angular: f64) -> Result<(), Physics2dError> {
        if !linear.is_finite() || !angular.is_finite() {
            return Err(Physics2dError::NonFinite {
                body: id,
                field: "velocity",
            });
        }
        let handle = self.dynamic_handle(id)?;
        if let Some(body) = self.rapier.bodies.get_mut(handle) {
            body.set_linvel(v(linear), false);
            body.set_angvel(angular as f32, false);
        }
        self.rapier.wake_up(handle, true);
        Ok(())
    }

    /// Applies an impulse (mass x velocity change) at the center of mass of a dynamic body.
    pub fn apply_impulse(&mut self, id: BodyId, impulse: Vec2) -> Result<(), Physics2dError> {
        if !impulse.is_finite() {
            return Err(Physics2dError::NonFinite {
                body: id,
                field: "impulse",
            });
        }
        let handle = self.dynamic_handle(id)?;
        if let Some(body) = self.rapier.bodies.get_mut(handle) {
            body.apply_impulse(v(impulse), false);
        }
        self.rapier.wake_up(handle, true);
        Ok(())
    }

    fn dynamic_handle(&self, id: BodyId) -> Result<RigidBodyHandle, Physics2dError> {
        let e = self.bodies.get(&id).ok_or(Physics2dError::UnknownBody(id))?;
        if e.desc.kind != BodyKind::Dynamic {
            return Err(Physics2dError::NotDynamic(id));
        }
        Ok(e.handle)
    }

    /// Casts a ray and returns the nearest hit, skipping sensors unless asked.
    ///
    /// Like 3D (and Roblox), the length of `dir` is the maximum distance in studs. Returns
    /// `None` when nothing is hit within that distance or `dir` is zero or not finite.
    /// Reflects the state of the last [`Self::sync_from`] or [`Self::step`].
    pub fn raycast(&self, origin: Vec2, dir: Vec2, filter: &RayFilter<'_>) -> Option<RayHit2d> {
        let max = dir.length();
        if max <= 0.0 || !max.is_finite() || !origin.is_finite() {
            return None;
        }
        let unit = dir * (1.0 / max);
        let ray = Ray::new(v(origin), v(unit));
        let skip: Vec<ColliderHandle> = filter
            .ignore
            .iter()
            .flat_map(|id| self.colliders_of(*id))
            .collect();
        let predicate = |h: ColliderHandle, _: &rapier2d::prelude::Collider| !skip.contains(&h);
        let mut q = QueryFilter::default().predicate(&predicate);
        if !filter.include_sensors {
            q = q.exclude_sensors();
        }
        if let Some(g) = filter.groups {
            q = q.groups(to_groups(g));
        }
        let (handle, hit) = self.rapier.cast_ray_and_get_normal(&ray, max as f32, true, q)?;
        let body = *self.owners.get(&handle)?;
        let distance = f64::from(hit.time_of_impact);
        Some(RayHit2d {
            body,
            point: origin + unit * distance,
            normal: from_v(hit.normal),
            distance,
        })
    }

    /// Bodies whose (solid or sensor) colliders contain `point`, in id order.
    pub fn bodies_at_point(&self, point: Vec2) -> Vec<BodyId> {
        if !point.is_finite() {
            return Vec::new();
        }
        let mut out: Vec<BodyId> = self
            .rapier
            .intersect_point(v(point), QueryFilter::default())
            .filter_map(|(h, _)| self.owners.get(&h).copied())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    fn colliders_of(&self, id: BodyId) -> Vec<ColliderHandle> {
        if let Some(e) = self.bodies.get(&id) {
            return e.colliders.clone();
        }
        self.characters.get(&id).map(|c| vec![c.collider]).unwrap_or_default()
    }

    /// Body pairs that started touching since the last call (sensors included). Each contact
    /// is reported once, and again only after the bodies separated and touched anew.
    /// Characters report what their moves bump into and stand on.
    pub fn drain_touched(&mut self) -> Vec<(BodyId, BodyId)> {
        self.tracker.drain_touched()
    }

    /// Sensor enter and exit events since the last call, in the order they happened.
    pub fn drain_sensor_events(&mut self) -> Vec<SensorEvent> {
        self.tracker.drain_sensor_events()
    }

    /// Character jumps, landings and drops since the last call.
    pub fn drain_character_events(&mut self) -> Vec<CharacterEvent> {
        std::mem::take(&mut self.character_events)
    }

    /// True while two bodies (or a body and a character) touch.
    pub fn is_touching(&self, a: BodyId, b: BodyId) -> bool {
        self.tracker.is_touching(a, b)
    }

    /// Adds a platformer character with its box centered at `position`.
    pub fn add_character(
        &mut self,
        id: BodyId,
        config: CharacterConfig,
        position: Vec2,
    ) -> Result<(), Physics2dError> {
        if self.bodies.contains_key(&id) || self.characters.contains_key(&id) {
            return Err(Physics2dError::DuplicateBody(id));
        }
        let c = Character::new(id, config, position, &mut self.rapier)?;
        self.owners.insert(c.collider, id);
        self.characters.insert(id, c);
        Ok(())
    }

    /// Removes a character; returns whether it existed.
    pub fn remove_character(&mut self, id: BodyId) -> bool {
        let Some(c) = self.characters.remove(&id) else {
            return false;
        };
        self.owners.remove(&c.collider);
        self.rapier.remove_body(c.body);
        self.tracker.forget(id);
        true
    }

    /// Sets what the player is pressing; call every frame before [`Self::step`].
    pub fn set_character_input(&mut self, id: BodyId, input: CharacterInput) -> Result<(), Physics2dError> {
        self.characters
            .get_mut(&id)
            .ok_or(Physics2dError::UnknownBody(id))?
            .set_input(input);
        Ok(())
    }

    /// Moves a character instantly (respawn), clearing its velocity.
    pub fn teleport_character(&mut self, id: BodyId, position: Vec2) -> Result<(), Physics2dError> {
        if !position.is_finite() {
            return Err(Physics2dError::NonFinite {
                body: id,
                field: "position",
            });
        }
        let c = self
            .characters
            .get_mut(&id)
            .ok_or(Physics2dError::UnknownBody(id))?;
        c.teleport(position, &mut self.rapier);
        Ok(())
    }

    /// Snapshot of a character after the last tick.
    pub fn character(&self, id: BodyId) -> Option<CharacterState> {
        self.characters.get(&id).map(|c| c.state(&self.owners))
    }

    /// Outlines of every collider, for the physics debug view.
    pub fn debug_shapes(&self) -> Vec<DebugShape2d> {
        let mut out = Vec::new();
        for (id, e) in &self.bodies {
            let Some(b) = self.rapier.bodies.get(e.handle) else {
                continue;
            };
            let p = from_pose(b.position());
            let kind = if b.is_kinematic() {
                DebugBodyKind::Kinematic
            } else {
                match e.desc.kind {
                    BodyKind::Static => DebugBodyKind::Static,
                    BodyKind::Dynamic => DebugBodyKind::Dynamic,
                    BodyKind::Kinematic => DebugBodyKind::Kinematic,
                }
            };
            for c in &e.desc.colliders {
                for geometry in shapes::debug_geometry(c, p) {
                    out.push(DebugShape2d {
                        body: *id,
                        kind,
                        sensor: c.sensor,
                        one_way: c.one_way.is_some(),
                        sleeping: b.is_sleeping(),
                        geometry,
                    });
                }
            }
        }
        for (id, c) in &self.characters {
            let s = c.state(&self.owners);
            let h = c.half_extents();
            let p = s.position;
            out.push(DebugShape2d {
                body: *id,
                kind: DebugBodyKind::Character,
                sensor: false,
                one_way: false,
                sleeping: false,
                geometry: DebugGeometry::Polygon {
                    points: vec![
                        p + Vec2::new(-h.x, -h.y),
                        p + Vec2::new(h.x, -h.y),
                        p + Vec2::new(h.x, h.y),
                        p + Vec2::new(-h.x, h.y),
                    ],
                },
            });
        }
        out
    }
}

fn build_joint(j: &JointDesc, relative_angle: f64) -> GenericJoint {
    let joint: GenericJoint = match &j.kind {
        JointKind::Revolute {
            anchor_a,
            anchor_b,
            limits,
            motor,
        } => {
            let mut b = RevoluteJointBuilder::new()
                .local_anchor1(v(*anchor_a))
                .local_anchor2(v(*anchor_b))
                .contacts_enabled(j.collide_connected);
            if let Some((lo, hi)) = limits {
                b = b.limits([*lo as f32, *hi as f32]);
            }
            if let Some(m) = motor {
                b = b
                    .motor_velocity(m.target_velocity as f32, MOTOR_GAIN)
                    .motor_max_force(m.max_force as f32);
            }
            b.build().into()
        }
        JointKind::Prismatic {
            anchor_a,
            anchor_b,
            axis,
            limits,
            motor,
        } => {
            let dir = axis.normalized().unwrap_or(Vec2::new(1.0, 0.0));
            let mut b = PrismaticJointBuilder::new(v(dir))
                .local_anchor1(v(*anchor_a))
                .local_anchor2(v(*anchor_b))
                .contacts_enabled(j.collide_connected);
            if let Some((lo, hi)) = limits {
                b = b.limits([*lo as f32, *hi as f32]);
            }
            if let Some(m) = motor {
                b = b
                    .motor_velocity(m.target_velocity as f32, MOTOR_GAIN)
                    .motor_max_force(m.max_force as f32);
            }
            b.build().into()
        }
        JointKind::Distance {
            anchor_a,
            anchor_b,
            min_length,
            max_length,
            spring,
        } => {
            let mut b = GenericJointBuilder::new(JointAxesMask::empty())
                .coupled_axes(JointAxesMask::LIN_AXES)
                .limits(JointAxis::LinX, [*min_length as f32, *max_length as f32])
                .local_anchor1(v(*anchor_a))
                .local_anchor2(v(*anchor_b))
                .contacts_enabled(j.collide_connected);
            if let Some(s) = spring {
                b = b
                    .motor_position(
                        JointAxis::LinX,
                        s.rest_length as f32,
                        s.stiffness as f32,
                        s.damping as f32,
                    )
                    .motor_model(JointAxis::LinX, rapier2d::prelude::MotorModel::ForceBased);
            }
            b.build()
        }
        JointKind::Weld { anchor_a, anchor_b } => FixedJointBuilder::new()
            .local_frame1(pose(Pose2::new(*anchor_a, 0.0)))
            .local_frame2(pose(Pose2::new(*anchor_b, relative_angle)))
            .contacts_enabled(j.collide_connected)
            .build()
            .into(),
    };
    joint
}

/// Velocity gain of joint motors: how hard a motor corrects its speed error each step,
/// limited by its `max_force`.
const MOTOR_GAIN: f32 = 100.0;
