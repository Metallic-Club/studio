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
//! - standing on a part adds that part's velocity ([`PhysicsWorld::velocity`]); leaving it keeps
//!   its horizontal velocity until landing;
//! - a part that moved into the character pushes it out, at most [`MAX_PUSH`] (0.5) studs per
//!   step so it can never be shoved through a thin wall in one frame.
//!
//! Touches: after moving, the controller overlaps the movement capsule and the 15 bone-attached
//! body volumes (see [`crate::body`]), each grown by 0.1 stud, with the scene, sensors included.
//! The volumes follow the pose set with [`CharacterController::set_body_pose`] (the animated
//! avatar), else the rest pose. Each pair of a character part and a scene part reports
//! [`CharacterEvent::Touched`] when contact starts and [`CharacterEvent::TouchEnded`] when it
//! stops; the capsule reports as the `HumanoidRootPart`, each volume as its body part (`Head`,
//! `UpperTorso`, `LeftHand`, ...). This is the only source of character touches: standing still
//! on lava, or lava appearing under a standing character, is reported too, and
//! [`PhysicsWorld::drain_touched`] never reports character parts. The runtime turns these events
//! into `BasePart.Touched` and `BasePart.TouchEnded` on the touched part, with the character
//! part as `hit`. When a character dies its contacts end and it reports no new touches.
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
use crate::avatar::{self, PartIds, ROOT_PART, VIOLET};
use crate::body::{
    BodyPart, BodyPose, Contact, ROOT_HIT, TOUCH_MARGIN, TouchQueryCost, TouchVolumes,
};
use crate::{GRAVITY, PhysicsWorld, from_vector, to_vector};
use metallic_core::classes::{MAX_JUMP_POWER, MAX_SLOPE_ANGLE, MAX_WALK_SPEED};
use metallic_core::{CoreError, InstanceId, Value, Vec3, World};
use rapier3d::control::{CharacterAutostep, CharacterLength, KinematicCharacterController};
use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::parry::shape::{Capsule, Shape};
use rapier3d::prelude::{ColliderHandle, Pose, QueryFilter, Vector};
use std::collections::HashMap;

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
    /// A part of the character started touching a scene part (once per contact, again only
    /// after separation). The runtime fires `part.Touched(hit)`.
    Touched {
        /// The character Model.
        character: InstanceId,
        /// The part touched (not a part of the character).
        part: InstanceId,
        /// The character part touching it: a body part, or the `HumanoidRootPart` for the
        /// movement capsule.
        hit: InstanceId,
    },
    /// A contact reported by [`CharacterEvent::Touched`] stopped (the parts separated or the
    /// character died). The runtime fires `part.TouchEnded(hit)`. Not reported for a part that
    /// was destroyed.
    TouchEnded {
        /// The character Model.
        character: InstanceId,
        /// The part no longer touched.
        part: InstanceId,
        /// The character part that touched it.
        hit: InstanceId,
    },
    /// A Checkpoint became the player's `RespawnLocation`.
    CheckpointReached {
        /// The Player.
        player: InstanceId,
        /// The Checkpoint part.
        checkpoint: InstanceId,
    },
}

/// Per-character movement state kept between frames.
#[derive(Debug, Clone, Copy)]
struct Motion {
    state: HumanoidState,
    /// Vertical velocity of the character's own motion (jumping, falling).
    vertical: f64,
    /// Horizontal velocity inherited from the part the character left, kept while airborne.
    inherited: Vec3,
    /// The part stood on after the last step.
    ground: Option<InstanceId>,
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
    /// The part stood on after the last step.
    pub ground: Option<InstanceId>,
    /// Facing in degrees around +Y (0 looks along -Z).
    pub yaw: f64,
    /// Seconds since death.
    pub dead_time: f64,
    /// Total velocity of the last step.
    pub velocity: Vec3,
    /// Velocity of the last step minus what the carrying part added.
    pub relative: Vec3,
}

/// Poses every character part found in `model` in the rest pose so the root is at `root` and the
/// figure faces `yaw` degrees around +Y (0 looks along -Z). Clients use it to place remote
/// characters' parts.
pub fn place_character(world: &mut World, model: InstanceId, root: Vec3, yaw: f64) {
    avatar::place(world, model, root, yaw);
}

/// Builds a character for `player` and makes it the player's `Character`.
///
/// The Model is named after the player and holds the character parts (a 2x2x1 `HumanoidRootPart`
/// and the 15 R15 body parts `Head`, `UpperTorso`, `LowerTorso`, `LeftUpperArm`, ..., `RightFoot`,
/// all invisible and Anchored; the body parts hold the bone-attached touch volumes, and the
/// runtime draws the skinned default avatar over them) and a `Humanoid` with default properties. The root is placed
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

