//! The audio-thread side: voices, groups, effects and the block mixer.
//!
//! [`MixRenderer`] owns all mixing state and is the only thing that runs on the audio thread
//! (or, headless, inside [`AudioEngine::render`](crate::AudioEngine::render)). The game thread
//! talks to it through a command channel and reads feedback from atomics, so neither side ever
//! waits for the other.
//!
//! Mixing runs in fixed blocks of [`BLOCK`] frames whatever the device buffer size, which makes
//! output bit-identical for the same commands no matter how callers slice `render` calls.
//! Commands are applied at block boundaries. The steady state does not allocate: buffers come
//! preallocated from the game thread and removed objects are sent back there to be freed.

use crate::asset::{SoundAsset, StreamSource};
use crate::effects::{Effect, EffectState, OnePole};
use crate::spatial::{OPEN_CUTOFF_HZ, spatialize};
use crate::types::{Emitter, Listener, RollOff, SpatialSettings};
use std::cmp::Ordering as CmpOrdering;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};

/// Frames per mix block (1.33 ms at 48 kHz). Commands and parameter changes land on block
/// boundaries; gain changes ramp linearly across one block.
pub const BLOCK: usize = 64;

/// Frames kept in a stream voice's decode window.
const STREAM_WINDOW: usize = 4096;
/// Frames retained when a stream window slides (for interpolation).
const STREAM_KEEP: usize = 2;

pub(crate) const STATE_STOPPED: u8 = 0;
pub(crate) const STATE_PLAYING: u8 = 1;
pub(crate) const STATE_PAUSED: u8 = 2;

/// Feedback of one sound from the audio thread to the game thread.
#[derive(Debug)]
pub(crate) struct SoundShared {
    /// Current position in seconds (f64 bits).
    pub(crate) position: AtomicU64,
    /// Length in seconds (f64 bits); NaN while unknown (a stream not yet read to the end).
    pub(crate) length: AtomicU64,
    pub(crate) state: AtomicU8,
    /// Sequence number of the last transport command applied.
    pub(crate) applied_seq: AtomicU64,
    pub(crate) loops: AtomicU32,
    pub(crate) ended: AtomicU32,
}

impl SoundShared {
    pub(crate) fn new(length_secs: Option<f64>) -> Self {
        Self {
            position: AtomicU64::new(0f64.to_bits()),
            length: AtomicU64::new(length_secs.unwrap_or(f64::NAN).to_bits()),
            state: AtomicU8::new(STATE_STOPPED),
            applied_seq: AtomicU64::new(0),
            loops: AtomicU32::new(0),
            ended: AtomicU32::new(0),
        }
    }

    pub(crate) fn position_secs(&self) -> f64 {
        f64::from_bits(self.position.load(Ordering::Acquire))
    }

    pub(crate) fn length_secs(&self) -> Option<f64> {
        let v = f64::from_bits(self.length.load(Ordering::Acquire));
        if v.is_nan() { None } else { Some(v) }
    }
}

/// Mixer-wide figures shared with the game thread.
#[derive(Debug, Default)]
pub(crate) struct SharedStats {
    pub(crate) real_voices: AtomicU32,
    pub(crate) virtual_voices: AtomicU32,
    pub(crate) frames: AtomicU64,
}

/// Decode window over a [`StreamSource`].
pub(crate) struct StreamVoice {
    source: Box<dyn StreamSource>,
    window: Vec<[f32; 2]>,
    scratch: Vec<f32>,
    start: u64,
    len: usize,
    eof: Option<u64>,
    failed: bool,
}

impl StreamVoice {
    pub(crate) fn new(source: Box<dyn StreamSource>) -> Self {
        let channels = source.channels().clamp(1, 2) as usize;
        let eof = source.length_frames();
        Self {
            source,
            window: vec![[0.0; 2]; STREAM_WINDOW],
            scratch: vec![0.0; STREAM_WINDOW * channels],
            start: 0,
            len: 0,
            eof,
            failed: false,
        }
    }

