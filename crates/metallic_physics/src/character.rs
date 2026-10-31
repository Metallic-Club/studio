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
//! - standing follows the support's last completed pose change, rotation included: the centre
//!   of the capsule's bottom sphere moves rigidly with the support (so the rider neither drifts
//!   in nor out on a tilted spinner) and idle facing turns by that same pose change's turn
//!   around world up (so position and facing agree whatever the step lengths). Explicit input
//!   still faces the world-space movement direction. Jumping keeps the foot contact's horizontal
//!   point velocity (`linear + angular × lever arm`) until landing;
//! - a part that moved into the character pushes it out, at most [`MAX_PUSH`] (0.5) studs per
//!   step so it can never be shoved through a thin wall in one frame.
//!
//! Support carry assumes one controller step before each physics step (the runtime's fixed
//! tick order, kept by the M2 server). It is deterministic and lives in the controller: the
//! contact ([`SupportContact`]) is part of [`CharacterState`], so a replay from a saved state
//! carries exactly like the original run. The carry is applied first and swept against every
//! part but the support itself, so another part can stop it; the character is not rigidly
//! parented to the support. Then pushes apply. The capsule/avatar stay upright on tilted
//! supports, and support pitch/roll never tilt the camera. Removed/rebuilt bodies or a root
//! position changed by a script discard the old contact. Rotating support restores the
//! skin clearance under the same push cap, preventing numerical floor interference from
//! clipping the next move. Anchored support rates come from PhysicsWorld's cache, because
//! Rapier's mover velocity is reset after landing at its target pose. Teleport classification
//! belongs to PhysicsWorld; this controller does not introduce another cutoff. A rider follows
//! the support's collision pose, which is one physics step behind a pose a script set this
//! tick (#273).
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
//! Rules: `Health <= 0` (or not a number), the root below `Workspace.FallenPartsDestroyHeight`,
//! or the root leaving the world (a non-finite coordinate or one beyond [`WORLD_LIMIT`]) makes the
//! character Dead (one [`CharacterEvent::Died`], input ignored, gravity still applies); after
//! [`RESPAWN_DELAY`] seconds the old Model is destroyed and a fresh one spawns. Touching a
//! Checkpoint whose `Order` is higher than the current `RespawnLocation`'s (any Checkpoint beats a
//! non-Checkpoint or none) makes it the player's `RespawnLocation` and colours it violet.
//!
//! Sanitising (#172, #272): `World::set` already clamps Humanoid tuning to the limits in
//! [`metallic_core::classes`] and refuses non-finite numbers. The controller still reads every
//! tuning value defensively (missing or non-finite becomes the default, then clamped to the same
//! limits), resets a non-finite movement state to rest, ignores a non-finite move direction, and
//! never writes a non-finite pose.
use crate::avatar::{self, ROOT_PART, VIOLET};
use crate::{GRAVITY, PhysicsWorld, from_vector, to_pose, to_vector};
use metallic_core::classes::{MAX_JUMP_POWER, MAX_SLOPE_ANGLE, MAX_WALK_SPEED};
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
/// A character whose root is farther than this from the origin on any axis, in studs, has left
/// the world and dies (a Metallic addition next to `FallenPartsDestroyHeight`). At this distance
/// the renderer's 32-bit positions are already off by a tenth of a stud.
pub const WORLD_LIMIT: f64 = 1e6;

/// Whether a root position is inside the world: finite, within [`WORLD_LIMIT`] on every axis
/// and not below the destroy height `floor`.
fn in_world(p: Vec3, floor: f64) -> bool {
    // `abs() <= limit` is false for NaN and the infinities.
    let inside = |c: f64| c.abs() <= WORLD_LIMIT;
    inside(p.x) && inside(p.y) && inside(p.z) && p.y >= floor
}

fn finite_or(x: f64, default: f64) -> f64 {
    if x.is_finite() { x } else { default }
}

