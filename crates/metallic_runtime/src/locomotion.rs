//! The default avatar's locomotion controller: a [`metallic_anim::StateMachine`] over the
//! avatar's clips, driven by the character's speed and ground state.
//!
//! States and transitions (blend times in seconds, crossfaded by `metallic_anim`):
//! - **Ground**: a 1D blend on `Speed` of idle, `walk_slow`, `walk` and `run` placed at 0, 6, 16
//!   and 24 studs/s ([`GAITS`]). The clips share one normalised phase (left heel strike at 0,
//!   right at 0.5) and each is authored for its own speed. The playback rate is set each frame
//!   to the speed over the blended authored speed ([`gait_rate`], 0.5 to 2), so the planted
//!   foot moves backward as fast as the character moves forward: no foot sliding.
//! - **Jump**: on take-off (the `Jump` trigger, 0.1). The clip starts at its launch frame
//!   ([`JUMP_LAUNCH`]): the physics jumps the moment the button is pressed, so the anticipation
//!   crouch before it is skipped.
//! - **Fall**: when the jump clip has played out (0.2), or after [`FALL_DELAY`] in the air without
//!   a jump (walking off a ledge; short drops down steps stay on the ground pose).
//! - **Land**: on touchdown (0.1); back to Ground (0.2) when the clip ends, or after
//!   [`LAND_HOLD_MOVING`] when the character is already moving again.
//!
//! A dead character's pose is held.
//!
//! Climb and swim clips exist in the asset, but the character controller has no climbing or
//! swimming state yet (`HumanoidState` is Running, Jumping, Freefall or Dead), so the machine has
//! no such states. The jump has no take-off window either: that needs the physics impulse
//! delayed, which is not an animation matter.
//!
//! The state machine only picks clips, times and weights. The clips themselves stay the asset's
//! own ([`metallic_render::metallic_asset`]); the machine is built over name-and-length stand-ins
//! for them and its [`MotionSample`]s are sampled from the asset clips and blended here.
use metallic_anim::{
    Clip, Condition, LoopMode, Motion, Parameter, ParameterValue, State, StateMachine,
    StateMachinePlayer, Transition,
};
use metallic_render::metallic_asset::{Pose, SkinnedModel};
use std::sync::Arc;

/// Gait clips and the speed (studs/s) each is authored for, slowest first. Idle sits at 0.
pub(crate) const GAITS: [(&str, f32); 3] = [("walk_slow", 6.0), ("walk", 16.0), ("run", 24.0)];
/// Seconds into the jump clip where the feet leave the ground (frame 6 at 30 fps).
pub(crate) const JUMP_LAUNCH: f32 = 6.0 / 30.0;
/// Time in the air without a jump before the fall pose starts.
pub(crate) const FALL_DELAY: f32 = 0.12;
/// Time the land pose holds before blending out when the character is moving.
pub(crate) const LAND_HOLD_MOVING: f32 = 0.1;
/// Speed (studs/s) above which a landing counts as moving.
const MOVING: f32 = 2.0;
/// Speeds below this are standing still (no shuffle from tiny pushes).
const STILL: f32 = 0.05;

/// Crossfade lengths in seconds.
const FADE_TAKEOFF: f32 = 0.1;
const FADE_TO_FALL: f32 = 0.2;
const FADE_LAND: f32 = 0.1;
const FADE_LAND_OUT: f32 = 0.2;

/// Limits of the playback rate that keeps the gait in step with the speed.
const RATE_RANGE: (f32, f32) = (0.5, 2.0);

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
    /// Ground state.
    pub support: Support,
}

/// State names of [`machine`].
pub(crate) mod state {
    pub const GROUND: &str = "Ground";
    pub const JUMP: &str = "Jump";
    pub const FALL: &str = "Fall";
    pub const LAND: &str = "Land";
}

