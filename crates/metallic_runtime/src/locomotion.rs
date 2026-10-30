//! The built-in locomotion animation controller: picks and blends the default avatar's clips from
//! the character's speed and whether it is on the ground.
//!
//! States and transitions:
//! - **Ground**: idle, `walk_slow`, `walk` and `run` blended by horizontal speed. Each pair of
//!   neighbours in [`GAITS`] is mixed linearly between their speeds (0, 6, 16 and 24 studs/s). The
//!   three gait clips share one normalised phase (left heel strike at 0, right at 0.5), advanced
//!   by speed divided by the blended stride, so at every speed the planted foot moves backward as
//!   fast as the character moves forward: no foot sliding.
//! - **Jump**: on take-off. The clip starts at its launch frame ([`JUMP_LAUNCH`]): the physics
//!   jumps the moment the button is pressed, so the anticipation crouch before it is skipped.
//! - **Fall**: when the jump clip has played out, or after [`FALL_DELAY`] in the air without a
//!   jump (walking off a ledge; short drops down steps stay on the ground pose).
//! - **Land**: on touchdown; back to Ground when the clip ends, or after [`LAND_HOLD_MOVING`] when
//!   the character is already moving again.
//!
//! Every state change crossfades from the last shown pose over 0.1 to 0.2 s (smoothstep), so
//! nothing pops. A dead character's pose is held.
//!
//! Climb and swim clips exist in the asset, but the character controller has no climbing or
//! swimming state yet (`HumanoidState` is Running, Jumping, Freefall or Dead). TODO: add Climb and
//! Swim states here, or in #40's state machine, once the controller reports them.
//!
//! Replacing this with `metallic_anim` (#40): the avatar only calls
//! [`Locomotion::update`]`(model, input, dt, pose)` once per frame with a [`LocomotionInput`]
//! (speed and ground state) and draws the [`Pose`] it gets back. A `metallic_anim` state machine
//! that takes the same two parameters and writes a pose can take its place in
//! `crate::avatar::Avatar` without touching the renderer or the game loop.
use metallic_render::metallic_asset::{Pose, SkinnedModel};

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

/// The controller's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    /// Idle and gaits blended by speed.
    Ground,
    /// The jump clip after take-off.
    Jump,
    /// The fall loop.
    Fall,
    /// The landing clip.
    Land,
}

/// Clip indices into `SkinnedModel::clips`; `None` when the asset lacks the clip (the rest pose
/// stands in).
struct Clips {
    idle: Option<usize>,
    gaits: [Option<usize>; 3],
    jump: Option<usize>,
    fall: Option<usize>,
    land: Option<usize>,
    /// Distance in studs one cycle of each gait covers: its speed times its clip length.
    strides: [f32; 3],
}

/// Picks, times and blends the locomotion clips of one character.
pub(crate) struct Locomotion {
    clips: Clips,
    state: State,
    /// Seconds in the current state.
    state_time: f32,
    /// Seconds in the air (0 on the ground).
    air_time: f32,
    /// Idle clip time.
    idle_time: f32,
    /// Shared gait phase, 0..1.
    phase: f32,
    /// The two ground clips blended last frame and the weight of the second.
    ground_mix: (Option<usize>, Option<usize>, f32),
    /// Pose shown when the current crossfade started, and the fade's progress and length.
    fade_from: Pose,
    fade: Option<(f32, f32)>,
    scratch: Pose,
    last_support: Support,
}

impl Locomotion {
    /// A controller for `model`, standing idle.
    pub fn new(model: &SkinnedModel) -> Locomotion {
        let find = |name: &str| model.clips.iter().position(|c| c.name == name);
        let rest = Pose::rest(&model.skeleton);
        let gaits = GAITS.map(|(name, _)| find(name));
        let mut strides = [0.0; 3];
        for (i, (clip, (_, speed))) in gaits.iter().zip(GAITS).enumerate() {
            strides[i] = speed
                * clip
                    .and_then(|c| model.clips.get(c))
                    .map_or(0.0, |c| c.duration);
        }
        Locomotion {
            clips: Clips {
                idle: find("idle"),
                gaits,
                jump: find("jump"),
                fall: find("fall"),
                land: find("land"),
                strides,
            },
            state: State::Ground,
            state_time: 0.0,
            air_time: 0.0,
            idle_time: 0.0,
            phase: 0.0,
            ground_mix: (None, None, 0.0),
            fade_from: rest.clone(),
            fade: None,
            scratch: rest,
            last_support: Support::Grounded,
        }
    }

