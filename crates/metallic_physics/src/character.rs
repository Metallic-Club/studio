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
//! belongs to PhysicsWorld; this controller does not introduce another cutoff.
//!
//! Script-moved supports (#374, #273): scripts run after physics, so an anchored support's body
//! is one move behind the pose a script already set. All collision work uses the body pose; at
//! the end of the step the rider gets a lead, the rigid step from the body pose to the scripted
//! pose ([`PhysicsWorld::scripted_pose`]), swept like the carry. The next physics step moves the
//! body there, so the rider stays one tick behind the script instead of two, in the same
//! characters-then-physics order.
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
//! Characters this controller steps touch each other volume against volume, each contact
//! reported once with the other character's part as `part` (see [`crate::limbs`]); characters
//! it does not step are ordinary scene parts. A character first stepped this tick is still a
//! scene part to the others for that one step.
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
use crate::limbs::CharacterPairs;
use crate::{GRAVITY, PhysicsWorld, from_vector, to_vector};
use metallic_core::classes::{MAX_JUMP_POWER, MAX_SLOPE_ANGLE, MAX_WALK_SPEED};
use metallic_core::{CoreError, InstanceId, Value, Vec3, World};
use rapier3d::control::{CharacterAutostep, CharacterLength, KinematicCharacterController};
use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::parry::shape::{Capsule, Shape};
use rapier3d::prelude::{ColliderHandle, Pose, QueryFilter, RigidBodyHandle, Vector};
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
    /// The support pose the rider was left aligned with: the pose a script had already given
    /// an anchored support (see [`PhysicsWorld::scripted_pose`]), else the body pose.
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