/// Fills `out` with the colliders of every BasePart inside `model`; `stack` is scratch space.
fn colliders_of(
    world: &World,
    physics: &PhysicsWorld,
    model: InstanceId,
    stack: &mut Vec<InstanceId>,
    out: &mut Vec<ColliderHandle>,
) {
    out.clear();
    stack.clear();
    stack.push(model);
    while let Some(id) = stack.pop() {
        if world.is_a(id, "BasePart")
            && let Some(h) = physics.collider_of(id)
        {
            out.push(h);
        }
        stack.extend(world.children(id).iter().copied());
    }
}

/// Reports [`CharacterEvent::TouchEnded`] for every contact of a character that is being
/// forgotten (destroyed, replaced or respawned), skipping touched parts that are gone.
fn end_all(events: &mut Vec<CharacterEvent>, world: &World, character: InstanceId, body: &Body) {
    for &c in &body.volumes.touching {
        if world.is_alive(c.part)
            && let Some(hit) = hit_of(&body.ids, c)
        {
            events.push(CharacterEvent::TouchEnded {
                character,
                part: c.part,
                hit,
            });
        }
    }
}

fn capsule() -> Capsule {
    Capsule::new_y(HALF_SEGMENT as f32, RADIUS as f32)
}

fn capsule_pose(root: Vec3) -> Pose {
    Pose::from_translation(to_vector(root - Vec3::new(0.0, CAPSULE_DROP, 0.0)))
}

/// A character's part ids and touch volumes.
#[derive(Default)]
struct Body {
    ids: PartIds,
    volumes: TouchVolumes,
}

/// Moves Humanoid characters; keeps their per-character state between frames.
#[derive(Default)]
pub struct CharacterController {
    motions: HashMap<InstanceId, Motion>,
    /// Part ids, body pose and contacts of each character.
    bodies: HashMap<InstanceId, Body>,
    events: Vec<CharacterEvent>,
    /// Scene parts that started touching the character in the current step (reused).
    started: Vec<InstanceId>,
    /// Contacts that ended in the current step (reused).
    ended: Vec<Contact>,
    /// The character's own colliders (reused).
    own: Vec<ColliderHandle>,
    /// Scratch stack for walking the character Model (reused).
    walk: Vec<InstanceId>,
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

