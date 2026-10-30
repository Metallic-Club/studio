//! The Humanoid character: spawning, the kinematic capsule controller, death, respawn and
//! checkpoints.
//!
//! A character is a Model named after its Player holding a `Humanoid` and the avatar parts (see
//! [`spawn_character`]). [`CharacterController::step`] moves it once per frame, before
//! [`PhysicsWorld::step`]: it reads the `HumanoidRootPart` Position (so scripts can teleport the
//! character by setting it), sweeps a capsule through the scene with Rapier's kinematic
//! character controller and writes the new pose to every avatar part.
//!
//! Movement (units are studs and seconds):
//! - horizontal speed is `Humanoid.WalkSpeed` (input direction longer than 1 is shortened to 1),
//!   with instant acceleration and full air control, as in Roblox; [`MoveInput::run`] multiplies
//!   it by [`RUN_MULTIPLIER`] (a Metallic addition) without changing `WalkSpeed`;
//! - a jump sets the upward velocity to `Humanoid.JumpPower` (50 -> apex 50^2 / (2 * 196.2) =
//!   6.37 studs); holding jump jumps again on landing;
//! - gravity is [`crate::GRAVITY`] (196.2), integrated exactly for constant acceleration;
//! - surfaces up to `Humanoid.MaxSlopeAngle` are walkable and never slid down; steps up to
//!   [`MAX_STEP_HEIGHT`] (1.5) are climbed; walking down steps and slopes snaps to the ground;
//! - standing follows the last completed motion of the support's foot contact, including
//!   rotation; idle facing follows its world-up angular rate. Explicit input still faces the
//!   world-space movement direction. Jumping keeps the contact's horizontal point velocity
//!   (`linear + angular × lever arm`) until landing;
//! - a part that moved into the character pushes it out, at most [`MAX_PUSH`] (0.5) studs per
//!   step so it can never be shoved through a thin wall in one frame.
//!
//! Support carry assumes one controller step before each physics step (the runtime's fixed
//! tick order). The body-local contact and sampled pose preserve finite rotation arcs rather
//! than accumulating tangent-integration drift. Collision sweeps can shorten the carry; the
//! character is not rigidly parented to the support. The capsule/avatar stay upright on tilted
//! supports, and support pitch/roll never tilt the camera. Removed/rebuilt bodies or a root
//! position changed by a script discard the old contact. Rotating support restores the
//! skin clearance under the same push cap, preventing numerical floor interference from
//! clipping a carry step. Anchored support rates come from PhysicsWorld's cache, because
//! Rapier's mover velocity is reset after landing at its target pose. Teleport classification belongs to
//! PhysicsWorld; this controller does not introduce another cutoff.
//!
//! Touches: after moving, the controller overlaps the whole avatar (the capsule and every avatar
//! part, each grown by 0.1 stud) with the scene, sensors included, and reports
//! [`CharacterEvent::Touched`] once per part when contact starts, and again only after the part
//! separated. This is the only source of character touches: standing still on lava, or lava
//! appearing under a standing character, is reported too, and [`PhysicsWorld::drain_touched`]
//! never reports character parts. The runtime (Tasks 7/8) turns these events into
//! `BasePart.Touched` on the touched part (with a character part as `hit`). Dead characters
//! report no touches.
//!
//! Rules: `Health <= 0` or the root below `Workspace.FallenPartsDestroyHeight` makes the
//! character Dead (one [`CharacterEvent::Died`], input ignored, gravity still applies); after
//! [`RESPAWN_DELAY`] seconds the old Model is destroyed and a fresh one spawns. Touching a
//! Checkpoint whose `Order` is higher than the current `RespawnLocation`'s (any Checkpoint beats a
//! non-Checkpoint or none) makes it the player's `RespawnLocation` and colours it violet.
use crate::avatar::{self, ROOT_PART, VIOLET};
use crate::{GRAVITY, PhysicsWorld, from_vector, to_pose, to_vector};
use metallic_core::{CoreError, InstanceId, Value, Vec3, World};
use rapier3d::control::{CharacterAutostep, CharacterLength, KinematicCharacterController};
use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::parry::shape::{Capsule, Cuboid, Shape};
use rapier3d::prelude::{ColliderHandle, Pose, QueryFilter, RigidBodyHandle, Vector};
use std::collections::{HashMap, HashSet};