    /// The current state.
    #[cfg(test)]
    pub fn state(&self) -> State {
        self.state
    }

    /// The ground clips mixed in the last update (by clip index) and the second one's weight.
    #[cfg(test)]
    pub fn ground_mix(&self) -> (Option<usize>, Option<usize>, f32) {
        self.ground_mix
    }

    /// Progress of the running crossfade (0 just started, 1 or `None` done).
    #[cfg(test)]
    pub fn fade_progress(&self) -> Option<f32> {
        self.fade.map(|(t, len)| (t / len).min(1.0))
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
        let speed = if input.speed.is_finite() {
            input.speed.max(0.0)
        } else {
            0.0
        };
        self.air_time = match input.support {
            Support::Grounded => 0.0,
            _ => self.air_time + dt,
        };
        let took_off = input.support == Support::Rising && self.last_support != Support::Rising;
        self.last_support = input.support;
        self.state_time += dt;
        self.idle_time += dt;
        self.advance_phase(speed, dt);

        let next = match (self.state, input.support) {
            (_, Support::Rising) if took_off => Some((State::Jump, FADE_TAKEOFF)),
            (State::Jump | State::Fall, Support::Grounded) => Some((State::Land, FADE_LAND)),
            (State::Jump, _) if self.state_time >= self.jump_length(model) => {
                Some((State::Fall, FADE_TO_FALL))
            }
            (State::Ground | State::Land, Support::Falling) if self.air_time >= FALL_DELAY => {
                Some((State::Fall, FADE_TO_FALL))
            }
            (State::Land, Support::Grounded)
                if self.state_time >= self.clip_length(model, self.clips.land)
                    || (speed > MOVING && self.state_time >= LAND_HOLD_MOVING) =>
            {
                Some((State::Ground, FADE_LAND_OUT))
            }
            _ => None,
        };
        if let Some((state, fade)) = next {
            // Fade from what is on screen now.
            self.fade_from.locals.clone_from(&pose.locals);
            self.fade = Some((0.0, fade));
            self.state = state;
            self.state_time = 0.0;
        }

        self.sample_state(model, speed, pose);
        if let Some((t, len)) = self.fade {
            let t = t + dt;
            let u = (t / len).clamp(0.0, 1.0);
            let w = u * u * (3.0 - 2.0 * u);
            self.scratch.locals.clone_from(&self.fade_from.locals);
            self.scratch.blend(pose, w);
            pose.locals.clone_from(&self.scratch.locals);
            self.fade = (u < 1.0).then_some((t, len));
        }
    }

    fn clip_length(&self, model: &SkinnedModel, clip: Option<usize>) -> f32 {
        clip.and_then(|c| model.clips.get(c))
            .map_or(0.0, |c| c.duration)
    }

    /// Time the jump clip plays after the launch frame.
    fn jump_length(&self, model: &SkinnedModel) -> f32 {
        (self.clip_length(model, self.clips.jump) - JUMP_LAUNCH).max(0.0)
    }

    /// Advances the shared gait phase by the distance covered over the blended stride.
    fn advance_phase(&mut self, speed: f32, dt: f32) {
        let (a, b, w) = gait_segment(speed);
        let stride_of = |g: Option<usize>| g.map(|i| self.clips.strides[i]);
        let stride = match (stride_of(a), stride_of(b)) {
            (Some(sa), Some(sb)) => sa + (sb - sa) * w,
            // Between idle and the slowest gait the gait keeps its own stride.
            (None, Some(s)) | (Some(s), None) => s,
            (None, None) => return,
        };
        if stride > 1e-4 {
            self.phase = (self.phase + speed * dt / stride).rem_euclid(1.0);
        }
    }

