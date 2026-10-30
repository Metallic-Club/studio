//! A small animation state machine: states play clips (or a 1D blend of clips), transitions
//! fire on parameter conditions and crossfade over a blend time.
//!
//! The definition ([`StateMachine`]) is plain data that saves to text; it refers to states,
//! clips and parameters by name. [`StateMachinePlayer`] checks the names once and then runs it.
//!
//! **Rules (kept deliberately simple):**
//! - Each [`StateMachinePlayer::update`] checks transitions out of the current state, then
//!   "any state" transitions, in the order they are written. The first one whose conditions all
//!   hold fires. A transition with no conditions and no exit time never fires (it would loop).
//! - A transition with `exit_time` waits until the current state has played that fraction of
//!   its clip (`1.0` = one full play), then fires if its conditions hold.
//! - Triggers are one-shot: a trigger stays set until a transition that tests it fires, or until
//!   the end of the next update, whichever comes first (no stale triggers hours later).
//! - A new transition may start while a blend is running; the old blend is dropped and the
//!   current mix fades out as one.

use crate::AnimError;
use crate::clip::Clip;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Kind and default value of a parameter.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ParameterValue {
    /// On/off, e.g. `Grounded`.
    Bool(bool),
    /// A number, e.g. `Speed`.
    Float(f32),
    /// A one-shot signal, e.g. `Jump`.
    Trigger,
}

/// A named input to the state machine, set by gameplay code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    /// Name used by conditions and setters.
    pub name: String,
    /// Kind and default value.
    pub default: ParameterValue,
}

/// A test on one parameter. All conditions of a transition must hold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    /// A bool parameter equals the value.
    Is(String, bool),
    /// A float parameter is greater than the value.
    Greater(String, f32),
    /// A float parameter is less than the value.
    Less(String, f32),
    /// A trigger parameter was fired.
    Triggered(String),
}

/// What a state plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Motion {
    /// One clip, by name.
    Clip(String),
    /// Clips placed along a float parameter (e.g. idle at 0, walk at 4, run at 12); the two
    /// nearest are blended and their playheads kept in step.
    Blend1D {
        /// Float parameter that picks the blend.
        parameter: String,
        /// `(position, clip name)`, sorted by position.
        points: Vec<(f32, String)>,
    },
}

/// One state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// Unique name, e.g. `"Run"`.
    pub name: String,
    /// What it plays.
    pub motion: Motion,
    /// Playback speed multiplier.
    #[serde(default = "one")]
    pub speed: f32,
}

fn one() -> f32 {
    1.0
}

/// A transition between states.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    /// Source state name; `None` means "from any state".
    pub from: Option<String>,
    /// Target state name.
    pub to: String,
    /// All must hold.
    #[serde(default)]
    pub conditions: Vec<Condition>,
    /// Crossfade length in seconds.
    #[serde(default)]
    pub blend_time: f32,
    /// Only fire after this fraction of the source state has played (e.g. `1.0` = when the clip
    /// finishes once).
    #[serde(default)]
    pub exit_time: Option<f32>,
}

/// A state machine definition (plain data, saved as text).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateMachine {
    /// Parameters.
    pub parameters: Vec<Parameter>,
    /// States; the machine starts in `entry`.
    pub states: Vec<State>,
    /// Name of the first state.
    pub entry: String,
    /// Transitions, checked in order.
    pub transitions: Vec<Transition>,
}

/// One clip contribution to the final pose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionSample {
    /// Index into the clip list given to [`StateMachinePlayer::new`].
    pub clip: usize,
    /// Clip time in seconds (already wrapped by the clip's loop mode).
    pub time: f32,
    /// Weight; all samples of one update add up to 1.
    pub weight: f32,
}

#[derive(Debug, Clone)]
enum CMotion {
    Clip(usize),
    Blend {
        param: usize,
        points: Vec<(f32, usize)>,
    },
}

#[derive(Debug, Clone)]
struct CState {
    motion: CMotion,
    speed: f32,
}

#[derive(Debug, Clone)]
enum CCondition {
    Is(usize, bool),
    Greater(usize, f32),
    Less(usize, f32),
    Triggered(usize),
}