    fn fetch(&mut self, frame: u64) -> [f32; 2] {
        if frame >= self.start && frame < self.start + self.len as u64 {
            return self.window[(frame - self.start) as usize];
        }
        if self.failed || self.eof.is_some_and(|end| frame >= end) {
            return [0.0; 2];
        }
        if frame < self.start || frame >= self.start + (self.len + STREAM_WINDOW) as u64 {
            if self.source.seek(frame).is_err() {
                self.failed = true;
                return [0.0; 2];
            }
            self.start = frame;
            self.len = 0;
        }
        let channels = self.source.channels().clamp(1, 2) as usize;
        while frame >= self.start + self.len as u64 {
            if self.len == STREAM_WINDOW {
                self.window.copy_within(STREAM_WINDOW - STREAM_KEEP.., 0);
                self.start += (STREAM_WINDOW - STREAM_KEEP) as u64;
                self.len = STREAM_KEEP;
            }
            let room = STREAM_WINDOW - self.len;
            let read = match self.source.read(&mut self.scratch[..room * channels]) {
                Ok(n) => n.min(room),
                Err(_) => {
                    self.failed = true;
                    return [0.0; 2];
                }
            };
            if read == 0 {
                self.eof = Some(self.start + self.len as u64);
                return [0.0; 2];
            }
            for i in 0..read {
                self.window[self.len + i] = if channels == 1 {
                    let s = self.scratch[i];
                    [s, s]
                } else {
                    [self.scratch[2 * i], self.scratch[2 * i + 1]]
                };
            }
            self.len += read;
        }
        self.window[(frame - self.start) as usize]
    }
}

/// Where a voice's samples come from.
pub(crate) enum Source {
    Asset(Arc<SoundAsset>),
    Stream(Box<StreamVoice>),
}

impl Source {
    pub(crate) fn sample_rate(&self) -> u32 {
        match self {
            Source::Asset(a) => a.sample_rate(),
            Source::Stream(s) => s.source.sample_rate().max(1),
        }
    }

    fn len_frames(&self) -> Option<u64> {
        match self {
            Source::Asset(a) => Some(a.frames() as u64),
            Source::Stream(s) => s.eof,
        }
    }

    #[inline]
    fn fetch(&mut self, frame: u64) -> [f32; 2] {
        match self {
            Source::Asset(a) => a.frame(frame),
            Source::Stream(s) => s.fetch(frame),
        }
    }
}

/// One sound on the audio thread.
pub(crate) struct Voice {
    pub(crate) source: Source,
    pub(crate) shared: Arc<SoundShared>,
    pub(crate) state: u8,
    /// Position in source frames.
    pub(crate) pos: f64,
    pub(crate) volume: f32,
    pub(crate) speed: f32,
    pub(crate) looped: bool,
    pub(crate) group: usize,
    pub(crate) priority: i32,
    pub(crate) emitter: Option<Emitter>,
    pub(crate) roll_off: RollOff,
    pub(crate) occlusion: f32,
    loops: u32,
    start_order: u64,
    /// Channel gains reached at the end of the last mixed block.
    gains: [f32; 2],
    /// Just (re)started: jump straight to the target gain instead of ramping.
    fresh: bool,
    /// Paused or stopped while audible: mix one more block fading to silence.
    fading: bool,
    /// Where to continue after the fade (pause keeps it, stop resets to 0).
    resume_pos: f64,
    lpf: OnePole,
    // Per-block scratch.
    target: [f32; 2],
    step: f64,
    lpf_a: f32,
    audibility: f32,
    real: bool,
}

impl Voice {
    pub(crate) fn new(source: Source, shared: Arc<SoundShared>, group: usize) -> Self {
        Self {
            source,
            shared,
            state: STATE_STOPPED,
            pos: 0.0,
            volume: 0.5,
            speed: 1.0,
            looped: false,
            group,
            priority: 0,
            emitter: None,
            roll_off: RollOff::default(),
            occlusion: 0.0,
            loops: 0,
            start_order: 0,
            gains: [0.0; 2],
            fresh: false,
            fading: false,
            resume_pos: 0.0,
            lpf: OnePole::default(),
            target: [0.0; 2],
            step: 0.0,
            lpf_a: 1.0,
            audibility: 0.0,
            real: false,
        }
    }

    fn secs_to_frames(&self, secs: f64) -> f64 {
        let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
        let frames = secs * self.source.sample_rate() as f64;
        match self.source.len_frames() {
            Some(len) => frames.min(len as f64),
            None => frames,
        }
    }

    fn publish_position(&self) {
        let secs = self.pos / self.source.sample_rate() as f64;
        self.shared
            .position
            .store(secs.to_bits(), Ordering::Release);
    }
}

/// An effect inserted on a group.
pub(crate) struct EffectSlot {
    pub(crate) id: u32,
    pub(crate) priority: i32,
    pub(crate) enabled: bool,
    pub(crate) state: EffectState,
}