/// Seconds between a character's death and its respawn (the Roblox default).
pub const RESPAWN_DELAY: f64 = 5.0;
/// Highest step, in studs, the character walks up without jumping.
pub const MAX_STEP_HEIGHT: f64 = 1.5;
/// Height of the `HumanoidRootPart` centre above the feet, in studs.
pub const ROOT_HEIGHT: f64 = 3.0;

/// Capsule radius (the torso is 2 studs wide).
const RADIUS: f64 = 1.0;
/// Half length of the capsule's straight segment: total height 2 * (1.5 + 1) = 5 studs.
const HALF_SEGMENT: f64 = 1.5;
/// Gap the sweep keeps between the capsule and the world.
const SKIN: f64 = 0.02;
/// The capsule bottom is this far above the feet; equal to [`SKIN`], so standing feet rest
/// exactly on the floor (root 3 studs above it).
const FOOT_LIFT: f64 = SKIN;
/// Root centre minus capsule centre.
const CAPSULE_DROP: f64 = ROOT_HEIGHT - FOOT_LIFT - HALF_SEGMENT - RADIUS;
/// How far below the feet the ground probe looks.
const GROUND_PROBE: f64 = 0.1;
/// Walking down a drop up to this height keeps the character on the ground.
const SNAP_DISTANCE: f64 = 1.5;
/// How much the touch overlap test grows the capsule and every avatar part, in studs.
const TOUCH_MARGIN: f64 = 0.1;
/// Most the character is pushed out of overlapping parts in one step, in studs.
pub const MAX_PUSH: f64 = 0.5;
/// Running ([`MoveInput::run`]) moves this many times faster than `Humanoid.WalkSpeed`.
pub const RUN_MULTIPLIER: f64 = 1.5;

/// Movement wish for one frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MoveInput {
    /// Desired walking direction in world space; Y is ignored and lengths above 1 count as 1
    /// (a gamepad stick at half tilt walks at half speed).
    pub dir: Vec3,
    /// Jump (when standing on something). Holding it jumps again on landing.
    pub jump: bool,
    /// Run at [`RUN_MULTIPLIER`] times `Humanoid.WalkSpeed` (scripts still read `WalkSpeed`).
    pub run: bool,
}

/// What a character is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HumanoidState {
    /// Standing or walking on a surface.
    Running,
    /// In the air and still rising after a jump.
    Jumping,
    /// In the air otherwise (falling, or walked off a ledge).
    Freefall,
    /// Health reached 0 or the character fell below the destroy height; waits to respawn.
    Dead,
}

/// Something that happened during [`CharacterController::step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharacterEvent {
    /// The character died (reported once per life; `Humanoid.Died` in scripts).
    Died {
        /// The character Model.
        character: InstanceId,
        /// Its Humanoid.
        humanoid: InstanceId,
    },
    /// The old character was destroyed and a new one spawned. Call
    /// [`PhysicsWorld::sync_from`] before the next physics step so the new parts get bodies.
    Respawned {
        /// The Player.
        player: InstanceId,
        /// The destroyed Model (its id is no longer valid).
        old: InstanceId,
        /// The new Model, now `Player.Character`.
        character: InstanceId,
    },
    /// The character started touching a part (once per contact, again only after separation).
    /// The runtime fires the part's `Touched` for it.
    Touched {
        /// The character Model.
        character: InstanceId,
        /// The part touched (not a part of the character).
        part: InstanceId,
    },
    /// A Checkpoint became the player's `RespawnLocation`.
    CheckpointReached {
        /// The Player.
        player: InstanceId,
        /// The Checkpoint part.
        checkpoint: InstanceId,
    },
}