fn is_finite_vec(v: Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

fn finite_vec_or_zero(v: Vec3) -> Vec3 {
    if is_finite_vec(v) { v } else { Vec3::default() }
}

/// A tuning number as the controller uses it: missing or non-finite becomes `default`, then
/// it is clamped to `0..=max`.
fn tuning_value(v: Option<f64>, default: f64, max: f64) -> f64 {
    finite_or(v.unwrap_or(default), default).clamp(0.0, max)
}

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

/// Where a standing character met its support after its last step, so the next step can carry
/// it through the support's pose change. Part of [`CharacterState`], so a replay from a saved
/// state carries exactly like the original run. Opaque: the body handle and poses only mean
/// something to the [`PhysicsWorld`] the contact was sampled from.
///
/// The carried point is the centre of the capsule's bottom sphere, which sits straight below
/// the root because the capsule stays upright. Moving it rigidly with the support keeps the
/// sphere at the same distance from a tilted surface; the foot contact itself sits off to the
/// side of the root on a tilt, so carrying it would drift on a tilted spinner. The foot contact
/// is kept only as the lever arm for the velocity a jump inherits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SupportContact {
    part: InstanceId,
    body: RigidBodyHandle,
    /// Bottom-sphere centre in the support's local frame.
    local_anchor: Vector,
    /// Foot contact in the support's local frame.
    local_contact: Vector,
    /// The support's pose when sampled.
    pose: Pose,
    /// The root position when sampled; a different root (a script moved it) drops the contact.
    root: Vec3,
}

/// What the support did since its contact was sampled.
#[derive(Debug, Clone, Copy, Default)]
struct SupportMotion {
    /// Displacement of the carried point (and so of the root).
    step: Vec3,
    /// Current velocity of the foot contact point.
    velocity: Vec3,
    /// Turn of the support around world up over the same pose change, in radians.
    yaw: f64,
}

/// Centre of the capsule's bottom sphere for a root position.
fn bottom_sphere_centre(root: Vec3) -> Vec3 {
    root - Vec3::new(0.0, CAPSULE_DROP + HALF_SEGMENT, 0.0)
}

impl SupportContact {
    /// The part the character stands on.
    pub fn part(&self) -> InstanceId {
        self.part
    }

    fn sample(
        physics: &PhysicsWorld,
        handle: ColliderHandle,
        point: Vec3,
        root: Vec3,
    ) -> Option<Self> {
        let part = physics.owner_of(handle)?;
        let entry = physics.entries.get(&part)?;
        let pose = *physics.rapier.bodies.get(entry.body)?.position();
        Some(Self {
            part,
            body: entry.body,
            local_anchor: pose.inverse_transform_point(to_vector(bottom_sphere_centre(root))),
            local_contact: pose.inverse_transform_point(to_vector(point)),
            pose,
            root,
        })
    }

