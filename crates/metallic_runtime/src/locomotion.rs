//! The default avatar's locomotion controller: a [`metallic_anim::StateMachine`] over the
//! avatar's clips, driven by the character's speed and ground state.
//!
//! States and transitions (blend times in seconds, crossfaded by `metallic_anim`):
//! - **Ground**: a 1D blend on `Speed` of idle, `walk_slow`, `walk` and `run` placed at 0, 6, 16
//!   and 24 studs/s ([`GAITS`]). The clips share one normalised phase (left heel strike at 0,
//!   right at 0.5, the same heel-to-flat roll-in) and each is authored for its own speed. A walk
//!   keeps a foot down longer than a run, so in a mix each gait's legs are sampled at a phase
//!   warped to line up toe-off as well (sync markers for strike and lift, [`warp_phase`]; the
//!   stance share is measured from each clip at load). The playback rate is set each frame so the
//!   blended stance covers the distance the character moves ([`gait_rate`], 0.5 to 2): no foot
//!   sliding, also between gaits.
//! - **Jump**: on take-off (the `Jump` trigger, [`FADE_TAKEOFF`]). The clip starts at its launch
//!   frame ([`JUMP_LAUNCH`]): the physics jumps the moment the button is pressed, so the
//!   anticipation crouch before it is skipped.
//! - **Fall**: at the top of the jump (the physics stops rising, [`FADE_TO_FALL`]), when the jump
//!   clip has played out, or after [`FALL_DELAY`] in the air without a jump (walking off a ledge;
//!   short drops down steps stay on the ground pose).
//! - **Land** / **LandHard**: on touchdown ([`FADE_LAND`], short so the impact reads). A landing
//!   faster than [`HARD_LANDING`] studs/s plays the deep `land_hard` squash (when the asset has
//!   it). Back to Ground ([`FADE_LAND_OUT`]) when the clip ends, or sooner when the character is
//!   already moving again.
//!
//! On the ground the upper body also leans into turns (centripetal acceleration) and into speed
//! changes, smoothed ([`LEAN_TIME`]) and capped: a cheap layer on the `spine` joint after the
//! blend.
//!
//! A dead character's pose is held. Walking off a ledge counts any time in the air (not only
//! falling) toward [`FALL_DELAY`]; a take-off always enters Jump first.
//!
//! Climb and swim clips exist in the asset, but the character controller has no climbing or
//! swimming state yet (`HumanoidState` is Running, Jumping, Freefall or Dead), so the machine has
//! no such states. The jump has no take-off window either: that needs the physics impulse
//! delayed, which is not an animation matter.
//!
//! The state machine only picks clips, times and weights. The clips themselves stay the asset's
//! own ([`metallic_asset`]); the machine is built over name-and-length stand-ins
//! for them and its [`MotionSample`]s are sampled from the asset clips and blended here.
//! [`Locomotion::update`] does not allocate.
use glam::Quat;
use metallic_anim::{
    Clip, Condition, LoopMode, Motion, MotionSample, Parameter, ParameterValue, State,
    StateMachine, StateMachinePlayer, Transition,
};
use metallic_asset::{Pose, SkinnedModel};
use std::sync::Arc;

/// Gait clips and the speed (studs/s) each is authored for, slowest first. Idle sits at 0.
pub(crate) const GAITS: [(&str, f32); 3] = [("walk_slow", 6.0), ("walk", 16.0), ("run", 24.0)];
/// Seconds into the jump clip where the feet leave the ground (frame 6 at 30 fps).
pub(crate) const JUMP_LAUNCH: f32 = 6.0 / 30.0;
/// Time in the air without a jump before the fall pose starts.
pub(crate) const FALL_DELAY: f32 = 0.12;
/// Time the land pose holds before blending out when the character is moving.
pub(crate) const LAND_HOLD_MOVING: f32 = 0.1;
/// The same for a hard landing (the squash reads, then the character runs on).
pub(crate) const LAND_HARD_HOLD_MOVING: f32 = 0.2;
/// Downward speed (studs/s) at touchdown above which the landing is hard. A jump on flat
/// ground lands at the jump speed, 50 studs/s; 75 is a drop of about 14 studs.
pub(crate) const HARD_LANDING: f32 = 75.0;
/// Speed (studs/s) above which a landing counts as moving.
const MOVING: f32 = 2.0;
/// Speeds below this are standing still (no shuffle from tiny pushes).
const STILL: f32 = 0.05;

/// Crossfade lengths in seconds.
pub(crate) const FADE_TAKEOFF: f32 = 0.08;
pub(crate) const FADE_TO_FALL: f32 = 0.22;
pub(crate) const FADE_LAND: f32 = 0.05;
pub(crate) const FADE_LAND_OUT: f32 = 0.2;

/// Limits of the playback rate that keeps the gait in step with the speed.
const RATE_RANGE: (f32, f32) = (0.5, 2.0);

/// Studs/s² of gravity, for the lean angles.
const GRAVITY: f32 = 196.2;
/// Share of the physical lean angle (atan(a / g)) shown, for turns and for speed changes.
const LEAN_TURN: f32 = 0.7;
const LEAN_ACCEL: f32 = 0.5;
/// Largest lean in radians: sideways into a turn, forward or back on a speed change.
const LEAN_MAX_ROLL: f32 = 0.21;
const LEAN_MAX_PITCH: f32 = 0.12;
/// Time constant of the lean (seconds).
pub(crate) const LEAN_TIME: f32 = 0.12;