/// Contact on the support after the last controller step. The local point and pose preserve
/// the finite rotation arc instead of repeatedly approximating it with a tangent.
#[derive(Debug, Clone, Copy)]
struct Support {
    part: InstanceId,
    body: RigidBodyHandle,
    local_point: Vector,
    pose: Pose,
    root: Vec3,
}

impl Support {
    fn sample(
        physics: &PhysicsWorld,
        handle: ColliderHandle,
        point: Vec3,
        root: Vec3,
    ) -> Option<Self> {
        let part = physics.owner_of(handle)?;
        let entry = physics.entries.get(&part)?;
        let body = physics.rapier.bodies.get(entry.body)?;
        Some(Self {
            part,
            body: entry.body,
            local_point: body.position().inverse_transform_point(to_vector(point)),
            pose: *body.position(),
            root,
        })
    }

    /// Completed support displacement, instantaneous contact velocity and world-up yaw rate.
    fn motion(self, physics: &PhysicsWorld) -> (Vec3, Vec3, f64) {
        let Some(entry) = physics
            .entries
            .get(&self.part)
            .filter(|e| e.body == self.body)
        else {
            return (Vec3::default(), Vec3::default(), 0.0);
        };
        let Some(body) = physics.rapier.bodies.get(entry.body) else {
            return (Vec3::default(), Vec3::default(), 0.0);
        };
        let angular = physics.angular_velocity(self.part).unwrap_or_default();
        let linear = physics.velocity(self.part).unwrap_or_default();
        // Anchored movers have been reset to fixed/zero Rapier velocities by PhysicsWorld.
        // Respect its cached rates, including a zero-rate move classified as a teleport.
        if linear.magnitude() == 0.0 && angular.magnitude() == 0.0 {
            return (Vec3::default(), Vec3::default(), 0.0);
        }
        let point = body.position().transform_point(self.local_point);
        let velocity = if body.is_dynamic() {
            // Rapier accounts for the dynamic body's world centre of mass.
            body.velocity_at_point(point)
        } else {
            to_vector(linear) + to_vector(angular).cross(point - body.translation())
        };
        (
            from_vector(point - self.pose.transform_point(self.local_point)),
            from_vector(velocity),
            angular.y,
        )
    }
}

/// Per-character movement state kept between frames.
#[derive(Debug, Clone, Copy)]
struct Motion {
    state: HumanoidState,
    /// Vertical velocity of the character's own motion (jumping, falling).
    vertical: f64,
    /// Horizontal velocity inherited from the part the character left, kept while airborne.
    inherited: Vec3,
    /// The contact stood on after the last step.
    ground: Option<Support>,
    /// Facing in degrees around +Y (0 looks along -Z).
    yaw: f64,
    /// Seconds since death.
    dead_time: f64,
    /// Total velocity of the last step.
    velocity: Vec3,
}

impl Default for Motion {
    fn default() -> Self {
        Motion {
            state: HumanoidState::Freefall,
            vertical: 0.0,
            inherited: Vec3::default(),
            ground: None,
            yaw: 0.0,
            dead_time: 0.0,
            velocity: Vec3::default(),
        }
    }
}