    /// The support's completed pose change since sampling, applied to the carried point; the
    /// contact's current velocity; and the pose change's turn around world up. Position and
    /// facing come from the same pose change, so they agree whatever the step lengths.
    fn motion(self, physics: &PhysicsWorld) -> SupportMotion {
        let Some(entry) = physics
            .entries
            .get(&self.part)
            .filter(|e| e.body == self.body)
        else {
            return SupportMotion::default();
        };
        let Some(body) = physics.rapier.bodies.get(entry.body) else {
            return SupportMotion::default();
        };
        let angular = physics.angular_velocity(self.part).unwrap_or_default();
        let linear = physics.velocity(self.part).unwrap_or_default();
        // Anchored movers have been reset to fixed/zero Rapier velocities by PhysicsWorld.
        // Respect its cached rates, including a zero-rate move classified as a teleport.
        if linear.magnitude() == 0.0 && angular.magnitude() == 0.0 {
            return SupportMotion::default();
        }
        let pose = body.position();
        let contact = pose.transform_point(self.local_contact);
        let velocity = if body.is_dynamic() {
            // Rapier accounts for the dynamic body's world centre of mass.
            body.velocity_at_point(contact)
        } else {
            to_vector(linear) + to_vector(angular).cross(contact - body.translation())
        };
        let step =
            pose.transform_point(self.local_anchor) - self.pose.transform_point(self.local_anchor);
        // Twist of the rotation change around +Y (swing-twist decomposition).
        let turn = pose.rotation * self.pose.rotation.inverse();
        let yaw = 2.0 * f64::from(turn.y).atan2(f64::from(turn.w));
        let yaw =
            (yaw + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
        SupportMotion {
            step: from_vector(step),
            velocity: from_vector(velocity),
            yaw,
        }
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
    ground: Option<SupportContact>,
    /// Facing in degrees around +Y (0 looks along -Z).
    yaw: f64,
    /// Seconds since death.
    dead_time: f64,
    /// Total velocity of the last step.
    velocity: Vec3,
    /// Velocity of the last step minus what the carrying part (or the part jumped from) added.
    relative: Vec3,
}

impl Motion {
    /// The same state with every non-finite number reset to rest (#172: a NaN vertical speed
    /// used to freeze the character for good).
    fn sanitised(mut self) -> Self {
        self.vertical = finite_or(self.vertical, 0.0);
        self.inherited = finite_vec_or_zero(self.inherited);
        self.yaw = finite_or(self.yaw, 0.0);
        self.dead_time = finite_or(self.dead_time, 0.0).max(0.0);
        self.velocity = finite_vec_or_zero(self.velocity);
        self.relative = finite_vec_or_zero(self.relative);
        self
    }
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
            relative: Vec3::default(),
        }
    }
}

/// Everything needed to rewind a character: its movement state plus the root position. Saved
/// with [`CharacterController::save_state`], applied with [`CharacterController::restore_state`];
/// client prediction replays inputs from a saved state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharacterState {
    /// `HumanoidRootPart` position.
    pub position: Vec3,
    /// Vertical velocity of the character's own motion (jumping, falling).
    pub vertical: f64,
    /// Horizontal velocity inherited from the part the character left, kept while airborne.
    pub inherited: Vec3,
    /// What the character is doing.
    pub state: HumanoidState,
    /// The part stood on after the last step (the part of `support`).
    pub ground: Option<InstanceId>,
    /// Where the character stood on `ground`, used to carry it through that part's next pose
    /// change. `restore_state` ignores it unless its part is `ground`.
    pub support: Option<SupportContact>,
    /// Facing in degrees around +Y (0 looks along -Z).
    pub yaw: f64,
    /// Seconds since death.
    pub dead_time: f64,
    /// Total velocity of the last step.
    pub velocity: Vec3,
    /// Velocity of the last step minus what the carrying part added.
    pub relative: Vec3,
}

/// Poses every avatar part found in `model` so the root is at `root` and the figure faces
/// `yaw` degrees around +Y (0 looks along -Z). Clients use it to place remote characters' limbs.
pub fn place_character(world: &mut World, model: InstanceId, root: Vec3, yaw: f64) {
    avatar::place(world, model, root, yaw);
}

