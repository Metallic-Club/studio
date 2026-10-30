//! The 2D platformer character controller.
//!
//! A character is a kinematic box moved by our own collide-and-slide shape casts, with the
//! "game feel" rules on top: coyote time, jump buffering, variable jump height, apex hang, faster falls, wall slide and wall jump, one-way platform
//! drop-through and moving-platform carry. Jumps are described in designer terms (height and
//! time to apex) and turned into gravity and launch speed with the usual projectile formulas
//! (`g = 2h / t^2`, `v = 2h / t`).
//!
//! Everything runs inside the world's fixed tick, so the feel does not depend on frame rate.
use crate::desc::{BodyId, CollisionGroups};
use crate::error::Physics2dError;
use crate::math::Vec2;
use crate::shapes::{from_v, v};
use rapier2d::parry::bounding_volume::BoundingVolume;
use rapier2d::parry::query::ShapeCastOptions;
use rapier2d::prelude::{
    ActiveCollisionTypes, ActiveEvents, Collider, ColliderBuilder, ColliderHandle, Group,
    InteractionGroups, InteractionTestMode, PhysicsWorld as RapierWorld, Pose, QueryFilter,
    RigidBodyBuilder, RigidBodyHandle, Rotation, SharedShape, Vector,
};
use std::collections::HashMap;

/// Tuning of a [`crate::PhysicsWorld2d`] character. Distances in studs, times in seconds,
/// speeds in studs per second. Defaults are tuned for a 2.4 x 4.8 stud character on a
/// 4-stud tile grid; `docs/agents/126-physics2d.md` explains where each number comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharacterConfig {
    /// Half width and half height of the character's box.
    pub half_extents: Vec2,
    /// Collision groups of the character's collider (and of what its moves collide with).
    pub groups: CollisionGroups,
    /// Top horizontal speed.
    pub run_speed: f64,
    /// Horizontal acceleration toward `run_speed` on the ground (studs/s^2).
    pub ground_acceleration: f64,
    /// Horizontal deceleration when stopping or turning on the ground (studs/s^2).
    pub ground_deceleration: f64,
    /// Multiplier on acceleration and deceleration in the air (0..1).
    pub air_control: f64,
    /// Apex height of a full (held) jump from the ground.
    pub jump_height: f64,
    /// Apex height of a tapped jump (jump released right away).
    pub min_jump_height: f64,
    /// Time from take-off to apex of a full jump, ignoring apex hang.
    pub time_to_apex: f64,
    /// Gravity multiplier while falling (snappier landings than take-offs).
    pub fall_gravity_multiplier: f64,
    /// Below this vertical speed near the apex, with jump held, gravity is reduced.
    pub apex_hang_speed: f64,
    /// Gravity multiplier during apex hang (0..1); `jump_height` already accounts for it.
    pub apex_gravity_multiplier: f64,
    /// Terminal falling speed.
    pub max_fall_speed: f64,
    /// Time after walking off a ledge during which a jump still counts as a ground jump.
    pub coyote_time: f64,
    /// Time a jump press is remembered before landing (or before touching a wall).
    pub jump_buffer_time: f64,
    /// Whether pressing into a wall while falling slows the fall.
    pub wall_slide: bool,
    /// Largest falling speed while wall sliding.
    pub wall_slide_speed: f64,
    /// Whether jumping off a wall is allowed.
    pub wall_jump: bool,
    /// Launch velocity of a wall jump: `x` away from the wall, `y` up.
    pub wall_jump_velocity: Vec2,
    /// Time after a wall jump during which horizontal input is ignored (so the jump carries
    /// away from the wall even while the player still holds toward it).
    pub wall_jump_lock_time: f64,
    /// How far beside the character a wall is detected.
    pub wall_probe_distance: f64,
    /// Steepest walkable slope in radians; steeper ground is a wall the character slides down.
    pub max_slope_angle: f64,
    /// Largest downward snap that keeps the character glued to slopes and stair edges.
    pub snap_to_ground: f64,
    /// How long a one-way platform is ignored after dropping through it.
    pub drop_through_time: f64,
    /// Add the platform's velocity to the jump when jumping off a moving platform.
    pub inherit_platform_velocity: bool,
    /// Mass used when the character pushes dynamic bodies.
    pub push_mass: f64,
    /// Gap kept between the character and surfaces (numerical skin).
    pub skin: f64,
}

