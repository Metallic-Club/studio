//! [`Animator`]: plays clips on one skeleton with fades, weights, priorities, masks and
//! additive layers. The Roblox `Animator` / `AnimationTrack` model, written for Rust.
//!
//! **How tracks combine (each frame, per joint):**
//! 1. Start from the rest pose.
//! 2. Go through priorities from lowest ([`AnimationPriority::Core`]) to highest. Within one
//!    priority, the tracks that animate the joint are averaged by weight. The result is laid over
//!    what lower priorities produced with the level's total weight (capped at 1). So a full-weight
//!    `Action` track replaces `Movement` on the joints it animates and leaves the others alone.
//! 3. Additive tracks are added on top, in priority order.
//!
//! Fading in, fading out and [`Animator::adjust_weight`] all move a track's weight linearly over
//! the given time. A crossfade is "stop A with fade *t*, play B with fade *t*".

use crate::clip::{Clip, LoopMode};
use crate::sampling::{ClipBinding, sample_clip};
use crate::skeleton::{JointMask, Pose, Skeleton};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Layer order of a playing track. Higher priorities override lower ones on the joints they
/// animate. Same names and order as Roblox's `Enum.AnimationPriority`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub enum AnimationPriority {
    /// Lowest; default character idle poses.
    #[default]
    Core,
    /// Standing still.
    Idle,
    /// Walking, running, swimming.
    Movement,
    /// Emotes, attacks, tool use.
    Action,
    /// Above `Action`.
    Action2,
    /// Above `Action2`.
    Action3,
    /// Highest.
    Action4,
}

/// How a track is combined with the layers below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BlendMode {
    /// Blend towards this track's pose (normal animations).
    #[default]
    Override,
    /// Add this track's change from its first frame (breathing, leaning, recoil).
    Additive,
}

/// Options for [`Animator::play`]. Defaults: 0.1 s fade, weight 1, speed 1 (as in Roblox).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayOptions {
    /// Seconds to fade in from zero weight.
    pub fade_time: f32,
    /// Target weight.
    pub weight: f32,
    /// Playback speed; `2` is twice as fast, negative plays backwards.
    pub speed: f32,
}

impl Default for PlayOptions {
    fn default() -> Self {
        Self {
            fade_time: 0.1,
            weight: 1.0,
            speed: 1.0,
        }
    }
}

/// Handle to a track loaded into an [`Animator`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrackHandle(u32);

/// Something that happened during [`Animator::update`].
#[derive(Debug, Clone, PartialEq)]
pub enum AnimatorEvent {
    /// Playback crossed an event marker.
    Marker {
        /// Track that crossed it.
        track: TrackHandle,
        /// Marker name.
        name: String,
        /// Marker value.
        value: String,
    },
    /// A looping track wrapped around.
    Looped(TrackHandle),
    /// A non-looping track reached its end; it now fades out.
    Ended(TrackHandle),
    /// A track finished fading out and no longer plays.
    Stopped(TrackHandle),
}

#[derive(Debug, Clone)]
struct PlayingTrack {
    handle: TrackHandle,
    clip: Arc<Clip>,
    binding: ClipBinding,
    priority: AnimationPriority,
    blend_mode: BlendMode,
    mask: Option<JointMask>,
    additive_reference: Option<Pose>,
    playing: bool,
    stopping: bool,
    ended: bool,
    time: f32,
    speed: f32,
    weight: f32,
    target_weight: f32,
    fade_rate: f32,
    fade_time: f32,
}

/// Plays clips on one skeleton.
#[derive(Debug, Clone)]
pub struct Animator {
    skeleton: Arc<Skeleton>,
    tracks: Vec<PlayingTrack>,
    next_handle: u32,
    scratch: Pose,
    level: Pose,
    level_weight: Vec<f32>,
}

impl Animator {
    /// An animator for `skeleton`.
    pub fn new(skeleton: Arc<Skeleton>) -> Self {
        let rest = Pose::rest(&skeleton);
        let n = skeleton.len();
        Self {
            skeleton,
            tracks: Vec::new(),
            next_handle: 1,
            scratch: rest.clone(),
            level: rest,
            level_weight: vec![0.0; n],
        }
    }

    /// The skeleton.
    pub fn skeleton(&self) -> &Skeleton {
        &self.skeleton
    }

    /// Loads a clip as a stopped track (Roblox `Animator:LoadAnimation`).
    pub fn load(&mut self, clip: Arc<Clip>) -> TrackHandle {
        let handle = TrackHandle(self.next_handle);
        self.next_handle += 1;
        let binding = ClipBinding::new(&clip, &self.skeleton);
        self.tracks.push(PlayingTrack {
            handle,
            clip,
            binding,
            priority: AnimationPriority::default(),
            blend_mode: BlendMode::default(),
            mask: None,
            additive_reference: None,
            playing: false,
            stopping: false,
            ended: false,
            time: 0.0,
            speed: 1.0,
            weight: 0.0,
            target_weight: 0.0,
            fade_rate: 0.0,
            fade_time: 0.1,
        });
        handle
    }