/// One mixer group (bus).
pub(crate) struct Group {
    pub(crate) parent: Option<usize>,
    pub(crate) volume: f32,
    current: f32,
    fresh: bool,
    bus: [[f32; 2]; BLOCK],
    // Boxed on purpose: slots are allocated on the game thread, moved in by pointer and sent
    // back there to be freed, so the audio thread never allocates or frees.
    #[allow(clippy::vec_box)]
    effects: Vec<Box<EffectSlot>>,
}

impl Group {
    pub(crate) fn new(parent: Option<usize>, volume: f32, max_effects: usize) -> Self {
        Self {
            parent,
            volume,
            current: volume,
            fresh: true,
            bus: [[0.0; 2]; BLOCK],
            effects: Vec::with_capacity(max_effects),
        }
    }
}

/// Messages from the game thread.
pub(crate) enum Command {
    AddSound {
        index: usize,
        voice: Box<Voice>,
    },
    RemoveSound {
        index: usize,
    },
    Play {
        index: usize,
        seq: u64,
        from_secs: f64,
    },
    Pause {
        index: usize,
        seq: u64,
    },
    Resume {
        index: usize,
        seq: u64,
    },
    Stop {
        index: usize,
        seq: u64,
    },
    Seek {
        index: usize,
        seq: u64,
        secs: f64,
    },
    SetVolume {
        index: usize,
        volume: f32,
    },
    SetSpeed {
        index: usize,
        speed: f32,
    },
    SetLooped {
        index: usize,
        looped: bool,
    },
    SetRollOff {
        index: usize,
        roll_off: RollOff,
    },
    SetEmitter {
        index: usize,
        emitter: Option<Emitter>,
    },
    SetGroup {
        index: usize,
        group: usize,
    },
    SetPriority {
        index: usize,
        priority: i32,
    },
    SetOcclusion {
        index: usize,
        occlusion: f32,
    },
    SetListener(Listener),
    SetSpatial(SpatialSettings),
    AddGroup {
        index: usize,
        group: Box<Group>,
    },
    RemoveGroup {
        index: usize,
    },
    SetGroupVolume {
        index: usize,
        volume: f32,
    },
    SetGroupParent {
        index: usize,
        parent: usize,
    },
    AddEffect {
        group: usize,
        slot: Box<EffectSlot>,
    },
    SetEffect {
        group: usize,
        id: u32,
        effect: Effect,
    },
    ReplaceEffect {
        group: usize,
        id: u32,
        state: Box<EffectState>,
    },
    SetEffectEnabled {
        group: usize,
        id: u32,
        enabled: bool,
    },
    RemoveEffect {
        group: usize,
        id: u32,
    },
}