impl Default for CharacterConfig {
    fn default() -> Self {
        CharacterConfig {
            half_extents: Vec2::new(1.2, 2.4),
            groups: CollisionGroups::ALL,
            run_speed: 28.0,
            ground_acceleration: 350.0,
            ground_deceleration: 420.0,
            air_control: 0.65,
            jump_height: 13.0,
            min_jump_height: 3.5,
            time_to_apex: 0.36,
            fall_gravity_multiplier: 1.6,
            apex_hang_speed: 18.0,
            apex_gravity_multiplier: 0.5,
            max_fall_speed: 100.0,
            coyote_time: 0.1,
            jump_buffer_time: 0.1,
            wall_slide: true,
            wall_slide_speed: 14.0,
            wall_jump: true,
            wall_jump_velocity: Vec2::new(40.0, 62.0),
            wall_jump_lock_time: 0.16,
            wall_probe_distance: 0.4,
            max_slope_angle: 50f64.to_radians(),
            snap_to_ground: 0.6,
            drop_through_time: 0.25,
            inherit_platform_velocity: true,
            push_mass: 20.0,
            skin: 0.02,
        }
    }
}

/// Gravity and launch speeds derived from a [`CharacterConfig`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JumpPhysics {
    /// Rising gravity (studs/s^2): `2 * jump_height / time_to_apex^2`.
    pub gravity: f64,
    /// Take-off speed that reaches exactly `jump_height`, apex hang included.
    pub jump_velocity: f64,
    /// Gravity multiplier while rising with jump released, chosen so a tap reaches
    /// `min_jump_height`.
    pub release_gravity_multiplier: f64,
}

fn check(ok: bool, what: &'static str) -> Result<(), Physics2dError> {
    if ok {
        Ok(())
    } else {
        Err(Physics2dError::InvalidConfig(what))
    }
}

impl CharacterConfig {
    /// Checks the configuration and derives the jump physics.
    pub fn jump_physics(&self) -> Result<JumpPhysics, Physics2dError> {
        let positive = [
            self.half_extents.x,
            self.half_extents.y,
            self.run_speed,
            self.jump_height,
            self.min_jump_height,
            self.time_to_apex,
            self.fall_gravity_multiplier,
            self.max_fall_speed,
            self.skin,
            self.push_mass,
        ];
        check(
            positive.iter().all(|x| x.is_finite() && *x > 0.0),
            "character sizes, speeds, heights, times and skin must be positive",
        )?;
        let non_negative = [
            self.ground_acceleration,
            self.ground_deceleration,
            self.air_control,
            self.apex_hang_speed,
            self.apex_gravity_multiplier,
            self.coyote_time,
            self.jump_buffer_time,
            self.wall_slide_speed,
            self.wall_jump_lock_time,
            self.wall_probe_distance,
            self.max_slope_angle,
            self.snap_to_ground,
            self.drop_through_time,
        ];
        check(
            non_negative.iter().all(|x| x.is_finite() && *x >= 0.0),
            "character tuning values must be finite and not negative",
        )?;
        check(
            self.wall_jump_velocity.is_finite(),
            "wall_jump_velocity must be finite",
        )?;
        check(
            self.min_jump_height <= self.jump_height,
            "min_jump_height must not exceed jump_height",
        )?;
        check(
            self.apex_gravity_multiplier > 0.0 && self.apex_gravity_multiplier <= 1.0,
            "apex_gravity_multiplier must be in (0, 1]",
        )?;
        check(
            self.max_slope_angle < std::f64::consts::FRAC_PI_2,
            "max_slope_angle must be below 90 degrees",
        )?;
        let h = self.jump_height;
        let g = 2.0 * h / (self.time_to_apex * self.time_to_apex);
        let a = self.apex_hang_speed;
        let m = self.apex_gravity_multiplier;
        // Rise = (v^2 - a^2) / 2g at full gravity, then a^2 / 2gm during the hang.
        let v2 = 2.0 * g * h + a * a - a * a / m;
        check(
            v2 > a * a,
            "apex_hang_speed too high for this jump (hang alone would exceed jump_height)",
        )?;
        let v0 = v2.sqrt();
        let release = (v2 / (2.0 * g * self.min_jump_height)).max(1.0);
        Ok(JumpPhysics {
            gravity: g,
            jump_velocity: v0,
            release_gravity_multiplier: release,
        })
    }
}