/// Builds a character for `player` and makes it the player's `Character`.
///
/// The Model is named after the player and holds the avatar parts (an invisible 2x2x1
/// `HumanoidRootPart`, Head 2x1x1, Torso 2x2x1 with a violet `Stripe`, arms and legs 1x2x1, all
/// titanium and Anchored) and a `Humanoid` with default properties. The root is placed
/// [`ROOT_HEIGHT`] above the top surface (rotation included) of the player's `RespawnLocation`,
/// even a non-colliding zone (the character then falls through it), else of the first
/// `SpawnLocation` in Workspace, else at (0, 3, 0). A previous character of the player is
/// destroyed. Call [`PhysicsWorld::sync_from`] afterwards.
///
/// Errors: [`CoreError::InvalidInstance`] if `player` does not exist or the world has no
/// Workspace.
pub fn spawn_character(world: &mut World, player: InstanceId) -> Result<InstanceId, CoreError> {
    let name = world.get(player, "Name")?;
    let workspace = world.root("Workspace").ok_or(CoreError::InvalidInstance)?;
    if let Ok(Value::Ref(Some(old))) = world.get(player, "Character") {
        world.destroy(old);
    }
    let root = spawn_point(world, player, workspace);
    let model = world.create("Model", Some(workspace))?;
    world.set(model, "Name", name)?;
    avatar::build(world, model, root)?;
    world.create("Humanoid", Some(model))?;
    world.set(player, "Character", Value::Ref(Some(model)))?;
    Ok(model)
}

fn vec_prop(world: &World, id: InstanceId, prop: &str) -> Option<Vec3> {
    match world.get(id, prop) {
        Ok(Value::Vector3(v)) => Some(v),
        _ => None,
    }
}

fn num_prop(world: &World, id: InstanceId, prop: &str) -> Option<f64> {
    match world.get(id, prop) {
        Ok(Value::Number(v)) => Some(v),
        _ => None,
    }
}

fn ref_prop(world: &World, id: InstanceId, prop: &str) -> Option<InstanceId> {
    match world.get(id, prop) {
        Ok(Value::Ref(r)) => r,
        _ => None,
    }
}

/// Root position for a new character: on top of the RespawnLocation, the first SpawnLocation,
/// or (0, 3, 0).
fn spawn_point(world: &World, player: InstanceId, workspace: InstanceId) -> Vec3 {
    let pad = ref_prop(world, player, "RespawnLocation")
        .filter(|&p| world.is_a(p, "BasePart"))
        .or_else(|| first_descendant(world, workspace, "SpawnLocation"));
    let on_top = pad.and_then(|p| {
        let pos = vec_prop(world, p, "Position")?;
        let size = vec_prop(world, p, "Size")?;
        let o = vec_prop(world, p, "Orientation")?;
        // Half the part's world-space height: its rotated extents projected on Y.
        let up = crate::sync::matrix_from_orientation(o)[1];
        let half = (up[0].abs() * size.x + up[1].abs() * size.y + up[2].abs() * size.z) / 2.0;
        Some(pos + Vec3::new(0.0, half + ROOT_HEIGHT, 0.0))
    });
    on_top.unwrap_or(Vec3::new(0.0, ROOT_HEIGHT, 0.0))
}

/// The first descendant of `root` (depth first, in child order) that is a `class`.
fn first_descendant(world: &World, root: InstanceId, class: &str) -> Option<InstanceId> {
    let mut stack: Vec<InstanceId> = world.children(root).iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        if world.is_a(id, class) {
            return Some(id);
        }
        stack.extend(world.children(id).iter().rev().copied());
    }
    None
}

/// The Player whose `Character` is `character`.
fn player_of(world: &World, character: InstanceId) -> Option<InstanceId> {
    let players = world.root("Players")?;
    world
        .children(players)
        .iter()
        .copied()
        .find(|&p| ref_prop(world, p, "Character") == Some(character))
}

/// All BaseParts inside `model`.
fn parts_of(world: &World, model: InstanceId) -> Vec<InstanceId> {
    let mut out = Vec::new();
    let mut stack = vec![model];
    while let Some(id) = stack.pop() {
        if world.is_a(id, "BasePart") {
            out.push(id);
        }
        stack.extend(world.children(id).iter().copied());
    }
    out
}

fn capsule() -> Capsule {
    Capsule::new_y(HALF_SEGMENT as f32, RADIUS as f32)
}

fn capsule_pose(root: Vec3) -> Pose {
    Pose::from_translation(to_vector(root - Vec3::new(0.0, CAPSULE_DROP, 0.0)))
}