/// Objects removed on the audio thread, sent back so the game thread frees them.
pub(crate) enum Trash {
    Voice(#[allow(dead_code)] Box<Voice>),
    Group(#[allow(dead_code)] Box<Group>),
    Effect(#[allow(dead_code)] Box<EffectSlot>),
    EffectState(#[allow(dead_code)] Box<EffectState>),
}

/// The mixing state, owned by the audio thread.
///
/// A backend calls [`MixRenderer::render`] whenever it needs samples. Exposed so custom
/// backends can be written outside this crate.
pub struct MixRenderer {
    sample_rate: u32,
    max_voices: usize,
    voices: Vec<Option<Box<Voice>>>,
    groups: Vec<Option<Box<Group>>>,
    listener: Listener,
    spatial: SpatialSettings,
    order: Vec<usize>,
    group_gain: Vec<f32>,
    start_counter: u64,
    commands: Receiver<Command>,
    trash: Sender<Trash>,
    stats: Arc<SharedStats>,
    block: [[f32; 2]; BLOCK],
    block_pos: usize,
    frames: u64,
}

impl std::fmt::Debug for MixRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MixRenderer")
            .field("sample_rate", &self.sample_rate)
            .field("max_voices", &self.max_voices)
            .field("frames", &self.frames)
            .finish_non_exhaustive()
    }
}

/// Construction parameters for [`MixRenderer`].
pub(crate) struct RendererParts {
    pub(crate) sample_rate: u32,
    pub(crate) max_voices: usize,
    pub(crate) max_sounds: usize,
    pub(crate) max_groups: usize,
    pub(crate) max_effects: usize,
    pub(crate) commands: Receiver<Command>,
    pub(crate) trash: Sender<Trash>,
    pub(crate) stats: Arc<SharedStats>,
}

impl MixRenderer {
    pub(crate) fn new(parts: RendererParts) -> Self {
        let mut groups: Vec<Option<Box<Group>>> = (0..parts.max_groups).map(|_| None).collect();
        for (i, slot) in groups
            .iter_mut()
            .enumerate()
            .take(crate::GroupId::BUILT_IN.len())
        {
            let parent = if i == 0 { None } else { Some(0) };
            *slot = Some(Box::new(Group::new(parent, 1.0, parts.max_effects)));
        }
        Self {
            sample_rate: parts.sample_rate,
            max_voices: parts.max_voices,
            voices: (0..parts.max_sounds).map(|_| None).collect(),
            groups,
            listener: Listener::default(),
            spatial: SpatialSettings::default(),
            order: Vec::with_capacity(parts.max_sounds),
            group_gain: vec![0.0; parts.max_groups],
            start_counter: 0,
            commands: parts.commands,
            trash: parts.trash,
            stats: parts.stats,
            block: [[0.0; 2]; BLOCK],
            block_pos: BLOCK,
            frames: 0,
        }
    }

    /// Output sample rate in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Fill `out` with interleaved stereo samples (`[l, r, l, r, ...]`), clamped to -1..=1.
    /// A trailing odd sample is set to 0.
    pub fn render(&mut self, out: &mut [f32]) {
        let (frames, rest) = out.as_chunks_mut::<2>();
        for frame in frames {
            if self.block_pos == BLOCK {
                self.apply_commands();
                let mut block = [[0.0f32; 2]; BLOCK];
                self.mix_block(&mut block);
                self.block = block;
                self.block_pos = 0;
            }
            let [l, r] = self.block[self.block_pos];
            frame[0] = l;
            frame[1] = r;
            self.block_pos += 1;
        }
        rest.fill(0.0);
    }

    fn discard(&self, trash: Trash) {
        // If the game thread is gone the object is simply dropped here.
        let _ = self.trash.send(trash);
    }

    fn voice(&mut self, index: usize) -> Option<&mut Voice> {
        self.voices.get_mut(index).and_then(|v| v.as_deref_mut())
    }

    fn group_mut(&mut self, index: usize) -> Option<&mut Group> {
        self.groups.get_mut(index).and_then(|g| g.as_deref_mut())
    }

    fn apply_commands(&mut self) {
        while let Ok(command) = self.commands.try_recv() {
            self.apply(command);
        }
    }

    fn apply(&mut self, command: Command) {
        match command {
            Command::AddSound { index, voice } => {
                if let Some(slot) = self.voices.get_mut(index) {
                    if let Some(old) = slot.replace(voice) {
                        self.discard(Trash::Voice(old));
                    }
                } else {
                    self.discard(Trash::Voice(voice));
                }
            }
            Command::RemoveSound { index } => {
                if let Some(old) = self.voices.get_mut(index).and_then(Option::take) {
                    self.discard(Trash::Voice(old));
                }
            }
            Command::Play {
                index,
                seq,
                from_secs,
            } => {
                self.start_counter += 1;
                let order = self.start_counter;
                if let Some(v) = self.voice(index) {
                    v.pos = v.secs_to_frames(from_secs);
                    v.state = STATE_PLAYING;
                    v.fresh = true;
                    v.fading = false;
                    v.start_order = order;
                    v.lpf = OnePole::default();
                    v.publish_position();
                    v.shared.state.store(STATE_PLAYING, Ordering::Release);
                    v.shared.applied_seq.store(seq, Ordering::Release);
                }
            }
            Command::Pause { index, seq } => {
                if let Some(v) = self.voice(index) {
                    if v.state == STATE_PLAYING {
                        v.state = STATE_PAUSED;
                        v.fading = v.gains != [0.0; 2];
                        v.resume_pos = v.pos;
                    }
                    v.publish_position();
                    v.shared.state.store(v.state, Ordering::Release);
                    v.shared.applied_seq.store(seq, Ordering::Release);
                }
            }
            Command::Resume { index, seq } => {
                self.start_counter += 1;
                let order = self.start_counter;
                if let Some(v) = self.voice(index) {
                    if v.state != STATE_PLAYING {
                        if v.fading {
                            v.fading = false;
                            v.pos = v.resume_pos;
                        }
                        v.state = STATE_PLAYING;
                        v.start_order = order;
                    }
                    v.shared.state.store(STATE_PLAYING, Ordering::Release);
                    v.shared.applied_seq.store(seq, Ordering::Release);
                }
            }
            Command::Stop { index, seq } => {
                if let Some(v) = self.voice(index) {
                    v.fading = v.state == STATE_PLAYING && v.gains != [0.0; 2];
                    v.state = STATE_STOPPED;
                    v.resume_pos = 0.0;
                    if !v.fading {
                        v.pos = 0.0;
                    }
                    v.shared.position.store(0f64.to_bits(), Ordering::Release);
                    v.shared.state.store(STATE_STOPPED, Ordering::Release);
                    v.shared.applied_seq.store(seq, Ordering::Release);
                }
            }
            Command::Seek { index, seq, secs } => {
                if let Some(v) = self.voice(index) {
                    let frames = v.secs_to_frames(secs);
                    if v.fading {
                        v.resume_pos = frames;
                    } else {
                        v.pos = frames;
                    }
                    let secs = frames / v.source.sample_rate() as f64;
                    v.shared.position.store(secs.to_bits(), Ordering::Release);
                    v.shared.applied_seq.store(seq, Ordering::Release);
                }
            }
            Command::SetVolume { index, volume } => {
                if let Some(v) = self.voice(index) {
                    v.volume = volume;
                }
            }
            Command::SetSpeed { index, speed } => {
                if let Some(v) = self.voice(index) {
                    v.speed = speed;
                }
            }
            Command::SetLooped { index, looped } => {
                if let Some(v) = self.voice(index) {
                    v.looped = looped;
                }
            }
            Command::SetRollOff { index, roll_off } => {
                if let Some(v) = self.voice(index) {
                    v.roll_off = roll_off;
                }
            }
            Command::SetEmitter { index, emitter } => {
                if let Some(v) = self.voice(index) {
                    v.emitter = emitter;
                }
            }
            Command::SetGroup { index, group } => {
                if let Some(v) = self.voice(index) {
                    v.group = group;
                }
            }
            Command::SetPriority { index, priority } => {
                if let Some(v) = self.voice(index) {
                    v.priority = priority;
                }
            }
            Command::SetOcclusion { index, occlusion } => {
                if let Some(v) = self.voice(index) {
                    v.occlusion = occlusion;
                }
            }
            Command::SetListener(listener) => self.listener = listener,
            Command::SetSpatial(spatial) => self.spatial = spatial,
            Command::AddGroup { index, group } => {
                if let Some(slot) = self.groups.get_mut(index) {
                    if let Some(old) = slot.replace(group) {
                        self.discard(Trash::Group(old));
                    }
                } else {
                    self.discard(Trash::Group(group));
                }
            }
            Command::RemoveGroup { index } => {
                if index != 0
                    && let Some(old) = self.groups.get_mut(index).and_then(Option::take)
                {
                    self.discard(Trash::Group(old));
                }
            }
            Command::SetGroupVolume { index, volume } => {
                if let Some(g) = self.group_mut(index) {
                    g.volume = volume;
                }
            }
            Command::SetGroupParent { index, parent } => {
                if parent < index
                    && let Some(g) = self.group_mut(index)
                {
                    g.parent = Some(parent);
                }
            }
            Command::AddEffect { group, slot } => {
                let rejected = match self.group_mut(group) {
                    Some(g) if g.effects.len() < g.effects.capacity() => {
                        // Higher priority runs first; equal priorities keep insertion order.
                        let at = g
                            .effects
                            .iter()
                            .position(|e| e.priority < slot.priority)
                            .unwrap_or(g.effects.len());
                        g.effects.insert(at, slot);
                        None
                    }
                    _ => Some(slot),
                };
                if let Some(slot) = rejected {
                    self.discard(Trash::Effect(slot));
                }
            }
            Command::SetEffect { group, id, effect } => {
                let rate = self.sample_rate;
                if let Some(slot) = self
                    .group_mut(group)
                    .and_then(|g| g.effects.iter_mut().find(|e| e.id == id))
                {
                    slot.state.set(&effect, rate);
                }
            }
            Command::ReplaceEffect { group, id, state } => {
                let old = match self
                    .group_mut(group)
                    .and_then(|g| g.effects.iter_mut().find(|e| e.id == id))
                {
                    Some(slot) => {
                        let mut state = state;
                        std::mem::swap(&mut slot.state, &mut *state);
                        state
                    }
                    None => state,
                };
                self.discard(Trash::EffectState(old));
            }
            Command::SetEffectEnabled { group, id, enabled } => {
                if let Some(slot) = self
                    .group_mut(group)
                    .and_then(|g| g.effects.iter_mut().find(|e| e.id == id))
                {
                    slot.enabled = enabled;
                }
            }
            Command::RemoveEffect { group, id } => {
                let removed = self.group_mut(group).and_then(|g| {
                    let at = g.effects.iter().position(|e| e.id == id)?;
                    Some(g.effects.remove(at))
                });
                if let Some(slot) = removed {
                    self.discard(Trash::Effect(slot));
                }
            }
        }
    }

    /// Mix one block of [`BLOCK`] frames into `out`.
    fn mix_block(&mut self, out: &mut [[f32; 2]; BLOCK]) {
        let fs = self.sample_rate as f32;
        for group in self.groups.iter_mut().flatten() {
            group.bus = [[0.0; 2]; BLOCK];
        }

        // Effective gain of every group chain (parents have lower indices).
        for i in 0..self.groups.len() {
            self.group_gain[i] = match &self.groups[i] {
                Some(g) => {
                    let parent = g
                        .parent
                        .filter(|p| *p < i)
                        .map_or(1.0, |p| self.group_gain[p]);
                    g.volume * parent
                }
                None => 0.0,
            };
        }

        // Targets of every playing voice.
        self.order.clear();
        for (i, slot) in self.voices.iter_mut().enumerate() {
            let Some(v) = slot.as_deref_mut() else {
                continue;
            };
            if v.state != STATE_PLAYING {
                continue;
            }
            if self.groups.get(v.group).is_none_or(Option::is_none) {
                v.group = 0;
            }
            let rate_ratio = v.source.sample_rate() as f64 / self.sample_rate as f64;
            let speed = v.speed as f64 * rate_ratio;
            match v.emitter {
                Some(emitter) => {
                    let s = spatialize(
                        &self.listener,
                        &emitter,
                        v.roll_off,
                        &self.spatial,
                        v.occlusion,
                    );
                    v.target = [v.volume * s.left, v.volume * s.right];
                    v.audibility = v.volume * s.gain * self.group_gain[v.group];
                    v.step = speed * s.doppler as f64;
                    v.lpf_a = if s.cutoff_hz >= OPEN_CUTOFF_HZ {
                        1.0
                    } else {
                        OnePole::coefficient(fs, s.cutoff_hz)
                    };
                }
                None => {
                    v.target = [v.volume; 2];
                    v.audibility = v.volume * self.group_gain[v.group];
                    v.step = speed;
                    v.lpf_a = 1.0;
                }
            }
            self.order.push(i);
        }

        // Voice limit: audible first, then priority, then loudness, then the earliest started.
        let voices = &self.voices;
        let key = |i: usize| {
            voices[i]
                .as_deref()
                .map(|v| (v.audibility > 1e-6, v.priority, v.audibility, v.start_order))
        };
        self.order
            .sort_unstable_by(|&a, &b| match (key(a), key(b)) {
                (Some(ka), Some(kb)) => {
                    kb.0.cmp(&ka.0)
                        .then(kb.1.cmp(&ka.1))
                        .then(kb.2.partial_cmp(&ka.2).unwrap_or(CmpOrdering::Equal))
                        .then(ka.3.cmp(&kb.3))
                }
                _ => CmpOrdering::Equal,
            });
        let mut real = 0u32;
        let mut virtual_count = 0u32;
        for (rank, &i) in self.order.iter().enumerate() {
            if let Some(v) = self.voices[i].as_deref_mut() {
                v.real = rank < self.max_voices && v.audibility > 1e-6;
                if v.real {
                    real += 1;
                } else {
                    virtual_count += 1;
                }
            }
        }

        // Mix voices into their group buses.
        let groups = &mut self.groups;
        for slot in self.voices.iter_mut() {
            let Some(v) = slot.as_deref_mut() else {
                continue;
            };
            let group = if groups.get(v.group).is_some_and(Option::is_some) {
                v.group
            } else {
                0
            };
            let Some(bus) = groups[group].as_deref_mut().map(|g| &mut g.bus) else {
                continue;
            };
            if v.fading {
                // One block fading to silence after pause/stop, then hold the saved position.
                let (from, step) = (v.gains, v.step);
                mix_samples(v, bus, from, [0.0; 2], step);
                v.gains = [0.0; 2];
                v.fading = false;
                v.pos = v.resume_pos;
                v.publish_position();
                continue;
            }
            if v.state != STATE_PLAYING {
                continue;
            }
            let target = if v.real { v.target } else { [0.0; 2] };
            if v.fresh {
                v.gains = target;
                v.fresh = false;
            }
            let outcome = if v.real || v.gains != [0.0; 2] {
                let from = v.gains;
                v.gains = target;
                let step = v.step;
                mix_samples(v, bus, from, target, step)
            } else {
                advance_virtual(v)
            };
            if outcome.loops > 0 {
                v.loops = v.loops.wrapping_add(outcome.loops);
                v.shared.loops.store(v.loops, Ordering::Release);
            }
            if let Some(len) = v.source.len_frames() {
                let secs = len as f64 / v.source.sample_rate() as f64;
                v.shared.length.store(secs.to_bits(), Ordering::Release);
            }
            if outcome.ended {
                v.state = STATE_STOPPED;
                v.pos = 0.0;
                v.gains = [0.0; 2];
                v.shared.state.store(STATE_STOPPED, Ordering::Release);
                v.shared.ended.fetch_add(1, Ordering::AcqRel);
            }
            v.publish_position();
        }

        // Groups, children before parents.
        for i in (1..self.groups.len()).rev() {
            let (lower, upper) = self.groups.split_at_mut(i);
            let Some(group) = upper[0].as_deref_mut() else {
                continue;
            };
            process_group(group);
            let parent = group
                .parent
                .filter(|p| *p < i && lower[*p].is_some())
                .unwrap_or(0);
            if let Some(parent) = lower[parent].as_deref_mut() {
                for (dst, src) in parent.bus.iter_mut().zip(group.bus.iter()) {
                    dst[0] += src[0];
                    dst[1] += src[1];
                }
            }
        }
        if let Some(master) = self.groups[0].as_deref_mut() {
            process_group(master);
            for (dst, src) in out.iter_mut().zip(master.bus.iter()) {
                dst[0] = src[0].clamp(-1.0, 1.0);
                dst[1] = src[1].clamp(-1.0, 1.0);
            }
        }

        self.frames += BLOCK as u64;
        self.stats.real_voices.store(real, Ordering::Relaxed);
        self.stats
            .virtual_voices
            .store(virtual_count, Ordering::Relaxed);
        self.stats.frames.store(self.frames, Ordering::Release);
    }
}

/// Run a group's effects, then its volume (ramped over the block).
fn process_group(group: &mut Group) {
    for slot in group.effects.iter_mut().filter(|e| e.enabled) {
        slot.state.process(&mut group.bus);
    }
    if group.fresh {
        group.current = group.volume;
        group.fresh = false;
    }
    let from = group.current;
    let to = group.volume;
    if from == to {
        if to != 1.0 {
            for frame in group.bus.iter_mut() {
                frame[0] *= to;
                frame[1] *= to;
            }
        }
    } else {
        let delta = (to - from) / BLOCK as f32;
        for (n, frame) in group.bus.iter_mut().enumerate() {
            let g = from + delta * (n + 1) as f32;
            frame[0] *= g;
            frame[1] *= g;
        }
    }
    group.current = to;
}

#[derive(Default)]
struct Outcome {
    loops: u32,
    ended: bool,
}

/// Advance a position by `delta` frames with loop/end handling.
fn advance(pos: &mut f64, delta: f64, looped: bool, len: Option<u64>, outcome: &mut Outcome) {
    *pos += delta;
    if let Some(len) = len {
        let len_f = len as f64;
        if *pos >= len_f {
            if looped && len > 0 {
                let wraps = (*pos / len_f).floor();
                *pos -= wraps * len_f;
                outcome.loops = outcome.loops.wrapping_add(wraps as u32);
            } else {
                *pos = len_f;
                outcome.ended = true;
            }
        }
    }
}

/// Over the voice limit: advance time without producing samples.
fn advance_virtual(v: &mut Voice) -> Outcome {
    let mut outcome = Outcome::default();
    let len = v.source.len_frames();
    advance(
        &mut v.pos,
        v.step * BLOCK as f64,
        v.looped,
        len,
        &mut outcome,
    );
    outcome
}

/// Resample, filter, pan and add one block of a voice into `bus`, ramping gains `from -> to`.
/// Assets take the [`mix_asset`] fast path; this generic loop serves streams.
fn mix_samples(
    v: &mut Voice,
    bus: &mut [[f32; 2]; BLOCK],
    from: [f32; 2],
    to: [f32; 2],
    step: f64,
) -> Outcome {
    let spatial = v.emitter.is_some();
    let (lpf_a, looped) = (v.lpf_a, v.looped);
    if let Source::Asset(asset) = &v.source {
        let ramp = Ramp {
            from,
            to,
            step,
            lpf_a,
            looped,
            spatial,
        };
        return mix_asset(asset, &mut v.pos, &mut v.lpf, bus, &ramp);
    }
    let mut outcome = Outcome::default();
    let delta = [
        (to[0] - from[0]) / BLOCK as f32,
        (to[1] - from[1]) / BLOCK as f32,
    ];
    for (n, frame) in bus.iter_mut().enumerate() {
        let base = v.pos.floor();
        let frac = (v.pos - base) as f32;
        let index = base as u64;
        let a = v.source.fetch(index);
        let len = v.source.len_frames();
        let next = index + 1;
        // Past the last frame a looped stream holds the last frame (fetching frame 0 would force
        // a seek); otherwise the sound fades to 0 like an asset.
        let b = match len {
            Some(len) if next >= len => {
                if looped {
                    a
                } else {
                    [0.0; 2]
                }
            }
            _ => v.source.fetch(next),
        };
        let mut s = [a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac];
        if spatial {
            let mono = 0.5 * (s[0] + s[1]);
            let mono = if lpf_a < 1.0 {
                v.lpf.tick(0, lpf_a, mono)
            } else {
                mono
            };
            s = [mono, mono];
        }
        let t = (n + 1) as f32;
        frame[0] += s[0] * (from[0] + delta[0] * t);
        frame[1] += s[1] * (from[1] + delta[1] * t);
        let len = v.source.len_frames();
        advance(&mut v.pos, step, looped, len, &mut outcome);
        if outcome.ended {
            break;
        }
    }
    outcome
}

/// Per-block mixing parameters of one voice.
struct Ramp {
    from: [f32; 2],
    to: [f32; 2],
    step: f64,
    lpf_a: f32,
    looped: bool,
    spatial: bool,
}

/// Fast path of [`mix_samples`] for in-memory assets (the common case): direct slice access,
/// no per-sample dispatch. Must produce exactly what the generic path would.
fn mix_asset(
    asset: &SoundAsset,
    pos: &mut f64,
    lpf: &mut OnePole,
    bus: &mut [[f32; 2]; BLOCK],
    ramp: &Ramp,
) -> Outcome {
    let mut outcome = Outcome::default();
    let len = asset.frames();
    let len_f = len as f64;
    let data = asset.samples();
    let mono = asset.channels() == 1;
    let delta = [
        (ramp.to[0] - ramp.from[0]) / BLOCK as f32,
        (ramp.to[1] - ramp.from[1]) / BLOCK as f32,
    ];
    let frame_at = |i: usize| -> [f32; 2] {
        if mono {
            let s = data[i];
            [s, s]
        } else {
            [data[2 * i], data[2 * i + 1]]
        }
    };
    if ramp.looped && len > 0 && *pos >= len_f {
        // A seek to the very end of a looped sound continues at the start.
        *pos %= len_f;
    }
    for (n, frame) in bus.iter_mut().enumerate() {
        let base = pos.floor();
        let frac = (*pos - base) as f32;
        let index = base as usize;
        if index >= len {
            // An empty asset, or a non-looped sound positioned at its end.
            outcome.ended = true;
            break;
        }
        let a = frame_at(index);
        let b = if index + 1 < len {
            frame_at(index + 1)
        } else if ramp.looped {
            frame_at(0)
        } else {
            [0.0; 2]
        };
        let mut s = [a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac];
        if ramp.spatial {
            let m = 0.5 * (s[0] + s[1]);
            let m = if ramp.lpf_a < 1.0 {
                lpf.tick(0, ramp.lpf_a, m)
            } else {
                m
            };
            s = [m, m];
        }
        let t = (n + 1) as f32;
        frame[0] += s[0] * (ramp.from[0] + delta[0] * t);
        frame[1] += s[1] * (ramp.from[1] + delta[1] * t);
        *pos += ramp.step;
        if *pos >= len_f {
            if ramp.looped && len > 0 {
                let wraps = (*pos / len_f).floor();
                *pos -= wraps * len_f;
                outcome.loops = outcome.loops.wrapping_add(wraps as u32);
            } else {
                *pos = len_f;
                outcome.ended = true;
                break;
            }
        }
    }
    outcome
}
