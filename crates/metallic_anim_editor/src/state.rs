//! Editor state shared by the widgets: the clip being edited, playhead and playback, key
//! selection, view ranges, and the edit operations (move, scale, delete, interpolation).
//!
//! Widgets only read input and call these operations, so editing behaviour is testable without
//! a UI, and Studio can later route each operation through its undo `History`.

use metallic_anim::{Clip, Interpolation, KEY_TIME_EPSILON, TangentMode, with_curve};
use std::collections::BTreeSet;

/// One key: track index and key index within the track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyRef {
    /// Track index in the clip.
    pub track: usize,
    /// Key index in the track's curve.
    pub key: usize,
}

/// Horizontal mapping between time and screen x.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeView {
    /// Time (seconds) at the left edge of the key area.
    pub start: f32,
    /// Zoom: pixels per second.
    pub px_per_sec: f32,
}

impl Default for TimeView {
    fn default() -> Self {
        Self {
            start: -0.1,
            px_per_sec: 300.0,
        }
    }
}

impl TimeView {
    /// Screen x of `time`, given the key area's left edge.
    pub fn x(&self, left: f32, time: f32) -> f32 {
        left + (time - self.start) * self.px_per_sec
    }

    /// Time at screen x.
    pub fn time(&self, left: f32, x: f32) -> f32 {
        self.start + (x - left) / self.px_per_sec
    }

    /// Zooms by `factor` keeping the time under `anchor_x` fixed.
    pub fn zoom_at(&mut self, left: f32, anchor_x: f32, factor: f32) {
        let t = self.time(left, anchor_x);
        self.px_per_sec = (self.px_per_sec * factor).clamp(20.0, 6000.0);
        self.start = t - (anchor_x - left) / self.px_per_sec;
    }

    /// Fits `[t0, t1]` into `width` pixels with a small margin.
    pub fn fit(&mut self, t0: f32, t1: f32, width: f32) {
        let span = (t1 - t0).max(0.05);
        let margin = span * 0.04;
        self.px_per_sec = (width / (span + 2.0 * margin)).clamp(20.0, 6000.0);
        self.start = t0 - margin;
    }
}

/// Vertical value range of the curve editor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValueView {
    /// Value at the bottom edge.
    pub min: f32,
    /// Value at the top edge.
    pub max: f32,
}

impl Default for ValueView {
    fn default() -> Self {
        Self {
            min: -1.5,
            max: 1.5,
        }
    }
}

/// A drag in progress (kept in the state so a drag survives frames and can be tested).
#[derive(Debug, Clone, PartialEq)]
pub enum DragOp {
    /// Moving selected keys by `delta` seconds (applied on release).
    MoveKeys {
        /// Time under the pointer when the drag started.
        grab_time: f32,
        /// Current offset in seconds (snapped).
        delta: f32,
    },
    /// Scaling selected keys in time about `pivot` (applied on release).
    ScaleKeys {
        /// Fixed time.
        pivot: f32,
        /// The selection end that was grabbed.
        grabbed: f32,
        /// Current scale factor.
        factor: f32,
    },
    /// Rubber-band selection in screen space.
    BoxSelect {
        /// Start corner.
        from: egui::Pos2,
        /// Current corner.
        to: egui::Pos2,
        /// Shift held: add to the selection.
        additive: bool,
    },
    /// Scrubbing the playhead.
    Scrub,
    /// Moving keys in the curve editor (time and value), applied live.
    CurveKeys {
        /// Component being edited.
        comp: usize,
        /// Pointer position (time, value) at the start.
        grab: (f32, f32),
        /// Original (time, value) of each selected key.
        originals: Vec<(KeyRef, f32, f32)>,
    },
    /// Dragging a Bezier handle in the curve editor.
    Handle {
        /// The key.
        key: KeyRef,
        /// Component.
        comp: usize,
        /// Which handle.
        side: crate::scalar::HandleSide,
    },
}