/// Moves Humanoid characters; keeps their per-character state between frames.
#[derive(Default)]
pub struct CharacterController {
    motions: HashMap<InstanceId, Motion>,
    /// Parts each character touched after its last step.
    touching: HashMap<InstanceId, HashSet<InstanceId>>,
    events: Vec<CharacterEvent>,
}

impl CharacterController {
    /// A controller with no characters yet (state is created on a character's first step).
    pub fn new() -> Self {
        Self::default()
    }

    /// The state of a character after its last step; `None` if it was never stepped or no
    /// longer exists.
    pub fn state(&self, character: InstanceId) -> Option<HumanoidState> {
        self.motions.get(&character).map(|m| m.state)
    }

    /// The character's velocity during its last step in studs per second (own motion plus the
    /// carrying part's).
    pub fn velocity(&self, character: InstanceId) -> Option<Vec3> {
        self.motions.get(&character).map(|m| m.velocity)
    }

    /// Takes the events of all steps since the last call, oldest first.
    pub fn drain_events(&mut self) -> Vec<CharacterEvent> {
        std::mem::take(&mut self.events)
    }

    /// Advances one character by `dt` seconds (call once per frame per character, before
    /// [`PhysicsWorld::step`]). See the module documentation for the rules.
    ///
    /// Does nothing if `character` no longer exists or lacks its `Humanoid` or
    /// `HumanoidRootPart`. After a [`CharacterEvent::Respawned`] the player's new character is
    /// in `Player.Character`; step that one from then on and call [`PhysicsWorld::sync_from`].
    pub fn step(
        &mut self,
        world: &mut World,
        physics: &PhysicsWorld,
        character: InstanceId,
        input: MoveInput,
        dt: f64,
    ) {
        self.motions.retain(|&id, _| world.is_alive(id));
        self.touching.retain(|&id, _| world.is_alive(id));
        let (Some(humanoid), Some(root)) = (
            world.find_first_child(character, "Humanoid"),
            world.find_first_child(character, ROOT_PART),
        ) else {
            return;
        };
        let Some(root_pos) = vec_prop(world, root, "Position") else {
            return;
        };
        let mut m = self.motions.get(&character).copied().unwrap_or_default();
        let dt = dt.max(0.0);

        if m.state == HumanoidState::Dead {
            m.dead_time += dt;
            if m.dead_time >= RESPAWN_DELAY
                && let Some(player) = player_of(world, character)
                && let Ok(new) = spawn_character(world, player)
            {
                self.motions.remove(&character);
                self.touching.remove(&character);
                self.events.push(CharacterEvent::Respawned {
                    player,
                    old: character,
                    character: new,
                });
                return;
            }
        } else {
            let health = num_prop(world, humanoid, "Health").unwrap_or(0.0);
            let floor = world
                .root("Workspace")
                .and_then(|w| num_prop(world, w, "FallenPartsDestroyHeight"))
                .unwrap_or(f64::NEG_INFINITY);
            if health <= 0.0 || root_pos.y < floor {
                m = Motion {
                    state: HumanoidState::Dead,
                    vertical: m.vertical.min(0.0),
                    yaw: m.yaw,
                    ..Motion::default()
                };
                self.events.push(CharacterEvent::Died {
                    character,
                    humanoid,
                });
            }
        }
        let alive = m.state != HumanoidState::Dead;
        let input = if alive { input } else { MoveInput::default() };
        let hum = |p: &str, default: f64| num_prop(world, humanoid, p).unwrap_or(default);
        let tuning = Tuning {
            walk_speed: hum("WalkSpeed", 16.0) * if input.run { RUN_MULTIPLIER } else { 1.0 },
            jump_power: hum("JumpPower", 50.0),
            max_slope: hum("MaxSlopeAngle", 89.0).clamp(0.0, 89.9),
        };

        let own: Vec<ColliderHandle> = parts_of(world, character)
            .into_iter()
            .filter_map(|p| physics.collider_of(p))
            .collect();
        let new_root = move_capsule(physics, &own, &mut m, root_pos, input, &tuning, dt);

        let flat = Vec3::new(input.dir.x, 0.0, input.dir.z);
        if flat.magnitude() > 1e-3 {
            m.yaw = (-flat.x).atan2(-flat.z).to_degrees();
        }
        avatar::place(world, character, new_root, m.yaw);
        let now = if alive {
            overlapping_parts(world, physics, &own, character, new_root)
        } else {
            HashSet::new()
        };
        let before = self.touching.remove(&character).unwrap_or_default();
        let mut started: Vec<InstanceId> = now.difference(&before).copied().collect();
        started.sort();
        for &part in &started {
            self.events
                .push(CharacterEvent::Touched { character, part });
        }
        self.reach_checkpoints(world, character, &started);
        self.touching.insert(character, now);
        self.motions.insert(character, m);
    }