    fn sample_state(&mut self, model: &SkinnedModel, speed: f32, pose: &mut Pose) {
        match self.state {
            State::Ground => {
                let (a, b, w) = gait_segment(speed);
                let clip_a = a.map_or(self.clips.idle, |i| self.clips.gaits[i]);
                let clip_b = b.map_or(self.clips.idle, |i| self.clips.gaits[i]);
                self.ground_mix = (clip_a, clip_b, w);
                let ta = self.ground_time(model, a);
                let tb = self.ground_time(model, b);
                sample(model, clip_a, ta, true, pose);
                if w > 0.0 {
                    sample(model, clip_b, tb, true, &mut self.scratch);
                    pose.blend(&self.scratch, w);
                }
            }
            State::Jump => sample(
                model,
                self.clips.jump,
                JUMP_LAUNCH + self.state_time,
                false,
                pose,
            ),
            State::Fall => sample(model, self.clips.fall, self.state_time, true, pose),
            State::Land => sample(model, self.clips.land, self.state_time, false, pose),
        }
    }

    /// Clip time for a ground clip: gaits follow the shared phase, idle its own clock.
    fn ground_time(&self, model: &SkinnedModel, gait: Option<usize>) -> f32 {
        match gait {
            Some(i) => self.phase * self.clip_length(model, self.clips.gaits[i]),
            None => self.idle_time,
        }
    }
}

/// The two neighbouring ground entries around `speed` and the weight of the second: `None` is
/// idle, `Some(i)` is `GAITS[i]`.
pub(crate) fn gait_segment(speed: f32) -> (Option<usize>, Option<usize>, f32) {
    if speed <= STILL {
        return (None, None, 0.0);
    }
    let mut lower: (Option<usize>, f32) = (None, 0.0);
    for (i, &(_, v)) in GAITS.iter().enumerate() {
        if speed < v {
            let w = (speed - lower.1) / (v - lower.1);
            return (lower.0, Some(i), w);
        }
        lower = (Some(i), v);
    }
    (lower.0, lower.0, 0.0)
}