/// What the player asks the character to do this frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CharacterInput {
    /// Horizontal stick or keys, -1 (left) to 1 (right).
    pub move_x: f64,
    /// Jump button held. A press (false to true) is buffered for `jump_buffer_time`; holding
    /// it makes the jump higher.
    pub jump: bool,
    /// Drop-through button held (usually "down"). A press while standing on a one-way platform
    /// drops through it.
    pub drop_through: bool,
}

/// Which side a wall is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallSide {
    /// Wall to the left of the character.
    Left,
    /// Wall to the right of the character.
    Right,
}

impl WallSide {
    fn sign(self) -> f64 {
        match self {
            WallSide::Left => -1.0,
            WallSide::Right => 1.0,
        }
    }
}

/// Read-only snapshot of a character after the last tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharacterState {
    /// Center of the character's box.
    pub position: Vec2,
    /// Current velocity (not counting platform carry).
    pub velocity: Vec2,
    /// Standing on walkable ground.
    pub grounded: bool,
    /// Normal of the ground under the character when grounded.
    pub ground_normal: Option<Vec2>,
    /// The body the character stands on, when grounded.
    pub ground_body: Option<BodyId>,
    /// Standing on a one-way platform (so a drop-through would work).
    pub on_one_way: bool,
    /// Wall beside the character while airborne.
    pub wall: Option<WallSide>,
    /// Currently sliding down a wall.
    pub wall_sliding: bool,
    /// Coyote time left (a jump now still counts as a ground jump while > 0).
    pub coyote_remaining: f64,
    /// Buffered jump time left.
    pub jump_buffer_remaining: f64,
    /// Facing: -1 left, 1 right (last horizontal input).
    pub facing: f64,
}

/// Something a character did during a step, for sounds, particles and animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharacterEventKind {
    /// Jumped from the ground (or within coyote time).
    Jumped,
    /// Jumped off a wall.
    WallJumped,
    /// Touched down after being airborne.
    Landed,
    /// Dropped through a one-way platform.
    DroppedThrough,
}

/// A [`CharacterEventKind`] tagged with the character it happened to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharacterEvent {
    /// The character.
    pub character: BodyId,
    /// What happened.
    pub kind: CharacterEventKind,
}

#[derive(Debug, Clone, Copy)]
struct Ground {
    collider: ColliderHandle,
    body: Option<RigidBodyHandle>,
    normal: Vec2,
    one_way: bool,
}

pub(crate) fn to_groups(g: CollisionGroups) -> InteractionGroups {
    InteractionGroups::new(
        Group::from_bits_retain(g.memberships),
        Group::from_bits_retain(g.filter),
        InteractionTestMode::And,
    )
}

/// Highest extent of a collider along the unit direction `n` (world space).
fn max_along(collider: &Collider, n: Vector) -> f32 {
    let rot = Rotation::from_rotation_arc(n, Vector::Y);
    let pose = Pose::from_rotation(rot) * *collider.position();
    collider.shape().compute_aabb(&pose).maxs.y
}

/// Moves `value` toward `target` by at most `step`.
fn approach(value: f64, target: f64, step: f64) -> f64 {
    if value < target {
        (value + step).min(target)
    } else {
        (value - step).max(target)
    }
}

/// Shared data the character tick needs from the world.
pub(crate) struct TickContext<'a> {
    pub rapier: &'a mut RapierWorld,
    pub one_way: &'a HashMap<ColliderHandle, Vector>,
    pub owners: &'a HashMap<ColliderHandle, BodyId>,
    pub dt: f64,
}

/// One platformer character inside the world.
pub(crate) struct Character {
    pub(crate) id: BodyId,
    config: CharacterConfig,
    jump: JumpPhysics,
    pub(crate) body: RigidBodyHandle,
    pub(crate) collider: ColliderHandle,
    shape: SharedShape,
    pub(crate) position: Vec2,
    pub(crate) prev_position: Vec2,
    velocity: Vec2,
    input: CharacterInput,
    jump_pressed: bool,
    drop_pressed: bool,
    grounded: bool,
    ground: Option<Ground>,
    ground_pose_before: Option<Pose>,
    platform_velocity: Vec2,
    coyote: f64,
    buffer: f64,
    wall: Option<WallSide>,
    wall_sliding: bool,
    wall_lock: f64,
    in_jump: bool,
    /// Horizontal platform speed inherited by the current jump.
    air_base_speed: f64,
    facing: f64,
    drop_ignore: Vec<(ColliderHandle, f64)>,
    pub(crate) touches: Vec<BodyId>,
}