/// Turn around world up of the rotation from `from` to `to`, in radians (swing-twist
/// decomposition), in `-pi..pi`.
fn twist_y(to: Pose, from: Pose) -> f64 {
    let turn = to.rotation * from.rotation.inverse();
    let yaw = 2.0 * f64::from(turn.y).atan2(f64::from(turn.w));
    (yaw + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
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

    /// The contact under the root and the support's pending move: the step from its body pose
    /// to the pose a script already gave it (zero for a simulated or still part), which the
    /// caller applies as a lead, and that move's turn around world up (radians).
    fn pending(
        physics: &PhysicsWorld,
        world: &World,
        handle: ColliderHandle,
        root: Vec3,
    ) -> Option<(InstanceId, Vec3, f64)> {
        let part = physics.owner_of(handle)?;
        let entry = physics.entries.get(&part)?;
        let body = *physics.rapier.bodies.get(entry.body)?.position();
        let scripted = physics.scripted_body_pose(world, part)?;
        if scripted == body {
            return Some((part, Vec3::default(), 0.0));
        }
        let anchor = to_vector(bottom_sphere_centre(root));
        let step = scripted.transform_point(body.inverse_transform_point(anchor)) - anchor;
        Some((part, from_vector(step), twist_y(scripted, body)))
    }

    /// Records the contact after the lead: `point` is the foot contact found against the body
    /// pose, `root` the final root, aligned with the scripted pose.
    fn sample(
        physics: &PhysicsWorld,
        world: &World,
        part: InstanceId,
        point: Vec3,
        root: Vec3,
    ) -> Option<Self> {
        let entry = physics.entries.get(&part)?;
        let body = *physics.rapier.bodies.get(entry.body)?.position();
        let pose = physics.scripted_body_pose(world, part)?;
        Some(Self {
            part,
            body: entry.body,
            local_anchor: pose.inverse_transform_point(to_vector(bottom_sphere_centre(root))),
            local_contact: body.inverse_transform_point(to_vector(point)),
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
        // A resting simulated body carries nothing. An anchored support always uses the exact
        // pose difference: after the lead it is zero unless the body did not reach the pose the
        // rider was aligned with, and then the rider must follow the body.
        if body.is_dynamic() && linear.magnitude() == 0.0 && angular.magnitude() == 0.0 {
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
        let yaw = twist_y(*pose, self.pose);
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
    /// Contacts between the stepped characters' volumes (see [`crate::limbs`]).
    pairs: CharacterPairs,
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
    /// minus the support's motion at the carried point, the capsule's bottom-sphere centre (or,
    /// in the air, minus the horizontal velocity kept from the part it left). Standing still on
    /// a moving or spinning platform, tilted too, gives zero; the animation uses this to pick
    /// idle, walk or run.
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
                ..TouchQueryCost::default()
            })
            .with_characters(self.pairs.cost.get(&character).copied().unwrap_or_default())
    }

    /// The parts of other characters touching `character`'s parts, as `(mine, theirs)`.
    pub fn touching_characters(&self, character: InstanceId) -> Vec<(InstanceId, InstanceId)> {
        self.pairs.touching(character)
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
                ground: state.support.filter(|s| Some(s.part) == state.ground),
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
        let pairs = &mut self.pairs;
        self.bodies.retain(|&id, body| {
            let alive = world.is_alive(id);
            if !alive {
                end_all(events, world, id, body);
                pairs.end(id, events);
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
                self.pairs.end(character, &mut self.events);
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
            move_capsule(physics, world, &own, &mut m, root_pos, input, &tuning, dt)
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
            // Other stepped characters meet this one volume against volume (see
            // `crate::limbs`), not as scene parts.
            let motions = &self.motions;
            let world_ref: &World = world;
            let skip = |h: ColliderHandle| {
                own.contains(&h)
                    || physics
                        .owner_of(h)
                        .and_then(|p| world_ref.parent(p))
                        .is_some_and(|m| m != character && motions.contains_key(&m))
            };
            body.volumes
                .query(physics, &skip, &probe, &probe_pose, &pose, new_root);
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
        if alive
            && let Some(me) = self.bodies.get(&character)
            && let Some(mine) = me.volumes.world.as_ref()
        {
            let motions = &self.motions;
            let world_ref: &World = world;
            let others = self
                .bodies
                .iter()
                .filter(move |&(&id, _)| {
                    id != character
                        && world_ref.is_alive(id)
                        && motions
                            .get(&id)
                            .is_some_and(|m| m.state != HumanoidState::Dead)
                })
                .filter_map(|(&id, b)| b.volumes.world.as_ref().map(|w| (id, w, &b.ids)));
            self.pairs
                .update(character, (mine, &me.ids), others, &mut self.events);
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
        self.pairs.end(character, &mut self.events);
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
#[allow(clippy::too_many_arguments)]
fn move_capsule(
    physics: &PhysicsWorld,
    world: &World,
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
    let mut support_moved = Vec3::default();
    if !jumping && let Some(support) = support {
        let moved = carry_sweep(physics, own, support.part, root, motion.step, dt);
        root = root + moved;
        support_moved = moved;
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
    // Airborne velocity kept from the part left; the support's own carry is `support_moved`.
    let carried = if grounded && !jumping {
        Vec3::default()
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
    let mut new_root = root + step;

    // 4. Head bump: rising but blocked from above.
    if m.vertical > 0.0 && desired.y > 1e-6 && step.y < desired.y * 0.5 {
        m.vertical = 0.0;
    }

    // 5. Ground under the feet.
    m.ground = None;
    if m.vertical <= 0.0
        && let Some((handle, normal, point)) = probe_ground(&queries, &shape, new_root)
        && normal.y >= t.max_slope.to_radians().cos()
        && let Some((part, lead, turn)) = SupportContact::pending(physics, world, handle, new_root)
    {
        // 6. Lead: collisions above used the support's body pose, but a script may already
        // have moved an anchored support; the next physics step applies that pose. Moving the
        // rider with it now keeps it one tick behind the script instead of two (#374, #273).
        let moved = carry_sweep(physics, own, part, new_root, lead, dt);
        new_root = new_root + moved;
        support_moved = support_moved + moved;
        m.yaw += turn.to_degrees();
        m.ground = SupportContact::sample(physics, world, part, point, new_root);
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
    if dt > 0.0 {
        // Pushes are not the character's or the support's motion.
        m.velocity = (new_root - sampled_root - pushed) * (1.0 / dt);
        // Relative to the support's motion at the carried point (the bottom-sphere centre), so
        // a rider standing still on a tilted spinner reads as still (#374).
        m.relative = m.velocity - carried - support_moved * (1.0 / dt);
    } else {
        m.velocity = Vec3::default();
        m.relative = Vec3::default();
    }
    new_root
}

/// Moves the capsule at `root` by `step`, the motion of the support `part` it stands on. The
/// sweep skips that part (the character moves with it) but another part can still stop it.
/// Returns the translation actually made.
fn carry_sweep(
    physics: &PhysicsWorld,
    own: &[ColliderHandle],
    part: InstanceId,
    root: Vec3,
    step: Vec3,
    dt: f64,
) -> Vec3 {
    if step.magnitude() <= 1e-9 {
        return Vec3::default();
    }
    let skip_support = |h: ColliderHandle, _: &rapier3d::prelude::Collider| {
        !own.contains(&h) && physics.owner_of(h) != Some(part)
    };
    let queries = physics.queries(
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
        &queries,
        &capsule(),
        &capsule_pose(root),
        to_vector(step),
        |_| {},
    );
    from_vector(moved.translation)
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