    /// Removes a track entirely.
    pub fn unload(&mut self, track: TrackHandle) {
        self.tracks.retain(|t| t.handle != track);
    }

    fn get_mut(&mut self, track: TrackHandle) -> Option<&mut PlayingTrack> {
        self.tracks.iter_mut().find(|t| t.handle == track)
    }

    fn get(&self, track: TrackHandle) -> Option<&PlayingTrack> {
        self.tracks.iter().find(|t| t.handle == track)
    }

    /// Starts (or restarts) a track from the beginning, fading in.
    pub fn play(&mut self, track: TrackHandle, options: PlayOptions) {
        let skeleton = Arc::clone(&self.skeleton);
        if let Some(t) = self.get_mut(track) {
            let restart = !t.playing || t.stopping;
            t.playing = true;
            t.stopping = false;
            t.ended = false;
            if restart {
                t.time = if options.speed < 0.0 {
                    t.clip.duration
                } else {
                    0.0
                };
            }
            t.speed = options.speed;
            t.fade_time = options.fade_time.max(0.0);
            set_fade(t, options.weight, options.fade_time);
            if t.blend_mode == BlendMode::Additive && t.additive_reference.is_none() {
                t.additive_reference = Some(reference_pose(t, &skeleton));
            }
        }
    }

    /// Fades a track out over `fade_time` seconds, then stops it.
    pub fn stop(&mut self, track: TrackHandle, fade_time: f32) {
        if let Some(t) = self.get_mut(track)
            && t.playing
        {
            t.stopping = true;
            set_fade(t, 0.0, fade_time);
        }
    }

    /// Stops every playing track.
    pub fn stop_all(&mut self, fade_time: f32) {
        let handles: Vec<_> = self
            .tracks
            .iter()
            .filter(|t| t.playing)
            .map(|t| t.handle)
            .collect();
        for h in handles {
            self.stop(h, fade_time);
        }
    }

    /// Crossfade: fades `from` out and `to` in over the same time.
    pub fn crossfade(&mut self, from: TrackHandle, to: TrackHandle, fade_time: f32) {
        self.stop(from, fade_time);
        self.play(
            to,
            PlayOptions {
                fade_time,
                ..PlayOptions::default()
            },
        );
    }

    /// Moves a playing track's weight to `weight` over `fade_time` seconds.
    pub fn adjust_weight(&mut self, track: TrackHandle, weight: f32, fade_time: f32) {
        if let Some(t) = self.get_mut(track)
            && t.playing
            && !t.stopping
        {
            set_fade(t, weight, fade_time);
        }
    }

    /// Sets playback speed.
    pub fn adjust_speed(&mut self, track: TrackHandle, speed: f32) {
        if let Some(t) = self.get_mut(track) {
            t.speed = speed;
        }
    }

    /// Sets a track's priority.
    pub fn set_priority(&mut self, track: TrackHandle, priority: AnimationPriority) {
        if let Some(t) = self.get_mut(track) {
            t.priority = priority;
        }
    }

    /// Sets how a track combines with lower layers.
    pub fn set_blend_mode(&mut self, track: TrackHandle, mode: BlendMode) {
        let skeleton = Arc::clone(&self.skeleton);
        if let Some(t) = self.get_mut(track) {
            t.blend_mode = mode;
            t.additive_reference = if mode == BlendMode::Additive {
                Some(reference_pose(t, &skeleton))
            } else {
                None
            };
        }
    }

    /// Limits a track to some joints (`None` = all joints).
    pub fn set_mask(&mut self, track: TrackHandle, mask: Option<JointMask>) {
        if let Some(t) = self.get_mut(track) {
            t.mask = mask;
        }
    }

    /// Jumps to playback time `time` (seconds).
    pub fn set_time_position(&mut self, track: TrackHandle, time: f32) {
        if let Some(t) = self.get_mut(track) {
            t.time = time.max(0.0);
        }
    }

    /// Current clip time of a track (wrapped by its loop mode).
    pub fn time_position(&self, track: TrackHandle) -> Option<f32> {
        self.get(track).map(|t| t.clip.local_time(t.time))
    }

    /// Current effective weight (after fades).
    pub fn weight(&self, track: TrackHandle) -> Option<f32> {
        self.get(track).map(|t| t.weight)
    }

    /// True while a track plays (including while it fades out).
    pub fn is_playing(&self, track: TrackHandle) -> bool {
        self.get(track).is_some_and(|t| t.playing)
    }