#[derive(Debug, Clone)]
struct CTransition {
    from: Option<usize>,
    to: usize,
    conditions: Vec<CCondition>,
    blend_time: f32,
    exit_time: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PValue {
    Bool(bool),
    Float(f32),
    Trigger(bool),
}

#[derive(Debug, Clone, Copy)]
struct Active {
    state: usize,
    /// Normalised progress: 1.0 = one full play of the state's (first) clip.
    progress: f32,
}

#[derive(Debug, Clone)]
struct Fading {
    samples: Vec<MotionSample>,
    elapsed: f32,
    duration: f32,
}

/// Runs a [`StateMachine`] over a fixed list of clips.
#[derive(Debug, Clone)]
pub struct StateMachinePlayer {
    clips: Vec<Arc<Clip>>,
    states: Vec<CState>,
    state_names: Vec<String>,
    transitions: Vec<CTransition>,
    param_index: HashMap<String, usize>,
    params: Vec<PValue>,
    current: Active,
    fading: Option<Fading>,
    rate: f32,
}

impl StateMachinePlayer {
    /// Checks every name in `machine` against `clips` and its own parameters and states.
    ///
    /// # Errors
    /// [`AnimError::UnknownState`], [`AnimError::UnknownClip`], [`AnimError::UnknownParameter`]
    /// or [`AnimError::ParameterType`] for the first problem found.
    pub fn new(machine: &StateMachine, clips: Vec<Arc<Clip>>) -> Result<Self, AnimError> {
        let clip_index: HashMap<&str, usize> = clips
            .iter()
            .enumerate()
            .map(|(i, c)| (c.name.as_str(), i))
            .collect();
        let find_clip = |n: &str| {
            clip_index
                .get(n)
                .copied()
                .ok_or_else(|| AnimError::UnknownClip(n.to_owned()))
        };
        let mut param_index = HashMap::new();
        let mut params = Vec::new();
        for p in &machine.parameters {
            param_index.insert(p.name.clone(), params.len());
            params.push(match p.default {
                ParameterValue::Bool(b) => PValue::Bool(b),
                ParameterValue::Float(f) => PValue::Float(f),
                ParameterValue::Trigger => PValue::Trigger(false),
            });
        }
        let find_param = |n: &str, want: &str| -> Result<usize, AnimError> {
            let i = *param_index
                .get(n)
                .ok_or_else(|| AnimError::UnknownParameter(n.to_owned()))?;
            let ok = matches!(
                (params[i], want),
                (PValue::Bool(_), "bool")
                    | (PValue::Float(_), "float")
                    | (PValue::Trigger(_), "trigger")
            );
            if ok {
                Ok(i)
            } else {
                Err(AnimError::ParameterType(n.to_owned()))
            }
        };
        let state_names: Vec<String> = machine.states.iter().map(|s| s.name.clone()).collect();
        let find_state = |n: &str| {
            state_names
                .iter()
                .position(|s| s == n)
                .ok_or_else(|| AnimError::UnknownState(n.to_owned()))
        };
        let mut states = Vec::new();
        for s in &machine.states {
            let motion = match &s.motion {
                Motion::Clip(c) => CMotion::Clip(find_clip(c)?),
                Motion::Blend1D { parameter, points } => {
                    let mut pts = Vec::new();
                    for (pos, c) in points {
                        pts.push((*pos, find_clip(c)?));
                    }
                    if pts.is_empty() {
                        return Err(AnimError::InvalidClip(format!(
                            "state `{}` has an empty blend",
                            s.name
                        )));
                    }
                    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
                    CMotion::Blend {
                        param: find_param(parameter, "float")?,
                        points: pts,
                    }
                }
            };
            states.push(CState {
                motion,
                speed: s.speed,
            });
        }
        let mut transitions = Vec::new();
        for t in &machine.transitions {
            let from = match &t.from {
                Some(n) => Some(find_state(n)?),
                None => None,
            };
            let mut conditions = Vec::new();
            for c in &t.conditions {
                conditions.push(match c {
                    Condition::Is(p, v) => CCondition::Is(find_param(p, "bool")?, *v),
                    Condition::Greater(p, v) => CCondition::Greater(find_param(p, "float")?, *v),
                    Condition::Less(p, v) => CCondition::Less(find_param(p, "float")?, *v),
                    Condition::Triggered(p) => CCondition::Triggered(find_param(p, "trigger")?),
                });
            }
            transitions.push(CTransition {
                from,
                to: find_state(&t.to)?,
                conditions,
                blend_time: t.blend_time.max(0.0),
                exit_time: t.exit_time,
            });
        }
        let entry = find_state(&machine.entry)?;
        Ok(Self {
            clips,
            states,
            state_names,
            transitions,
            param_index,
            params,
            current: Active {
                state: entry,
                progress: 0.0,
            },
            fading: None,
            rate: 1.0,
        })
    }