/// Everything the animation widgets edit and display.
#[derive(Debug, Clone)]
pub struct EditorState {
    /// The clip being edited.
    pub clip: Clip,
    /// Playhead time in seconds.
    pub time: f32,
    /// Playing or paused.
    pub playing: bool,
    /// Loop playback in the editor (independent of the clip's own loop mode).
    pub looping: bool,
    /// Playback speed multiplier.
    pub speed: f32,
    /// Snap key moves and scrubbing to frames.
    pub snap: bool,
    /// Record keys (K). Off by default: value edits only create keys while this is on (no
    /// silent auto-key).
    pub recording: bool,
    /// Selected keys.
    pub selection: BTreeSet<KeyRef>,
    /// Track shown in the curve editor and highlighted in the preview.
    pub active_track: Option<usize>,
    /// Time axis shared by the dope sheet and curve editor.
    pub time_view: TimeView,
    /// Value axis of the curve editor.
    pub value_view: ValueView,
    /// Current drag, if any.
    pub drag: Option<DragOp>,
    /// Set when the clip changed this frame (for autosave or undo hooks).
    pub dirty: bool,
}

impl EditorState {
    /// Starts editing `clip` with the playhead at 0.
    pub fn new(clip: Clip) -> Self {
        let active_track = if clip.tracks.is_empty() {
            None
        } else {
            Some(0)
        };
        Self {
            clip,
            time: 0.0,
            playing: false,
            looping: true,
            speed: 1.0,
            snap: true,
            recording: false,
            selection: BTreeSet::new(),
            active_track,
            time_view: TimeView::default(),
            value_view: ValueView::default(),
            drag: None,
            dirty: false,
        }
    }

    /// Advances playback by `dt` seconds of wall time.
    pub fn tick(&mut self, dt: f32) {
        if !self.playing {
            return;
        }
        let d = self.clip.duration.max(1e-3);
        self.time += dt * self.speed;
        if self.time > d || self.time < 0.0 {
            if self.looping {
                self.time = self.time.rem_euclid(d);
            } else {
                self.time = self.time.clamp(0.0, d);
                self.playing = false;
            }
        }
    }

    /// Snaps a time to frames when snapping is on.
    pub fn snap_time(&self, t: f32) -> f32 {
        if self.snap {
            self.clip.snap_to_frame(t)
        } else {
            t
        }
    }

    /// Current frame number.
    pub fn frame(&self) -> i64 {
        (self.time * self.clip.frame_rate).round() as i64
    }

    /// Time of key `k`.
    pub fn key_time(&self, k: KeyRef) -> Option<f32> {
        self.clip
            .tracks
            .get(k.track)
            .and_then(|t| t.channel.key_time(k.key))
    }

    /// All keys of the clip.
    pub fn all_keys(&self) -> Vec<KeyRef> {
        let mut out = Vec::new();
        for (ti, t) in self.clip.tracks.iter().enumerate() {
            for key in 0..t.channel.key_count() {
                out.push(KeyRef { track: ti, key });
            }
        }
        out
    }

    /// Time span of the selection.
    pub fn selection_span(&self) -> Option<(f32, f32)> {
        let mut it = self.selection.iter().filter_map(|k| self.key_time(*k));
        let first = it.next()?;
        Some(it.fold((first, first), |(a, b), t| (a.min(t), b.max(t))))
    }

    /// Next key time after the playhead (any track), if any.
    pub fn next_key_time(&self) -> Option<f32> {
        self.all_keys()
            .iter()
            .filter_map(|k| self.key_time(*k))
            .filter(|t| *t > self.time + KEY_TIME_EPSILON)
            .min_by(f32::total_cmp)
    }

    /// Previous key time before the playhead, if any.
    pub fn prev_key_time(&self) -> Option<f32> {
        self.all_keys()
            .iter()
            .filter_map(|k| self.key_time(*k))
            .filter(|t| *t < self.time - KEY_TIME_EPSILON)
            .max_by(f32::total_cmp)
    }