/// Playback rate that keeps the feet planted at `speed`. `ground` lists `(speed, clip length)`
/// of the ground clips, slowest first, idle first of all. One cycle of a gait covers its speed
/// times its length; the machine blends clip lengths linearly, so the rate is the distance
/// covered per second over the stride the blend plays per second. Between idle and the slowest
/// gait the stride is that gait's own.
fn gait_rate(ground: &[(f32, f32)], speed: f32) -> f32 {
    if speed <= STILL || ground.len() < 2 {
        return 1.0;
    }
    let i = ground
        .iter()
        .skip(1)
        .position(|&(v, _)| speed < v)
        .map_or(ground.len() - 1, |p| p + 1);
    let (a, b) = (ground[i.saturating_sub(1)], ground[i]);
    let w = if b.0 > a.0 {
        ((speed - a.0) / (b.0 - a.0)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let length = a.1 + (b.1 - a.1) * w;
    let stride = if i == 1 {
        b.0 * b.1
    } else {
        a.0 * a.1 + (b.0 * b.1 - a.0 * a.1) * w
    };
    if stride > 1e-4 {
        (speed * length / stride).clamp(RATE_RANGE.0, RATE_RANGE.1)
    } else {
        1.0
    }
}

/// The locomotion state machine. `land_length` is the land clip's length in seconds (the moving
/// landing leaves after [`LAND_HOLD_MOVING`] of it).
fn machine(land_length: f32) -> StateMachine {
    let clip = |name: &str| State {
        name: name.into(),
        motion: Motion::Clip(name.to_lowercase()),
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
    StateMachine {
        parameters: vec![
            Parameter {
                name: "Speed".into(),
                default: ParameterValue::Float(0.0),
            },
            Parameter {
                name: "AirTime".into(),
                default: ParameterValue::Float(0.0),
            },
            Parameter {
                name: "Grounded".into(),
                default: ParameterValue::Bool(true),
            },
            Parameter {
                name: "Jump".into(),
                default: ParameterValue::Trigger,
            },
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
            clip(state::JUMP),
            clip(state::FALL),
            clip(state::LAND),
        ],
        entry: state::GROUND.into(),
        transitions: vec![
            tr(
                None,
                state::JUMP,
                vec![Condition::Triggered("Jump".into())],
                FADE_TAKEOFF,
                None,
            ),
            tr(
                Some(state::JUMP),
                state::LAND,
                vec![grounded()],
                FADE_LAND,
                None,
            ),
            tr(
                Some(state::FALL),
                state::LAND,
                vec![grounded()],
                FADE_LAND,
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
            tr(
                Some(state::LAND),
                state::FALL,
                vec![in_air()],
                FADE_TO_FALL,
                None,
            ),
            tr(
                Some(state::LAND),
                state::GROUND,
                vec![grounded()],
                FADE_LAND_OUT,
                Some(1.0),
            ),
            tr(
                Some(state::LAND),
                state::GROUND,
                vec![grounded(), Condition::Greater("Speed".into(), MOVING)],
                FADE_LAND_OUT,
                Some(LAND_HOLD_MOVING / land_length.max(1e-3)),
            ),
        ],
    }
}

/// Picks, times and blends the locomotion clips of one character. Without a player the asset
/// lacks a clip the machine needs, and the rest pose stands in.
pub(crate) struct Locomotion {
    player: Option<StateMachinePlayer>,
    /// Per player clip: index into `SkinnedModel::clips` and seconds to add to the player's clip
    /// time (the jump clip starts at its launch frame).
    sources: Vec<(usize, f32)>,
    /// `(speed, clip length)` of idle and the gaits, for [`gait_rate`].
    ground: Vec<(f32, f32)>,
    scratch: Pose,
    air_time: f32,
    last_support: Support,
}

impl Locomotion {
    /// A controller for `model`, standing idle.
    pub fn new(model: &SkinnedModel) -> Locomotion {
        let mut sources = Vec::new();
        let mut clips = Vec::new();
        let mut land_length = 0.0;
        let mut ground = Vec::new();
        let named = [
            ("idle", true),
            ("jump", false),
            ("fall", true),
            ("land", false),
        ];
        for (name, looped) in named.into_iter().chain(GAITS.map(|(n, _)| (n, true))) {
            let Some(i) = model.clips.iter().position(|c| c.name == name) else {
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
            if name == "idle" {
                ground.push((0.0, duration));
            }
            if let Some(&(_, v)) = GAITS.iter().find(|(n, _)| *n == name) {
                ground.push((v, duration));
            }
            if name == "land" {
                land_length = duration;
            }
            let mut clip = Clip::new(name, duration);
            clip.loop_mode = if looped {
                LoopMode::Loop
            } else {
                LoopMode::Once
            };
            clips.push(Arc::new(clip));
            sources.push((i, offset));
        }
        let player = StateMachinePlayer::new(&machine(land_length), clips)
            .map_err(|e| eprintln!("warning: the avatar's animation machine is invalid ({e})"))
            .ok();
        Locomotion {
            player,
            sources,
            ground,
            ..Locomotion::without_player(model)
        }
    }

    fn without_player(model: &SkinnedModel) -> Locomotion {
        Locomotion {
            player: None,
            sources: Vec::new(),
            ground: Vec::new(),
            scratch: Pose::rest(&model.skeleton),
            air_time: 0.0,
            last_support: Support::Grounded,
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
        self.air_time = match input.support {
            Support::Grounded => 0.0,
            _ => self.air_time + dt,
        };
        // The names are the machine's own, checked when it was built.
        if input.support == Support::Rising && self.last_support != Support::Rising {
            let _ = player.fire("Jump");
        }
        self.last_support = input.support;
        let _ = player.set_float("Speed", speed);
        let _ = player.set_float("AirTime", self.air_time);
        let _ = player.set_bool("Grounded", input.support == Support::Grounded);
        let ground = player.current_state() == state::GROUND;
        player.set_playback_rate(if ground {
            gait_rate(&self.ground, speed)
        } else {
            1.0
        });
        player.update(dt);

        // Blend the asset clips by the machine's weights (a running weighted average).
        pose.locals.clone_from(&model.skeleton.rest);
        let mut accumulated = 0.0;
        for s in player.motion_samples() {
            let Some(&(clip, offset)) = self.sources.get(s.clip) else {
                continue;
            };
            self.scratch.locals.clone_from(&model.skeleton.rest);
            model.clips[clip].sample(s.time + offset, &mut self.scratch);
            accumulated += s.weight;
            pose.blend(&self.scratch, s.weight / accumulated);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avatar::{STUD_METRES, default_model};
    use metallic_render::glam::{Mat4, Vec3};

    const DT: f32 = 1.0 / 60.0;

    fn model() -> Arc<SkinnedModel> {
        default_model().expect("default avatar")
    }

    fn input(speed: f32, support: Support) -> LocomotionInput {
        LocomotionInput { speed, support }
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
        for (i, &(v, _)) in g.iter().enumerate().skip(1) {
            assert!((gait_rate(g, v) - 1.0).abs() < 1e-4, "gait {i} at {v}");
        }
        // Between gaits and beyond the run the rate stays inside its limits.
        for v in [1.0, 3.0, 11.0, 20.0, 30.0, 100.0] {
            let r = gait_rate(g, v);
            assert!((RATE_RANGE.0..=RATE_RANGE.1).contains(&r), "{v}: {r}");
        }
        assert_eq!(gait_rate(g, 100.0), RATE_RANGE.1);
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
        // The jump clip plays 0.4 s after its launch frame, then the fall loop takes over.
        run(&mut l, &m, &mut pose, input(0.0, Support::Falling), 0.1);
        assert_eq!(l.state(), state::JUMP);
        run(&mut l, &m, &mut pose, input(0.0, Support::Falling), 0.1);
        assert_eq!(l.state(), state::FALL);
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
        for speed in [6.0, 11.0, 16.0, 20.0, 24.0] {
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
            assert!(
                median < 0.05 * speed,
                "{speed} studs/s: median slip {median}"
            );
            // Between gaits two poses are mixed, so the soles that count as planted change from
            // frame to frame and the tail is wider; the median is what the eye sees.
            let authored = GAITS.iter().any(|&(_, v)| v == speed);
            let tail = if authored { 0.1 } else { 0.5 };
            assert!(
                p90 < tail * speed,
                "{speed} studs/s: 90th percentile slip {p90}"
            );
        }
    }
}