/// Whether the character stands on something, from the character controller's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Support {
    /// Standing or walking on a surface.
    Grounded,
    /// In the air and rising after a jump.
    Rising,
    /// In the air otherwise.
    Falling,
    /// Dead: hold the pose.
    Frozen,
}

/// What the controller reads each frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LocomotionInput {
    /// Horizontal speed of the character's own motion in studs/s (not counting a platform that
    /// carries it).
    pub speed: f32,
    /// Vertical velocity in studs/s (negative falling), for the landing strength.
    pub vertical: f32,
    /// How fast the shown facing turns, radians/s (positive = turning left), for the lean.
    pub turn_rate: f32,
    /// Ground state.
    pub support: Support,
}

impl LocomotionInput {
    /// Input with no vertical speed and no turning.
    pub fn new(speed: f32, support: Support) -> LocomotionInput {
        LocomotionInput {
            speed,
            vertical: 0.0,
            turn_rate: 0.0,
            support,
        }
    }
}

/// State names of [`machine`].
pub(crate) mod state {
    pub const GROUND: &str = "Ground";
    pub const JUMP: &str = "Jump";
    pub const FALL: &str = "Fall";
    pub const LAND: &str = "Land";
    pub const LAND_HARD: &str = "LandHard";
}

/// One ground clip, for [`gait_rate`] and the stance warp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GroundClip {
    /// Authored speed, studs/s (0 for idle).
    pub speed: f32,
    /// Clip length, seconds.
    pub length: f32,
    /// Share of the cycle a foot is on the ground (heel strike at 0 to toe-off).
    pub stance: f32,
}