impl Character {
    pub(crate) fn new(
        id: BodyId,
        config: CharacterConfig,
        position: Vec2,
        rapier: &mut RapierWorld,
    ) -> Result<Self, Physics2dError> {
        let jump = config.jump_physics()?;
        check(position.is_finite(), "character position must be finite")?;
        let (hx, hy) = (config.half_extents.x as f32, config.half_extents.y as f32);
        let collider = ColliderBuilder::cuboid(hx, hy)
            .collision_groups(to_groups(config.groups))
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .active_collision_types(ActiveCollisionTypes::all());
        let (body, collider) = rapier.insert(
            // Never asleep: a sleeping kinematic body breaks the backend's island bookkeeping
            // when something starts touching it.
            RigidBodyBuilder::kinematic_position_based()
                .translation(v(position))
                .can_sleep(false),
            collider,
        );
        Ok(Character {
            id,
            config,
            jump,
            body,
            collider,
            shape: SharedShape::cuboid(hx, hy),
            position,
            prev_position: position,
            velocity: Vec2::ZERO,
            input: CharacterInput::default(),
            jump_pressed: false,
            drop_pressed: false,
            grounded: false,
            ground: None,
            ground_pose_before: None,
            platform_velocity: Vec2::ZERO,
            coyote: 0.0,
            buffer: 0.0,
            wall: None,
            wall_sliding: false,
            wall_lock: 0.0,
            in_jump: false,
            air_base_speed: 0.0,
            facing: 1.0,
            drop_ignore: Vec::new(),
            touches: Vec::new(),
        })
    }

    pub(crate) fn half_extents(&self) -> Vec2 {
        self.config.half_extents
    }