    /// Applies new times to keys (any tracks), merging collisions so moved keys win, and keeps
    /// the selection pointing at the moved keys.
    pub fn retime_keys(&mut self, new_times: &[(KeyRef, f32)]) {
        let tracks: BTreeSet<usize> = new_times.iter().map(|(k, _)| k.track).collect();
        let mut new_selection: BTreeSet<KeyRef> = self
            .selection
            .iter()
            .filter(|k| !tracks.contains(&k.track))
            .copied()
            .collect();
        for ti in tracks {
            let Some(track) = self.clip.tracks.get_mut(ti) else {
                continue;
            };
            let mut moves: Vec<(usize, f32)> = new_times
                .iter()
                .filter(|(k, _)| k.track == ti)
                .map(|(k, t)| (k.key, if t.is_finite() { t.max(0.0) } else { 0.0 }))
                .collect();
            // Remove stationary keys that a moved key lands on (the moved key wins).
            let moving: BTreeSet<usize> = moves.iter().map(|m| m.0).collect();
            let times = track.channel.key_times();
            let mut doomed: Vec<usize> = (0..times.len())
                .filter(|i| !moving.contains(i))
                .filter(|&i| {
                    moves
                        .iter()
                        .any(|&(_, t)| (times[i] - t).abs() <= KEY_TIME_EPSILON)
                })
                .collect();
            doomed.sort_unstable_by(|a, b| b.cmp(a));
            for &d in &doomed {
                track.channel.remove_key(d);
                for m in &mut moves {
                    if m.0 > d {
                        m.0 -= 1;
                    }
                }
            }
            let moved_keys: Vec<usize> = moves.iter().map(|m| m.0).collect();
            let remap = track.channel.set_key_times(&moves);
            for k in moved_keys {
                if let Some(&nk) = remap.get(k) {
                    new_selection.insert(KeyRef { track: ti, key: nk });
                }
            }
        }
        self.selection = new_selection;
        self.dirty = true;
    }

    /// Moves the selected keys by `delta` seconds (snapped per key when snapping is on).
    pub fn move_selected(&mut self, delta: f32) {
        let moves: Vec<(KeyRef, f32)> = self
            .selection
            .iter()
            .filter_map(|k| self.key_time(*k).map(|t| (*k, self.snap_time(t + delta))))
            .collect();
        self.retime_keys(&moves);
    }

    /// Scales the selected keys' times about `pivot` by `factor` (snapped when snapping is on).
    pub fn scale_selected(&mut self, pivot: f32, factor: f32) {
        let factor = if factor.is_finite() {
            factor.max(0.0)
        } else {
            1.0
        };
        let moves: Vec<(KeyRef, f32)> = self
            .selection
            .iter()
            .filter_map(|k| {
                self.key_time(*k)
                    .map(|t| (*k, self.snap_time(pivot + (t - pivot) * factor)))
            })
            .collect();
        self.retime_keys(&moves);
    }

    /// Deletes the selected keys.
    pub fn delete_selected(&mut self) {
        let mut keys: Vec<KeyRef> = self.selection.iter().copied().collect();
        keys.sort_by(|a, b| b.cmp(a));
        for k in keys {
            if let Some(t) = self.clip.tracks.get_mut(k.track) {
                t.channel.remove_key(k.key);
            }
        }
        self.selection.clear();
        self.dirty = true;
    }

    /// Sets the interpolation of every selected key.
    pub fn set_selected_interpolation(&mut self, interp: Interpolation) {
        for k in self.selection.clone() {
            if let Some(t) = self.clip.tracks.get_mut(k.track) {
                t.channel.set_key_interpolation(k.key, interp);
            }
        }
        self.dirty = true;
    }