/// Playback rate that keeps the feet planted at `speed`. `ground` lists the ground clips, slowest
/// first, idle first of all. Between two gaits each clip's stance is warped to the blended stance
/// ([`warp_phase`]), so the feet of every gait in the mix touch down and lift off together. The
/// planted foot of a gait covers `speed * length * stance` per step, and the rate makes the
/// blended stance cover the distance the character moves meanwhile. Between idle and the slowest
/// gait the stride is that gait's own.
fn gait_rate(ground: &[GroundClip], speed: f32) -> f32 {
    if speed <= STILL || ground.len() < 2 {
        return 1.0;
    }
    let i = ground
        .iter()
        .skip(1)
        .position(|g| speed < g.speed)
        .map_or(ground.len() - 1, |p| p + 1);
    let (a, b) = (ground[i.saturating_sub(1)], ground[i]);
    let w = if b.speed > a.speed {
        ((speed - a.speed) / (b.speed - a.speed)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let length = a.length + (b.length - a.length) * w;
    let covered = |g: GroundClip| g.speed * g.length * g.stance;
    let (stance, stride) = if i == 1 {
        (b.stance, covered(b))
    } else {
        (
            a.stance + (b.stance - a.stance) * w,
            covered(a) + (covered(b) - covered(a)) * w,
        )
    };
    if stride > 1e-4 {
        (speed * length * stance / stride).clamp(RATE_RANGE.0, RATE_RANGE.1)
    } else {
        1.0
    }
}

/// Maps a cycle phase of the mix (heel strike at 0) to the phase of a gait whose foot is on the
/// ground for `stance` of its cycle, when the mix's stance is `target`: strike and toe-off line
/// up, the stance and the swing each scaled linearly.
fn warp_phase(phase: f32, stance: f32, target: f32) -> f32 {
    let p = phase.rem_euclid(1.0);
    if (stance - target).abs() < 1e-5 || target <= 0.0 || target >= 1.0 {
        return p;
    }
    if p < target {
        p * stance / target
    } else {
        stance + (p - target) * (1.0 - stance) / (1.0 - target)
    }
}

/// Share of the cycle the left foot of a looping gait clip is on the ground, measured from the
/// clip: the ball of the foot (`leftToes`) moves backward with the ground from the heel strike at
/// phase 0 until toe-off, and forward through the swing. `None` when the clip does not show that.
fn measure_stance(model: &SkinnedModel, clip: usize) -> Option<f32> {
    const N: usize = 240;
    let sk = &model.skeleton;
    let toe = sk.names.iter().position(|n| n == "leftToes")?;
    let mut chain = vec![toe];
    while let Some(p) = sk.parents[*chain.last()?] {
        chain.push(p);
    }
    let c = &model.clips[clip];
    let mut pose = Pose::rest(sk);
    let z: Vec<f32> = (0..=N)
        .map(|i| {
            pose.locals.clone_from(&sk.rest);
            c.sample(c.duration * i as f32 / N as f32, &mut pose);
            let mut g = sk.base;
            for &k in chain.iter().rev() {
                g *= pose.locals[k].to_mat4();
            }
            // The model faces +Z.
            g.w_axis.z
        })
        .collect();
    // Toe-off: the first time after the strike that the ball moves forward for a while.
    let i = (N / 20..N - 4).find(|&i| (0..4).all(|k| z[i + k + 1] > z[i + k]))?;
    let stance = i as f32 / N as f32;
    (0.1..0.9).contains(&stance).then_some(stance)
}

/// The locomotion state machine. `land_length` and `land_hard_length` are the land clips'
/// lengths in seconds (a moving landing leaves after its hold time).
fn machine(land_length: f32, land_hard_length: f32) -> StateMachine {
    let clip = |name: &str, clip: &str| State {
        name: name.into(),
        motion: Motion::Clip(clip.into()),
        speed: 1.0,
    };
    let mut points = vec![(0.0, "idle".to_owned())];
    points.extend(GAITS.map(|(name, speed)| (speed, name.to_owned())));
    let tr = |from: Option<&str>, to: &str, conditions: Vec<Condition>, blend_time, exit_time| {
        Transition {
            from: from.map(Into::into),
            to: to.into(),
            conditions,
            blend_time,
            exit_time,
        }
    };
    let grounded = || Condition::Is("Grounded".into(), true);
    let in_air = || Condition::Greater("AirTime".into(), FALL_DELAY);
    let hard = || Condition::Greater("Impact".into(), HARD_LANDING);
    let moving = || Condition::Greater("Speed".into(), MOVING);
    let param = |name: &str, default| Parameter {
        name: name.into(),
        default,
    };
    let mut transitions = vec![tr(
        None,
        state::JUMP,
        vec![Condition::Triggered("Jump".into())],
        FADE_TAKEOFF,
        None,
    )];
    for from in [state::JUMP, state::FALL] {
        transitions.push(tr(
            Some(from),
            state::LAND_HARD,
            vec![grounded(), hard()],
            FADE_LAND,
            None,
        ));
        transitions.push(tr(
            Some(from),
            state::LAND,
            vec![grounded()],
            FADE_LAND,
            None,
        ));
    }
    transitions.extend([
        // The top of the jump: the physics stops rising.
        tr(
            Some(state::JUMP),
            state::FALL,
            vec![Condition::Is("Rising".into(), false)],
            FADE_TO_FALL,
            None,
        ),
        tr(
            Some(state::JUMP),
            state::FALL,
            vec![],
            FADE_TO_FALL,
            Some(1.0),
        ),
        tr(
            Some(state::GROUND),
            state::FALL,
            vec![in_air()],
            FADE_TO_FALL,
            None,
        ),
    ]);
    for (land, length, hold) in [
        (state::LAND, land_length, LAND_HOLD_MOVING),
        (state::LAND_HARD, land_hard_length, LAND_HARD_HOLD_MOVING),
    ] {
        transitions.extend([
            tr(Some(land), state::FALL, vec![in_air()], FADE_TO_FALL, None),
            tr(
                Some(land),
                state::GROUND,
                vec![grounded()],
                FADE_LAND_OUT,
                Some(1.0),
            ),
            tr(
                Some(land),
                state::GROUND,
                vec![grounded(), moving()],
                FADE_LAND_OUT,
                Some(hold / length.max(1e-3)),
            ),
        ]);
    }
    StateMachine {
        parameters: vec![
            param("Speed", ParameterValue::Float(0.0)),
            param("AirTime", ParameterValue::Float(0.0)),
            param("Impact", ParameterValue::Float(0.0)),
            param("Grounded", ParameterValue::Bool(true)),
            param("Rising", ParameterValue::Bool(false)),
            param("Jump", ParameterValue::Trigger),
        ],
        states: vec![
            State {
                name: state::GROUND.into(),
                motion: Motion::Blend1D {
                    parameter: "Speed".into(),
                    points,
                },
                speed: 1.0,
            },
            clip(state::JUMP, "jump"),
            clip(state::FALL, "fall"),
            clip(state::LAND, "land"),
            clip(state::LAND_HARD, "land_hard"),
        ],
        entry: state::GROUND.into(),
        transitions,
    }
}

/// Picks, times and blends the locomotion clips of one character. Without a player the asset
/// lacks a clip the machine needs, and the rest pose stands in.
pub(crate) struct Locomotion {
    player: Option<StateMachinePlayer>,
    /// Per player clip: index into `SkinnedModel::clips`, seconds to add to the player's clip
    /// time (the jump clip starts at its launch frame), and a gait's stance share.
    sources: Vec<(usize, f32, Option<f32>)>,
    /// Idle and the gaits, for [`gait_rate`].
    ground: Vec<GroundClip>,
    /// Joints of the left and the right leg (upper leg to toes), for the stance warp.
    legs: [Vec<usize>; 2],
    /// Reused each frame: the machine's clip mix.
    samples: Vec<MotionSample>,
    scratch: Pose,
    leg_scratch: Pose,
    air_time: f32,
    /// Fastest downward speed in the current time in the air (studs/s).
    peak_fall: f32,
    last_support: Support,
    /// The `spine` joint and its ancestors (root first), for the lean.
    spine: Option<(usize, Vec<usize>)>,
    /// Current lean (radians): sideways (positive = toward the character's left) and forward.
    lean: (f32, f32),
    /// Smoothed speed and its rate of change, for the acceleration lean.
    smooth_speed: f32,
}

impl Locomotion {
    /// A controller for `model`, standing idle.
    pub fn new(model: &SkinnedModel) -> Locomotion {
        let mut sources = Vec::new();
        let mut clips = Vec::new();
        let mut lengths = (0.0, 0.0);
        let mut ground = Vec::new();
        let named = [
            ("idle", "idle", true),
            ("jump", "jump", false),
            ("fall", "fall", true),
            ("land", "land", false),
            ("land_hard", "land_hard", false),
        ];
        let gaits = GAITS.map(|(n, _)| (n, n, true));
        for (name, asset_name, looped) in named.into_iter().chain(gaits) {
            // An asset without a hard landing plays the soft one for both.
            let find = |n: &str| model.clips.iter().position(|c| c.name == n);
            let Some(i) =
                find(asset_name).or_else(|| (name == "land_hard").then(|| find("land")).flatten())
            else {
                eprintln!("warning: the avatar has no `{name}` clip; it stays in its rest pose");
                return Locomotion::without_player(model);
            };
            let (offset, duration) = match name {
                "jump" => (
                    JUMP_LAUNCH,
                    (model.clips[i].duration - JUMP_LAUNCH).max(0.0),
                ),
                _ => (0.0, model.clips[i].duration),
            };
            let gait = GAITS.iter().find(|(n, _)| *n == name).map(|&(_, v)| v);
            let stance = gait.map(|_| measure_stance(model, i).unwrap_or(0.0));
            if name == "idle" || gait.is_some() {
                ground.push(GroundClip {
                    speed: gait.unwrap_or(0.0),
                    length: duration,
                    stance: stance.unwrap_or(0.0),
                });
            }
            match name {
                "land" => lengths.0 = duration,
                "land_hard" => lengths.1 = duration,
                _ => {}
            }
            let mut clip = Clip::new(name, duration);
            clip.loop_mode = if looped {
                LoopMode::Loop
            } else {
                LoopMode::Once
            };
            clips.push(Arc::new(clip));
            sources.push((i, offset, stance));
        }
        // Without a measured stance for every gait, no warp: all gaits share one stance.
        if ground.iter().skip(1).any(|g| g.stance <= 0.0) {
            eprintln!("warning: could not measure the gait clips' stance; blends may slide");
            for g in ground.iter_mut().skip(1) {
                g.stance = 0.5;
            }
            for st in sources.iter_mut().filter_map(|s| s.2.as_mut()) {
                *st = 0.5;
            }
        }
        let player = StateMachinePlayer::new(&machine(lengths.0, lengths.1), clips)
            .map_err(|e| eprintln!("warning: the avatar's animation machine is invalid ({e})"))
            .ok();
        let names = &model.skeleton.names;
        let spine = names.iter().position(|n| n == "spine").map(|j| {
            let mut chain = Vec::new();
            let mut p = model.skeleton.parents[j];
            while let Some(k) = p {
                chain.push(k);
                p = model.skeleton.parents[k];
            }
            chain.reverse();
            (j, chain)
        });
        let leg = |side: &str| {
            ["UpperLeg", "LowerLeg", "Foot", "Toes"]
                .iter()
                .filter_map(|b| names.iter().position(|n| *n == format!("{side}{b}")))
                .collect::<Vec<_>>()
        };
        Locomotion {
            player,
            sources,
            ground,
            legs: [leg("left"), leg("right")],
            spine,
            ..Locomotion::without_player(model)
        }
    }

    fn without_player(model: &SkinnedModel) -> Locomotion {
        Locomotion {
            player: None,
            sources: Vec::new(),
            ground: Vec::new(),
            legs: [Vec::new(), Vec::new()],
            samples: Vec::with_capacity(8),
            scratch: Pose::rest(&model.skeleton),
            leg_scratch: Pose::rest(&model.skeleton),
            air_time: 0.0,
            peak_fall: 0.0,
            last_support: Support::Grounded,
            spine: None,
            lean: (0.0, 0.0),
            smooth_speed: 0.0,
        }
    }

    /// The current state's name (see [`state`]).
    #[cfg(test)]
    pub fn state(&self) -> &str {
        self.player.as_ref().map_or("", |p| p.current_state())
    }

    /// True while a crossfade is running.
    #[cfg(test)]
    pub fn is_blending(&self) -> bool {
        self.player
            .as_ref()
            .is_some_and(StateMachinePlayer::is_blending)
    }

    /// The clips mixed now.
    #[cfg(test)]
    pub fn motion_samples(&self) -> Vec<metallic_anim::MotionSample> {
        self.player
            .as_ref()
            .map_or_else(Vec::new, StateMachinePlayer::motion_samples)
    }

    /// The current lean (sideways, forward) in radians.
    #[cfg(test)]
    pub fn lean(&self) -> (f32, f32) {
        self.lean
    }

    /// Advances by `dt` seconds and writes the pose to show into `pose` (which must belong to
    /// `model`'s skeleton). Call once per displayed frame.
    pub fn update(
        &mut self,
        model: &SkinnedModel,
        input: LocomotionInput,
        dt: f32,
        pose: &mut Pose,
    ) {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        if input.support == Support::Frozen {
            return;
        }
        let Some(player) = self.player.as_mut() else {
            pose.locals.clone_from(&model.skeleton.rest);
            return;
        };
        let speed = if input.speed.is_finite() && input.speed > STILL {
            input.speed
        } else {
            0.0
        };
        let grounded = input.support == Support::Grounded;
        self.air_time = if grounded { 0.0 } else { self.air_time + dt };
        if !grounded && input.vertical.is_finite() {
            self.peak_fall = self.peak_fall.max(-input.vertical);
        }
        // The names are the machine's own, checked when it was built.
        if input.support == Support::Rising && self.last_support != Support::Rising {
            let _ = player.fire("Jump");
        }
        self.last_support = input.support;
        let _ = player.set_float("Speed", speed);
        let _ = player.set_float("AirTime", self.air_time);
        let _ = player.set_float("Impact", self.peak_fall);
        let _ = player.set_bool("Grounded", grounded);
        let _ = player.set_bool("Rising", input.support == Support::Rising);
        let on_ground = player.current_state() == state::GROUND;
        player.set_playback_rate(if on_ground {
            gait_rate(&self.ground, speed)
        } else {
            1.0
        });
        player.update(dt);
        if grounded {
            self.peak_fall = 0.0;
        }

        // Blend the asset clips by the machine's weights (a running weighted average). The legs
        // of a gait in a mix are sampled at a stance-warped phase, so the feet of every gait in
        // the mix are on the ground over the same part of the cycle (each leg on its own: the
        // right foot strikes half a cycle later).
        player.motion_samples_into(&mut self.samples);
        let (mut stance_sum, mut gait_weight) = (0.0, 0.0);
        for s in &self.samples {
            if let Some(&(_, _, Some(st))) = self.sources.get(s.clip) {
                stance_sum += st * s.weight;
                gait_weight += s.weight;
            }
        }
        let target = if gait_weight > 0.0 {
            stance_sum / gait_weight
        } else {
            0.0
        };
        let clips = player.clips();
        pose.locals.clone_from(&model.skeleton.rest);
        let mut accumulated = 0.0;
        for s in &self.samples {
            let Some(&(clip, offset, stance)) = self.sources.get(s.clip) else {
                continue;
            };
            let asset = &model.clips[clip];
            self.scratch.locals.clone_from(&model.skeleton.rest);
            asset.sample(s.time + offset, &mut self.scratch);
            if let Some(stance) = stance
                && (stance - target).abs() > 1e-4
            {
                let length = clips[s.clip].duration.max(1e-4);
                let phase = s.time / length;
                for (side, joints) in self.legs.iter().enumerate() {
                    let half = side as f32 * 0.5;
                    let warped = warp_phase(phase - half, stance, target) + half;
                    asset.sample(warped.rem_euclid(1.0) * length, &mut self.leg_scratch);
                    for &j in joints {
                        self.scratch.locals[j] = self.leg_scratch.locals[j];
                    }
                    // Keep the thigh's orientation in the pelvis's parent space from the warped
                    // sample too: the pelvis tilts and twists with the step.
                    if let Some(&top) = joints.first()
                        && let Some(h) = model.skeleton.parents[top]
                    {
                        let r = self.scratch.locals[h].rotation.inverse()
                            * self.leg_scratch.locals[h].rotation
                            * self.leg_scratch.locals[top].rotation;
                        self.scratch.locals[top].rotation = r.normalize();
                    }
                }
            }
            accumulated += s.weight;
            pose.blend(&self.scratch, s.weight / accumulated);
        }

        // Lean into turns and speed changes while on the ground.
        let k = if dt > 0.0 {
            1.0 - (-dt / LEAN_TIME).exp()
        } else {
            0.0
        };
        let previous = self.smooth_speed;
        self.smooth_speed += (speed - self.smooth_speed) * k;
        let accel = if dt > 0.0 {
            (self.smooth_speed - previous) / dt
        } else {
            0.0
        };
        let turn = if input.turn_rate.is_finite() {
            input.turn_rate
        } else {
            0.0
        };
        let target = if on_ground && grounded {
            (
                ((speed * turn).atan2(GRAVITY) * LEAN_TURN).clamp(-LEAN_MAX_ROLL, LEAN_MAX_ROLL),
                (accel.atan2(GRAVITY) * LEAN_ACCEL).clamp(-LEAN_MAX_PITCH, LEAN_MAX_PITCH),
            )
        } else {
            (0.0, 0.0)
        };
        self.lean.0 += (target.0 - self.lean.0) * k;
        self.lean.1 += (target.1 - self.lean.1) * k;
        self.apply_lean(model, pose);
    }

    /// Tilts the `spine` joint (and so the upper body) by the lean: model space, the model
    /// faces +Z with +Y up, so its left is +X.
    fn apply_lean(&self, model: &SkinnedModel, pose: &mut Pose) {
        let Some((j, chain)) = &self.spine else {
            return;
        };
        let (side, forward) = self.lean;
        if side.abs() < 1e-5 && forward.abs() < 1e-5 {
            return;
        }
        let mut g = model.skeleton.base.to_scale_rotation_translation().1;
        for &k in chain {
            g *= pose.locals[k].rotation;
        }
        // Up (+Y) toward the left (+X) is a negative turn about +Z; toward the front (+Z) a
        // positive turn about +X.
        let r = Quat::from_rotation_z(-side) * Quat::from_rotation_x(forward);
        let local = &mut pose.locals[*j].rotation;
        *local = (g.inverse() * r * g * *local).normalize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avatar::{STUD_METRES, default_model};
    use glam::{Mat4, Vec3};

    const DT: f32 = 1.0 / 60.0;

    fn model() -> Arc<SkinnedModel> {
        default_model().expect("default avatar")
    }

    fn input(speed: f32, support: Support) -> LocomotionInput {
        LocomotionInput::new(speed, support)
    }

    /// Model-space position of a joint in `pose`.
    fn joint_position(m: &SkinnedModel, pose: &Pose, name: &str) -> Vec3 {
        let j = m.skeleton.names.iter().position(|n| n == name).unwrap();
        let mut chain = vec![j];
        while let Some(p) = m.skeleton.parents[*chain.last().unwrap()] {
            chain.push(p);
        }
        let mut g = m.skeleton.base;
        for &k in chain.iter().rev() {
            g *= pose.locals[k].to_mat4();
        }
        g.transform_point3(Vec3::ZERO)
    }

    fn run(l: &mut Locomotion, m: &SkinnedModel, pose: &mut Pose, i: LocomotionInput, secs: f32) {
        for _ in 0..(secs / DT).round() as usize {
            l.update(m, i, DT, pose);
        }
    }

    /// Weights of the clips mixed now, by clip name.
    fn mix(l: &Locomotion) -> Vec<(String, f32)> {
        let player = l.player.as_ref().unwrap();
        l.motion_samples()
            .iter()
            .map(|s| (player.clips()[s.clip].name.clone(), s.weight))
            .collect()
    }

    fn weight_of(l: &Locomotion, clip: &str) -> f32 {
        mix(l)
            .iter()
            .filter(|(n, _)| n == clip)
            .map(|(_, w)| w)
            .sum()
    }

    #[test]
    fn speed_picks_and_weights_the_ground_clips() {
        let m = model();
        let mut l = Locomotion::new(&m);
        let mut pose = Pose::rest(&m.skeleton);
        for (speed, expect) in [
            (0.0, vec![("idle", 1.0)]),
            (0.03, vec![("idle", 1.0)]),
            (3.0, vec![("idle", 0.5), ("walk_slow", 0.5)]),
            (6.0, vec![("walk_slow", 1.0)]),
            (11.0, vec![("walk_slow", 0.5), ("walk", 0.5)]),
            (16.0, vec![("walk", 1.0)]),
            (20.0, vec![("walk", 0.5), ("run", 0.5)]),
            (24.0, vec![("run", 1.0)]),
            (40.0, vec![("run", 1.0)]),
        ] {
            l.update(&m, input(speed, Support::Grounded), DT, &mut pose);
            assert_eq!(l.state(), state::GROUND);
            let total: f32 = mix(&l).iter().map(|(_, w)| w).sum();
            assert!((total - 1.0).abs() < 1e-5, "at {speed}: {:?}", mix(&l));
            assert_eq!(mix(&l).len(), expect.len(), "at {speed}: {:?}", mix(&l));
            for (clip, w) in expect {
                assert!(
                    (weight_of(&l, clip) - w).abs() < 1e-5,
                    "at {speed}: {clip} has {:?}",
                    mix(&l)
                );
            }
        }
    }

    #[test]
    fn the_playback_rate_follows_speed_over_authored_speed() {
        let m = model();
        let l = Locomotion::new(&m);
        let g = &l.ground;
        assert_eq!(gait_rate(g, 0.0), 1.0);
        for (i, gc) in g.iter().enumerate().skip(1) {
            let v = gc.speed;
            assert!((gait_rate(g, v) - 1.0).abs() < 1e-4, "gait {i} at {v}");
            // Measured from the clips: a walk stays on the ground longer than a run.
            assert!(
                gc.stance > 0.1 && gc.stance < 0.7,
                "gait {i}: stance {}",
                gc.stance
            );
        }
        assert!(g[1].stance > g[3].stance, "{g:?}");
        // The warp keeps strike and toe-off and is the identity at the mix's own stance.
        assert_eq!(warp_phase(0.3, 0.3, 0.3), 0.3);
        assert!((warp_phase(0.4, 0.5, 0.4) - 0.5).abs() < 1e-6);
        assert!((warp_phase(0.7, 0.5, 0.4) - 0.75).abs() < 1e-6);
        // Between gaits and beyond the run the rate stays inside its limits.
        for v in [1.0, 3.0, 11.0, 20.0, 30.0, 100.0] {
            let r = gait_rate(g, v);
            assert!((RATE_RANGE.0..=RATE_RANGE.1).contains(&r), "{v}: {r}");
        }
        assert_eq!(gait_rate(g, 100.0), RATE_RANGE.1);
    }

    #[test]
    fn airborne_states_play_at_normal_rate_whatever_the_speed() {
        let m = model();
        let mut l = Locomotion::new(&m);
        let mut pose = Pose::rest(&m.skeleton);
        run(&mut l, &m, &mut pose, input(40.0, Support::Grounded), 0.3);
        // Past the take-off fade the jump clip advances 1:1 with time although the ground rate
        // at 40 studs/s is 1.67.
        run(&mut l, &m, &mut pose, input(40.0, Support::Rising), 0.2);
        let t0 = l.motion_samples()[0].time;
        run(&mut l, &m, &mut pose, input(40.0, Support::Rising), 0.1);
        let t1 = l.motion_samples()[0].time;
        assert!(((t1 - t0) - 0.1).abs() < 1e-3, "advanced {}", t1 - t0);
    }

    #[test]
    fn a_jump_goes_through_jump_fall_and_land() {
        let m = model();
        let mut l = Locomotion::new(&m);
        let mut pose = Pose::rest(&m.skeleton);
        run(&mut l, &m, &mut pose, input(0.0, Support::Grounded), 0.5);
        assert_eq!(l.state(), state::GROUND);
        // Take-off.
        l.update(&m, input(0.0, Support::Rising), DT, &mut pose);
        assert_eq!(l.state(), state::JUMP);
        assert!(l.is_blending());
        run(&mut l, &m, &mut pose, input(0.0, Support::Rising), 0.25);
        assert_eq!(l.state(), state::JUMP);
        // At the top of the jump (the physics stops rising) the fall loop takes over.
        l.update(&m, input(0.0, Support::Falling), DT, &mut pose);
        assert_eq!(l.state(), state::FALL);
        run(&mut l, &m, &mut pose, input(0.0, Support::Falling), 0.2);
        // Touchdown, then back to idle when the landing clip ends.
        l.update(&m, input(0.0, Support::Grounded), DT, &mut pose);
        assert_eq!(l.state(), state::LAND);
        run(&mut l, &m, &mut pose, input(0.0, Support::Grounded), 0.45);
        assert_eq!(l.state(), state::LAND);
        run(&mut l, &m, &mut pose, input(0.0, Support::Grounded), 0.1);
        assert_eq!(l.state(), state::GROUND);
    }

    #[test]
    fn short_drops_stay_on_the_ground_pose_and_moving_landings_end_early() {
        let m = model();
        let mut l = Locomotion::new(&m);
        let mut pose = Pose::rest(&m.skeleton);
        run(&mut l, &m, &mut pose, input(16.0, Support::Grounded), 0.3);
        // Stepping down a ledge: a few frames in the air keep the walk.
        run(&mut l, &m, &mut pose, input(16.0, Support::Falling), 0.1);
        assert_eq!(l.state(), state::GROUND);
        run(&mut l, &m, &mut pose, input(16.0, Support::Falling), 0.05);
        assert_eq!(l.state(), state::FALL);
        l.update(&m, input(16.0, Support::Grounded), DT, &mut pose);
        assert_eq!(l.state(), state::LAND);
        run(
            &mut l,
            &m,
            &mut pose,
            input(16.0, Support::Grounded),
            LAND_HOLD_MOVING + 2.0 * DT,
        );
        assert_eq!(l.state(), state::GROUND);
    }

    #[test]
    fn crossfade_weights_sum_to_one_and_hand_over_in_the_blend_time() {
        let m = model();
        let mut l = Locomotion::new(&m);
        let mut pose = Pose::rest(&m.skeleton);
        run(&mut l, &m, &mut pose, input(24.0, Support::Grounded), 0.3);
        l.update(&m, input(24.0, Support::Rising), DT, &mut pose);
        assert_eq!(l.state(), state::JUMP);
        let mut frames = 0;
        while l.is_blending() {
            let total: f32 = mix(&l).iter().map(|(_, w)| w).sum();
            assert!((total - 1.0).abs() < 1e-5, "weights {:?}", mix(&l));
            // Run still carries weight until the fade ends.
            assert!(weight_of(&l, "run") > 0.0);
            l.update(&m, input(24.0, Support::Rising), DT, &mut pose);
            frames += 1;
            assert!(frames < 20, "fade never ended");
        }
        assert!((frames as f32 * DT - FADE_TAKEOFF).abs() < 2.0 * DT);
        assert_eq!(mix(&l).len(), 1);
        assert!((weight_of(&l, "jump") - 1.0).abs() < 1e-5);
    }

    #[test]
    fn the_jump_clip_starts_at_its_launch_frame_and_dead_characters_hold_the_pose() {
        let m = model();
        let mut l = Locomotion::new(&m);
        let mut pose = Pose::rest(&m.skeleton);
        run(&mut l, &m, &mut pose, input(0.0, Support::Grounded), 0.3);
        // Past the 0.1 s fade the pose is the jump clip at its launch frame (+ elapsed time).
        run(&mut l, &m, &mut pose, input(0.0, Support::Rising), 0.2);
        let jump = m.clips.iter().find(|c| c.name == "jump").unwrap();
        let s = &l.motion_samples()[0];
        assert!(s.time > 0.0 && s.time < 0.3, "jump clip time {}", s.time);
        let mut expect = Pose::rest(&m.skeleton);
        jump.sample(s.time + JUMP_LAUNCH, &mut expect);
        assert_eq!(pose, expect);
        // Dead: frozen.
        let held = pose.clone();
        run(&mut l, &m, &mut pose, input(0.0, Support::Frozen), 1.0);
        assert_eq!(pose, held);
    }

    /// Feet in contact with the ground stay put in the world while the character moves at each
    /// gait's authored speed: the clip phase follows the distance covered. Measured on every sole
    /// vertex within 0.005 studs of the floor, 240 times a second over one second. A sole that
    /// rolls from heel to ball moves its vertices a little around the contact point, so the test
    /// bounds the median and the 90th percentile rather than the single worst sample (touchdown).
    /// A clip played at the wrong rate (the 24 fps export bug) slips by about 20% of the speed.
    #[test]
    fn planted_feet_do_not_slide_at_gait_and_in_between_speeds() {
        let m = model();
        let lod = &m.lods[2];
        let feet = m.materials.iter().position(|s| s.name == "M_Feet").unwrap() as u8;
        let studs = Mat4::from_scale(Vec3::splat(1.0 / STUD_METRES));
        let mut failures = Vec::new();
        for speed in [6.0, 8.5, 11.0, 13.5, 16.0, 20.0, 24.0] {
            let mut l = Locomotion::new(&m);
            let mut pose = Pose::rest(&m.skeleton);
            run(&mut l, &m, &mut pose, input(speed, Support::Grounded), 0.5);
            let dt = 1.0 / 240.0;
            let mut joints = Vec::new();
            let mut last: Vec<Option<Vec3>> = vec![None; lod.vertices.len()];
            let mut slips: Vec<f32> = Vec::new();
            let mut travelled = 0.0;
            for _ in 0..240 {
                l.update(&m, input(speed, Support::Grounded), dt, &mut pose);
                travelled += speed * dt;
                pose.skin_matrices(&m.skeleton, studs, &mut joints);
                for (i, v) in lod.vertices.iter().enumerate() {
                    if v.material != feet {
                        continue;
                    }
                    let p = Vec3::from_array(v.position);
                    let posed: Vec3 = (0..4)
                        .map(|k| joints[v.joints[k] as usize].transform_point3(p) * v.weights[k])
                        .sum();
                    // The model faces +Z and moves forward along it.
                    let world = posed + Vec3::new(0.0, 0.0, travelled);
                    let now = (posed.y < 0.005).then_some(world);
                    if let (Some(a), Some(b)) = (last[i], now) {
                        slips.push((b - a).with_y(0.0).length() / dt);
                    }
                    last[i] = now;
                }
            }
            assert!(slips.len() > 500, "{speed}: feet never touch the ground");
            slips.sort_by(f32::total_cmp);
            let at = |q: f32| slips[((slips.len() - 1) as f32 * q) as usize];
            let (median, p90) = (at(0.5), at(0.9));
            println!("{speed} studs/s: planted sole slip median {median:.2}, p90 {p90:.2} studs/s");
            if median >= 0.05 * speed {
                failures.push(format!("{speed} studs/s: median slip {median}"));
            }
            // The gaits share one phase, the contacts (left heel strike at 0, right at 0.5) and the
            // heel-to-flat roll-in (0.07 of the cycle, #280), so the tail stays small between
            // gaits as well.
            if p90 >= 0.1 * speed {
                failures.push(format!("{speed} studs/s: 90th percentile slip {p90}"));
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn landings_faster_than_the_hard_limit_play_the_hard_landing() {
        let m = model();
        for (vertical, expect) in [(-50.0, state::LAND), (-110.0, state::LAND_HARD)] {
            let mut l = Locomotion::new(&m);
            let mut pose = Pose::rest(&m.skeleton);
            run(&mut l, &m, &mut pose, input(0.0, Support::Grounded), 0.2);
            let mut falling = input(0.0, Support::Falling);
            falling.vertical = vertical;
            run(&mut l, &m, &mut pose, falling, 0.5);
            assert_eq!(l.state(), state::FALL);
            l.update(&m, input(0.0, Support::Grounded), DT, &mut pose);
            assert_eq!(l.state(), expect, "landing at {vertical}");
            // The next landing starts from a fresh impact.
            run(&mut l, &m, &mut pose, input(0.0, Support::Grounded), 2.0);
            assert_eq!(l.state(), state::GROUND);
            run(&mut l, &m, &mut pose, input(0.0, Support::Falling), 0.3);
            l.update(&m, input(0.0, Support::Grounded), DT, &mut pose);
            assert_eq!(l.state(), state::LAND);
        }
    }

    #[test]
    fn the_upper_body_leans_into_turns_and_speed_changes() {
        let m = model();
        let mut l = Locomotion::new(&m);
        let mut pose = Pose::rest(&m.skeleton);
        run(&mut l, &m, &mut pose, input(16.0, Support::Grounded), 1.0);
        let (side, forward) = l.lean();
        assert!(
            side.abs() < 1e-3 && forward.abs() < 1e-3,
            "{side} {forward}"
        );
        let straight = joint_position(&m, &pose, "head");
        // Turning left at a run: lean to the left (+X in model space), capped.
        let mut turning = input(16.0, Support::Grounded);
        turning.turn_rate = 4.0;
        // Same phase of the cycle as `straight` (whole cycles later), so only the lean differs.
        let cycle = l.ground[2].length;
        run(&mut l, &m, &mut pose, turning, 3.0 * cycle);
        let (side, _) = l.lean();
        assert!(side > 0.1 && side <= LEAN_MAX_ROLL, "lean {side}");
        let leaning = joint_position(&m, &pose, "head");
        assert!(leaning.x > straight.x + 0.05, "{straight} -> {leaning}");
        // Stopping: a short lean back, then upright again.
        run(&mut l, &m, &mut pose, input(16.0, Support::Grounded), 1.0);
        run(&mut l, &m, &mut pose, input(0.0, Support::Grounded), 0.1);
        assert!(l.lean().1 < -0.03, "braking lean {:?}", l.lean());
        run(&mut l, &m, &mut pose, input(0.0, Support::Grounded), 1.5);
        assert!(l.lean().0.abs() < 1e-3 && l.lean().1.abs() < 1e-3);
        // No lean in the air.
        let mut air = input(16.0, Support::Falling);
        air.turn_rate = 4.0;
        run(&mut l, &m, &mut pose, air, 1.0);
        assert!(l.lean().0.abs() < 1e-3);
    }

    #[test]
    fn the_per_frame_update_reuses_its_buffers() {
        let m = model();
        let mut l = Locomotion::new(&m);
        let mut pose = Pose::rest(&m.skeleton);
        let script = |i: usize| match (i / 30) % 6 {
            0 => input(0.0, Support::Grounded),
            1 => input(11.0, Support::Grounded),
            2 => input(20.0, Support::Rising),
            3 => input(20.0, Support::Falling),
            4 => input(24.0, Support::Grounded),
            _ => input(3.0, Support::Grounded),
        };
        for i in 0..360 {
            l.update(&m, script(i), DT, &mut pose);
        }
        let buffers = (
            l.samples.as_ptr(),
            l.samples.capacity(),
            pose.locals.as_ptr(),
        );
        for i in 0..720 {
            l.update(&m, script(i), DT, &mut pose);
        }
        assert_eq!(
            (
                l.samples.as_ptr(),
                l.samples.capacity(),
                pose.locals.as_ptr()
            ),
            buffers
        );
    }
}