    pub(crate) fn set_input(&mut self, input: CharacterInput) {
        let move_x = if input.move_x.is_finite() {
            input.move_x.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        if input.jump && !self.input.jump {
            self.jump_pressed = true;
        }
        if input.drop_through && !self.input.drop_through {
            self.drop_pressed = true;
        }
        self.input = CharacterInput { move_x, ..input };
    }

    pub(crate) fn teleport(&mut self, position: Vec2, rapier: &mut RapierWorld) {
        self.position = position;
        self.prev_position = position;
        self.velocity = Vec2::ZERO;
        self.grounded = false;
        self.ground = None;
        self.coyote = 0.0;
        if let Some(b) = rapier.bodies.get_mut(self.body) {
            b.set_translation(v(position), true);
        }
    }

    pub(crate) fn state(&self, owners: &HashMap<ColliderHandle, BodyId>) -> CharacterState {
        CharacterState {
            position: self.position,
            velocity: self.velocity,
            grounded: self.grounded,
            ground_normal: self.ground.map(|g| g.normal),
            ground_body: self.ground.and_then(|g| owners.get(&g.collider).copied()),
            on_one_way: self.ground.is_some_and(|g| g.one_way),
            wall: self.wall,
            wall_sliding: self.wall_sliding,
            coyote_remaining: self.coyote,
            jump_buffer_remaining: self.buffer,
            facing: self.facing,
        }
    }

    /// Remembers the pose of the ground body before the world steps, for platform carry.
    pub(crate) fn before_world_step(&mut self, rapier: &RapierWorld) {
        self.ground_pose_before = self
            .ground
            .and_then(|g| g.body)
            .and_then(|b| rapier.bodies.get(b))
            .map(|b| *b.position());
    }

    /// Advances the character by one fixed tick (after the world stepped).
    pub(crate) fn tick(&mut self, ctx: &mut TickContext<'_>, events: &mut Vec<CharacterEvent>) {
        let dt = ctx.dt;
        let cfg = self.config;
        let g = self.jump.gravity;

        // Platform carry: follow the ground body's motion during this tick, rotation included.
        let mut carry = Vec2::ZERO;
        let carry_ground = self.ground.map(|g| g.collider);
        if let (Some(ground), Some(before)) = (self.ground, self.ground_pose_before.take())
            && let Some(after) = ground.body.and_then(|b| ctx.rapier.bodies.get(b))
        {
            let p = v(self.position);
            let moved = after.position().transform_point(before.inverse_transform_point(p));
            carry = from_v(moved - p);
        }
        self.platform_velocity = if self.grounded { carry * (1.0 / dt) } else { Vec2::ZERO };

        // Timers.
        if std::mem::take(&mut self.jump_pressed) {
            self.buffer = cfg.jump_buffer_time;
        } else {
            self.buffer = (self.buffer - dt).max(0.0);
        }
        self.coyote = if self.grounded {
            cfg.coyote_time
        } else {
            (self.coyote - dt).max(0.0)
        };
        self.wall_lock = (self.wall_lock - dt).max(0.0);
        for (_, t) in &mut self.drop_ignore {
            *t -= dt;
        }
        self.drop_ignore.retain(|(_, t)| *t > 0.0);
        if self.input.move_x.abs() > 0.1 {
            self.facing = self.input.move_x.signum();
        }

        // Drop through a one-way platform.
        let mut jumped = false;
        if std::mem::take(&mut self.drop_pressed)
            && self.grounded
            && let Some(ground) = self.ground.filter(|g| g.one_way)
        {
            self.drop_ignore.push((ground.collider, cfg.drop_through_time));
            self.grounded = false;
            self.ground = None;
            self.coyote = 0.0;
            self.buffer = 0.0;
            events.push(CharacterEvent {
                character: self.id,
                kind: CharacterEventKind::DroppedThrough,
            });
        }

        // Jump: ground (or coyote) first, then wall.
        if self.buffer > 0.0 {
            if self.grounded || self.coyote > 0.0 {
                self.velocity.y = self.jump.jump_velocity;
                if cfg.inherit_platform_velocity {
                    // Like jumping off a moving train: the platform's speed is kept for the
                    // whole jump and the player steers relative to it.
                    self.air_base_speed = self.platform_velocity.x;
                    self.velocity.x += self.platform_velocity.x;
                    self.velocity.y += self.platform_velocity.y.max(0.0);
                }
                jumped = true;
                events.push(CharacterEvent {
                    character: self.id,
                    kind: CharacterEventKind::Jumped,
                });
            } else if cfg.wall_jump
                && let Some(side) = self.wall
            {
                self.velocity = Vec2::new(
                    -side.sign() * cfg.wall_jump_velocity.x,
                    cfg.wall_jump_velocity.y,
                );
                self.wall_lock = cfg.wall_jump_lock_time;
                self.air_base_speed = 0.0;
                self.facing = -side.sign();
                jumped = true;
                events.push(CharacterEvent {
                    character: self.id,
                    kind: CharacterEventKind::WallJumped,
                });
            }
            if jumped {
                self.buffer = 0.0;
                self.coyote = 0.0;
                self.grounded = false;
                self.ground = None;
                self.in_jump = true;
            }
        }

        // Horizontal: accelerate toward the target speed; input is ignored right after a wall jump.
        if self.wall_lock <= 0.0 {
            let base = if self.grounded { 0.0 } else { self.air_base_speed };
            let target = base + self.input.move_x * cfg.run_speed;
            let speeding_up = target.abs() > self.velocity.x.abs() - 1e-9
                && (self.velocity.x == 0.0 || target.signum() == self.velocity.x.signum());
            let mut rate = if speeding_up {
                cfg.ground_acceleration
            } else {
                cfg.ground_deceleration
            };
            if !self.grounded {
                rate *= cfg.air_control;
            }
            self.velocity.x = approach(self.velocity.x, target, rate * dt);
        }

        // Vertical: designer gravity with release, apex-hang and fall multipliers.
        let mut dy = 0.0;
        self.wall_sliding = false;
        if !self.grounded {
            let vy = self.velocity.y;
            let gravity = if self.in_jump && vy > 0.0 && !self.input.jump {
                g * self.jump.release_gravity_multiplier
            } else if self.in_jump && self.input.jump && vy.abs() < cfg.apex_hang_speed {
                g * cfg.apex_gravity_multiplier
            } else if vy < 0.0 {
                g * cfg.fall_gravity_multiplier
            } else {
                g
            };
            let mut vy_new = (vy - gravity * dt).max(-cfg.max_fall_speed);
            if cfg.wall_slide
                && vy_new < -cfg.wall_slide_speed
                && let Some(side) = self.wall
                && self.input.move_x * side.sign() > 0.1
            {
                vy_new = -cfg.wall_slide_speed;
                self.wall_sliding = true;
            }
            // Trapezoid rule: exact for constant gravity within the tick.
            dy = (vy + vy_new) * 0.5 * dt;
            self.velocity.y = vy_new;
        }

        // Carry first, as its own move that ignores the platform (it already moved into the
        // character's old spot); walls and ceilings still stop it.
        if carry.length() > 1e-6 {
            self.slide(ctx, carry, carry_ground, false);
        }
        self.depenetrate(ctx);
        let was_grounded = self.grounded;
        let walking = was_grounded && !jumped;
        let hits = self.slide(ctx, Vec2::new(self.velocity.x * dt, dy), None, walking);

        // React to what was hit: stop at ceilings and walls.
        let max_slope_cos = cfg.max_slope_angle.cos();
        let mut touches: Vec<BodyId> = Vec::new();
        for hit in &hits {
            let n = hit.normal;
            if n.y < -0.3 && self.velocity.y > 0.0 {
                self.velocity.y = 0.0;
            }
            if n.y < max_slope_cos && n.x * self.velocity.x < 0.0 && n.x.abs() > 0.3 {
                self.velocity.x = 0.0;
            }
            if let Some(&id) = ctx.owners.get(&hit.collider)
                && !touches.contains(&id)
            {
                touches.push(id);
            }
        }
        self.push_bodies(ctx, &hits);

        // Ground: snap down slopes and stair edges when walking, then look for walkable ground.
        if walking && self.velocity.y <= 0.0 {
            let reach = cfg.snap_to_ground + cfg.skin;
            if let Some(hit) =
                self.cast(ctx, self.position, Vec2::new(0.0, -1.0), reach, None, OneWay::Solid)
                && hit.normal.y >= max_slope_cos
            {
                self.position.y -= (hit.toi - cfg.skin).max(0.0);
            }
        }
        self.ground = if self.velocity.y <= 0.0 {
            self.find_ground(ctx)
        } else {
            None
        };
        self.grounded = self.ground.is_some();
        if let Some(ground) = self.ground {
            self.velocity.y = 0.0;
            self.in_jump = false;
            self.air_base_speed = 0.0;
            if let Some(&id) = ctx.owners.get(&ground.collider)
                && !touches.contains(&id)
            {
                touches.push(id);
            }
            if !was_grounded {
                events.push(CharacterEvent {
                    character: self.id,
                    kind: CharacterEventKind::Landed,
                });
            }
        } else if self.velocity.y <= 0.0 && !self.input.jump {
            self.in_jump = false;
        }
        self.wall = if self.grounded { None } else { self.find_wall(ctx) };
        self.touches = touches;

        if let Some(b) = ctx.rapier.bodies.get_mut(self.body) {
            b.set_next_kinematic_translation(v(self.position));
        }
    }

    /// Bottom of the character along direction `n` when its center is at `center`.
    fn bottom_along(&self, center: Vec2, n: Vec2) -> f64 {
        let h = self.config.half_extents;
        center.dot(n) - (h.x * n.x.abs() + h.y * n.y.abs())
    }

    /// Whether a collider blocks this character's moves right now.
    fn blocks(
        &self,
        ctx: &TickContext<'_>,
        handle: ColliderHandle,
        collider: &Collider,
        exclude: Option<ColliderHandle>,
        one_way: OneWay,
    ) -> bool {
        if Some(handle) == exclude || self.drop_ignore.iter().any(|(i, _)| *i == handle) {
            return false;
        }
        match (ctx.one_way.get(&handle), one_way) {
            (None, _) => true,
            (Some(_), OneWay::Ignore) => false,
            (Some(local_n), OneWay::Solid) => {
                // Solid only while the character is above the platform's top surface.
                let n = collider.position().rotation * *local_n;
                let top = f64::from(max_along(collider, n));
                let tolerance = (self.config.skin * 2.0).max(0.05);
                self.bottom_along(self.position, from_v(n)) >= top - tolerance
            }
        }
    }

    /// Sweeps the character's box from `from` along the unit vector `dir` for `distance`.
    fn cast(
        &self,
        ctx: &TickContext<'_>,
        from: Vec2,
        dir: Vec2,
        distance: f64,
        exclude: Option<ColliderHandle>,
        one_way: OneWay,
    ) -> Option<MoveHit> {
        let predicate =
            |h: ColliderHandle, c: &Collider| -> bool { self.blocks(ctx, h, c, exclude, one_way) };
        let filter = QueryFilter::default()
            .exclude_sensors()
            .exclude_collider(self.collider)
            .groups(to_groups(self.config.groups))
            .predicate(&predicate);
        let options = ShapeCastOptions {
            max_time_of_impact: distance as f32,
            target_distance: 0.0,
            stop_at_penetration: false,
            compute_impact_geometry_on_penetration: true,
        };
        let pipeline = ctx.rapier.query_pipeline_with_filter(filter);
        let start = Pose::from_translation(v(from));
        pipeline
            .cast_shape(&start, v(dir), &*self.shape, options)
            .map(|(collider, hit)| MoveHit {
                collider,
                toi: f64::from(hit.time_of_impact),
                normal: from_v(hit.normal1),
            })
    }

    /// Collide and slide: moves by `desired`, stopping a skin's width before obstacles and
    /// sliding along them. While `walking`, a walkable slope keeps the horizontal speed (the
    /// character runs up it); steeper slopes act as walls. Returns what was hit.
    fn slide(
        &mut self,
        ctx: &TickContext<'_>,
        desired: Vec2,
        exclude: Option<ColliderHandle>,
        walking: bool,
    ) -> Vec<MoveHit> {
        let skin = self.config.skin;
        let max_slope_cos = self.config.max_slope_angle.cos();
        let mut hits = Vec::new();
        let mut remaining = desired;
        for _ in 0..4 {
            let len = remaining.length();
            let Some(dir) = remaining.normalized().filter(|_| len > 1e-7) else {
                break;
            };
            let Some(hit) = self.cast(ctx, self.position, dir, len, exclude, OneWay::Solid) else {
                self.position += remaining;
                break;
            };
            let advance = (hit.toi - skin).max(0.0);
            self.position += dir * advance;
            let mut rest = remaining - dir * advance;
            let n = hit.normal;
            let walkable = n.y >= max_slope_cos;
            if walkable && walking && rest.x.abs() > 1e-9 {
                // Run up (or along) the slope with the same horizontal progress.
                let tangent = Vec2::new(n.y, -n.x);
                rest = tangent * (rest.x / tangent.x.max(1e-6));
            } else {
                // Steep slopes act as vertical walls so they cannot be climbed.
                let plane = if !walkable && n.y > 0.0 {
                    Vec2::new(n.x, 0.0).normalized().unwrap_or(n)
                } else {
                    n
                };
                rest -= plane * rest.dot(plane).min(0.0);
            }
            hits.push(hit);
            remaining = rest;
        }
        hits
    }

    /// Pushes the character out of anything it overlaps (or nearly touches) so every move
    /// starts a skin's width away from surfaces.
    fn depenetrate(&mut self, ctx: &TickContext<'_>) {
        let max_push = self.config.half_extents.y * 0.5;
        let mut pushed = 0.0;
        for _ in 0..4 {
            let correction = self.penetration_correction(ctx);
            let len = correction.length();
            if len < 1e-9 || pushed >= max_push {
                break;
            }
            let step = len.min(max_push - pushed);
            self.position += correction * (step / len);
            pushed += step;
        }
    }

    /// The smallest push that restores a skin's width of clearance from every overlapping or
    /// nearly touching collider.
    fn penetration_correction(&self, ctx: &TickContext<'_>) -> Vec2 {
        let skin = self.config.skin;
        let predicate =
            |h: ColliderHandle, c: &Collider| -> bool { self.blocks(ctx, h, c, None, OneWay::Solid) };
        let filter = QueryFilter::default()
            .exclude_sensors()
            .exclude_collider(self.collider)
            .groups(to_groups(self.config.groups))
            .predicate(&predicate);
        let pipeline = ctx.rapier.query_pipeline_with_filter(filter);
        let pos = Pose::from_translation(v(self.position));
        let aabb = self.shape.compute_aabb(&pos).loosened(skin as f32);
        let mut correction = Vec2::ZERO;
        for (_, collider) in pipeline.intersect_aabb_conservative(aabb) {
            let pos12 = pos.inv_mul(collider.position());
            if let Ok(Some(contact)) =
                pipeline
                    .dispatcher
                    .contact(&pos12, &*self.shape, collider.shape(), skin as f32)
            {
                let dist = f64::from(contact.dist);
                if dist < skin * 0.5 {
                    // normal1 points from the character toward the obstacle: move away.
                    let away = -from_v(contact.normal1);
                    let need = skin - dist - correction.dot(away);
                    if need > 0.0 {
                        correction += away * need;
                    }
                }
            }
        }
        correction
    }

    /// Gives dynamic bodies the character walked into a shove.
    fn push_bodies(&self, ctx: &mut TickContext<'_>, hits: &[MoveHit]) {
        for hit in hits {
            let Some(parent) = ctx
                .rapier
                .colliders
                .get(hit.collider)
                .and_then(|c| c.parent())
            else {
                continue;
            };
            let into = -self.velocity.dot(hit.normal);
            if into <= 0.0 {
                continue;
            }
            if let Some(body) = ctx.rapier.bodies.get_mut(parent)
                && body.is_dynamic()
            {
                // Momentum the character carries into the body this tick.
                let impulse = -hit.normal * (self.config.push_mass * into * ctx.dt * PUSH_RATE);
                body.apply_impulse(v(impulse), false);
                // Wake through the island manager so its bookkeeping stays consistent.
                ctx.rapier.wake_up(parent, true);
            }
        }
    }

    fn find_ground(&self, ctx: &TickContext<'_>) -> Option<Ground> {
        let reach = self.config.skin * 3.0 + 0.05;
        let hit = self.cast(ctx, self.position, Vec2::new(0.0, -1.0), reach, None, OneWay::Solid)?;
        if hit.normal.y < self.config.max_slope_angle.cos() {
            return None;
        }
        let body = ctx.rapier.colliders.get(hit.collider).and_then(|c| c.parent());
        Some(Ground {
            collider: hit.collider,
            body,
            normal: hit.normal,
            one_way: ctx.one_way.contains_key(&hit.collider),
        })
    }

    fn find_wall(&self, ctx: &TickContext<'_>) -> Option<WallSide> {
        let reach = self.config.wall_probe_distance + self.config.skin;
        let mut found = None;
        for side in [WallSide::Left, WallSide::Right] {
            let dir = Vec2::new(side.sign(), 0.0);
            if let Some(hit) = self.cast(ctx, self.position, dir, reach, None, OneWay::Ignore)
                && hit.normal.x * side.sign() < -0.7
            {
                // Prefer the wall the player presses toward.
                if found.is_none() || self.input.move_x * side.sign() > 0.0 {
                    found = Some(side);
                }
            }
        }
        found
    }
}

/// How hard the character shoves dynamic bodies: per second of contact, the body receives
/// this many times the character's momentum into it.
const PUSH_RATE: f64 = 10.0;

/// How a query treats one-way platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OneWay {
    /// Solid while the character is above them.
    Solid,
    /// Never solid (wall probes).
    Ignore,
}