    /// Applies the checkpoint rule to every newly touched Checkpoint.
    fn reach_checkpoints(
        &mut self,
        world: &mut World,
        character: InstanceId,
        started: &[InstanceId],
    ) {
        let touched: Vec<InstanceId> = started
            .iter()
            .copied()
            .filter(|&p| world.is_a(p, "Checkpoint"))
            .collect();
        if touched.is_empty() {
            return;
        }
        let Some(player) = player_of(world, character) else {
            return;
        };
        for cp in touched {
            let order = num_prop(world, cp, "Order").unwrap_or(0.0);
            let current = ref_prop(world, player, "RespawnLocation")
                .filter(|&c| world.is_a(c, "Checkpoint"))
                .and_then(|c| num_prop(world, c, "Order"))
                .unwrap_or(f64::NEG_INFINITY);
            if order > current
                && world
                    .set(player, "RespawnLocation", Value::Ref(Some(cp)))
                    .is_ok()
            {
                let _ = world.set(cp, "Color", Value::Color3(VIOLET));
                self.events.push(CharacterEvent::CheckpointReached {
                    player,
                    checkpoint: cp,
                });
            }
        }
    }
}

/// Parts (not the character's own) overlapping the capsule or any avatar part, each grown by
/// [`TOUCH_MARGIN`]; sensors included. Reads the avatar parts' pose from `world`.
fn overlapping_parts(
    world: &World,
    physics: &PhysicsWorld,
    own: &[ColliderHandle],
    character: InstanceId,
    root: Vec3,
) -> HashSet<InstanceId> {
    let skip = |h: ColliderHandle, _: &rapier3d::prelude::Collider| !own.contains(&h);
    let queries = physics.queries(QueryFilter::default().predicate(&skip));
    let mut out = HashSet::new();
    let probe = Capsule::new_y(HALF_SEGMENT as f32, (RADIUS + TOUCH_MARGIN) as f32);
    out.extend(
        queries
            .intersect_shape(capsule_pose(root), &probe)
            .filter_map(|(h, _)| physics.owner_of(h)),
    );
    for part in parts_of(world, character) {
        let (Some(pos), Some(o), Some(size)) = (
            vec_prop(world, part, "Position"),
            vec_prop(world, part, "Orientation"),
            vec_prop(world, part, "Size"),
        ) else {
            continue;
        };
        let half = size * 0.5 + Vec3::new(TOUCH_MARGIN, TOUCH_MARGIN, TOUCH_MARGIN);
        let shape = Cuboid::new(to_vector(half));
        out.extend(
            queries
                .intersect_shape(to_pose(pos, o), &shape)
                .filter_map(|(h, _)| physics.owner_of(h)),
        );
    }
    out
}

/// Humanoid properties the movement uses.
struct Tuning {
    walk_speed: f64,
    jump_power: f64,
    /// Degrees.
    max_slope: f64,
}