    /// Advances time by `dt` seconds: moves playheads and fades, and reports markers, loops and
    /// stops in the order they happened per track.
    pub fn update(&mut self, dt: f32) -> Vec<AnimatorEvent> {
        let mut events = Vec::new();
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        for t in &mut self.tracks {
            if !t.playing {
                continue;
            }
            // Playhead.
            let d = t.clip.duration;
            let old = t.time;
            let mut new = old + dt * t.speed;
            if t.clip.loop_mode == LoopMode::Once && !t.ended && d > 0.0 {
                let reached_end = if t.speed >= 0.0 { new >= d } else { new <= 0.0 };
                if reached_end {
                    new = new.clamp(0.0, d);
                    t.ended = true;
                }
            }
            if t.speed >= 0.0 {
                let from = if old == 0.0 { -f32::EPSILON } else { old };
                for m in t.clip.markers_crossed(from, new) {
                    events.push(AnimatorEvent::Marker {
                        track: t.handle,
                        name: m.name.clone(),
                        value: m.value.clone(),
                    });
                }
            }
            if t.clip.loop_mode != LoopMode::Once
                && d > 0.0
                && (new / d).floor() != (old / d).floor()
            {
                events.push(AnimatorEvent::Looped(t.handle));
            }
            t.time = new;
            let mut just_ended = false;
            if t.ended && !t.stopping {
                events.push(AnimatorEvent::Ended(t.handle));
                t.stopping = true;
                just_ended = true;
                let fade = t.fade_time;
                set_fade(t, 0.0, fade);
            }
            // Weight fade (an end-of-clip fade-out starts on the next update, holding the last
            // frame, so a long frame cannot skip it).
            if t.weight != t.target_weight && !(just_ended && t.fade_rate > 0.0) {
                if t.fade_rate <= 0.0 {
                    t.weight = t.target_weight;
                } else if t.weight < t.target_weight {
                    t.weight = (t.weight + t.fade_rate * dt).min(t.target_weight);
                } else {
                    t.weight = (t.weight - t.fade_rate * dt).max(t.target_weight);
                }
            }
            if t.stopping && t.weight <= 0.0 {
                t.playing = false;
                t.stopping = false;
                events.push(AnimatorEvent::Stopped(t.handle));
            }
        }
        events
    }

    /// Writes the combined pose of all playing tracks into `pose`.
    pub fn evaluate(&mut self, pose: &mut Pose) {
        let skeleton = &*self.skeleton;
        pose.reset_to_rest(skeleton);
        let mut order: Vec<usize> = (0..self.tracks.len())
            .filter(|&i| self.tracks[i].playing && self.tracks[i].weight > 0.0)
            .collect();
        order.sort_by_key(|&i| {
            (
                self.tracks[i].blend_mode == BlendMode::Additive,
                self.tracks[i].priority,
            )
        });

        let mut i = 0;
        while i < order.len() {
            let first = &self.tracks[order[i]];
            if first.blend_mode == BlendMode::Additive {
                break;
            }
            let priority = first.priority;
            self.level.reset_to_rest(skeleton);
            self.level_weight.iter_mut().for_each(|w| *w = 0.0);
            while i < order.len()
                && self.tracks[order[i]].priority == priority
                && self.tracks[order[i]].blend_mode == BlendMode::Override
            {
                let t = &self.tracks[order[i]];
                self.scratch.reset_to_rest(skeleton);
                sample_clip(
                    &t.clip,
                    &t.binding,
                    skeleton,
                    t.clip.local_time(t.time),
                    &mut self.scratch,
                );
                for j in 0..skeleton.len() {
                    if !t.binding.animates(j) {
                        continue;
                    }
                    let w = t.weight * t.mask.as_ref().map_or(1.0, |m| m.weight(j));
                    if w <= 0.0 {
                        continue;
                    }
                    let lw = &mut self.level_weight[j];
                    if *lw <= 0.0 {
                        self.level.local[j] = self.scratch.local[j];
                        *lw = w;
                    } else {
                        let total = *lw + w;
                        self.level.local[j] =
                            self.level.local[j].lerp(&self.scratch.local[j], w / total);
                        *lw = total;
                    }
                }
                i += 1;
            }
            for j in 0..skeleton.len() {
                let lw = self.level_weight[j].min(1.0);
                if lw > 0.0 {
                    pose.local[j] = pose.local[j].lerp(&self.level.local[j], lw);
                }
            }
        }
        for &ti in &order[i..] {
            let t = &self.tracks[ti];
            let Some(reference) = &t.additive_reference else {
                continue;
            };
            self.scratch.reset_to_rest(skeleton);
            sample_clip(
                &t.clip,
                &t.binding,
                skeleton,
                t.clip.local_time(t.time),
                &mut self.scratch,
            );
            let delta = self.scratch.additive_delta(reference);
            pose.add_additive(&delta, t.weight, t.mask.as_ref());
        }
    }
}

fn set_fade(t: &mut PlayingTrack, target: f32, fade_time: f32) {
    t.target_weight = target.max(0.0);
    let span = (t.target_weight - t.weight).abs();
    t.fade_rate = if fade_time > 0.0 {
        span / fade_time
    } else {
        0.0
    };
}

fn reference_pose(t: &PlayingTrack, skeleton: &Skeleton) -> Pose {
    let mut p = Pose::rest(skeleton);
    sample_clip(&t.clip, &t.binding, skeleton, 0.0, &mut p);
    p
}