    fn param(&mut self, name: &str) -> Result<&mut PValue, AnimError> {
        let i = *self
            .param_index
            .get(name)
            .ok_or_else(|| AnimError::UnknownParameter(name.to_owned()))?;
        self.params
            .get_mut(i)
            .ok_or_else(|| AnimError::UnknownParameter(name.to_owned()))
    }

    /// Sets a bool parameter.
    ///
    /// # Errors
    /// Unknown name or not a bool.
    pub fn set_bool(&mut self, name: &str, value: bool) -> Result<(), AnimError> {
        match self.param(name)? {
            PValue::Bool(b) => {
                *b = value;
                Ok(())
            }
            _ => Err(AnimError::ParameterType(name.to_owned())),
        }
    }

    /// Sets a float parameter.
    ///
    /// # Errors
    /// Unknown name or not a float.
    pub fn set_float(&mut self, name: &str, value: f32) -> Result<(), AnimError> {
        match self.param(name)? {
            PValue::Float(f) => {
                *f = value;
                Ok(())
            }
            _ => Err(AnimError::ParameterType(name.to_owned())),
        }
    }

    /// Fires a trigger parameter.
    ///
    /// # Errors
    /// Unknown name or not a trigger.
    pub fn fire(&mut self, name: &str) -> Result<(), AnimError> {
        match self.param(name)? {
            PValue::Trigger(t) => {
                *t = true;
                Ok(())
            }
            _ => Err(AnimError::ParameterType(name.to_owned())),
        }
    }

    /// Scales how fast states play (default 1): `2.0` plays every clip twice as fast. Gameplay
    /// code sets it each frame to match motion to the clip, e.g. a walk cycle to the real speed.
    /// Negative and non-finite values count as 0.
    pub fn set_playback_rate(&mut self, rate: f32) {
        self.rate = if rate.is_finite() { rate.max(0.0) } else { 0.0 };
    }

    /// Name of the current state.
    pub fn current_state(&self) -> &str {
        self.state_names
            .get(self.current.state)
            .map_or("", String::as_str)
    }

    /// True while a crossfade is running.
    pub fn is_blending(&self) -> bool {
        self.fading.is_some()
    }

    /// Advances by `dt` seconds and fires at most one transition. Returns the name of the state
    /// entered, if any.
    pub fn update(&mut self, dt: f32) -> Option<String> {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        let len = self.state_length(self.current.state);
        let speed = self.states.get(self.current.state).map_or(1.0, |s| s.speed);
        self.current.progress += if len > 0.0 {
            dt * speed * self.rate / len
        } else {
            0.0
        };
        if let Some(f) = &mut self.fading {
            f.elapsed += dt;
            if f.elapsed >= f.duration {
                self.fading = None;
            }
        }
        let mut entered = None;
        let chosen = self.transitions.iter().enumerate().find(|(_, t)| {
            let source_ok = match t.from {
                Some(s) => s == self.current.state,
                None => t.to != self.current.state,
            };
            if !source_ok {
                return false;
            }
            if t.conditions.is_empty() && t.exit_time.is_none() {
                return false;
            }
            let exit_ok = match t.exit_time {
                Some(e) => self.current.progress >= e,
                None => true,
            };
            exit_ok && t.conditions.iter().all(|c| self.check(c))
        });
        if let Some((ti, _)) = chosen {
            let t = self.transitions[ti].clone();
            // Consume the triggers this transition used.
            for c in &t.conditions {
                if let CCondition::Triggered(p) = c
                    && let Some(PValue::Trigger(v)) = self.params.get_mut(*p)
                {
                    *v = false;
                }
            }
            let outgoing = self.samples();
            self.fading = if t.blend_time > 0.0 {
                Some(Fading {
                    samples: outgoing,
                    elapsed: 0.0,
                    duration: t.blend_time,
                })
            } else {
                None
            };
            self.current = Active {
                state: t.to,
                progress: 0.0,
            };
            entered = self.state_names.get(t.to).cloned();
        }
        // Unused triggers expire.
        for p in &mut self.params {
            if let PValue::Trigger(v) = p {
                *v = false;
            }
        }
        entered
    }

    fn check(&self, c: &CCondition) -> bool {
        match *c {
            CCondition::Is(p, v) => self.params.get(p) == Some(&PValue::Bool(v)),
            CCondition::Greater(p, v) => {
                matches!(self.params.get(p), Some(PValue::Float(f)) if *f > v)
            }
            CCondition::Less(p, v) => {
                matches!(self.params.get(p), Some(PValue::Float(f)) if *f < v)
            }
            CCondition::Triggered(p) => self.params.get(p) == Some(&PValue::Trigger(true)),
        }
    }