/// Samples clip `clip` at `time` (wrapped when `looped`) over the rest pose.
fn sample(model: &SkinnedModel, clip: Option<usize>, time: f32, looped: bool, pose: &mut Pose) {
    pose.locals.clone_from(&model.skeleton.rest);
    let Some(c) = clip.and_then(|c| model.clips.get(c)) else {
        return;
    };
    let t = if looped && c.duration > 0.0 {
        time.rem_euclid(c.duration)
    } else {
        time
    };
    c.sample(t, pose);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avatar::{STUD_METRES, default_model};
    use metallic_render::glam::{Mat4, Vec3};
    use std::sync::Arc;

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

    fn clip_name(m: &SkinnedModel, c: Option<usize>) -> &str {
        c.map_or("-", |i| m.clips[i].name.as_str())
    }

    #[test]
    fn speed_picks_and_weights_the_ground_clips() {
        assert_eq!(gait_segment(0.0), (None, None, 0.0));
        assert_eq!(gait_segment(3.0), (None, Some(0), 0.5));
        assert_eq!(gait_segment(11.0), (Some(0), Some(1), 0.5));
        assert_eq!(gait_segment(20.0), (Some(1), Some(2), 0.5));
        assert_eq!(gait_segment(40.0), (Some(2), Some(2), 0.0));
        let m = model();
        let mut l = Locomotion::new(&m);
        let mut pose = Pose::rest(&m.skeleton);
        for (speed, a, b, w) in [
            (0.0, "idle", "idle", 0.0),
            (3.0, "idle", "walk_slow", 0.5),
            (6.0, "walk_slow", "walk", 0.0),
            (16.0, "walk", "run", 0.0),
            (20.0, "walk", "run", 0.5),
            (24.0, "run", "run", 0.0),
        ] {
            l.update(&m, input(speed, Support::Grounded), DT, &mut pose);
            assert_eq!(l.state(), State::Ground);
            let (ca, cb, weight) = l.ground_mix();
            assert_eq!((clip_name(&m, ca), clip_name(&m, cb)), (a, b), "at {speed}");
            assert!((weight - w).abs() < 1e-5, "at {speed}: {weight}");
        }
    }

    #[test]
    fn a_jump_goes_through_jump_fall_and_land() {
        let m = model();
        let mut l = Locomotion::new(&m);
        let mut pose = Pose::rest(&m.skeleton);
        run(&mut l, &m, &mut pose, input(0.0, Support::Grounded), 0.5);
        assert_eq!(l.state(), State::Ground);
        // Take-off.
        l.update(&m, input(0.0, Support::Rising), DT, &mut pose);
        assert_eq!(l.state(), State::Jump);
        assert!(l.fade_progress().is_some_and(|p| p < 0.5));
        run(&mut l, &m, &mut pose, input(0.0, Support::Rising), 0.25);
        assert_eq!(l.state(), State::Jump);
        // The jump clip plays 0.4 s after its launch frame, then the fall loop takes over.
        run(&mut l, &m, &mut pose, input(0.0, Support::Falling), 0.1);
        assert_eq!(l.state(), State::Jump);
        run(&mut l, &m, &mut pose, input(0.0, Support::Falling), 0.1);
        assert_eq!(l.state(), State::Fall);
        // Touchdown, then back to idle when the landing clip ends.
        l.update(&m, input(0.0, Support::Grounded), DT, &mut pose);
        assert_eq!(l.state(), State::Land);
        run(&mut l, &m, &mut pose, input(0.0, Support::Grounded), 0.45);
        assert_eq!(l.state(), State::Land);
        run(&mut l, &m, &mut pose, input(0.0, Support::Grounded), 0.1);
        assert_eq!(l.state(), State::Ground);
    }

    #[test]
    fn short_drops_stay_on_the_ground_pose_and_moving_landings_end_early() {
        let m = model();
        let mut l = Locomotion::new(&m);
        let mut pose = Pose::rest(&m.skeleton);
        run(&mut l, &m, &mut pose, input(16.0, Support::Grounded), 0.3);
        // Stepping down a ledge: a few frames in the air keep the walk.
        run(&mut l, &m, &mut pose, input(16.0, Support::Falling), 0.1);
        assert_eq!(l.state(), State::Ground);
        run(&mut l, &m, &mut pose, input(16.0, Support::Falling), 0.05);
        assert_eq!(l.state(), State::Fall);
        l.update(&m, input(16.0, Support::Grounded), DT, &mut pose);
        assert_eq!(l.state(), State::Land);
        run(
            &mut l,
            &m,
            &mut pose,
            input(16.0, Support::Grounded),
            LAND_HOLD_MOVING + DT,
        );
        assert_eq!(l.state(), State::Ground);
    }

    #[test]
    fn crossfades_last_their_length_and_dead_characters_hold_the_pose() {
        let m = model();
        let mut l = Locomotion::new(&m);
        let mut pose = Pose::rest(&m.skeleton);
        run(&mut l, &m, &mut pose, input(24.0, Support::Grounded), 0.3);
        let running = pose.clone();
        l.update(&m, input(24.0, Support::Rising), DT, &mut pose);
        // One frame into a 0.1 s fade the pose is still closer to the run than the jump is.
        let hips = m.skeleton.joint("hips").unwrap();
        let apart = |a: &Pose, b: &Pose| {
            a.locals[hips]
                .rotation
                .angle_between(b.locals[hips].rotation)
        };
        let mut jump_pose = Pose::rest(&m.skeleton);
        sample(&m, l.clips.jump, JUMP_LAUNCH + DT, false, &mut jump_pose);
        assert!(apart(&pose, &running) < apart(&jump_pose, &running));
        run(&mut l, &m, &mut pose, input(24.0, Support::Rising), 0.1);
        assert_eq!(l.fade_progress(), None, "fade done after 0.1 s");
        // Dead: frozen.
        let held = pose.clone();
        run(&mut l, &m, &mut pose, input(24.0, Support::Frozen), 1.0);
        assert_eq!(pose, held);
    }

    /// Feet in contact with the ground stay put in the world while the character moves at each
    /// gait's authored speed: the clip phase follows the distance covered. Measured on every sole
    /// vertex within 0.005 studs of the floor, 240 times a second over one second. A sole that
    /// rolls from heel to ball moves its vertices a little around the contact point, so the test
    /// bounds the median and the 90th percentile rather than the single worst sample (touchdown).
    /// A clip played at the wrong rate (the 24 fps export bug) slips by about 20% of the speed.
    #[test]
    fn planted_feet_do_not_slide_at_the_authored_speeds() {
        let m = model();
        let lod = &m.lods[2];
        let feet = m.materials.iter().position(|s| s.name == "M_Feet").unwrap() as u8;
        let studs = Mat4::from_scale(Vec3::splat(1.0 / STUD_METRES));
        for (_, speed) in GAITS {
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
            assert!(
                p90 < 0.1 * speed,
                "{speed} studs/s: 90th percentile slip {p90}"
            );
        }
    }
}