/// Sweeps the capsule for one frame and returns the new root position; updates `m`.
fn move_capsule(
    physics: &PhysicsWorld,
    own: &[ColliderHandle],
    m: &mut Motion,
    root: Vec3,
    input: MoveInput,
    t: &Tuning,
    dt: f64,
) -> Vec3 {
    let skip = |h: ColliderHandle, _: &rapier3d::prelude::Collider| !own.contains(&h);
    let filter = QueryFilter::default().exclude_sensors().predicate(&skip);
    let queries = physics.queries(filter);
    let shape = capsule();

    // 1. Parts that moved into the character push it out. A rotating support also restores
    // its skin gap; otherwise tiny numerical drift lets a floor clip horizontal carry.
    let sampled_root = root;
    let grounded = m.state == HumanoidState::Running;
    let support = m.ground.filter(|s| grounded && s.root == sampled_root);
    let push = push_out(&queries, &shape, root);
    let mut root = root + push;
    if dt > 0.0
        && let Some(support) = support
        && physics
            .angular_velocity(support.part)
            .is_some_and(|angular| angular.magnitude() > 0.0)
        && let Some((handle, normal, point)) = probe_ground(&queries, &shape, root)
        && physics.owner_of(handle) == Some(support.part)
        && normal.y >= t.max_slope.to_radians().cos()
    {
        let sole = to_vector(root - Vec3::new(0.0, CAPSULE_DROP + HALF_SEGMENT, 0.0))
            - to_vector(normal) * RADIUS as f32;
        let gap = f64::from((sole - to_vector(point)).dot(to_vector(normal)));
        if gap >= -1e-4 {
            let correction = (SKIN - gap).clamp(0.0, (MAX_PUSH - push.magnitude()).max(0.0));
            root = root + normal * correction;
        }
    }

    // 2. Velocity for this frame.
    let mut wish = Vec3::new(input.dir.x, 0.0, input.dir.z);
    if wish.magnitude() > 1.0 {
        wish = wish.unit();
    }
    let walk = wish * t.walk_speed;
    let (support_step, carry, yaw_rate) = if grounded && dt > 0.0 {
        support
            .map(|support| support.motion(physics))
            .unwrap_or_default()
    } else {
        (Vec3::default(), Vec3::default(), 0.0)
    };
    let jumping = grounded && input.jump;
    let dy = if grounded && !jumping {
        m.vertical = 0.0;
        m.yaw += (yaw_rate * dt).to_degrees();
        support_step.y
    } else {
        if jumping {
            m.vertical = t.jump_power;
            m.inherited = Vec3::new(carry.x, 0.0, carry.z);
        }
        let dy = m.vertical * dt - 0.5 * GRAVITY * dt * dt;
        m.vertical -= GRAVITY * dt;
        dy
    };
    if grounded && !jumping {
        m.inherited = Vec3::new(carry.x, 0.0, carry.z);
    }
    let horizontal = if grounded && !jumping {
        walk * dt + Vec3::new(support_step.x, 0.0, support_step.z)
    } else {
        (walk + m.inherited) * dt
    };
    let desired = Vec3::new(horizontal.x, dy, horizontal.z);

    // 3. Sweep.
    let slope = (t.max_slope as f32).to_radians();
    let kcc = KinematicCharacterController {
        offset: CharacterLength::Absolute(SKIN as f32),
        autostep: Some(CharacterAutostep {
            max_height: CharacterLength::Absolute(MAX_STEP_HEIGHT as f32),
            min_width: CharacterLength::Absolute(0.5),
            include_dynamic_bodies: false,
        }),
        max_slope_climb_angle: slope,
        min_slope_slide_angle: slope,
        snap_to_ground: (grounded && !jumping)
            .then_some(CharacterLength::Absolute(SNAP_DISTANCE as f32)),
        ..KinematicCharacterController::default()
    };
    // With no wish to move, skip the sweep: Rapier would depenetrate on its own then, past the
    // MAX_PUSH limit.
    let step = if desired.magnitude() > 1e-6 {
        let moved = kcc.move_shape(
            dt as f32,
            &queries,
            &shape,
            &capsule_pose(root),
            to_vector(desired),
            |_| {},
        );
        from_vector(moved.translation)
    } else {
        Vec3::default()
    };
    let new_root = root + step;

    // 4. Head bump: rising but blocked from above.
    if m.vertical > 0.0 && desired.y > 1e-6 && step.y < desired.y * 0.5 {
        m.vertical = 0.0;
    }

    // 5. Ground under the feet.
    m.ground = None;
    if m.vertical <= 0.0
        && let Some((handle, normal, point)) = probe_ground(&queries, &shape, new_root)
        && normal.y >= t.max_slope.to_radians().cos()
    {
        m.ground = Support::sample(physics, handle, point, new_root);
        m.vertical = 0.0;
        m.inherited = Vec3::default();
    }
    if m.state != HumanoidState::Dead {
        m.state = if m.ground.is_some() {
            HumanoidState::Running
        } else if m.vertical > 0.0 {
            HumanoidState::Jumping
        } else {
            HumanoidState::Freefall
        };
    }
    m.velocity = if dt > 0.0 {
        (new_root - root) * (1.0 / dt)
    } else {
        Vec3::default()
    };
    new_root
}