/// Builds a character for `player` and makes it the player's `Character`.
///
/// The Model is named after the player and holds the avatar parts (a 2x2x1 `HumanoidRootPart`,
/// Head 2x1x1, Torso 2x2x1, arms and legs 1x2x1, all invisible and Anchored: they are the
/// character's collision and touch volumes, and the runtime draws the skinned default avatar
/// over them) and a `Humanoid` with default properties. The root is placed
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
/// or (0, 3, 0). A pad whose top lies outside the world ([`WORLD_LIMIT`]) is skipped and the
/// default is used, so the new character is not killed for leaving the world as soon as it
/// spawns, again and again (#345).
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
    let on_top = on_top.filter(|&p| in_world(p, f64::NEG_INFINITY));
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

    /// The character's own velocity during its last step: [`CharacterController::velocity`]
    /// minus the velocity of the part it stands on (or, in the air, the horizontal velocity kept
    /// from the part it left). Standing still on a moving platform gives zero; the animation
    /// uses this to pick idle, walk or run.
    pub fn relative_velocity(&self, character: InstanceId) -> Option<Vec3> {
        self.motions.get(&character).map(|m| m.relative)
    }

    /// The character's movement state and root position, to rewind to later. A character that
    /// was never stepped saves the state its first step would start from. `None` if it has no
    /// `HumanoidRootPart` with a position.
    pub fn save_state(&self, world: &World, character: InstanceId) -> Option<CharacterState> {
        let root = world.find_first_child(character, ROOT_PART)?;
        let position = vec_prop(world, root, "Position")?;
        let m = self.motions.get(&character).copied().unwrap_or_default();
        Some(CharacterState {
            position,
            vertical: m.vertical,
            inherited: m.inherited,
            state: m.state,
            ground: m.ground.map(|s| s.part),
            support: m.ground,
            yaw: m.yaw,
            dead_time: m.dead_time,
            velocity: m.velocity,
            relative: m.relative,
        })
    }

    /// Sets the character's movement state and places the whole avatar at `state.position` and
    /// `state.yaw`. The set of parts it touches is left alone, so no touch is reported twice.
    pub fn restore_state(
        &mut self,
        world: &mut World,
        character: InstanceId,
        state: &CharacterState,
    ) {
        self.motions.insert(
            character,
            Motion {
                state: state.state,
                vertical: state.vertical,
                inherited: state.inherited,
                ground: state.support.filter(|s| Some(s.part) == state.ground),
                yaw: state.yaw,
                dead_time: state.dead_time,
                velocity: state.velocity,
                relative: state.relative,
            }
            .sanitised(),
        );
        if is_finite_vec(state.position) {
            avatar::place(world, character, state.position, finite_or(state.yaw, 0.0));
        }
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
        let mut m = self
            .motions
            .get(&character)
            .copied()
            .unwrap_or_default()
            .sanitised();
        let dt = dt.max(0.0);
        let floor = world
            .root("Workspace")
            .and_then(|w| num_prop(world, w, "FallenPartsDestroyHeight"))
            .unwrap_or(f64::NEG_INFINITY);

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
            // `World::set` refuses NaN, so a NaN Health cannot be written today; the is_nan
            // check stays as a guard in case a future path stores one (#172, #345).
            if health.is_nan() || health <= 0.0 || !in_world(root_pos, floor) {
                self.die(&mut m, character, humanoid);
            }
        }
        let alive = m.state != HumanoidState::Dead;
        let mut input = if alive { input } else { MoveInput::default() };
        input.dir = finite_vec_or_zero(input.dir);
        let hum = |p: &str, default: f64, max: f64| {
            tuning_value(num_prop(world, humanoid, p), default, max)
        };
        let tuning = Tuning {
            walk_speed: hum("WalkSpeed", 16.0, MAX_WALK_SPEED)
                * if input.run { RUN_MULTIPLIER } else { 1.0 },
            jump_power: hum("JumpPower", 50.0, MAX_JUMP_POWER),
            max_slope: hum("MaxSlopeAngle", 89.0, MAX_SLOPE_ANGLE),
        };

        let own: Vec<ColliderHandle> = parts_of(world, character)
            .into_iter()
            .filter_map(|p| physics.collider_of(p))
            .collect();
        // A root far outside the world (huge or non-finite) is not swept through the scene:
        // the character is already dead and waits to respawn where it is.
        let lost = !in_world(root_pos, f64::NEG_INFINITY);
        let mut new_root = if lost {
            root_pos
        } else {
            move_capsule(physics, &own, &mut m, root_pos, input, &tuning, dt)
        };
        m = m.sanitised();
        if !is_finite_vec(new_root) {
            // Never write a non-finite pose: the world would refuse it and the character would
            // hang where it was.
            new_root = root_pos;
        }
        if alive && !in_world(new_root, floor) {
            self.die(&mut m, character, humanoid);
        }
        let alive = m.state != HumanoidState::Dead;

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

    /// Makes the character Dead and reports [`CharacterEvent::Died`] (once per life).
    fn die(&mut self, m: &mut Motion, character: InstanceId, humanoid: InstanceId) {
        *m = Motion {
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

    // 1. Carry: the support's completed pose change moves the bottom-sphere centre, and so the
    // root, rigidly with it. The sweep skips the support itself (the character moves with it)
    // but another part can still stop the carry.
    let sampled_root = root;
    let grounded = m.state == HumanoidState::Running;
    let jumping = grounded && input.jump;
    let support = m.ground.filter(|s| grounded && s.root == sampled_root);
    let motion = match support {
        Some(support) if dt > 0.0 => support.motion(physics),
        _ => SupportMotion::default(),
    };
    let mut root = root;
    if !jumping
        && let Some(support) = support
        && motion.step.magnitude() > 1e-9
    {
        let skip_support = |h: ColliderHandle, _: &rapier3d::prelude::Collider| {
            !own.contains(&h) && physics.owner_of(h) != Some(support.part)
        };
        let carry_queries = physics.queries(
            QueryFilter::default()
                .exclude_sensors()
                .predicate(&skip_support),
        );
        let kcc = KinematicCharacterController {
            offset: CharacterLength::Absolute(SKIN as f32),
            autostep: None,
            snap_to_ground: None,
            ..KinematicCharacterController::default()
        };
        let moved = kcc.move_shape(
            dt as f32,
            &carry_queries,
            &shape,
            &capsule_pose(root),
            to_vector(motion.step),
            |_| {},
        );
        root = root + from_vector(moved.translation);
    }

    // 2. Parts that moved into the character push it out. A rotating support also restores
    // its skin gap; otherwise tiny numerical drift lets a floor clip the next move.
    let push = push_out(&queries, &shape, root);
    let mut pushed = push;
    if dt > 0.0
        && let Some(support) = support
        && physics
            .angular_velocity(support.part)
            .is_some_and(|angular| angular.magnitude() > 0.0)
        && let Some((handle, normal, point)) = probe_ground(&queries, &shape, root + push)
        && physics.owner_of(handle) == Some(support.part)
        && normal.y >= t.max_slope.to_radians().cos()
    {
        let sole = to_vector(bottom_sphere_centre(root + push)) - to_vector(normal) * RADIUS as f32;
        let gap = f64::from((sole - to_vector(point)).dot(to_vector(normal)));
        if gap >= -1e-4 {
            let correction = (SKIN - gap).clamp(0.0, (MAX_PUSH - push.magnitude()).max(0.0));
            pushed = pushed + normal * correction;
        }
    }
    let root = root + pushed;

    // Velocity for this frame.
    let mut wish = Vec3::new(input.dir.x, 0.0, input.dir.z);
    if wish.magnitude() > 1.0 {
        wish = wish.unit();
    }
    let walk = wish * t.walk_speed;
    let carry = motion.velocity;
    let dy = if grounded && !jumping {
        m.vertical = 0.0;
        m.yaw += motion.yaw.to_degrees();
        0.0
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
    let carried = if grounded && !jumping {
        carry
    } else {
        m.inherited
    };
    let horizontal = if grounded && !jumping {
        walk * dt
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
        m.ground = SupportContact::sample(physics, handle, point, new_root);
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
        // Pushes are not the character's or the support's motion.
        (new_root - sampled_root - pushed) * (1.0 / dt)
    } else {
        Vec3::default()
    };
    m.relative = m.velocity - carried;
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