    /// The character's facing after its last step, degrees around +Y (0 looks along -Z).
    pub fn yaw(&self, character: InstanceId) -> Option<f64> {
        self.motions.get(&character).map(|m| m.yaw)
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

    /// Sets the pose the character's touch volumes follow from now on: each body part's frame
    /// relative to the `HumanoidRootPart` centre (see [`BodyPose`]). The runtime calls it every
    /// frame with the animated avatar's bones; the next [`CharacterController::step`] uses it.
    /// A pose with a non-finite value is ignored. Until a pose is set the volumes use
    /// [`BodyPose::rest`] at the character's facing.
    pub fn set_body_pose(&mut self, character: InstanceId, pose: &BodyPose) {
        if pose.is_finite() {
            self.bodies.entry(character).or_default().volumes.animated = Some(*pose);
        }
    }

    /// Goes back to the rest pose for the character's touch volumes.
    pub fn clear_body_pose(&mut self, character: InstanceId) {
        if let Some(b) = self.bodies.get_mut(&character) {
            b.volumes.animated = None;
        }
    }

    /// The pose the character's touch volumes use: the one set with
    /// [`CharacterController::set_body_pose`], else the rest pose at its facing.
    pub fn body_pose(&self, character: InstanceId) -> BodyPose {
        let yaw = self.motions.get(&character).map_or(0.0, |m| m.yaw);
        self.bodies
            .get(&character)
            .map_or_else(|| BodyPose::rest(yaw), |b| b.volumes.pose(yaw))
    }

    /// What touches `part` after the character's last step, in [`BodyPart::ALL`] order with the
    /// movement capsule (`None`, reported as the `HumanoidRootPart`) last.
    pub fn touching(&self, character: InstanceId, part: InstanceId) -> Vec<Option<BodyPart>> {
        self.bodies.get(&character).map_or_else(Vec::new, |b| {
            b.volumes
                .touching
                .iter()
                .filter(|c| c.part == part)
                .map(|c| BodyPart::ALL.get(usize::from(c.hit)).copied())
                .collect()
        })
    }

    /// What the character's last touch query cost: parts tested (at most
    /// [`crate::MAX_TOUCH_CANDIDATES`]), narrow-phase tests (at most 16 per part), parts left out
    /// at the cap, and how many queries so far reached the cap.
    pub fn touch_query_cost(&self, character: InstanceId) -> TouchQueryCost {
        self.bodies
            .get(&character)
            .map_or_else(TouchQueryCost::default, |b| TouchQueryCost {
                parts: b.volumes.tested_parts,
                tests: b.volumes.tests,
                dropped: b.volumes.dropped,
                capped_queries: b.volumes.capped_queries,
            })
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
            ground: m.ground,
            yaw: m.yaw,
            dead_time: m.dead_time,
            velocity: m.velocity,
            relative: m.relative,
        })
    }

    /// Sets the character's movement state and places the whole character at `state.position`
    /// and `state.yaw` (body parts in the current [`CharacterController::body_pose`]). The set of
    /// parts it touches is left alone, so no touch is reported twice.
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
                ground: state.ground,
                yaw: state.yaw,
                dead_time: state.dead_time,
                velocity: state.velocity,
                relative: state.relative,
            }
            .sanitised(),
        );
        if is_finite_vec(state.position) {
            let yaw = finite_or(state.yaw, 0.0);
            let body = self.bodies.entry(character).or_default();
            body.ids.refresh(world, character);
            let pose = body.volumes.pose(yaw);
            avatar::place_pose(world, &body.ids, state.position, yaw, &pose);
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
        // A character destroyed or replaced without dying still ends its contacts.
        let events = &mut self.events;
        self.bodies.retain(|&id, body| {
            let alive = world.is_alive(id);
            if !alive {
                end_all(events, world, id, body);
            }
            alive
        });
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
                if let Some(body) = self.bodies.remove(&character) {
                    end_all(&mut self.events, world, character, &body);
                }
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

        let mut own = std::mem::take(&mut self.own);
        colliders_of(world, physics, character, &mut self.walk, &mut own);
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
        let body = self.bodies.entry(character).or_default();
        body.ids.refresh(world, character);
        let pose = body.volumes.pose(m.yaw);
        avatar::place_pose(world, &body.ids, new_root, m.yaw, &pose);
        let ids = body.ids;
        let capped_before = body.volumes.capped_queries;
        let mut started = std::mem::take(&mut self.started);
        let mut ended = std::mem::take(&mut self.ended);
        started.clear();
        ended.clear();
        let events = &mut self.events;
        let on_start = |c: Contact| {
            if let Some(hit) = hit_of(&ids, c) {
                events.push(CharacterEvent::Touched {
                    character,
                    part: c.part,
                    hit,
                });
                started.push(c.part);
            }
        };
        let on_end = |c: Contact| ended.push(c);
        if alive {
            let probe = Capsule::new_y(HALF_SEGMENT as f32, (RADIUS + TOUCH_MARGIN) as f32);
            let probe_pose = capsule_pose(new_root);
            body.volumes
                .query(physics, &own, &probe, &probe_pose, &pose, new_root);
            body.volumes.commit(on_start, on_end);
        } else {
            body.volumes.clear(on_end);
        }
        if capped_before == 0 && body.volumes.capped_queries > 0 {
            eprintln!(
                "warning: character {character:?} touches more than {} parts; the farthest are \
                 ignored until others leave",
                crate::MAX_TOUCH_CANDIDATES
            );
        }
        for &c in &ended {
            if let Some(hit) = hit_of(&ids, c)
                && world.is_alive(c.part)
            {
                self.events.push(CharacterEvent::TouchEnded {
                    character,
                    part: c.part,
                    hit,
                });
            }
        }
        started.sort_unstable();
        started.dedup();
        self.reach_checkpoints(world, character, &started);
        self.started = started;
        self.ended = ended;
        self.own = own;
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

/// The character part of a contact: the root for the capsule, else the body part.
fn hit_of(ids: &PartIds, c: Contact) -> Option<InstanceId> {
    if c.hit == ROOT_HIT {
        ids.root()
    } else {
        BodyPart::ALL
            .get(usize::from(c.hit))
            .and_then(|&p| ids.body(p))
    }
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

    // 1. Parts that moved into the character push it out.
    let root = root + push_out(&queries, &shape, root);

    // 2. Velocity for this frame.
    let mut wish = Vec3::new(input.dir.x, 0.0, input.dir.z);
    if wish.magnitude() > 1.0 {
        wish = wish.unit();
    }
    let walk = wish * t.walk_speed;
    let grounded = m.state == HumanoidState::Running;
    let carry = if grounded {
        m.ground
            .and_then(|g| physics.velocity(g))
            .unwrap_or_default()
    } else {
        Vec3::default()
    };
    let jumping = grounded && input.jump;
    let dy = if grounded && !jumping {
        m.vertical = 0.0;
        carry.y * dt
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
        walk + Vec3::new(carry.x, 0.0, carry.z)
    } else {
        walk + m.inherited
    };
    let desired = Vec3::new(horizontal.x * dt, dy, horizontal.z * dt);

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
        && let Some((handle, normal)) = probe_ground(&queries, &shape, new_root)
        && normal.y >= t.max_slope.to_radians().cos()
    {
        m.ground = physics.owner_of(handle);
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

/// The solid part right under the capsule at `root` and its surface normal.
fn probe_ground(
    queries: &rapier3d::prelude::QueryPipeline,
    shape: &Capsule,
    root: Vec3,
) -> Option<(ColliderHandle, Vec3)> {
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
    Some((handle, from_vector(hit.normal1)))
}