/// Something a character move ran into.
#[derive(Debug, Clone, Copy)]
struct MoveHit {
    collider: ColliderHandle,
    /// Distance travelled along the cast direction until contact.
    toi: f64,
    /// Surface normal at the contact, pointing out of the obstacle.
    normal: Vec2,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_jump_reaches_designed_height_analytically() {
        let cfg = CharacterConfig::default();
        let j = cfg.jump_physics().expect("valid default config");
        let a = cfg.apex_hang_speed;
        let m = cfg.apex_gravity_multiplier;
        let rise = (j.jump_velocity.powi(2) - a * a) / (2.0 * j.gravity) + a * a / (2.0 * j.gravity * m);
        assert!((rise - cfg.jump_height).abs() < 1e-9);
        let tap = j.jump_velocity.powi(2) / (2.0 * j.gravity * j.release_gravity_multiplier);
        assert!((tap - cfg.min_jump_height).abs() < 1e-9);
    }

    #[test]
    fn invalid_config_is_rejected() {
        let cfg = CharacterConfig {
            time_to_apex: f64::NAN,
            ..CharacterConfig::default()
        };
        assert!(cfg.jump_physics().is_err());
        let cfg = CharacterConfig {
            min_jump_height: 20.0,
            ..CharacterConfig::default()
        };
        assert!(cfg.jump_physics().is_err());
        let cfg = CharacterConfig {
            apex_hang_speed: 1000.0,
            ..CharacterConfig::default()
        };
        assert!(cfg.jump_physics().is_err());
    }

    #[test]
    fn approach_never_overshoots() {
        assert_eq!(approach(0.0, 10.0, 3.0), 3.0);
        assert_eq!(approach(9.0, 10.0, 3.0), 10.0);
        assert_eq!(approach(5.0, -10.0, 20.0), -10.0);
    }
}