    /// Sets the tangent mode of every selected key.
    pub fn set_selected_tangent_mode(&mut self, mode: TangentMode) {
        for k in self.selection.clone() {
            if let Some(t) = self.clip.tracks.get_mut(k.track) {
                t.channel.set_key_tangent_mode(k.key, mode);
            }
        }
        self.dirty = true;
    }

    /// Adds a key at the playhead on `track` (holding the current value) and selects it
    /// ("Key selected", I). Returns the key's index.
    pub fn key_track_at_playhead(&mut self, track: usize) -> Option<usize> {
        let time = self.snap_time(self.time);
        let t = self.clip.tracks.get_mut(track)?;
        if let Some(existing) = t
            .channel
            .key_times()
            .iter()
            .position(|kt| (kt - time).abs() <= KEY_TIME_EPSILON)
        {
            self.selection.insert(KeyRef {
                track,
                key: existing,
            });
            return Some(existing);
        }
        let key = t.channel.insert_key_at(time)?;
        // Indices after the new key shift by one.
        self.selection = self
            .selection
            .iter()
            .map(|k| {
                if k.track == track && k.key >= key {
                    KeyRef {
                        key: k.key + 1,
                        ..*k
                    }
                } else {
                    *k
                }
            })
            .collect();
        self.selection.insert(KeyRef { track, key });
        self.dirty = true;
        Some(key)
    }

    /// Sets component `comp` of `track` at the playhead to `value`, creating or updating the key
    /// there. Only works while [`EditorState::recording`] is on; returns whether it keyed.
    pub fn record_value(&mut self, track: usize, comp: usize, value: f32) -> bool {
        if !self.recording {
            return false;
        }
        let Some(key) = self.key_track_at_playhead(track) else {
            return false;
        };
        if let Some(t) = self.clip.tracks.get_mut(track) {
            crate::scalar::set_key_value(&mut t.channel, key, comp, value);
            self.dirty = true;
            return true;
        }
        false
    }

    /// Interpolation shared by all selected keys, if they agree.
    pub fn selected_interpolation(&self) -> Option<Interpolation> {
        let mut it = self.selection.iter().filter_map(|k| {
            self.clip
                .tracks
                .get(k.track)
                .and_then(|t| t.channel.key_interpolation(k.key))
        });
        let first = it.next()?;
        it.all(|i| i == first).then_some(first)
    }

    /// Tangent mode shared by all selected keys, if they agree.
    pub fn selected_tangent_mode(&self) -> Option<TangentMode> {
        let mut it = self.selection.iter().filter_map(|k| {
            self.clip
                .tracks
                .get(k.track)
                .and_then(|t| t.channel.key_tangent_mode(k.key))
        });
        let first = it.next()?;
        it.all(|i| i == first).then_some(first)
    }

    /// Fits the curve editor's value range to the active track over the clip.
    pub fn frame_all_values(&mut self) {
        let Some(track) = self.active_track.and_then(|i| self.clip.tracks.get(i)) else {
            return;
        };
        let comps = crate::scalar::labels(&track.channel).len();
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        let end = self.clip.duration.max(self.clip.last_key_time());
        for c in 0..comps {
            for i in 0..=200 {
                if let Some(v) = crate::scalar::sample(&track.channel, c, end * i as f32 / 200.0) {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
            for k in 0..track.channel.key_count() {
                for side in [
                    crate::scalar::HandleSide::In,
                    crate::scalar::HandleSide::Out,
                ] {
                    if let Some((_, v)) = crate::scalar::handle_point(&track.channel, k, c, side) {
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                }
            }
        }
        if !lo.is_finite() || !hi.is_finite() {
            return;
        }
        let span = (hi - lo).max(0.1);
        self.value_view = ValueView {
            min: lo - span * 0.12,
            max: hi + span * 0.12,
        };
    }

    /// Number of keys in the clip.
    pub fn key_count(&self) -> usize {
        self.clip
            .tracks
            .iter()
            .map(|t| with_curve!(&t.channel, c => c.len()))
            .sum()
    }
}