/// Translation that moves the capsule at `root` out of every solid it overlaps.
fn push_out(queries: &rapier3d::prelude::QueryPipeline, shape: &Capsule, root: Vec3) -> Vec3 {
    let mut total = Vec3::default();
    for _ in 0..4 {
        let pose = capsule_pose(root + total);
        let aabb = shape.compute_aabb(&pose);
        let mut pushed = false;
        for (_, collider) in queries.intersect_aabb_conservative(aabb) {
            let pos12 = pose.inv_mul(collider.position());
            let Ok(Some(contact)) =
                queries
                    .dispatcher
                    .contact(&pos12, shape, collider.shape(), 0.0)
            else {
                continue;
            };
            if contact.dist < -1e-4 {
                // normal1 points from the character towards the obstacle.
                let depth = f64::from(SKIN as f32 - contact.dist);
                let dir = from_vector(pose.rotation * contact.normal1);
                total = total - dir * depth;
                pushed = true;
                break;
            }
        }
        if !pushed || total.magnitude() > MAX_PUSH {
            break;
        }
    }
    // A deep overlap is resolved over several steps rather than by one long jump that could
    // carry the character through a thin wall.
    if total.magnitude() > MAX_PUSH {
        total = total.unit() * MAX_PUSH;
    }
    total
}

/// The solid part under the capsule, its world-space surface normal and contact point.
fn probe_ground(
    queries: &rapier3d::prelude::QueryPipeline,
    shape: &Capsule,
    root: Vec3,
) -> Option<(ColliderHandle, Vec3, Vec3)> {
    let (handle, hit) = queries.cast_shape(
        &capsule_pose(root),
        Vector::new(0.0, -1.0, 0.0),
        shape,
        ShapeCastOptions {
            max_time_of_impact: (SKIN + GROUND_PROBE) as f32,
            target_distance: 0.0,
            stop_at_penetration: true,
            compute_impact_geometry_on_penetration: true,
        },
    )?;
    // A penetrating cast can choose different coplanar witnesses on a broad flat face.
    // Project the capsule's lowest support point onto that contact plane instead, so an
    // arbitrary tangential witness shift cannot change the rotation lever arm each tick.
    let sole = to_vector(root - Vec3::new(0.0, CAPSULE_DROP + HALF_SEGMENT, 0.0))
        - hit.normal1 * RADIUS as f32;
    let point = sole - hit.normal1 * (sole - hit.witness1).dot(hit.normal1);
    Some((handle, from_vector(hit.normal1), from_vector(point)))
}