    fn float_param(&self, i: usize) -> f32 {
        match self.params.get(i) {
            Some(PValue::Float(f)) => *f,
            _ => 0.0,
        }
    }

    /// Length in seconds that defines 100% progress of a state: its clip, or for a blend the
    /// weighted length of the two active clips.
    fn state_length(&self, state: usize) -> f32 {
        match self.states.get(state).map(|s| &s.motion) {
            Some(CMotion::Clip(c)) => self.clips.get(*c).map_or(0.0, |c| c.duration),
            Some(CMotion::Blend { param, points }) => {
                let (a, b, w) = blend_pick(points, self.float_param(*param));
                let la = self.clips.get(a).map_or(0.0, |c| c.duration);
                let lb = self.clips.get(b).map_or(0.0, |c| c.duration);
                la + (lb - la) * w
            }
            None => 0.0,
        }
    }

    fn samples(&self) -> Vec<MotionSample> {
        let mut out = Vec::new();
        let p = self.current.progress;
        let at = |clip: usize, weight: f32| {
            let c = self.clips.get(clip);
            let t = c.map_or(0.0, |c| c.local_time(p * c.duration));
            MotionSample {
                clip,
                time: t,
                weight,
            }
        };
        match self.states.get(self.current.state).map(|s| &s.motion) {
            Some(CMotion::Clip(c)) => out.push(at(*c, 1.0)),
            Some(CMotion::Blend { param, points }) => {
                let (a, b, w) = blend_pick(points, self.float_param(*param));
                if a == b || w <= 0.0 {
                    out.push(at(a, 1.0));
                } else if w >= 1.0 {
                    out.push(at(b, 1.0));
                } else {
                    out.push(at(a, 1.0 - w));
                    out.push(at(b, w));
                }
            }
            None => {}
        }
        out
    }

    /// The clips to blend for the current frame, with weights adding up to 1. During a
    /// crossfade the outgoing mix fades out. It is frozen at the moment the transition fired, so
    /// the fade needs no second playhead.
    pub fn motion_samples(&self) -> Vec<MotionSample> {
        let mut current = self.samples();
        if let Some(f) = &self.fading {
            let u = if f.duration > 0.0 {
                (f.elapsed / f.duration).clamp(0.0, 1.0)
            } else {
                1.0
            };
            // Smoothstep, so a fade eases in and out instead of starting and stopping at full rate.
            let w_in = u * u * (3.0 - 2.0 * u);
            for s in &mut current {
                s.weight *= w_in;
            }
            current.extend(f.samples.iter().map(|s| MotionSample {
                weight: s.weight * (1.0 - w_in),
                ..*s
            }));
        }
        current.retain(|s| s.weight > 0.0);
        current
    }

    /// The clips, as given to [`StateMachinePlayer::new`].
    pub fn clips(&self) -> &[Arc<Clip>] {
        &self.clips
    }
}

/// The two points around `x` and the blend factor between them.
fn blend_pick(points: &[(f32, usize)], x: f32) -> (usize, usize, f32) {
    let Some(first) = points.first() else {
        return (0, 0, 0.0);
    };
    if x <= first.0 {
        return (first.1, first.1, 0.0);
    }
    for w in points.windows(2) {
        let (p0, c0) = w[0];
        let (p1, c1) = w[1];
        if x <= p1 {
            let span = p1 - p0;
            let t = if span > 0.0 { (x - p0) / span } else { 1.0 };
            return (c0, c1, t);
        }
    }
    let last = points.last().map_or(first.1, |p| p.1);
    (last, last, 0.0)
}

/// Blends `samples` (from [`StateMachinePlayer::motion_samples`]) into `pose`, starting from the
/// rest pose. `bindings[i]` must be the binding of `clips[i]` for this skeleton; `scratch` is a
/// reusable work pose.
pub fn blend_motion_samples(
    samples: &[MotionSample],
    clips: &[Arc<Clip>],
    bindings: &[crate::ClipBinding],
    skeleton: &crate::Skeleton,
    scratch: &mut crate::Pose,
    pose: &mut crate::Pose,
) {
    pose.reset_to_rest(skeleton);
    let mut accumulated = 0.0f32;
    for s in samples {
        let (Some(clip), Some(binding)) = (clips.get(s.clip), bindings.get(s.clip)) else {
            continue;
        };
        if s.weight <= 0.0 {
            continue;
        }
        scratch.reset_to_rest(skeleton);
        crate::sample_clip(clip, binding, skeleton, s.time, scratch);
        accumulated += s.weight;
        // Running weighted average: blend the new pose in by its share of the weight so far.
        pose.blend_toward(scratch, s.weight / accumulated, None);
    }
}
